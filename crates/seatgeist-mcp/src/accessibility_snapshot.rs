use super::{
    object_schema, optional_bool, optional_string, optional_u64, required_string, tool,
    with_post_action_properties,
};
use anyhow::{Result, bail};
use libseatgeist::{AccessibilityRefActionRequest, AccessibilitySnapshotRequest, DaemonRequest};
use serde_json::{Value, json};
pub(crate) fn snapshot(arguments: &Value) -> Result<DaemonRequest> {
    let window_id = optional_string(arguments, "window_id")?;
    let root_ref = optional_string(arguments, "root_ref")?;
    if window_id.is_some() == root_ref.is_some() {
        bail!("supply exactly one window_id or root_ref");
    }
    if ["session_id", "output", "max_edge", "timeout_ms"]
        .iter()
        .any(|k| arguments.get(k).is_some())
        || optional_bool(arguments, "include_image")? == Some(true)
    {
        bail!("accessibility snapshots do not accept image/session options");
    }
    let depth = optional_u64(arguments, "depth")?.unwrap_or(8);
    let max_nodes = optional_u64(arguments, "max_nodes")?.unwrap_or(256);
    if depth > 32 || !(1..=4096).contains(&max_nodes) {
        bail!("depth must be 0..32; max_nodes must be 1..4096");
    }
    Ok(DaemonRequest::AccessibilitySnapshot(
        AccessibilitySnapshotRequest {
            window_id,
            root_ref,
            skeleton: optional_bool(arguments, "skeleton")?.unwrap_or(false),
            depth: depth as usize,
            max_nodes: max_nodes as usize,
            interactive_only: optional_bool(arguments, "interactive_only")?.unwrap_or(false),
            compact: optional_bool(arguments, "compact")?.unwrap_or(false),
        },
    ))
}
pub(crate) fn action(arguments: &Value) -> Result<DaemonRequest> {
    if [
        "session_id",
        "semantic_handle",
        "target_window_id",
        "target_pid",
        "target_app_id",
        "target_title_contains",
        "desktop_revision",
        "expected_active_window",
        "expected_active_app",
        "active_title_contains",
    ]
    .iter()
    .any(|key| arguments.get(key).is_some())
    {
        bail!(
            "ref actions use only the snapshot-bound target; copied guards and raw sessions are not accepted"
        );
    }
    let action = required_string(arguments, "ref_action")?
        .parse()
        .map_err(anyhow::Error::msg)?;
    Ok(DaemonRequest::AccessibilityRefAction(
        AccessibilityRefActionRequest {
            ref_id: required_string(arguments, "ref_id")?,
            action,
            text: optional_string(arguments, "text")?,
            destructive: optional_bool(arguments, "destructive")?.unwrap_or(false),
            target_guard: None,
        },
    ))
}
pub(crate) fn properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "window_id",
            json!({"type":"string","description":"Exact KWin UUID from inventory; mutually exclusive with root_ref."}),
        ),
        (
            "root_ref",
            json!({"type":"string","description":"Drill into a region using its full @sg1:...:eN ref; preserves the snapshot namespace and original expiry."}),
        ),
        (
            "skeleton",
            json!({"type":"boolean","description":"Read three child levels and report cut containers; never build a full tree just to truncate output."}),
        ),
        ("depth", json!({"type":"integer","minimum":0,"maximum":32})),
        (
            "max_nodes",
            json!({"type":"integer","minimum":1,"maximum":4096}),
        ),
        ("interactive_only", json!({"type":"boolean"})),
        ("compact", json!({"type":"boolean"})),
    ]
}
pub(crate) fn tools() -> Vec<Value> {
    vec![
        tool(
            "seatgeist.a11y_snapshot",
            "Accessibility Snapshot",
            "Read a bounded exact-KWin-window tree with owner-bound snapshot-qualified refs. Use skeleton then root_ref to drill; no physical focus or capture chooser.",
            object_schema(properties(), vec![]),
        ),
        tool(
            "seatgeist.a11y_ref_action",
            "Accessibility Ref Action",
            "Invoke one freshly revalidated owner-bound ref through normal daemon policy and journal. Never substitutes another node or raw input. Set destructive=true for destructive operations; inspect delivery/retry before deciding again.",
            object_schema(
                with_post_action_properties(vec![
                    ("ref_id", json!({"type":"string"})),
                    (
                        "ref_action",
                        json!({"type":"string","enum":["press","select","focus","set_text"]}),
                    ),
                    ("text", json!({"type":"string"})),
                    ("destructive", json!({"type":"boolean"})),
                ]),
                vec!["ref_id", "ref_action"],
            ),
        ),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_an_explicit_target_and_bounded_budget() {
        assert!(snapshot(&json!({})).is_err());
        assert!(snapshot(&json!({"window_id":"kwin","root_ref":"@e1"})).is_err());
        assert!(snapshot(&json!({"window_id":"kwin","max_nodes":4097})).is_err());
        assert!(snapshot(&json!({"root_ref":"@sg1:opaque:e1","skeleton":true})).is_ok());
    }
    #[test]
    fn core_paths_expose_structured_snapshot_and_ref_action_without_weakening_image_sessions() {
        let snapshot = super::super::daemon_request_for_tool(
            "seatgeist.snapshot",
            &json!({"format":"accessibility","window_id":"kwin","skeleton":true}),
        )
        .unwrap();
        assert!(matches!(snapshot, DaemonRequest::AccessibilitySnapshot(_)));
        assert!(super::super::daemon_request_for_tool("seatgeist.snapshot", &json!({})).is_err());
        assert!(
            super::super::daemon_request_for_tool(
                "seatgeist.snapshot",
                &json!({"format":"image","root_ref":"@ref"})
            )
            .is_err()
        );
        let action = super::super::daemon_request_for_tool(
            "seatgeist.act",
            &json!({"action":"invoke_ref","ref_id":"@qualified:e1","ref_action":"press"}),
        )
        .unwrap();
        assert!(matches!(action, DaemonRequest::AccessibilityRefAction(_)));
        assert!(super::super::daemon_request_for_tool("seatgeist.act", &json!({"action":"invoke_ref","ref_id":"@ref","ref_action":"press","session_id":"other"})).is_err());
    }
    #[test]
    fn retry_advice_is_visible_in_compact_mcp_error_text() {
        let response = libseatgeist::DaemonResponse::Error {
            kind: libseatgeist::ErrorKind::BackendFailed,
            reason_code: None,
            message: "reply lost".into(),
            disposition: Some(libseatgeist::ActionDisposition::unknown()),
        };
        let text = super::super::compact_tool_text("seatgeist.act", &response);
        assert!(text.contains("delivery=unknown retry=observe"));
    }
}
