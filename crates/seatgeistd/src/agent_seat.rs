use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use libseatgeist::{
    AgentSeatRuntimeStatus, CoordinateSpace, Point, PointerButton, SeatgeistError, WindowInfo,
};
use seatgeist_backend::{TargetedInputBackend, TargetedInputContext, TargetedInputDelivery};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, oneshot};
use uuid::Uuid;

#[path = "agent_seat_plan.rs"]
mod plan;

pub(crate) const KWIN_AGENT_SEAT_BACKEND: &str = "kwin_agent_seat_v2";
const LEGACY_BACKEND: &str = "kwin_agent_seat_v1";
const ACTION_TIMEOUT: Duration = Duration::from_secs(3);
const ACTION_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(2);
const BACKEND_HEARTBEAT_TTL: Duration = Duration::from_secs(5);
const BUILD_ID_PREFIX: &str = "SEATGEIST_AGENT_SEAT_BUILD_ID:";

#[derive(Debug, Clone, Default)]
pub(crate) struct KwinAgentSeatBackend {
    registered: Arc<AtomicBool>,
    inner: Arc<Mutex<AgentSeatState>>,
    wake: Arc<Notify>,
    lanes: Option<crate::interaction::InteractionSessionStore>,
}

#[derive(Debug, Default)]
struct AgentSeatState {
    pending: VecDeque<PendingAction>,
    waiters: HashMap<Uuid, oneshot::Sender<AgentSeatActionResult>>,
    last_seen: Option<Instant>,
    caller: Option<String>,
    backend: Option<String>,
    build_id: Option<String>,
    last_delivery: Option<DeliveryValidation>,
}

#[derive(Debug)]
struct DeliveryValidation {
    observed_at: Instant,
    ok: bool,
    error_kind: Option<String>,
}

#[derive(Debug)]
struct PendingAction {
    id: Uuid,
    window_id: String,
    payload: String,
}

#[derive(Debug, Deserialize)]
struct AgentSeatActionResult {
    id: Uuid,
    ok: bool,
    backend: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ExpiringAction<'a, T> {
    #[serde(flatten)]
    action: &'a T,
    expires_at_unix_ms: u128,
}

fn action_payload<T: Serialize>(action: &T) -> Result<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(serde_json::to_string(&ExpiringAction {
        action,
        expires_at_unix_ms: (now + ACTION_TIMEOUT).as_millis(),
    })?)
}

#[derive(Debug, Serialize)]
struct KeyComboAction<'a> {
    id: Uuid,
    action: &'static str,
    lane_id: &'a str,
    window_id: &'a str,
    keycodes: &'a [u16],
}

#[derive(Debug, Serialize)]
struct KeySequenceAction<'a> {
    id: Uuid,
    action: &'static str,
    lane_id: &'a str,
    window_id: &'a str,
    chords: &'a [Vec<u16>],
}

#[derive(Debug, Serialize)]
struct PointerMoveAction<'a> {
    id: Uuid,
    action: &'static str,
    lane_id: &'a str,
    window_id: &'a str,
    x: f64,
    y: f64,
}

#[derive(Debug, Serialize)]
struct PointerClickAction<'a> {
    id: Uuid,
    action: &'static str,
    lane_id: &'a str,
    window_id: &'a str,
    x: f64,
    y: f64,
    button: u32,
    clicks: u8,
}

#[derive(Debug, Serialize)]
struct PointerDragAction<'a> {
    id: Uuid,
    action: &'static str,
    lane_id: &'a str,
    window_id: &'a str,
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
    button: u32,
}

#[derive(Debug, Serialize)]
struct PointerScrollAction<'a> {
    id: Uuid,
    action: &'static str,
    lane_id: &'a str,
    window_id: &'a str,
    vertical: i32,
    horizontal: i32,
}

impl KwinAgentSeatBackend {
    pub(crate) fn with_lanes(lanes: crate::interaction::InteractionSessionStore) -> Self {
        Self {
            lanes: Some(lanes),
            ..Self::default()
        }
    }

    pub(crate) fn register(&self, backend: &str, caller: &str) -> Result<()> {
        self.register_identified(backend, None, caller)
    }

