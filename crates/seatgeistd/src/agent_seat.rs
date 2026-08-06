use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use libseatgeist::{CoordinateSpace, Point, PointerButton, SeatgeistError, WindowInfo};
use seatgeist_backend::{TargetedInputBackend, TargetedInputContext, TargetedInputDelivery};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, oneshot};
use uuid::Uuid;

pub(crate) const KWIN_AGENT_SEAT_BACKEND: &str = "kwin_agent_seat_v1";
const ACTION_TIMEOUT: Duration = Duration::from_secs(3);
const ACTION_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(2);
const BACKEND_HEARTBEAT_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub(crate) struct KwinAgentSeatBackend {
    registered: Arc<AtomicBool>,
    inner: Arc<Mutex<AgentSeatState>>,
    wake: Arc<Notify>,
}

#[derive(Debug, Default)]
struct AgentSeatState {
    pending: VecDeque<PendingAction>,
    waiters: HashMap<Uuid, oneshot::Sender<AgentSeatActionResult>>,
    last_seen: Option<Instant>,
    caller: Option<String>,
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
    pub(crate) fn register(&self, backend: &str, caller: &str) -> Result<()> {
        if backend != KWIN_AGENT_SEAT_BACKEND {
            bail!("unsupported KWin agent-seat backend: {backend}");
        }
        if caller.trim().is_empty() {
            bail!("KWin agent-seat backend caller is missing");
        }
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin agent-seat queue lock is poisoned"))?;
        state.last_seen = Some(Instant::now());
        state.caller = Some(caller.to_string());
        self.registered.store(true, Ordering::Release);
        Ok(())
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
        let payload = self.take(caller)?;
        if !payload.is_empty() {
            return Ok(payload);
        }
        let _ = tokio::time::timeout(ACTION_LONG_POLL_TIMEOUT, notified).await;
        self.take(caller)
    }

    pub(crate) fn complete(&self, caller: &str, payload: &str) -> Result<()> {
        let result: AgentSeatActionResult =
            serde_json::from_str(payload).context("parse KWin agent-seat completion")?;
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin agent-seat queue lock is poisoned"))?;
        require_registered_caller(&state, caller)?;
        let sender = state
            .waiters
            .remove(&result.id)
            .ok_or_else(|| anyhow::anyhow!("unknown KWin agent-seat action id"))?;
        sender
            .send(result)
            .map_err(|_| anyhow::anyhow!("KWin agent-seat action receiver closed"))
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
        let payload = serde_json::to_string(action).map_err(|error| backend_error(error.into()))?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self.inner.lock().map_err(|_| {
                backend_error(anyhow::anyhow!("KWin agent-seat queue lock is poisoned"))
            })?;
            state.pending.push_back(PendingAction {
                id,
                window_id: window_id.to_string(),
                payload,
            });
            state.waiters.insert(id, sender);
        }
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
            .unwrap_or_else(|| KWIN_AGENT_SEAT_BACKEND.to_string());
        if backend != KWIN_AGENT_SEAT_BACKEND {
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

fn require_registered_caller(state: &AgentSeatState, caller: &str) -> Result<()> {
    if state.caller.as_deref() == Some(caller) {
        return Ok(());
    }
    bail!("KWin agent-seat queue caller is not the registered compositor connection")
}

#[async_trait]
impl TargetedInputBackend for KwinAgentSeatBackend {
    fn backend_name(&self) -> &'static str {
        KWIN_AGENT_SEAT_BACKEND
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
        assert_eq!(request["action"], "key_combo");
        assert_eq!(request["lane_id"], context().lane_id);
        assert_eq!(request["window_id"], target().id);
        assert_eq!(request["keycodes"], serde_json::json!([29, 30]));
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
        assert_eq!(request["action"], "key_sequence");
        assert_eq!(
            request["chords"],
            serde_json::json!([[30], [42, 30], [42, 2]])
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
