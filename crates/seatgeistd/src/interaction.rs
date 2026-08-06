use std::{
    collections::{HashMap, HashSet},
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use libseatgeist::{
    ActionConfirmation, ActionResult, ActionSettleBackend, ActionSettleCondition,
    ActionSettleResult, DaemonRequest, FocusWindowRequest, WindowInfo,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::session_owner::SessionOwner;

const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const DEFAULT_LEASE_DEADLINE: Duration = Duration::from_millis(1_000);
const MAX_AGENT_LANES: usize = 4;
const AGENT_TARGET_QUIET_TIME: Duration = Duration::from_millis(350);

pub(crate) trait FocusBackend: fmt::Debug + Send + Sync {
    fn name(&self) -> &'static str;
    fn focus(&self, window_id: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct KwinFocusBackend;

impl FocusBackend for KwinFocusBackend {
    fn name(&self) -> &'static str {
        "kwin"
    }

    fn focus(&self, window_id: &str) -> Result<()> {
        seatgeist_kwin::focus_window(window_id).map_err(anyhow::Error::msg)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PinnedTarget {
    pub session_id: String,
    pub lane_id: String,
    pub window: WindowInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractionStatus {
    pub bound: bool,
    pub session_id: Option<String>,
    pub window_id: Option<String>,
    pub app_id: Option<String>,
    pub pid: Option<u32>,
    pub expires_in_ms: Option<u64>,
    pub owner_tool: Option<String>,
    pub owner_pid: Option<u32>,
    pub owner_scope: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct InteractionSessionStore {
    sessions: Arc<Mutex<HashMap<String, InteractionSession>>>,
    seat_lease: Arc<Mutex<()>>,
}

impl fmt::Debug for InteractionSessionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InteractionSessionStore(..)")
    }
}

struct InteractionSession {
    id: String,
    window_id: String,
    app_id: Option<String>,
    pid: Option<u32>,
    owner: SessionOwner,
    lane_id: String,
    expires_at: tokio::time::Instant,
}

impl InteractionSessionStore {
    pub async fn retain_active_capture_sessions(&self, active_session_ids: &HashSet<String>) {
        self.sessions
            .lock()
            .await
            .retain(|session_id, _| active_session_ids.contains(session_id));
    }

    pub async fn bind(
        &self,
        session_id: String,
        window: &WindowInfo,
        owner: SessionOwner,
    ) -> Result<()> {
        let app_id = window
            .app_id
            .as_deref()
            .map(str::trim)
            .filter(|app| !app.is_empty())
            .ok_or_else(|| anyhow::anyhow!("interaction target has no app id"))?;
        let pid = window
            .pid
            .ok_or_else(|| anyhow::anyhow!("interaction target has no process id"))?;
        let mut sessions = self.sessions.lock().await;
        let now = tokio::time::Instant::now();
        sessions.retain(|_, session| session.expires_at > now);
        let owner_identity = owner.identity();
        if sessions.values().any(|session| {
            session.window_id == window.id && session.owner.identity() != owner_identity
        }) {
            bail!("agent target in use: another agent owns the window interaction lease");
        }
        let lane_id = sessions
            .values()
            .find(|session| session.owner.identity() == owner_identity)
            .map(|session| session.lane_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let active_owners = sessions
            .values()
            .map(|session| session.owner.identity())
            .collect::<HashSet<_>>();
        if !active_owners.contains(&owner_identity) && active_owners.len() >= MAX_AGENT_LANES {
            bail!(
                "agent lane quota reached: at most {MAX_AGENT_LANES} agent owners may hold interaction sessions"
            );
        }
        sessions.insert(
            session_id.clone(),
            InteractionSession {
                id: session_id,
                window_id: window.id.clone(),
                app_id: Some(app_id.to_string()),
                pid: Some(pid),
                owner,
                lane_id,
                expires_at: tokio::time::Instant::now() + DEFAULT_SESSION_TTL,
            },
        );
        Ok(())
    }

    pub async fn clear(&self, session_id: &str) -> Result<()> {
        if self.sessions.lock().await.remove(session_id).is_none() {
            bail!("interaction session id is not active");
        }
        Ok(())
    }

    pub async fn clear_if_present(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }

    pub async fn renew(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let active = sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("interaction target lost: no bound sticky session"))?;
        active.expires_at = tokio::time::Instant::now() + DEFAULT_SESSION_TTL;
        Ok(())
    }

    pub async fn resolve(&self, session_id: &str, windows: &[WindowInfo]) -> Result<PinnedTarget> {
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("interaction target lost: no bound sticky session"))?;
        if tokio::time::Instant::now() >= session.expires_at {
            bail!("interaction target lost: sticky session expired");
        }
        let window = windows
            .iter()
            .find(|window| window.id == session.window_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interaction target lost: pinned window closed"))?;
        if window.app_id != session.app_id || window.pid != session.pid {
            bail!("interaction target lost: pinned window identity changed");
        }
        Ok(PinnedTarget {
            session_id: session.id.clone(),
            lane_id: session.lane_id.clone(),
            window,
        })
    }

    pub async fn status(&self, session_id: &str) -> InteractionStatus {
        let sessions = self.sessions.lock().await;
        let Some(session) = sessions.get(session_id) else {
            return InteractionStatus {
                bound: false,
                session_id: None,
                window_id: None,
                app_id: None,
                pid: None,
                expires_in_ms: None,
                owner_tool: None,
                owner_pid: None,
                owner_scope: None,
            };
        };
        let remaining = session
            .expires_at
            .saturating_duration_since(tokio::time::Instant::now());
        InteractionStatus {
            bound: !remaining.is_zero(),
            session_id: Some(session.id.clone()),
            window_id: Some(session.window_id.clone()),
            app_id: session.app_id.clone(),
            pid: session.pid,
            expires_in_ms: Some(remaining.as_millis().min(u128::from(u64::MAX)) as u64),
            owner_tool: session.owner.tool().map(str::to_string),
            owner_pid: Some(session.owner.pid()),
            owner_scope: Some(session.owner.scope().as_str().to_string()),
        }
    }

    pub async fn clear_if_target_invalid(
        &self,
        session_id: &str,
        windows: &[WindowInfo],
    ) -> Result<bool> {
        if self.resolve(session_id, windows).await.is_ok() {
            return Ok(false);
        }
        self.clear(session_id).await?;
        Ok(true)
    }

    pub async fn acquire_seat_lease(&self) -> Result<OwnedMutexGuard<()>> {
        tokio::time::timeout(DEFAULT_LEASE_DEADLINE, self.seat_lease.clone().lock_owned())
            .await
            .map_err(|_| {
                anyhow::anyhow!("focus lease conflict: seat action mutex deadline expired")
            })
    }
}

pub(crate) async fn execute_raw_action<F, Fut>(
    runtime: &super::DaemonRuntime,
    session_id: Option<&str>,
    action: F,
) -> Result<ActionResult>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ActionResult>>,
{
    let Some(session_id) = session_id else {
        return action().await;
    };
    require_live_capture_session(runtime, session_id).await?;
    let windows = read_windows(runtime).await?;
    let initial = runtime
        .interaction_session_store
        .resolve(session_id, &windows)
        .await?;
    super::enforce_app_policy_for_app(
        &runtime.app_policy,
        initial.window.app_id.as_deref(),
        "pinned interaction target",
    )?;
    let _seat_lease = runtime
        .interaction_session_store
        .acquire_seat_lease()
        .await?;
    require_live_capture_session(runtime, session_id).await?;

    let windows = read_windows(runtime).await?;
    let target = runtime
        .interaction_session_store
        .resolve(session_id, &windows)
        .await?;
    super::enforce_app_policy_for_app(
        &runtime.app_policy,
        target.window.app_id.as_deref(),
        "pinned interaction target",
    )?;
    let focus_request = DaemonRequest::FocusWindow(FocusWindowRequest {
        window_id: target.window.id.clone(),
        guard: None,
    });
    super::enforce_panic_stop(&runtime.panic_stop, &focus_request)?;
    super::enforce_human_input_pause(
        &runtime.safety_settings,
        &runtime.activity_tracker,
        &focus_request,
    )?;
    let previous_window = read_active_window(runtime).await?;
    let activity_snapshot = runtime.activity_tracker.snapshot();

    let lease_id = Uuid::new_v4();
    let already_active = previous_window
        .as_ref()
        .is_some_and(|window| window.id == target.window.id);
    if !already_active {
        let policy_result = super::enforce_policy_with_approvals(
            &runtime.policy,
            &runtime.approval_store,
            &focus_request,
        );
        runtime.journal.record_focus_lease_step(
            "interaction_focus_policy",
            session_id,
            lease_id,
            &target.window,
            "policy",
            policy_result.is_ok(),
        )?;
        policy_result.with_context(|| {
            format!("sticky focus lease {lease_id} policy check for session {session_id}")
        })?;
        let focus_backend = runtime.window_backend.as_ref();
        let focus_result = focus_backend
            .focus_window(target.window.id.clone())
            .await
            .map_err(anyhow::Error::msg);
        runtime.journal.record_focus_lease_step(
            "interaction_focus",
            session_id,
            lease_id,
            &target.window,
            focus_backend.backend_name(),
            focus_result.is_ok(),
        )?;
        focus_result.with_context(|| {
            format!("sticky focus lease {lease_id} focus for session {session_id}")
        })?;
    }

    let verified = wait_for_active_target(
        runtime.window_backend.as_ref(),
        &target.window.id,
        Duration::from_millis(750),
    )
    .await?;
    runtime.journal.record_focus_lease_step(
        "interaction_focus_verify",
        session_id,
        lease_id,
        &target.window,
        runtime.window_backend.backend_name(),
        verified,
    )?;
    if !verified {
        bail!(
            "focus lease conflict: pinned target did not become active before input; lease={lease_id} session={session_id}"
        );
    }

    require_live_capture_session(runtime, session_id).await?;
    let injection_safe = runtime.activity_tracker.safe_since(activity_snapshot);
    runtime.journal.record_focus_lease_step(
        "interaction_input_activity",
        session_id,
        lease_id,
        &target.window,
        super::activity::KWIN_INPUT_SPY_BACKEND,
        injection_safe,
    )?;
    require_injection_activity_safe(injection_safe)?;
    let mut result = action().await?;
    runtime.interaction_session_store.renew(session_id).await?;
    let restoration = restore_previous_focus(
        runtime,
        session_id,
        lease_id,
        &target.window,
        previous_window.as_ref(),
        activity_snapshot,
    )
    .await?;
    result.id = lease_id;
    let activity = runtime.activity_tracker.status();
    if let Err(error) = runtime
        .session_execution_store
        .record_focus_lease(
            session_id,
            super::session_execution::FocusLeaseExecution {
                lease_id,
                focus_reacquired: !already_active,
                focus_restored: restoration.restored,
                restoration: restoration.reason.to_string(),
                activity_backend: activity.backend,
                activity_trusted: activity.trusted,
                last_activity_class: activity.last_class.map(str::to_string),
                last_activity_provenance: activity.last_provenance.map(str::to_string),
            },
        )
        .await
    {
        tracing::warn!(%error, %session_id, %lease_id, "could not update session focus metadata");
    }
    let message = result.message.take().unwrap_or_default();
    let focus_reacquired = !already_active;
    result.message = Some(format!(
        "{message} session={} focus_reacquired={focus_reacquired} focus_restored={} restoration={}",
        target.session_id, restoration.restored, restoration.reason
    ));
    Ok(result)
}

pub(crate) async fn execute_agent_seat_action<F, Fut>(
    runtime: &super::DaemonRuntime,
    session_id: Option<&str>,
    safety_class: libseatgeist::SafetyClass,
    action: F,
) -> Result<ActionResult>
where
    F: FnOnce(PinnedTarget) -> Fut,
    Fut: Future<Output = Result<ActionResult>>,
{
    let session_id = session_id.ok_or_else(|| {
        anyhow::anyhow!("kwin_agent_seat requires session_id for an exact pinned target")
    })?;
    require_live_capture_session(runtime, session_id).await?;
    let windows = read_windows(runtime).await?;
    let target = runtime
        .interaction_session_store
        .resolve(session_id, &windows)
        .await?;
    super::enforce_app_policy_for_app(
        &runtime.app_policy,
        target.window.app_id.as_deref(),
        "pinned agent-seat target",
    )?;
    require_live_capture_session(runtime, session_id).await?;

    let target_activity = runtime
        .activity_tracker
        .target_interference_state(&target.window.id, AGENT_TARGET_QUIET_TIME);
    require_agent_target_quiet(target_activity)?;
    let activity_snapshot = runtime.activity_tracker.target_snapshot(&target.window.id);

    let mut result = action(target.clone()).await?;
    runtime.interaction_session_store.renew(session_id).await?;
    let backend = result
        .message
        .as_deref()
        .and_then(|message| {
            message
                .split_whitespace()
                .find_map(|part| part.strip_prefix("backend="))
        })
        .unwrap_or(crate::agent_seat::KWIN_AGENT_SEAT_BACKEND);
    runtime.journal.record_agent_seat_delivery(
        session_id,
        &target.lane_id,
        result.id,
        &target.window,
        backend,
        safety_class,
    )?;
    if let Err(error) = runtime
        .session_execution_store
        .record_focus_lease(
            session_id,
            super::session_execution::FocusLeaseExecution {
                lease_id: result.id,
                focus_reacquired: false,
                focus_restored: false,
                restoration: "independent_agent_seat".to_string(),
                activity_backend: None,
                activity_trusted: false,
                last_activity_class: None,
                last_activity_provenance: None,
            },
        )
        .await
    {
        tracing::warn!(
            %error,
            %session_id,
            action_id = %result.id,
            "could not update independent agent-seat metadata"
        );
    }
    let message = result.message.take().unwrap_or_default();
    // The KWin activity plugin publishes over D-Bus. Give an event already in
    // flight one short turn to reach the daemon before declaring delivery
    // independent of the physical seat.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let target_activity_safe = runtime
        .activity_tracker
        .target_safe_since(&target.window.id, activity_snapshot);
    let preemption = if !target_activity.available {
        "unavailable"
    } else if target_activity_safe {
        "quiet"
    } else {
        "user_preempted"
    };
    let delivery = if target_activity_safe {
        "delivery_ack"
    } else {
        "delivered_but_unconfirmed"
    };
    let lane_label = target.lane_id.split('-').next().unwrap_or("unknown");
    result.message = Some(format!(
        "{message} session={} lane={} focus_reacquired=false focus_restored=false restoration=independent_agent_seat target_preemption={preemption} target_delivery={delivery}",
        target.session_id, lane_label
    ));
    if !target_activity_safe {
        result.ok = false;
        let mut observation = super::observation::post_action_window_only(runtime).await;
        observation.settle = Some(ActionSettleResult {
            confirmation: ActionConfirmation::UserPreempted,
            condition: ActionSettleCondition::None,
            backend: ActionSettleBackend::DeliveryAck,
            target_scoped: true,
            event: Some("physical_user_input".to_string()),
            settled: false,
            timed_out: false,
            timeout_ms: 0,
            interval_ms: 0,
            samples: 1,
            elapsed_ms: 0,
            before_revision: None,
            after_revision: observation.revision.clone().unwrap_or_default(),
        });
        result.observation = Some(observation);
    }
    Ok(result)
}

fn require_agent_target_quiet(state: super::activity::TargetInterferenceState) -> Result<()> {
    if !state.available || !state.fresh {
        return Ok(());
    }
    bail!(
        "agent target user active: physical input reached this window {}ms ago; wait until it is quiet for {}ms and capture a fresh frame",
        state.age_ms.unwrap_or_default(),
        AGENT_TARGET_QUIET_TIME.as_millis()
    )
}

fn require_injection_activity_safe(safe: bool) -> Result<()> {
    if safe {
        return Ok(());
    }
    bail!("human input activity occurred during the focus lease; input was not injected")
}

async fn require_live_capture_session(
    runtime: &super::DaemonRuntime,
    session_id: &str,
) -> Result<()> {
    if let Err(err) = runtime
        .capture_session_store
        .require_active(session_id)
        .await
    {
        let _ = runtime.interaction_session_store.clear(session_id).await;
        return Err(err);
    }
    Ok(())
}

async fn read_windows(runtime: &super::DaemonRuntime) -> Result<Vec<WindowInfo>> {
    runtime
        .window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)
}

async fn read_active_window(runtime: &super::DaemonRuntime) -> Result<Option<WindowInfo>> {
    runtime
        .window_backend
        .active_window()
        .await
        .map_err(anyhow::Error::msg)
}

struct FocusRestoration {
    restored: bool,
    reason: &'static str,
}

async fn restore_previous_focus(
    runtime: &super::DaemonRuntime,
    session_id: &str,
    lease_id: Uuid,
    target: &WindowInfo,
    previous: Option<&WindowInfo>,
    activity_snapshot: super::activity::ActivitySnapshot,
) -> Result<FocusRestoration> {
    let Some(previous) = previous else {
        return Ok(FocusRestoration {
            restored: false,
            reason: "no_previous_window",
        });
    };
    if previous.id == target.id {
        return Ok(FocusRestoration {
            restored: false,
            reason: "target_was_already_active",
        });
    }

    // Allow the compositor plugin's queued DBus activity update to arrive
    // before deciding whether the user's focus may be restored.
    tokio::time::sleep(Duration::from_millis(25)).await;
    let activity_safe = runtime.activity_tracker.safe_since(activity_snapshot);
    runtime.journal.record_focus_lease_step(
        "interaction_restore_activity",
        session_id,
        lease_id,
        previous,
        super::activity::KWIN_INPUT_SPY_BACKEND,
        activity_safe,
    )?;
    if !activity_safe {
        let reason = if runtime.activity_tracker.status().trusted {
            "user_activity_detected"
        } else {
            "activity_provenance_unavailable"
        };
        return Ok(FocusRestoration {
            restored: false,
            reason,
        });
    }

    if read_active_window(runtime)
        .await?
        .is_none_or(|window| window.id != target.id)
    {
        runtime.journal.record_focus_lease_step(
            "interaction_restore_focus_guard",
            session_id,
            lease_id,
            previous,
            runtime.window_backend.backend_name(),
            false,
        )?;
        return Ok(FocusRestoration {
            restored: false,
            reason: "focus_changed_during_lease",
        });
    }

    let windows = read_windows(runtime).await?;
    let Some(resolved_previous) = windows.iter().find(|window| {
        window.id == previous.id && window.app_id == previous.app_id && window.pid == previous.pid
    }) else {
        runtime.journal.record_focus_lease_step(
            "interaction_restore_target",
            session_id,
            lease_id,
            previous,
            runtime.window_backend.backend_name(),
            false,
        )?;
        return Ok(FocusRestoration {
            restored: false,
            reason: "previous_window_lost",
        });
    };

    let app_policy = super::enforce_app_policy_for_app(
        &runtime.app_policy,
        resolved_previous.app_id.as_deref(),
        "focus restoration target",
    );
    runtime.journal.record_focus_lease_step(
        "interaction_restore_app_policy",
        session_id,
        lease_id,
        resolved_previous,
        "policy",
        app_policy.is_ok(),
    )?;
    if app_policy.is_err() {
        return Ok(FocusRestoration {
            restored: false,
            reason: "restore_app_policy_denied",
        });
    }

    let restore_request = DaemonRequest::FocusWindow(FocusWindowRequest {
        window_id: resolved_previous.id.clone(),
        guard: None,
    });
    let focus_policy = super::enforce_policy_with_approvals(
        &runtime.policy,
        &runtime.approval_store,
        &restore_request,
    );
    runtime.journal.record_focus_lease_step(
        "interaction_restore_policy",
        session_id,
        lease_id,
        resolved_previous,
        "policy",
        focus_policy.is_ok(),
    )?;
    if focus_policy.is_err() {
        return Ok(FocusRestoration {
            restored: false,
            reason: "restore_policy_denied",
        });
    }

    if !runtime.activity_tracker.safe_since(activity_snapshot) {
        runtime.journal.record_focus_lease_step(
            "interaction_restore_activity_recheck",
            session_id,
            lease_id,
            resolved_previous,
            super::activity::KWIN_INPUT_SPY_BACKEND,
            false,
        )?;
        return Ok(FocusRestoration {
            restored: false,
            reason: "user_activity_detected",
        });
    }

    let backend = runtime.window_backend.as_ref();
    let focus_result = backend
        .focus_window(resolved_previous.id.clone())
        .await
        .map_err(anyhow::Error::msg);
    runtime.journal.record_focus_lease_step(
        "interaction_restore_focus",
        session_id,
        lease_id,
        resolved_previous,
        backend.backend_name(),
        focus_result.is_ok(),
    )?;
    if focus_result.is_err() {
        return Ok(FocusRestoration {
            restored: false,
            reason: "restore_backend_failed",
        });
    }

    let restored = wait_for_active_target(
        runtime.window_backend.as_ref(),
        &resolved_previous.id,
        Duration::from_millis(750),
    )
    .await?;
    runtime.journal.record_focus_lease_step(
        "interaction_restore_verify",
        session_id,
        lease_id,
        resolved_previous,
        runtime.window_backend.backend_name(),
        restored,
    )?;
    Ok(FocusRestoration {
        restored,
        reason: if restored {
            "restored"
        } else {
            "restore_unconfirmed"
        },
    })
}

pub(crate) async fn wait_for_active_target(
    window_backend: &dyn seatgeist_backend::WindowBackend,
    target_window_id: &str,
    timeout: Duration,
) -> Result<bool> {
    let started = Instant::now();
    loop {
        if window_backend
            .active_window()
            .await
            .map_err(anyhow::Error::msg)?
            .is_some_and(|window| window.id == target_window_id)
        {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, pid: u32) -> WindowInfo {
        WindowInfo {
            id: id.to_string(),
            app_id: Some("org.mozilla.firefox".to_string()),
            title: "Firefox".to_string(),
            pid: Some(pid),
            monitor_id: None,
            geometry: None,
        }
    }

    #[tokio::test]
    async fn pinned_target_survives_user_focus_changes_but_not_identity_changes() {
        let store = InteractionSessionStore::default();
        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(1),
            )
            .await
            .expect("session binds");
        let resolved = store
            .resolve(
                "interaction-1",
                &[window("kate-1", 7), window("firefox-1", 42)],
            )
            .await
            .expect("unrelated active window does not affect resolution");
        assert_eq!(resolved.window.id, "firefox-1");

        let error = store
            .resolve("interaction-1", &[window("firefox-1", 99)])
            .await
            .expect_err("reopened target fails");
        assert!(error.to_string().contains("identity changed"));
    }

    #[tokio::test]
    async fn multiple_pinned_targets_remain_independently_addressable() {
        let store = InteractionSessionStore::default();
        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(1),
            )
            .await
            .expect("first target binds");
        store
            .bind(
                "interaction-2".to_string(),
                &window("firefox-2", 43),
                SessionOwner::test_process(2),
            )
            .await
            .expect("second target binds");
        let windows = [window("firefox-1", 42), window("firefox-2", 43)];
        assert_eq!(
            store
                .resolve("interaction-1", &windows)
                .await
                .expect("first resolves")
                .window
                .id,
            "firefox-1"
        );
        assert_eq!(
            store
                .resolve("interaction-2", &windows)
                .await
                .expect("second resolves")
                .window
                .id,
            "firefox-2"
        );
        store.clear("interaction-1").await.expect("first clears");
        assert!(store.status("interaction-2").await.bound);
    }

    #[tokio::test]
    async fn one_window_has_one_agent_owner_but_the_user_is_not_locked() {
        let store = InteractionSessionStore::default();
        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(1),
            )
            .await
            .expect("first agent owns the target");
        let error = store
            .bind(
                "interaction-2".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(2),
            )
            .await
            .expect_err("another agent cannot share the target");
        assert!(error.to_string().contains("agent target in use"));

        store
            .clear("interaction-1")
            .await
            .expect("first agent releases the target");
        store
            .bind(
                "interaction-2".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(2),
            )
            .await
            .expect("another agent may bind after release");
    }

    #[tokio::test]
    async fn one_owner_reuses_one_lane_across_different_windows() {
        let store = InteractionSessionStore::default();
        let owner = SessionOwner::test_process(1);
        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                owner.clone(),
            )
            .await
            .expect("first target binds");
        store
            .bind("interaction-2".to_string(), &window("firefox-2", 43), owner)
            .await
            .expect("same owner may bind another target");
        let windows = [window("firefox-1", 42), window("firefox-2", 43)];
        let first = store
            .resolve("interaction-1", &windows)
            .await
            .expect("first resolves");
        let second = store
            .resolve("interaction-2", &windows)
            .await
            .expect("second resolves");
        assert_eq!(first.lane_id, second.lane_id);
    }

    #[tokio::test]
    async fn active_agent_lane_count_is_bounded() {
        let store = InteractionSessionStore::default();
        for owner in 1..=MAX_AGENT_LANES {
            store
                .bind(
                    format!("interaction-{owner}"),
                    &window(&format!("firefox-{owner}"), owner as u32),
                    SessionOwner::test_process(owner as u32),
                )
                .await
                .expect("lane within quota binds");
        }
        let error = store
            .bind(
                "interaction-overflow".to_string(),
                &window("firefox-overflow", 99),
                SessionOwner::test_process(99),
            )
            .await
            .expect_err("fifth owner exceeds the lane quota");
        assert!(error.to_string().contains("agent lane quota reached"));
    }

    #[tokio::test]
    async fn ended_capture_sessions_release_window_and_lane_leases() {
        let store = InteractionSessionStore::default();
        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(1),
            )
            .await
            .expect("first target binds");
        store.retain_active_capture_sessions(&HashSet::new()).await;
        store
            .bind(
                "interaction-2".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(2),
            )
            .await
            .expect("ended capture releases the target lease");
    }

    #[tokio::test]
    async fn clear_requires_matching_session_id() {
        let store = InteractionSessionStore::default();
        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(1),
            )
            .await
            .expect("session binds");
        assert!(store.clear("other").await.is_err());
        assert!(store.status("interaction-1").await.bound);
        store.clear("interaction-1").await.expect("matching clear");
        assert!(!store.status("interaction-1").await.bound);
    }

    #[tokio::test]
    async fn renew_requires_matching_bound_session() {
        let store = InteractionSessionStore::default();
        let error = store
            .renew("interaction-1")
            .await
            .expect_err("unbound session cannot renew");
        assert!(error.to_string().contains("no bound sticky session"));

        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(1),
            )
            .await
            .expect("session binds");
        let error = store
            .renew("other")
            .await
            .expect_err("wrong id cannot renew");
        assert!(error.to_string().contains("no bound sticky session"));
        store
            .renew("interaction-1")
            .await
            .expect("matching live session renews");
        let status = store.status("interaction-1").await;
        assert!(status.bound);
        assert_eq!(status.owner_tool.as_deref(), Some("test-client"));
        assert_eq!(status.owner_pid, Some(1));
        assert_eq!(status.owner_scope.as_deref(), Some("process"));
    }

    #[tokio::test]
    async fn status_validation_clears_closed_or_reopened_target() {
        let store = InteractionSessionStore::default();
        store
            .bind(
                "interaction-1".to_string(),
                &window("firefox-1", 42),
                SessionOwner::test_process(1),
            )
            .await
            .expect("session binds");
        assert!(
            !store
                .clear_if_target_invalid("interaction-1", &[window("firefox-1", 42)])
                .await
                .expect("same identity stays valid")
        );
        assert!(store.status("interaction-1").await.bound);

        assert!(
            store
                .clear_if_target_invalid("interaction-1", &[window("firefox-2", 42)])
                .await
                .expect("replacement id invalidates the old target")
        );
        assert!(!store.status("interaction-1").await.bound);
    }

    #[test]
    fn human_activity_during_lease_aborts_before_injection() {
        require_injection_activity_safe(true).expect("quiet lease may inject");
        let error = require_injection_activity_safe(false)
            .expect_err("interfered lease must fail before injection");
        assert!(error.to_string().contains("input was not injected"));
    }

    #[test]
    fn target_local_user_activity_pauses_only_while_fresh() {
        require_agent_target_quiet(crate::activity::TargetInterferenceState {
            available: false,
            fresh: false,
            age_ms: None,
        })
        .expect("legacy activity backend preserves compatibility");
        require_agent_target_quiet(crate::activity::TargetInterferenceState {
            available: true,
            fresh: false,
            age_ms: Some(800),
        })
        .expect("quiet target may receive agent input");
        let error = require_agent_target_quiet(crate::activity::TargetInterferenceState {
            available: true,
            fresh: true,
            age_ms: Some(40),
        })
        .expect_err("fresh same-window input preempts the agent");
        assert!(error.to_string().contains("agent target user active"));
        assert!(error.to_string().contains("350ms"));
    }
}