    pub(crate) fn register_with_build_id(
        &self,
        backend: &str,
        build_id: &str,
        caller: &str,
    ) -> Result<()> {
        let build_id = build_id.strip_prefix(BUILD_ID_PREFIX).unwrap_or(build_id);
        if build_id.len() != 64 || !build_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("invalid KWin agent-seat build id");
        }
        self.register_identified(backend, Some(build_id.to_ascii_lowercase()), caller)
    }

    fn register_identified(
        &self,
        backend: &str,
        build_id: Option<String>,
        caller: &str,
    ) -> Result<()> {
        if backend != KWIN_AGENT_SEAT_BACKEND && backend != LEGACY_BACKEND {
            bail!("unsupported KWin agent-seat backend: {backend}");
        }
        if caller.trim().is_empty() {
            bail!("KWin agent-seat backend caller is missing");
        }
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin agent-seat queue lock is poisoned"))?;
        if state.caller.as_deref() != Some(caller)
            || state.backend.as_deref() != Some(backend)
            || state.build_id != build_id
        {
            // Never transfer old work to a replacement host or wire protocol.
            state.pending.clear();
            state.waiters.clear();
            state.last_delivery = None;
        }
        state.last_seen = Some(Instant::now());
        state.caller = Some(caller.to_string());
        state.backend = Some(backend.to_string());
        state.build_id = build_id;
        self.registered.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn runtime_status(&self) -> Option<AgentSeatRuntimeStatus> {
        let state = self.inner.lock().ok()?;
        let protocol = state.backend.clone()?;
        let heartbeat_age = state.last_seen?.elapsed();
        let (delivery_validation, last_delivery_age_ms, last_delivery_error_kind) = state
            .last_delivery
            .as_ref()
            .map(|delivery| {
                (
                    if delivery.ok {
                        "delivery_verified"
                    } else if delivery.error_kind.as_deref() == Some("physical_input_collision") {
                        "user_interrupted"
                    } else if delivery.error_kind.as_deref().is_some_and(target_refusal) {
                        "target_refused"
                    } else {
                        "delivery_failed"
                    },
                    Some(duration_millis(delivery.observed_at.elapsed())),
                    delivery.error_kind.clone(),
                )
            })
            .unwrap_or(("heartbeat_only", None, None));
        Some(AgentSeatRuntimeStatus {
            protocol,
            build_id: state.build_id.clone(),
            heartbeat_fresh: heartbeat_age <= BACKEND_HEARTBEAT_TTL,
            heartbeat_age_ms: duration_millis(heartbeat_age),
            delivery_validation: delivery_validation.to_string(),
            last_delivery_age_ms,
            last_delivery_error_kind,
        })
    }

    pub(crate) fn take(&self, caller: &str) -> Result<String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin agent-seat queue lock is poisoned"))?;
        require_registered_caller(&state, caller)?;
        state.last_seen = Some(Instant::now());
        Ok(state
            .pending
            .pop_front()
            .map_or_else(String::new, |action| action.payload))
    }

    pub(crate) async fn take_wait(&self, caller: &str) -> Result<String> {
        let notified = self.wake.notified();
        let mut payload = self.take(caller)?;
        if payload.is_empty() {
            let _ = tokio::time::timeout(ACTION_LONG_POLL_TIMEOUT, notified).await;
            payload = self.take(caller)?;
        }
        if !payload.is_empty() {
            if let Some(lanes) = &self.lanes {
                // Snapshot at dequeue, not submission: closed/expired owners
                // cannot retain native seats through stale queued actions.
                let mut value: serde_json::Value = serde_json::from_str(&payload)?;
                value["active_lane_ids"] = serde_json::to_value(lanes.active_lane_ids().await)?;
                payload = serde_json::to_string(&value)?;
            }
        }
        Ok(payload)
    }

    pub(crate) fn complete(&self, caller: &str, payload: &str) -> Result<()> {
        let result: AgentSeatActionResult =
            serde_json::from_str(payload).context("parse KWin agent-seat completion")?;
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin agent-seat queue lock is poisoned"))?;
        require_registered_caller(&state, caller)?;
        if result
            .backend
            .as_ref()
            .is_some_and(|backend| Some(backend) != state.backend.as_ref())
        {
            bail!("KWin agent-seat completion reported unexpected backend");
        }
        let sender = state
            .waiters
            .remove(&result.id)
            .ok_or_else(|| anyhow::anyhow!("unknown KWin agent-seat action id"))?;
        if sender.is_closed() {
            bail!("KWin agent-seat action receiver closed");
        }
        let delivery = DeliveryValidation {
            observed_at: Instant::now(),
            ok: result.ok,
            error_kind: result
                .error
                .as_deref()
                .and_then(agent_seat_error_kind)
                .map(str::to_string),
        };
        // Late, unknown or malformed acknowledgements must not turn readiness
        // green. Only a completion accepted by a live request counts.
        sender
            .send(result)
            .map_err(|_| anyhow::anyhow!("KWin agent-seat action receiver closed"))?;
        state.last_delivery = Some(delivery);
        Ok(())
    }

    async fn submit<T: Serialize>(
        &self,
        id: Uuid,
        window_id: &str,
        action: &T,
    ) -> seatgeist_backend::Result<TargetedInputDelivery> {
        if !self.ready() {
            return Err(backend_error(anyhow::anyhow!(
                "KWin agent-seat plugin is not registered; install and enable seatgeistagentseat or select another input backend"
            )));
        }
        let value = serde_json::to_value(action).map_err(|err| backend_error(err.into()))?;
        // Plan and validate even when temporarily talking to the old plugin.
        let events = plan::plan(value.clone()).map_err(backend_error)?;
        let (sender, receiver) = oneshot::channel();
        let expected_backend = {
            let mut state = self.inner.lock().map_err(|_| {
                backend_error(anyhow::anyhow!("KWin agent-seat queue lock is poisoned"))
            })?;
            let backend = state
                .backend
                .clone()
                .ok_or_else(|| backend_error(anyhow::anyhow!("seat host is not registered")))?;
            let payload = if backend == KWIN_AGENT_SEAT_BACKEND {
                action_payload(&serde_json::json!({
                    "id": id, "action": "events", "protocol": plan::HOST_PROTOCOL,
                    "lane_id": value["lane_id"], "window_id": window_id, "events": events,
                }))
            } else {
                action_payload(action)
            }
            .map_err(backend_error)?;
            state.pending.push_back(PendingAction {
                id,
                window_id: window_id.to_string(),
                payload,
            });
            state.waiters.insert(id, sender);
            backend
        };
        self.wake.notify_one();
        let result = match tokio::time::timeout(ACTION_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.remove(id);
                return Err(backend_error(anyhow::anyhow!(
                    "KWin agent-seat completion channel closed"
                )));
            }
            Err(_) => {
                self.remove(id);
                if let Ok(mut state) = self.inner.lock() {
                    state.last_delivery = Some(DeliveryValidation {
                        observed_at: Instant::now(),
                        ok: false,
                        error_kind: Some("delivery_timeout".to_string()),
                    });
                }
                return Err(backend_error(anyhow::anyhow!(
                    "KWin agent-seat plugin did not complete action {id} in time"
                )));
            }
        };
        if !result.ok {
            return Err(backend_error(anyhow::anyhow!(
                "KWin agent-seat action failed: {}",
                result.error.as_deref().unwrap_or("unknown plugin error")
            )));
        }
        let backend = result
            .backend
            .filter(|backend| !backend.trim().is_empty())
            .unwrap_or_else(|| expected_backend.clone());
        if backend != expected_backend {
            return Err(backend_error(anyhow::anyhow!(
                "KWin agent-seat completion reported unexpected backend {backend}"
            )));
        }
        Ok(TargetedInputDelivery {
            action_id: id,
            backend,
        })
    }

    fn remove(&self, id: Uuid) {
        if let Ok(mut state) = self.inner.lock() {
            state.pending.retain(|action| action.id != id);
            state.waiters.remove(&id);
        }
    }

    pub(crate) fn cancel_pending_for_target(&self, window_id: &str) -> usize {
        let Ok(mut state) = self.inner.lock() else {
            return 0;
        };
        let cancelled = state
            .pending
            .iter()
            .filter(|action| action.window_id == window_id)
            .map(|action| action.id)
            .collect::<Vec<_>>();
        state.pending.retain(|action| action.window_id != window_id);
        for id in &cancelled {
            if let Some(sender) = state.waiters.remove(id) {
                let _ = sender.send(AgentSeatActionResult {
                    id: *id,
                    ok: false,
                    backend: Some(KWIN_AGENT_SEAT_BACKEND.to_string()),
                    error: Some(
                        "agent target received physical user input before delivery".to_string(),
                    ),
                });
            }
        }
        cancelled.len()
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn agent_seat_error_kind(error: &str) -> Option<&'static str> {
    let error = error.to_ascii_lowercase();
    if error.contains("firefox shared-process") {
        Some("firefox_shared_process")
    } else if error.contains("not a native wayland") {
        Some("native_wayland_required")
    } else if error.contains("target did not bind") {
        Some("client_binding_unavailable")
    } else if error.contains("bind") || error.contains("input device") {
        Some("seat_binding_failed")
    } else if error.contains("user input") {
        Some("physical_input_collision")
    } else if error.contains("target") || error.contains("window") || error.contains("surface") {
        Some("target_lost")
    } else if error.contains("expired") || error.contains("deadline") {
        Some("delivery_timeout")
    } else if error.contains("lane") || error.contains("quota") {
        Some("lane_unavailable")
    } else if error.trim().is_empty() {
        None
    } else {
        Some("backend_rejected")
    }
}

