//! Bounded accessibility observations tied to exact KWin windows.
use crate::{AccessibilityAction, AccessibilityBounds, WindowInfo};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessibilitySnapshotRequest {
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub root_ref: Option<String>,
    #[serde(default)]
    pub skeleton: bool,
    #[serde(default = "default_depth")]
    pub depth: usize,
    #[serde(default = "default_nodes")]
    pub max_nodes: usize,
    #[serde(default)]
    pub interactive_only: bool,
    #[serde(default)]
    pub compact: bool,
}
fn default_depth() -> usize {
    8
}
fn default_nodes() -> usize {
    256
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilitySnapshot {
    pub snapshot_id: String,
    pub window: WindowInfo,
    pub root: AccessibilitySnapshotNode,
    pub complete: bool,
    pub nodes_observed: usize,
    pub ref_count: usize,
    pub expires_in_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilitySnapshotNode {
    pub ref_id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub value_truncated: bool,
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<AccessibilityBounds>,
    pub actions: Vec<AccessibilityAction>,
    pub children_count: usize,
    pub subtree_truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AccessibilitySnapshotNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessibilityRefActionRequest {
    pub ref_id: String,
    pub action: AccessibilityAction,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub destructive: bool,
    /// Filled from the owner-bound store by the daemon; never accepted on wire.
    #[serde(skip)]
    pub target_guard: Option<crate::TargetWindowGuard>,
}

/// Delivery is separate from observation/settle confirmation. None of these
/// values authorizes automatic replay or substitution of another backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputDelivery {
    NotDelivered,
    Delivered,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryAdvice {
    Safe,
    Observe,
    AfterChange,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDisposition {
    pub delivery: InputDelivery,
    pub retry: RetryAdvice,
}
impl ActionDisposition {
    pub const fn not_delivered(retry: RetryAdvice) -> Self {
        Self {
            delivery: InputDelivery::NotDelivered,
            retry,
        }
    }
    pub const fn unknown() -> Self {
        Self {
            delivery: InputDelivery::Unknown,
            retry: RetryAdvice::Observe,
        }
    }
    pub const fn delivered() -> Self {
        Self {
            delivery: InputDelivery::Delivered,
            retry: RetryAdvice::Observe,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DaemonRequest, DaemonResponse};
    #[test]
    fn compact_snapshot_leaves_round_trip_through_daemon_response() {
        let response: DaemonResponse = serde_json::from_value(serde_json::json!({
            "type": "accessibility_snapshot",
            "data": {
                "snapshot_id": "sg1:fixture",
                "window": {
                    "id": "kwin-fixture", "app_id": "fixture", "title": "Fixture",
                    "pid": 123, "monitor_id": null,
                    "geometry": { "x": 0, "y": 0, "width": 100, "height": 100,
                        "space": "logical_pixel" }
                },
                "root": {
                    "ref_id": "@sg1:fixture:e1", "role": "button", "name": "Fixture",
                    "value_truncated": false, "sensitive": false,
                    "actions": ["press"], "children_count": 0,
                    "subtree_truncated": false
                },
                "complete": true, "nodes_observed": 1, "ref_count": 1,
                "expires_in_ms": 300000
            }
        }))
        .unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        assert!(wire["data"]["root"].get("children").is_none());
        assert!(wire["data"]["root"].get("states").is_none());
        assert_eq!(
            serde_json::from_value::<DaemonResponse>(wire).unwrap(),
            response
        );
    }
    #[test]
    fn snapshot_defaults_and_ref_wire_contract_are_explicit() {
        let request: DaemonRequest = serde_json::from_value(
            serde_json::json!({"method":"accessibility_snapshot","window_id":"kwin"}),
        )
        .unwrap();
        let DaemonRequest::AccessibilitySnapshot(snapshot) = request else {
            panic!("snapshot")
        };
        assert_eq!(snapshot.depth, 8);
        assert_eq!(snapshot.max_nodes, 256);
        let request = AccessibilityRefActionRequest {
            ref_id: "@sg1:opaque:e1".into(),
            action: AccessibilityAction::Press,
            text: None,
            destructive: false,
            target_guard: Some(crate::TargetWindowGuard {
                expected_window_id: "kwin".into(),
                expected_app_id: None,
                expected_pid: None,
                title_contains: None,
            }),
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert!(wire.get("target_guard").is_none());
        let decoded: AccessibilityRefActionRequest = serde_json::from_value(wire).unwrap();
        assert!(decoded.target_guard.is_none());
    }
    #[test]
    fn old_response_without_disposition_remains_readable_and_new_advice_round_trips() {
        let old: DaemonResponse = serde_json::from_value(serde_json::json!({"type":"error","data":{"kind":"backend_failed","message":"lost reply"}})).unwrap();
        assert!(old.disposition().is_none());
        let response = DaemonResponse::Error {
            kind: crate::ErrorKind::BackendFailed,
            reason_code: None,
            message: "lost reply".into(),
            disposition: Some(ActionDisposition::unknown()),
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["data"]["disposition"]["delivery"], "unknown");
        assert_eq!(wire["data"]["disposition"]["retry"], "observe");
        assert_eq!(
            serde_json::from_value::<DaemonResponse>(wire).unwrap(),
            response
        );
    }
}
