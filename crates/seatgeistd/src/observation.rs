use anyhow::Result;
use libseatgeist::{
    AccessibilityNode, DesktopObservation, FocusedAccessibilityTreeRequest, Observation,
    ObserveRequest, WindowInfo, WindowInventory, WindowInventoryWaitRequest,
    WindowInventoryWaitResult,
};
use seatgeist_backend::{ScreenBackend, WindowBackend};
use sha2::{Digest, Sha256};

use crate::{
    DaemonRuntime, SafetySettings, focused_accessibility_tree, kwin_bridge::WindowListState,
};

pub(crate) async fn desktop(
    request: ObserveRequest,
    window_backend: &dyn WindowBackend,
    screen_backend: &dyn ScreenBackend,
    window_list_state: &WindowListState,
    safety_settings: &SafetySettings,
) -> Result<DesktopObservation> {
    let monitors = screen_backend.list_monitors().await.unwrap_or_default();
    let windows = window_backend.list_windows().await.unwrap_or_default();
    let active_window = window_backend.active_window().await.unwrap_or_default();
    let screenshot = match request.screenshot {
        Some(request) => {
            Some(super::capture_screenshot(request, safety_settings, window_list_state).await?)
        }
        None => None,
    };

    Ok(DesktopObservation {
        desktop_revision: active_window_revision(&active_window),
        active_window,
        windows,
        monitors,
        screenshot,
    })
}

pub(crate) async fn post_action(runtime: &DaemonRuntime) -> Observation {
    let mut issues = Vec::new();
    let active_window = match runtime.window_backend.active_window().await {
        Ok(window) => window,
        Err(_) => {
            issues.push("active_window_unavailable".to_string());
            None
        }
    };
    let focused_accessibility = match focused_accessibility_tree(FocusedAccessibilityTreeRequest {
        depth: 0,
        max_nodes: 64,
    }) {
        Ok(node) => node.map(compact_accessibility),
        Err(_) => {
            issues.push("accessibility_unavailable".to_string());
            None
        }
    };
    let revision = revision(&active_window, &focused_accessibility);
    Observation {
        active_window,
        target_window: None,
        windows: Vec::new(),
        monitors: Vec::new(),
        focused_accessibility,
        target_accessibility: None,
        screenshot_path: None,
        revision: Some(revision),
        issues,
        settle: None,
    }
}

pub(crate) async fn window_inventory(
    window_backend: &dyn WindowBackend,
) -> Result<WindowInventory> {
    let mut windows = window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)?;
    windows.sort_by(|left, right| left.id.cmp(&right.id));
    let active_window = window_backend
        .active_window()
        .await
        .map_err(anyhow::Error::msg)?;
    let encoded = serde_json::to_vec(&(&active_window, &windows)).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"seatgeist-window-inventory-v1\0");
    hasher.update(encoded);
    Ok(WindowInventory {
        revision: format!("wi1:{:x}", hasher.finalize()),
        active_window,
        windows,
        semantic_handles: Vec::new(),
    })
}

pub(crate) async fn wait_for_window_inventory(
    window_backend: &dyn WindowBackend,
    request: WindowInventoryWaitRequest,
) -> Result<WindowInventoryWaitResult> {
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(request.timeout_ms.clamp(1, 30_000));
    loop {
        let inventory = window_inventory(window_backend).await?;
        if inventory.revision != request.after_revision {
            return Ok(WindowInventoryWaitResult {
                changed: true,
                timed_out: false,
                elapsed_ms: started.elapsed().as_millis() as u64,
                inventory,
            });
        }
        if started.elapsed() >= timeout {
            return Ok(WindowInventoryWaitResult {
                changed: false,
                timed_out: true,
                elapsed_ms: started.elapsed().as_millis() as u64,
                inventory,
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn compact_accessibility(mut node: AccessibilityNode) -> AccessibilityNode {
    node.value = None;
    node.value_truncated = false;
    node.children.clear();
    node
}

fn revision(
    active_window: &Option<WindowInfo>,
    focused_accessibility: &Option<AccessibilityNode>,
) -> String {
    let encoded = serde_json::to_vec(&(active_window, focused_accessibility)).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(encoded);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn active_window_revision(active_window: &Option<WindowInfo>) -> String {
    let encoded = serde_json::to_vec(active_window).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"seatgeist-active-window-v1\0");
    hasher.update(encoded);
    format!("aw1:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn desktop_without_screenshot_uses_injected_backends() {
        let window_backend = seatgeist_testkit::MockWindowBackend::default();
        let screen_backend = seatgeist_testkit::MockScreenBackend::default();
        let observation = desktop(
            ObserveRequest { screenshot: None },
            &window_backend,
            &screen_backend,
            &WindowListState::default(),
            &SafetySettings::default(),
        )
        .await
        .expect("mock-backed observation succeeds");

        assert_eq!(
            observation.windows,
            vec![seatgeist_testkit::sample_window()]
        );
        assert_eq!(
            observation.active_window,
            Some(seatgeist_testkit::sample_window())
        );
        assert_eq!(
            observation.monitors,
            vec![seatgeist_testkit::sample_monitor()]
        );
        assert_eq!(observation.screenshot, None);
        assert!(observation.desktop_revision.starts_with("aw1:"));
    }
}