fn target_refusal(kind: &str) -> bool {
    matches!(
        kind,
        "firefox_shared_process"
            | "native_wayland_required"
            | "client_binding_unavailable"
            | "target_lost"
            | "lane_unavailable"
    )
}

fn require_registered_caller(state: &AgentSeatState, caller: &str) -> Result<()> {
    if state.caller.as_deref() == Some(caller) {
        return Ok(());
    }
    bail!("KWin agent-seat queue caller is not the registered compositor connection")
}

#[async_trait]
impl TargetedInputBackend for KwinAgentSeatBackend {
    fn backend_name(&self) -> &'static str {
        if self
            .inner
            .lock()
            .ok()
            .is_some_and(|state| state.backend.as_deref() == Some(LEGACY_BACKEND))
        {
            LEGACY_BACKEND
        } else {
            KWIN_AGENT_SEAT_BACKEND
        }
    }

    fn ready(&self) -> bool {
        self.registered.load(Ordering::Acquire)
            && self
                .inner
                .lock()
                .ok()
                .and_then(|state| state.last_seen)
                .is_some_and(|last_seen| last_seen.elapsed() <= BACKEND_HEARTBEAT_TTL)
    }

    async fn key_combo(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        keycodes: &[u16],
    ) -> seatgeist_backend::Result<TargetedInputDelivery> {
        validate_context(context)?;
        if keycodes.is_empty() || keycodes.len() > 8 {
            return Err(backend_error(anyhow::anyhow!(
                "agent-seat key combo must contain between 1 and 8 keys"
            )));
        }
        let id = Uuid::new_v4();
        self.submit(
            id,
            &target.id,
            &KeyComboAction {
                id,
                action: "key_combo",
                lane_id: &context.lane_id,
                window_id: &target.id,
                keycodes,
            },
        )
        .await
    }

    async fn key_sequence(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        chords: &[Vec<u16>],
    ) -> seatgeist_backend::Result<TargetedInputDelivery> {
        validate_context(context)?;
        if chords.is_empty()
            || chords.len() > 8192
            || chords
                .iter()
                .any(|chord| chord.is_empty() || chord.len() > 2)
        {
            return Err(backend_error(anyhow::anyhow!(
                "agent-seat key sequence must contain 1..8192 chords of 1..2 keys"
            )));
        }
        let id = Uuid::new_v4();
        self.submit(
            id,
            &target.id,
            &KeySequenceAction {
                id,
                action: "key_sequence",
                lane_id: &context.lane_id,
                window_id: &target.id,
                chords,
            },
        )
        .await
    }

    async fn move_pointer(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        point: Point,
    ) -> seatgeist_backend::Result<TargetedInputDelivery> {
        validate_context(context)?;
        validate_window_local(point)?;
        let id = Uuid::new_v4();
        self.submit(
            id,
            &target.id,
            &PointerMoveAction {
                id,
                action: "pointer_move",
                lane_id: &context.lane_id,
                window_id: &target.id,
                x: point.x,
                y: point.y,
            },
        )
        .await
    }

    async fn click(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        point: Point,
        button: PointerButton,
        clicks: u8,
    ) -> seatgeist_backend::Result<TargetedInputDelivery> {
        validate_context(context)?;
        validate_window_local(point)?;
        let id = Uuid::new_v4();
        self.submit(
            id,
            &target.id,
            &PointerClickAction {
                id,
                action: "pointer_click",
                lane_id: &context.lane_id,
                window_id: &target.id,
                x: point.x,
                y: point.y,
                button: button_code(button),
                clicks,
            },
        )
        .await
    }

    async fn drag(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        from: Point,
        to: Point,
        button: PointerButton,
    ) -> seatgeist_backend::Result<TargetedInputDelivery> {
        validate_context(context)?;
        validate_window_local(from)?;
        validate_window_local(to)?;
        let id = Uuid::new_v4();
        self.submit(
            id,
            &target.id,
            &PointerDragAction {
                id,
                action: "pointer_drag",
                lane_id: &context.lane_id,
                window_id: &target.id,
                from_x: from.x,
                from_y: from.y,
                to_x: to.x,
                to_y: to.y,
                button: button_code(button),
            },
        )
        .await
    }

    async fn scroll(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        vertical: i32,
        horizontal: i32,
    ) -> seatgeist_backend::Result<TargetedInputDelivery> {
        validate_context(context)?;
        let id = Uuid::new_v4();
        self.submit(
            id,
            &target.id,
            &PointerScrollAction {
                id,
                action: "pointer_scroll",
                lane_id: &context.lane_id,
                window_id: &target.id,
                vertical,
                horizontal,
            },
        )
        .await
    }
}

