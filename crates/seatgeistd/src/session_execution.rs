use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use libseatgeist::{
    ActionSettleResult, SafetyClass, SessionExecutionStatus, SessionFocusLeaseStatus,
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) const COOPERATIVE_FOCUS_POLICY: &str = "reacquire_verify_inject_restore_if_safe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendRole {
    RawInput,
    Semantic,
    Other,
}

#[derive(Debug, Clone)]
pub(crate) struct SuccessfulExecution {
    pub(crate) method: String,
    pub(crate) safety_class: SafetyClass,
    pub(crate) backend: Option<String>,
    pub(crate) backend_role: BackendRole,
    pub(crate) action_id: Option<Uuid>,
    pub(crate) settle: Option<ActionSettleResult>,
}

#[derive(Debug, Clone)]
pub(crate) struct FocusLeaseExecution {
    pub(crate) lease_id: Uuid,
    pub(crate) focus_reacquired: bool,
    pub(crate) focus_restored: bool,
    pub(crate) restoration: String,
    pub(crate) activity_backend: Option<String>,
    pub(crate) activity_trusted: bool,
    pub(crate) last_activity_class: Option<String>,
    pub(crate) last_activity_provenance: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionExecutionStore {
    records: Arc<Mutex<HashMap<String, SessionExecutionStatus>>>,
}

impl SessionExecutionStore {
    pub(crate) async fn open(
        &self,
        session_id: String,
        capture_backend: String,
        cooperative_focus: bool,
    ) {
        let mut records = self.records.lock().await;
        records.insert(
            session_id,
            SessionExecutionStatus {
                capture_backend,
                semantic_backend: None,
                raw_input_backend: None,
                last_action_backend: None,
                last_action_method: None,
                last_action_safety_class: None,
                last_action_id: None,
                last_action_unix_ms: None,
                target_policy_result: None,
                last_policy_result: None,
                cooperative_focus_policy: cooperative_focus
                    .then(|| COOPERATIVE_FOCUS_POLICY.to_string()),
                activity_backend: None,
                activity_trusted: false,
                last_activity_class: None,
                last_activity_provenance: None,
                focus_lease: None,
                settle: None,
            },
        );
    }

    pub(crate) async fn clear(&self, session_id: &str) -> Result<()> {
        let removed = self.records.lock().await.remove(session_id);
        if removed.is_none() {
            bail!("session execution id is not active");
        }
        Ok(())
    }

    pub(crate) async fn status(&self, session_id: &str) -> Option<SessionExecutionStatus> {
        self.records.lock().await.get(session_id).cloned()
    }

    pub(crate) async fn record_target_policy(&self, session_id: &str, result: &str) -> Result<()> {
        self.with_status_mut(session_id, |status| {
            status.target_policy_result = Some(result.to_string());
        })
        .await
    }

    pub(crate) async fn record_success(
        &self,
        session_id: &str,
        execution: SuccessfulExecution,
    ) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.with_status_mut(session_id, |status| {
            status.last_action_method = Some(execution.method);
            status.last_action_safety_class = Some(execution.safety_class);
            status.last_action_id = execution.action_id;
            status.last_action_unix_ms = Some(now);
            status.last_policy_result = Some("allow".to_string());
            status.last_action_backend = execution.backend.clone();
            match execution.backend_role {
                BackendRole::RawInput => status.raw_input_backend = execution.backend,
                BackendRole::Semantic => status.semantic_backend = execution.backend,
                BackendRole::Other => {}
            }
            if let Some(settle) = execution.settle {
                status.settle = Some(settle);
            }
        })
        .await
    }

    pub(crate) async fn record_focus_lease(
        &self,
        session_id: &str,
        execution: FocusLeaseExecution,
    ) -> Result<()> {
        self.with_status_mut(session_id, |status| {
            status.activity_backend = execution.activity_backend;
            status.activity_trusted = execution.activity_trusted;
            status.last_activity_class = execution.last_activity_class;
            status.last_activity_provenance = execution.last_activity_provenance;
            status.focus_lease = Some(SessionFocusLeaseStatus {
                lease_id: execution.lease_id,
                focus_reacquired: execution.focus_reacquired,
                focus_restored: execution.focus_restored,
                restoration: execution.restoration,
            });
        })
        .await
    }

