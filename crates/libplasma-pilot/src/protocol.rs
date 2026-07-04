use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{
    AccessibilityNode, BackendCapability, CoordinateSpace, MonitorInfo, Observation, SafetyClass,
    ToolApprovalLevel, WindowInfo,
};

pub const DEFAULT_CLIPBOARD_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStatus {
    pub service: String,
    pub version: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub capabilities: Vec<BackendCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyStatus {
    pub default_observe: ToolApprovalLevel,
    pub default_control: ToolApprovalLevel,
    pub default_clipboard_read: ToolApprovalLevel,
    pub default_clipboard_write: ToolApprovalLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub sequence: u64,
    pub unix_time_ms: u64,
    pub method: String,
    pub ok: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotInfo {
    pub path: PathBuf,
    pub backend: String,
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub transform: ScreenshotTransform,
    pub coordinate_space: CoordinateSpace,
    pub monitors: Vec<MonitorInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotTransform {
    pub source_coordinate_space: CoordinateSpace,
    pub output_coordinate_space: CoordinateSpace,
    pub source_origin_x: u32,
    pub source_origin_y: u32,
    pub scale_x: f64,
    pub scale_y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotRequest {
    pub output: PathBuf,
    pub max_edge: Option<u32>,
    pub full_resolution: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotTileRequest {
    pub output: PathBuf,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub max_edge: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalTailRequest {
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObserveRequest {
    pub screenshot: Option<ScreenshotRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardText {
    pub text: String,
    pub truncated: bool,
    pub original_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardGetRequest {
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardSetRequest {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusedAccessibilityTreeRequest {
    pub depth: usize,
    pub max_nodes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityFindRequest {
    pub role: Option<String>,
    pub name_contains: Option<String>,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub depth: usize,
    pub max_results: usize,
    pub max_nodes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopObservation {
    pub active_window: Option<WindowInfo>,
    pub windows: Vec<WindowInfo>,
    pub monitors: Vec<MonitorInfo>,
    pub screenshot: Option<ScreenshotInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusWindowRequest {
    pub window_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum DaemonRequest {
    Health,
    Capabilities,
    PolicyStatus,
    ListMonitors,
    ListWindows,
    ActiveWindow,
    Observe(ObserveRequest),
    Screenshot(ScreenshotRequest),
    ScreenshotTile(ScreenshotTileRequest),
    ClipboardGet(ClipboardGetRequest),
    ClipboardSet(ClipboardSetRequest),
    FocusedAccessibilityTree(FocusedAccessibilityTreeRequest),
    AccessibilityFind(AccessibilityFindRequest),
    JournalTail(JournalTailRequest),
    FocusWindow(FocusWindowRequest),
}

impl DaemonRequest {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Capabilities => "capabilities",
            Self::PolicyStatus => "policy_status",
            Self::ListMonitors => "list_monitors",
            Self::ListWindows => "list_windows",
            Self::ActiveWindow => "active_window",
            Self::Observe(_) => "observe",
            Self::Screenshot(_) => "screenshot",
            Self::ScreenshotTile(_) => "screenshot_tile",
            Self::ClipboardGet(_) => "clipboard_get",
            Self::ClipboardSet(_) => "clipboard_set",
            Self::FocusedAccessibilityTree(_) => "focused_accessibility_tree",
            Self::AccessibilityFind(_) => "accessibility_find",
            Self::JournalTail(_) => "journal_tail",
            Self::FocusWindow(_) => "focus_window",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DaemonResponse {
    Health(HealthStatus),
    Capabilities(CapabilitySet),
    PolicyStatus(PolicyStatus),
    Monitors(Vec<MonitorInfo>),
    Windows(Vec<WindowInfo>),
    ActiveWindow(Option<WindowInfo>),
    Observation(Box<DesktopObservation>),
    Screenshot(ScreenshotInfo),
    ClipboardText(ClipboardText),
    AccessibilityTree(Option<AccessibilityNode>),
    AccessibilityMatches(Vec<AccessibilityNode>),
    Journal(Vec<JournalEntry>),
    Action(Box<ActionResult>),
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub id: Uuid,
    pub tool: String,
    pub safety_class: SafetyClass,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub id: Uuid,
    pub ok: bool,
    pub observation: Option<Observation>,
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_health_request_as_snake_case_method() {
        let encoded =
            serde_json::to_string(&DaemonRequest::Health).expect("health request serializes");
        assert_eq!(encoded, r#"{"method":"health"}"#);
        let decoded: DaemonRequest =
            serde_json::from_str(&encoded).expect("health request deserializes");
        assert_eq!(decoded, DaemonRequest::Health);
    }

    #[test]
    fn serializes_capabilities_response_with_type_tag() {
        let response = DaemonResponse::Capabilities(CapabilitySet {
            capabilities: vec![BackendCapability::DaemonHealth],
        });
        let encoded = serde_json::to_string(&response).expect("capabilities response serializes");
        assert!(encoded.contains(r#""type":"capabilities""#));
        assert!(encoded.contains(r#""daemon_health""#));
    }

    #[test]
    fn serializes_screenshot_request_with_output_path() {
        let request = DaemonRequest::Screenshot(ScreenshotRequest {
            output: PathBuf::from("/tmp/plasma-pilot.png"),
            max_edge: Some(1600),
            full_resolution: false,
        });
        let encoded = serde_json::to_string(&request).expect("screenshot request serializes");
        assert!(encoded.contains(r#""method":"screenshot""#));
        assert!(encoded.contains(r#"/tmp/plasma-pilot.png"#));
        assert!(encoded.contains(r#""max_edge":1600"#));
    }

    #[test]
    fn serializes_monitor_response_with_type_tag() {
        let response = DaemonResponse::Monitors(Vec::new());
        let encoded = serde_json::to_string(&response).expect("monitor response serializes");
        assert_eq!(encoded, r#"{"type":"monitors","data":[]}"#);
    }

    #[test]
    fn serializes_windows_response_with_type_tag() {
        let response = DaemonResponse::Windows(Vec::new());
        let encoded = serde_json::to_string(&response).expect("windows response serializes");
        assert_eq!(encoded, r#"{"type":"windows","data":[]}"#);
    }

    #[test]
    fn serializes_screenshot_tile_request() {
        let request = DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
            output: PathBuf::from("/tmp/plasma-pilot-tile.png"),
            x: 100,
            y: 200,
            width: 800,
            height: 600,
            max_edge: Some(400),
        });
        let encoded = serde_json::to_string(&request).expect("tile request serializes");
        assert!(encoded.contains(r#""method":"screenshot_tile""#));
        assert!(encoded.contains(r#""x":100"#));
        assert!(encoded.contains(r#""max_edge":400"#));
    }

    #[test]
    fn serializes_journal_tail_request() {
        let request = DaemonRequest::JournalTail(JournalTailRequest { limit: 10 });
        let encoded = serde_json::to_string(&request).expect("journal request serializes");
        assert_eq!(encoded, r#"{"method":"journal_tail","limit":10}"#);
    }

    #[test]
    fn serializes_focus_window_request() {
        let request = DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
        });
        let encoded = serde_json::to_string(&request).expect("focus request serializes");
        assert!(encoded.contains(r#""method":"focus_window""#));
        assert!(encoded.contains(r#""window_id":"{96d3c5da-75ec-4a2a-b75f-05c4c077153b}""#));
    }

    #[test]
    fn serializes_observe_request_with_optional_screenshot() {
        let request = DaemonRequest::Observe(ObserveRequest {
            screenshot: Some(ScreenshotRequest {
                output: PathBuf::from("/tmp/observe.png"),
                max_edge: Some(1200),
                full_resolution: false,
            }),
        });
        let encoded = serde_json::to_string(&request).expect("observe request serializes");
        assert!(encoded.contains(r#""method":"observe""#));
        assert!(encoded.contains(r#"/tmp/observe.png"#));
        assert!(encoded.contains(r#""max_edge":1200"#));
    }

    #[test]
    fn serializes_clipboard_requests() {
        let get = DaemonRequest::ClipboardGet(ClipboardGetRequest {
            max_bytes: Some(DEFAULT_CLIPBOARD_MAX_BYTES),
        });
        let encoded = serde_json::to_string(&get).expect("clipboard get request serializes");
        assert_eq!(encoded, r#"{"method":"clipboard_get","max_bytes":65536}"#);

        let set = DaemonRequest::ClipboardSet(ClipboardSetRequest {
            text: "hello".to_string(),
        });
        let encoded = serde_json::to_string(&set).expect("clipboard set request serializes");
        assert_eq!(encoded, r#"{"method":"clipboard_set","text":"hello"}"#);
    }

    #[test]
    fn serializes_focused_accessibility_tree_request() {
        let request = DaemonRequest::FocusedAccessibilityTree(FocusedAccessibilityTreeRequest {
            depth: 2,
            max_nodes: 100,
        });
        let encoded = serde_json::to_string(&request).expect("accessibility request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"focused_accessibility_tree","depth":2,"max_nodes":100}"#
        );
    }

    #[test]
    fn serializes_accessibility_find_request() {
        let request = DaemonRequest::AccessibilityFind(AccessibilityFindRequest {
            role: Some("button".to_string()),
            name_contains: Some("ok".to_string()),
            app: None,
            window_name_contains: None,
            depth: 1,
            max_results: 8,
            max_nodes: 256,
        });
        let encoded = serde_json::to_string(&request).expect("a11y find request serializes");
        assert!(encoded.contains(r#""method":"accessibility_find""#));
        assert!(encoded.contains(r#""role":"button""#));
        assert!(encoded.contains(r#""name_contains":"ok""#));
    }
}