fn validate_context(context: &TargetedInputContext) -> seatgeist_backend::Result<()> {
    Uuid::parse_str(&context.lane_id).map_err(|_| {
        SeatgeistError::InvalidRequest("agent-seat lane id must be an opaque UUID".to_string())
    })?;
    Ok(())
}

fn validate_window_local(point: Point) -> seatgeist_backend::Result<()> {
    if point.space != CoordinateSpace::WindowLocal {
        return Err(SeatgeistError::InvalidRequest(
            "kwin_agent_seat requires window_local pointer coordinates".to_string(),
        ));
    }
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(SeatgeistError::InvalidRequest(
            "agent-seat pointer coordinates must be finite".to_string(),
        ));
    }
    Ok(())
}

fn button_code(button: PointerButton) -> u32 {
    match button {
        PointerButton::Left => 0x110,
        PointerButton::Right => 0x111,
        PointerButton::Middle => 0x112,
    }
}

fn backend_error(error: anyhow::Error) -> SeatgeistError {
    SeatgeistError::BackendUnavailable(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_actions_carry_the_original_delivery_deadline() {
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let value: serde_json::Value = serde_json::from_str(
            &action_payload(&serde_json::json!({"action": "key_combo", "keycodes": [8]})).unwrap(),
        )
        .unwrap();
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let deadline = u128::from(value["expires_at_unix_ms"].as_u64().unwrap());
        assert!(deadline >= before + ACTION_TIMEOUT.as_millis());
        assert!(deadline <= after + ACTION_TIMEOUT.as_millis());
        assert_eq!(value["action"], "key_combo");
        assert_eq!(value["keycodes"], serde_json::json!([8]));
    }

    fn target() -> WindowInfo {
        WindowInfo {
            id: "45837f40-43a8-4be5-b9d7-50d2ff8f79b3".to_string(),
            app_id: Some("org.example.Editor".to_string()),
            title: "Editor".to_string(),
            pid: Some(42),
            monitor_id: None,
            geometry: None,
        }
    }

    fn context() -> TargetedInputContext {
        TargetedInputContext {
            lane_id: "0194e9f8-1910-7e24-b5bd-52d184b6427f".to_string(),
        }
    }

    #[tokio::test]
    async fn key_combo_round_trips_through_plugin_pull_queue() {
        let backend = KwinAgentSeatBackend::default();
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "kwin-owner")
            .expect("backend registers");
        assert!(
            backend
                .take("intruder")
                .expect_err("another D-Bus caller cannot poll")
                .to_string()
                .contains("not the registered compositor")
        );
        let sender = backend.clone();
        let action = tokio::spawn(async move {
            sender
                .key_combo(&context(), &target(), &[29, 30])
                .await
                .expect("action completes")
        });

        tokio::task::yield_now().await;
        let payload = backend
            .take_wait("kwin-owner")
            .await
            .expect("long-poll queue wakes for the action");
        let request: serde_json::Value = serde_json::from_str(&payload).expect("request is JSON");
        assert_eq!(request["action"], "events");
        assert_eq!(request["protocol"], 1);
        assert_eq!(request["lane_id"], context().lane_id);
        assert_eq!(request["window_id"], target().id);
        assert_eq!(
            request["events"],
            serde_json::to_value(
                plan::plan(serde_json::json!({"action":"key_combo","keycodes":[29,30]})).unwrap()
            )
            .unwrap()
        );
        assert!(
            backend
                .complete(
                    "intruder",
                    &serde_json::json!({
                        "id": request["id"],
                        "ok": true,
                        "backend": KWIN_AGENT_SEAT_BACKEND
                    })
                    .to_string(),
                )
                .expect_err("another D-Bus caller cannot complete")
                .to_string()
                .contains("not the registered compositor")
        );
        backend
            .complete(
                "kwin-owner",
                &serde_json::json!({
                    "id": request["id"],
                    "ok": true,
                    "backend": KWIN_AGENT_SEAT_BACKEND
                })
                .to_string(),
            )
            .expect("completion is accepted");
        let delivery = action.await.expect("task joins");
        assert_eq!(delivery.backend, KWIN_AGENT_SEAT_BACKEND);
        let status = backend.runtime_status().expect("runtime status exists");
        assert_eq!(status.protocol, KWIN_AGENT_SEAT_BACKEND);
        assert_eq!(status.delivery_validation, "delivery_verified");
        assert!(status.heartbeat_fresh);
    }

    #[test]
    fn identified_registration_reports_normalized_build_and_rejects_bad_ids() {
        let backend = KwinAgentSeatBackend::default();
        let build_id = "A".repeat(64);
        backend
            .register_with_build_id(
                KWIN_AGENT_SEAT_BACKEND,
                &format!("{BUILD_ID_PREFIX}{build_id}"),
                "kwin-owner",
            )
            .expect("identified backend registers");
        let status = backend.runtime_status().expect("runtime status exists");
        assert_eq!(
            status.build_id.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(status.delivery_validation, "heartbeat_only");
        assert!(
            backend
                .register_with_build_id(KWIN_AGENT_SEAT_BACKEND, "not-a-build", "kwin-owner")
                .is_err()
        );
    }

    #[test]
    fn closed_receiver_and_wrong_backend_cannot_verify_delivery() {
        let backend = KwinAgentSeatBackend::default();
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "kwin-owner")
            .unwrap();
        let id = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        backend.inner.lock().unwrap().waiters.insert(id, sender);
        assert!(
            backend
                .complete(
                    "kwin-owner",
                    &serde_json::json!({
                        "id": id, "ok": true, "backend": "wrong-backend",
                    })
                    .to_string()
                )
                .is_err()
        );
        assert_eq!(
            backend.runtime_status().unwrap().delivery_validation,
            "heartbeat_only"
        );
        drop(receiver);
        assert!(
            backend
                .complete(
                    "kwin-owner",
                    &serde_json::json!({
                        "id": id, "ok": true, "backend": KWIN_AGENT_SEAT_BACKEND,
                    })
                    .to_string()
                )
                .is_err()
        );
        assert_eq!(
            backend.runtime_status().unwrap().delivery_validation,
            "heartbeat_only"
        );
    }

    #[test]
    fn target_refusals_are_not_backend_failures_and_unknown_acks_do_not_change_health() {
        let backend = KwinAgentSeatBackend::default();
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "kwin-owner")
            .unwrap();
        for (error, expected) in [
            (
                "Firefox shared-process window focus is not seat-isolated",
                "target_refused",
            ),
            (
                "target is not a native Wayland client window",
                "target_refused",
            ),
            (
                "target did not bind the agent-seat input device before the deadline",
                "target_refused",
            ),
            (
                "agent target received user input before delivery",
                "user_interrupted",
            ),
            (
                "agent-seat action expired before delivery",
                "delivery_failed",
            ),
            (
                "agent-seat binding observation is unavailable",
                "delivery_failed",
            ),
        ] {
            let id = Uuid::new_v4();
            let (sender, receiver) = oneshot::channel();
            backend.inner.lock().unwrap().waiters.insert(id, sender);
            backend
                .complete(
                    "kwin-owner",
                    &serde_json::json!({
                        "id": id, "ok": false, "backend": KWIN_AGENT_SEAT_BACKEND, "error": error,
                    })
                    .to_string(),
                )
                .unwrap();
            assert!(!receiver.blocking_recv().unwrap().ok);
            assert_eq!(
                backend.runtime_status().unwrap().delivery_validation,
                expected
            );
        }
        assert!(
            backend
                .complete(
                    "kwin-owner",
                    &serde_json::json!({
                        "id": Uuid::new_v4(), "ok": true, "backend": KWIN_AGENT_SEAT_BACKEND,
                    })
                    .to_string()
                )
                .is_err()
        );
        assert_eq!(
            backend.runtime_status().unwrap().delivery_validation,
            "delivery_failed"
        );
    }

    #[tokio::test]
    async fn type_text_is_lowered_to_us_key_chords_without_text_in_queue() {
        let backend = KwinAgentSeatBackend::default();
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "kwin-owner")
            .expect("backend registers");
        let sender = backend.clone();
        let action = tokio::spawn(async move {
            crate::input_actions::agent_type_text(
                libseatgeist::TypeTextRequest {
                    text: "aA!".to_string(),
                    guard: None,
                    session_id: Some("capture-1".to_string()),
                },
                &context(),
                &target(),
                &sender,
            )
            .await
            .expect("text action completes")
        });

        tokio::task::yield_now().await;
        let payload = backend.take("kwin-owner").expect("queue can be read");
        let request: serde_json::Value = serde_json::from_str(&payload).expect("request is JSON");
        assert_eq!(request["action"], "events");
        assert_eq!(
            request["events"],
            serde_json::to_value(
                plan::plan(serde_json::json!({"action":"key_sequence",
                "chords":[[30], [42,30], [42,2]]}))
                .unwrap()
            )
            .unwrap()
        );
        assert!(!payload.contains("aA!"));
        backend
            .complete(
                "kwin-owner",
                &serde_json::json!({
                    "id": request["id"],
                    "ok": true,
                    "backend": KWIN_AGENT_SEAT_BACKEND
                })
                .to_string(),
            )
            .expect("completion is accepted");
        let result = action.await.expect("task joins");
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|message| message.contains("length=3"))
        );
    }

    #[tokio::test]
    async fn legacy_host_is_supported_and_protocol_replacement_drops_old_work() {
        let backend = KwinAgentSeatBackend::default();
        backend.register(LEGACY_BACKEND, "kwin-owner").unwrap();
        assert_eq!(backend.backend_name(), LEGACY_BACKEND);
        let sender = backend.clone();
        let action =
            tokio::spawn(async move { sender.key_combo(&context(), &target(), &[30]).await });
        let payload = backend.take_wait("kwin-owner").await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(request["action"], "key_combo");
        assert_eq!(request["keycodes"], serde_json::json!([30]));
        backend
            .complete(
                "kwin-owner",
                &serde_json::json!({"id":request["id"],"ok":true,
            "backend":LEGACY_BACKEND})
                .to_string(),
            )
            .unwrap();
        assert_eq!(action.await.unwrap().unwrap().backend, LEGACY_BACKEND);

        let sender = backend.clone();
        let old = tokio::spawn(async move { sender.key_combo(&context(), &target(), &[30]).await });
        tokio::task::yield_now().await;
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "replacement-owner")
            .unwrap();
        assert!(
            old.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("channel closed")
        );
        assert!(backend.take("replacement-owner").unwrap().is_empty());
        assert!(backend.take("kwin-owner").is_err());
    }

    #[tokio::test]
    async fn dequeue_roster_preserves_other_owners_and_excludes_closed_lanes() {
        let lanes = crate::interaction::InteractionSessionStore::default();
        lanes
            .bind(
                "one".into(),
                &target(),
                crate::session_owner::SessionOwner::test_process(101),
            )
            .await
            .unwrap();
        let mut second = target();
        second.id = Uuid::new_v4().to_string();
        lanes
            .bind(
                "two".into(),
                &second,
                crate::session_owner::SessionOwner::test_process(102),
            )
            .await
            .unwrap();
        let first_lane = lanes.resolve("one", &[target()]).await.unwrap().lane_id;
        let second_lane = lanes.resolve("two", &[second]).await.unwrap().lane_id;
        let backend = KwinAgentSeatBackend::with_lanes(lanes.clone());
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "kwin-owner")
            .unwrap();
        let sender = backend.clone();
        let queued_lane = first_lane.clone();
        let action = tokio::spawn(async move {
            sender
                .key_combo(
                    &TargetedInputContext {
                        lane_id: queued_lane,
                    },
                    &target(),
                    &[30],
                )
                .await
        });
        tokio::task::yield_now().await;
        lanes.clear("one").await.unwrap();
        let request: serde_json::Value =
            serde_json::from_str(&backend.take_wait("kwin-owner").await.unwrap()).unwrap();
        assert_eq!(request["lane_id"], first_lane);
        assert_eq!(request["active_lane_ids"], serde_json::json!([second_lane]));
        backend
            .complete(
                "kwin-owner",
                &serde_json::json!({"id":request["id"],"ok":false,
            "error":"agent lane lease is inactive"})
                .to_string(),
            )
            .unwrap();
        assert!(action.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn actions_fail_closed_until_plugin_registers() {
        let backend = KwinAgentSeatBackend::default();
        let error = backend
            .key_combo(&context(), &target(), &[28])
            .await
            .expect_err("unregistered backend fails");
        assert!(error.to_string().contains("plugin is not registered"));
        assert!(
            backend
                .take("unregistered")
                .expect_err("unregistered caller fails")
                .to_string()
                .contains("not the registered compositor")
        );
    }

    #[tokio::test]
    async fn physical_activity_cancels_only_queued_actions_for_that_window() {
        let backend = KwinAgentSeatBackend::default();
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "kwin-owner")
            .expect("backend registers");
        let sender = backend.clone();
        let action = tokio::spawn(async move {
            sender
                .key_combo(&context(), &target(), &[28])
                .await
                .expect_err("same-window user activity cancels queued input")
        });
        tokio::task::yield_now().await;
        assert_eq!(backend.cancel_pending_for_target("another-window"), 0);
        assert_eq!(backend.cancel_pending_for_target(&target().id), 1);
        let error = action.await.expect("task joins");
        assert!(
            error
                .to_string()
                .contains("physical user input before delivery")
        );
        assert!(
            backend
                .take("kwin-owner")
                .expect("queue remains readable")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn pointer_actions_require_window_local_coordinates() {
        let backend = KwinAgentSeatBackend::default();
        backend
            .register(KWIN_AGENT_SEAT_BACKEND, "kwin-owner")
            .expect("backend registers");
        let error = backend
            .move_pointer(
                &context(),
                &target(),
                Point {
                    x: 10.0,
                    y: 20.0,
                    space: CoordinateSpace::LogicalPixel,
                },
            )
            .await
            .expect_err("global coordinate fails");
        assert!(error.to_string().contains("requires window_local"));
    }
}