    async fn with_status_mut(
        &self,
        session_id: &str,
        update: impl FnOnce(&mut SessionExecutionStatus),
    ) -> Result<()> {
        let mut records = self.records.lock().await;
        let active = records
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("session execution state is not active"))?;
        update(active);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libseatgeist::{ActionSettleBackend, ActionSettleCondition};

    fn settle() -> ActionSettleResult {
        ActionSettleResult {
            confirmation: libseatgeist::ActionConfirmation::Confirmed,
            condition: ActionSettleCondition::AccessibilityChange,
            backend: ActionSettleBackend::AtspiEvent,
            target_scoped: true,
            event: Some("object:state-changed".to_string()),
            settled: true,
            timed_out: false,
            timeout_ms: 1_000,
            interval_ms: 100,
            samples: 1,
            elapsed_ms: 12,
            before_revision: Some("before".to_string()),
            after_revision: "after".to_string(),
        }
    }

    #[tokio::test]
    async fn records_backend_policy_activity_focus_and_settle_metadata() {
        let store = SessionExecutionStore::default();
        store
            .open(
                "capture-1".to_string(),
                "portal_screencast_pipewire".to_string(),
                true,
            )
            .await;
        store
            .record_target_policy("capture-1", "allow")
            .await
            .expect("target policy records");
        let action_id = Uuid::new_v4();
        store
            .record_success(
                "capture-1",
                SuccessfulExecution {
                    method: "type_text".to_string(),
                    safety_class: SafetyClass::ControlKeyboard,
                    backend: Some("uinput".to_string()),
                    backend_role: BackendRole::RawInput,
                    action_id: Some(action_id),
                    settle: Some(settle()),
                },
            )
            .await
            .expect("success records");
        let lease_id = Uuid::new_v4();
        store
            .record_focus_lease(
                "capture-1",
                FocusLeaseExecution {
                    lease_id,
                    focus_reacquired: true,
                    focus_restored: true,
                    restoration: "restored".to_string(),
                    activity_backend: Some("kwin_input_spy_v1".to_string()),
                    activity_trusted: true,
                    last_activity_class: Some("keyboard".to_string()),
                    last_activity_provenance: Some("seatgeist_injected".to_string()),
                },
            )
            .await
            .expect("focus lease records");

        let status = store.status("capture-1").await.expect("status exists");
        assert_eq!(status.capture_backend, "portal_screencast_pipewire");
        assert_eq!(status.raw_input_backend.as_deref(), Some("uinput"));
        assert_eq!(status.target_policy_result.as_deref(), Some("allow"));
        assert_eq!(status.last_policy_result.as_deref(), Some("allow"));
        assert_eq!(status.last_action_id, Some(action_id));
        assert!(status.last_action_unix_ms.is_some());
        assert_eq!(
            status.cooperative_focus_policy.as_deref(),
            Some(COOPERATIVE_FOCUS_POLICY)
        );
        assert!(status.activity_trusted);
        assert_eq!(
            status.focus_lease.as_ref().map(|lease| lease.lease_id),
            Some(lease_id)
        );
        assert_eq!(
            status.settle.as_ref().map(|settle| settle.backend),
            Some(ActionSettleBackend::AtspiEvent)
        );
    }

    #[tokio::test]
    async fn rejects_wrong_ids_and_clears_matching_state() {
        let store = SessionExecutionStore::default();
        store
            .open("capture-1".to_string(), "mock".to_string(), false)
            .await;
        assert!(
            store
                .record_target_policy("capture-other", "allow")
                .await
                .is_err()
        );
        assert!(store.clear("capture-other").await.is_err());
        assert!(store.status("capture-1").await.is_some());
        store.clear("capture-1").await.expect("matching clear");
        assert!(store.status("capture-1").await.is_none());
    }

    #[tokio::test]
    async fn keeps_execution_metadata_for_multiple_capture_sessions() {
        let store = SessionExecutionStore::default();
        store
            .open("capture-1".to_string(), "kwin".to_string(), true)
            .await;
        store
            .open("capture-2".to_string(), "kwin".to_string(), true)
            .await;
        store
            .record_target_policy("capture-1", "allow")
            .await
            .expect("first record updates");
        assert_eq!(
            store
                .status("capture-1")
                .await
                .and_then(|status| status.target_policy_result),
            Some("allow".to_string())
        );
        assert_eq!(
            store
                .status("capture-2")
                .await
                .and_then(|status| status.target_policy_result),
            None
        );
    }
}
