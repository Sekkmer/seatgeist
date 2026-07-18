use anyhow::{Context, Result, bail};
use libseatgeist::{AccessibilityNode, TargetWindowGuard, WindowInfo};
use seatgeist_atspi::AccessibilityMatch;
use seatgeist_backend::WindowBackend;

use crate::{AppPolicy, window_safety::enforce_app_policy_for_app};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticActionTarget {
    pub node: AccessibilityNode,
    pub event_target: Option<seatgeist_backend::AccessibilityEventTarget>,
    pub window: Option<WindowInfo>,
}

impl SemanticActionTarget {
    pub fn uncorrelated(node: AccessibilityNode) -> Self {
        Self {
            node,
            event_target: None,
            window: None,
        }
    }
}

impl std::ops::Deref for SemanticActionTarget {
    type Target = AccessibilityNode;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSemanticTarget {
    pub window: WindowInfo,
    pub node: AccessibilityNode,
    pub event_target: seatgeist_backend::AccessibilityEventTarget,
}

impl From<ResolvedSemanticTarget> for SemanticActionTarget {
    fn from(target: ResolvedSemanticTarget) -> Self {
        Self {
            node: target.node,
            event_target: Some(target.event_target),
            window: Some(target.window),
        }
    }
}

pub(crate) async fn authorize_semantic_target(
    node: AccessibilityNode,
    matches: Vec<AccessibilityMatch>,
    guard: Option<&TargetWindowGuard>,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
) -> Result<SemanticActionTarget> {
    let Some(guard) = guard else {
        return Ok(SemanticActionTarget::uncorrelated(node));
    };
    let mut candidate = matches
        .into_iter()
        .find(|candidate| accessibility_tree_contains(&candidate.node, &node.id))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "target-window correlation failed: resolved accessibility node lost its application context"
            )
        })?;
    candidate.node = node;
    let windows = window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)
        .context("target-window correlation could not list windows")?;
    let resolved = resolve_semantic_target(candidate, &windows, guard)?;
    enforce_app_policy_for_app(
        app_policy,
        resolved.window.app_id.as_deref(),
        "resolved semantic target",
    )?;
    Ok(resolved.into())
}

fn accessibility_tree_contains(node: &AccessibilityNode, id: &str) -> bool {
    node.id == id
        || node
            .children
            .iter()
            .any(|child| accessibility_tree_contains(child, id))
}

pub(crate) fn resolve_semantic_target(
    candidate: AccessibilityMatch,
    windows: &[WindowInfo],
    guard: &TargetWindowGuard,
) -> Result<ResolvedSemanticTarget> {
    let event_target = candidate
        .event_target()
        .map_err(|error| anyhow::anyhow!(error))?;
    let window = windows
        .iter()
        .find(|window| window.id == guard.expected_window_id)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "target-window guard failed: window {} no longer exists",
                guard.expected_window_id
            )
        })?;
    validate_window_guard(&window, guard)?;

    let kwin_pid = window.pid.ok_or_else(|| {
        anyhow::anyhow!("target-window correlation failed: KWin target has no process id")
    })?;
    let accessibility_pid = candidate.process_id.ok_or_else(|| {
        anyhow::anyhow!(
            "target-window correlation failed: accessibility application has no process id"
        )
    })?;
    if accessibility_pid != kwin_pid {
        bail!("target-window correlation failed: KWin and accessibility process ids differ");
    }

    let accessibility_window = candidate
        .window_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "target-window correlation failed: accessibility target has no containing window"
            )
        })?;
    if !text_identity_matches(&window.title, accessibility_window) {
        bail!("target-window correlation failed: KWin title does not match accessibility window");
    }

    let accessibility_application = candidate.application_name.trim();
    let window_app = window
        .app_id
        .as_deref()
        .map(str::trim)
        .filter(|app| !app.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("target-window correlation failed: KWin target has no app id")
        })?;
    if accessibility_application.is_empty()
        || !application_identity_matches(window_app, accessibility_application)
    {
        bail!(
            "target-window correlation failed: KWin app does not match accessibility application"
        );
    }

    Ok(ResolvedSemanticTarget {
        window,
        node: candidate.node,
        event_target,
    })
}

