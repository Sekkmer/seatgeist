use std::{collections::HashMap, fmt, sync::Arc};

use anyhow::Result;
use libseatgeist::{JournalClientContext, WindowInfo};
use tokio::sync::Mutex;

use crate::{interaction::DEFAULT_SESSION_TTL, session_owner::SessionOwner};

#[derive(Clone, Default)]
pub(crate) struct LaunchedWindowStore {
    windows: Arc<Mutex<HashMap<String, LaunchedWindowLease>>>,
}

impl fmt::Debug for LaunchedWindowStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LaunchedWindowStore(..)")
    }
}

#[derive(Debug, Clone)]
struct LaunchedWindowLease {
    app_id: Option<String>,
    pid: Option<u32>,
    owner: SessionOwner,
    owner_process_pid: u32,
    expires_at: tokio::time::Instant,
}

impl LaunchedWindowStore {
    pub(crate) async fn record(
        &self,
        window: &WindowInfo,
        client: Option<&JournalClientContext>,
    ) -> Result<()> {
        let owner = SessionOwner::from_client(client)?;
        let owner_process_pid = client
            .and_then(|client| client.pid)
            .ok_or_else(|| anyhow::anyhow!("launch ownership requires a trusted Unix peer pid"))?;
        let mut windows = self.windows.lock().await;
        prune_expired(&mut windows);
        windows.insert(
            window.id.clone(),
            LaunchedWindowLease {
                app_id: window.app_id.clone(),
                pid: window.pid,
                owner,
                owner_process_pid,
                expires_at: tokio::time::Instant::now() + DEFAULT_SESSION_TTL,
            },
        );
        Ok(())
    }

    pub(crate) async fn is_owned_exact(
        &self,
        window: &WindowInfo,
        client: Option<&JournalClientContext>,
    ) -> bool {
        let Ok(owner) = SessionOwner::from_client(client) else {
            return false;
        };
        let Some(owner_process_pid) = client.and_then(|client| client.pid) else {
            return false;
        };
        let mut windows = self.windows.lock().await;
        prune_expired(&mut windows);
        windows.get(&window.id).is_some_and(|lease| {
            lease.owner.identity() == owner.identity()
                && lease.owner_process_pid == owner_process_pid
                && lease.app_id == window.app_id
                && lease.pid == window.pid
        })
    }

    pub(crate) async fn remove(&self, window_id: &str) {
        self.windows.lock().await.remove(window_id);
    }

    #[cfg(test)]
    async fn expire_for_test(&self, window_id: &str) {
        if let Some(lease) = self.windows.lock().await.get_mut(window_id) {
            lease.expires_at = tokio::time::Instant::now();
        }
    }
}

fn prune_expired(windows: &mut HashMap<String, LaunchedWindowLease>) {
    let now = tokio::time::Instant::now();
    windows.retain(|_, lease| lease.expires_at > now);
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

    fn cli_client(pid: u32) -> JournalClientContext {
        JournalClientContext {
            tool: Some("seatgeist-cli".to_string()),
            pid: Some(pid),
            process_name: Some("seatgeist-cli".to_string()),
        }
    }

    fn window() -> WindowInfo {
        WindowInfo {
            id: "window-1".to_string(),
            title: "Agent window".to_string(),
            app_id: Some("firefox".to_string()),
            pid: Some(42),
            geometry: None,
            monitor_id: None,
        }
    }

    #[tokio::test]
    async fn exact_owner_and_unchanged_window_identity_are_required() {
        let store = LaunchedWindowStore::default();
        let original = window();
        store
            .record(&original, Some(&client(10)))
            .await
            .expect("launch ownership records");
        assert!(store.is_owned_exact(&original, Some(&client(10))).await);
        assert!(!store.is_owned_exact(&original, Some(&client(11))).await);

        let mut changed = original.clone();
        changed.pid = Some(43);
        assert!(!store.is_owned_exact(&changed, Some(&client(10))).await);
    }

    #[tokio::test]
    async fn expired_launch_ownership_cannot_authorize_cleanup() {
        let store = LaunchedWindowStore::default();
        let window = window();
        store
            .record(&window, Some(&client(10)))
            .await
            .expect("launch ownership records");
        store.expire_for_test(&window.id).await;
        assert!(!store.is_owned_exact(&window, Some(&client(10))).await);
    }

    #[tokio::test]
    async fn cleanup_is_process_scoped_even_when_capture_cli_sessions_are_tool_scoped() {
        let store = LaunchedWindowStore::default();
        let window = window();
        store
            .record(&window, Some(&cli_client(10)))
            .await
            .expect("launch ownership records");
        assert!(store.is_owned_exact(&window, Some(&cli_client(10))).await);
        assert!(!store.is_owned_exact(&window, Some(&cli_client(11))).await);
    }
}
