use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use libseatgeist::{JournalClientContext, SemanticWindowHandle, TargetWindowGuard, WindowInfo};
use uuid::Uuid;

use crate::session_owner::{SessionOwner, SessionOwnerIdentity};

const HANDLE_PREFIX: &str = "sh1:";
const HANDLE_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticHandleStore {
    inner: Arc<Mutex<HashMap<String, HandleRecord>>>,
}

#[derive(Debug, Clone)]
struct HandleRecord {
    owner: SessionOwnerIdentity,
    guard: TargetWindowGuard,
    expires_at: Instant,
}

impl SemanticHandleStore {
    pub(crate) fn issue_for_windows(
        &self,
        windows: &[WindowInfo],
        client: Option<&JournalClientContext>,
    ) -> Result<Vec<SemanticWindowHandle>> {
        let owner = SessionOwner::from_client(client)?.identity();
        let now = Instant::now();
        let mut records = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("semantic handle store lock is poisoned"))?;
        records.retain(|_, record| record.expires_at > now);
        let mut handles = Vec::with_capacity(windows.len());
        for window in windows {
            let handle = format!("{HANDLE_PREFIX}{}", Uuid::new_v4());
            records.insert(
                handle.clone(),
                HandleRecord {
                    owner: owner.clone(),
                    guard: TargetWindowGuard {
                        expected_window_id: window.id.clone(),
                        expected_app_id: window.app_id.clone(),
                        expected_pid: window.pid,
                        title_contains: None,
                    },
                    expires_at: now + HANDLE_TTL,
                },
            );
            handles.push(SemanticWindowHandle {
                handle,
                window_id: window.id.clone(),
                expires_in_ms: HANDLE_TTL.as_millis() as u64,
                one_shot: true,
            });
        }
        Ok(handles)
    }

    pub(crate) fn consume(
        &self,
        handle: &str,
        client: Option<&JournalClientContext>,
    ) -> Result<TargetWindowGuard> {
        if !handle.starts_with(HANDLE_PREFIX) {
            bail!("invalid semantic handle");
        }
        let requester = SessionOwner::from_client(client)?.identity();
        let now = Instant::now();
        let mut records = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("semantic handle store lock is poisoned"))?;
        let record = records.get(handle).cloned().ok_or_else(|| {
            anyhow::anyhow!("semantic handle is unknown, expired, or already used")
        })?;
        if record.expires_at <= now {
            bail!("semantic handle expired");
        }
        if record.owner != requester {
            bail!("semantic handle owner mismatch");
        }
        records.remove(handle);
        Ok(record.guard)
    }
}

pub(crate) fn encoded_handle(guard: &TargetWindowGuard) -> Option<&str> {
    guard
        .expected_window_id
        .starts_with(HANDLE_PREFIX)
        .then_some(guard.expected_window_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(pid: u32) -> JournalClientContext {
        JournalClientContext {
            tool: Some("seatgeist-mcp".to_string()),
            pid: Some(pid),
            process_name: Some("seatgeist-mcp".to_string()),
        }
    }

    #[test]
    fn handles_are_owner_bound_and_one_shot() {
        let store = SemanticHandleStore::default();
        let window = seatgeist_testkit::sample_window();
        let issued = store
            .issue_for_windows(std::slice::from_ref(&window), Some(&client(10)))
            .expect("issue handle");
        assert!(store.consume(&issued[0].handle, Some(&client(11))).is_err());

        let issued = store
            .issue_for_windows(std::slice::from_ref(&window), Some(&client(10)))
            .expect("issue replacement");
        let guard = store
            .consume(&issued[0].handle, Some(&client(10)))
            .expect("owner consumes handle");
        assert_eq!(guard.expected_window_id, window.id);
        assert!(store.consume(&issued[0].handle, Some(&client(10))).is_err());
    }
}