fn validate_window_guard(window: &WindowInfo, guard: &TargetWindowGuard) -> Result<()> {
    if let Some(expected) = guard.expected_app_id.as_deref()
        && window.app_id.as_deref() != Some(expected)
    {
        bail!("target-window guard failed: app id changed");
    }
    if let Some(expected) = guard.expected_pid
        && window.pid != Some(expected)
    {
        bail!("target-window guard failed: pid changed");
    }
    if let Some(expected) = guard.title_contains.as_deref()
        && !window
            .title
            .to_ascii_lowercase()
            .contains(&expected.to_ascii_lowercase())
    {
        bail!("target-window guard failed: title changed");
    }
    Ok(())
}

fn text_identity_matches(left: &str, right: &str) -> bool {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    !left.is_empty() && !right.is_empty() && (left.contains(&right) || right.contains(&left))
}

fn application_identity_matches(left: &str, right: &str) -> bool {
    let left = identity_token(left);
    let right = identity_token(right);
    left.len() >= 3 && right.len() >= 3 && (left.contains(&right) || right.contains(&left))
}

fn identity_token(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(application: &str, window: Option<&str>) -> AccessibilityMatch {
        AccessibilityMatch {
            node: AccessibilityNode {
                id: "atspi://org.mozilla.firefox/node".to_string(),
                role: "button".to_string(),
                name: Some("Continue".to_string()),
                value: None,
                value_truncated: false,
                sensitive: false,
                states: Vec::new(),
                bounds: None,
                available_actions: vec!["press".to_string()],
                actions: vec![libseatgeist::AccessibilityAction::Press],
                children: Vec::new(),
            },
            application_name: application.to_string(),
            application_bus_name: "org.mozilla.firefox".to_string(),
            process_id: Some(4242),
            window_name: window.map(ToOwned::to_owned),
            window_node_id: Some(
                "atspi://org.mozilla.firefox/org/a11y/atspi/accessible/window".to_string(),
            ),
        }
    }

    fn firefox_window() -> WindowInfo {
        WindowInfo {
            id: "kwin-firefox-1".to_string(),
            app_id: Some("org.mozilla.firefox".to_string()),
            title: "Example - Mozilla Firefox".to_string(),
            pid: Some(4242),
            monitor_id: None,
            geometry: None,
        }
    }

    fn guard() -> TargetWindowGuard {
        TargetWindowGuard {
            expected_window_id: "kwin-firefox-1".to_string(),
            expected_app_id: Some("org.mozilla.firefox".to_string()),
            expected_pid: Some(4242),
            title_contains: Some("Example".to_string()),
        }
    }

    #[test]
    fn resolves_matching_kwin_and_accessibility_identity() {
        let target = resolve_semantic_target(
            candidate("Firefox", Some("Example - Mozilla Firefox")),
            &[firefox_window()],
            &guard(),
        )
        .expect("matching identity resolves");
        assert_eq!(target.window.id, "kwin-firefox-1");
        assert_eq!(target.node.name.as_deref(), Some("Continue"));
        assert_eq!(
            target.event_target.window_node_id,
            "atspi://org.mozilla.firefox/org/a11y/atspi/accessible/window"
        );
    }

    #[test]
    fn rejects_reopened_or_mismatched_targets() {
        let mut reopened = firefox_window();
        reopened.pid = Some(9999);
        assert!(
            resolve_semantic_target(
                candidate("Firefox", Some("Example - Mozilla Firefox")),
                &[reopened],
                &guard()
            )
            .expect_err("pid generation mismatch fails")
            .to_string()
            .contains("pid changed")
        );
        assert!(
            resolve_semantic_target(
                candidate("Firefox", Some("Other window")),
                &[firefox_window()],
                &guard()
            )
            .expect_err("ambiguous accessibility window fails")
            .to_string()
            .contains("correlation failed")
        );
    }
}
