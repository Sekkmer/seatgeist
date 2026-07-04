use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{
    AccessibilityAction, AccessibilityNode, BackendCapability, CoordinateSpace, MonitorInfo,
    Observation, Point, PointerButton, SafetyClass, ToolApprovalLevel, WindowInfo,
};

pub const DEFAULT_CLIPBOARD_MAX_BYTES: usize = 64 * 1024;
pub const DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS: u64 = 250;
pub const DEFAULT_WAIT_FOR_CHANGE_THRESHOLD: f64 = 0.01;

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
    pub default_destructive_actions: ToolApprovalLevel,
    pub default_secret_fields: ToolApprovalLevel,
    pub default_full_resolution_screenshot: ToolApprovalLevel,
    pub default_clipboard_read: ToolApprovalLevel,
    pub default_clipboard_write: ToolApprovalLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicStopStatus {
    pub enabled: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KwinBridgeStatus {
    pub dbus_service_registered: bool,
    pub active_window_update_seen: bool,
    pub active_window: Option<WindowInfo>,
    pub package_dir: PathBuf,
    pub package_installed: bool,
    pub config_path: PathBuf,
    pub script_enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UinputStatus {
    pub path: PathBuf,
    pub available: bool,
    pub exists: bool,
    pub is_char_device: bool,
    pub mode: Option<u32>,
    pub owner_uid: Option<u32>,
    pub owner_gid: Option<u32>,
    pub process_uid: u32,
    pub process_gid: u32,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputBackendStatus {
    pub uinput_available: bool,
    pub remote_desktop_portal: RemoteDesktopPortalStatus,
    pub libei: LibeiStatus,
    pub preferred_available_backend: Option<String>,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopPortalStatus {
    pub busctl_available: bool,
    pub portal_service_available: bool,
    pub remote_desktop_interface_available: bool,
    pub kde_portal_service_available: bool,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibeiStatus {
    pub pkg_config_available: bool,
    pub client_library_available: bool,
    pub socket_env_present: bool,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointerCalibrationStatus {
    pub coordinate_space: CoordinateSpace,
    pub bounds: PointerPhysicalBounds,
    pub monitors: Vec<PointerMonitorCalibration>,
    pub sample_points: Vec<PointerCalibrationPoint>,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerPhysicalBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointerMonitorCalibration {
    pub id: String,
    pub name: Option<String>,
    pub logical_origin_x: i32,
    pub logical_origin_y: i32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub physical_origin_x: i32,
    pub physical_origin_y: i32,
    pub physical_width: u32,
    pub physical_height: u32,
    pub scale_factor: f64,
    pub transform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerCalibrationPoint {
    pub label: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPanicStopRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub sequence: u64,
    pub unix_time_ms: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_class: Option<SafetyClass>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub guard_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_window_before: Option<JournalWindowContext>,
    pub ok: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalWindowContext {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayTrace {
    pub version: u32,
    pub description: Option<String>,
    pub steps: Vec<TraceStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceStep {
    pub label: Option<String>,
    pub request: DaemonRequest,
    pub expect_response_type: Option<String>,
    pub expect_ok: Option<bool>,
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

impl ScreenshotTransform {
    pub fn output_to_source_point(&self, output_x: f64, output_y: f64) -> Option<Point> {
        if self.scale_x <= 0.0 || self.scale_y <= 0.0 {
            return None;
        }
        Some(Point {
            x: f64::from(self.source_origin_x) + output_x / self.scale_x,
            y: f64::from(self.source_origin_y) + output_y / self.scale_y,
            space: self.source_coordinate_space,
        })
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObserveRequest {
    pub screenshot: Option<ScreenshotRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitForChangeRequest {
    pub output: PathBuf,
    pub max_edge: Option<u32>,
    pub timeout_ms: u64,
    pub interval_ms: u64,
    pub threshold: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitForChangeResult {
    pub changed: bool,
    pub captures: u32,
    pub elapsed_ms: u64,
    pub score: f64,
    pub threshold: f64,
    pub screenshot: ScreenshotInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardText {
    pub text: String,
    pub truncated: bool,
    pub original_bytes: usize,
    pub backend: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityInvokeRequest {
    pub node_id: String,
    pub action: AccessibilityAction,
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilitySetTextRequest {
    pub node_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityInsertTextRequest {
    pub node_id: String,
    pub offset: i32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityDeleteTextRequest {
    pub node_id: String,
    pub start_offset: i32,
    pub end_offset: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityCopyTextRequest {
    pub node_id: String,
    pub start_offset: i32,
    pub end_offset: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityCutTextRequest {
    pub node_id: String,
    pub start_offset: i32,
    pub end_offset: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityPasteTextRequest {
    pub node_id: String,
    pub offset: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveWindowGuard {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeTextRequest {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyComboRequest {
    pub combo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovePointerRequest {
    pub point: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClickPointerRequest {
    pub point: Point,
    pub button: PointerButton,
    pub clicks: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DragPointerRequest {
    pub from: Point,
    pub to: Point,
    pub button: PointerButton,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollPointerRequest {
    pub vertical: i32,
    pub horizontal: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClickButtonRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive: bool,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTextFieldRequest {
    pub name: String,
    pub text: String,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateTabRequest {
    pub name: String,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateLinkRequest {
    pub name: String,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToggleCheckRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetValueRequest {
    pub name: String,
    pub value: f64,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectMenuRequest {
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive: bool,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

fn is_false(value: &bool) -> bool {
    !*value
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum DaemonRequest {
    Health,
    Capabilities,
    PolicyStatus,
    PanicStopStatus,
    SetPanicStop(SetPanicStopRequest),
    KwinBridgeStatus,
    UinputStatus,
    InputBackendStatus,
    PointerCalibration,
    ListMonitors,
    ListWindows,
    ActiveWindow,
    Observe(ObserveRequest),
    Screenshot(ScreenshotRequest),
    ScreenshotTile(ScreenshotTileRequest),
    WaitForChange(WaitForChangeRequest),
    ClipboardGet(ClipboardGetRequest),
    ClipboardSet(ClipboardSetRequest),
    FocusedAccessibilityTree(FocusedAccessibilityTreeRequest),
    AccessibilityFind(AccessibilityFindRequest),
    AccessibilityInvoke(AccessibilityInvokeRequest),
    AccessibilitySetText(AccessibilitySetTextRequest),
    AccessibilityInsertText(AccessibilityInsertTextRequest),
    AccessibilityDeleteText(AccessibilityDeleteTextRequest),
    AccessibilityCopyText(AccessibilityCopyTextRequest),
    AccessibilityCutText(AccessibilityCutTextRequest),
    AccessibilityPasteText(AccessibilityPasteTextRequest),
    TypeText(TypeTextRequest),
    KeyCombo(KeyComboRequest),
    MovePointer(MovePointerRequest),
    ClickPointer(ClickPointerRequest),
    DragPointer(DragPointerRequest),
    ScrollPointer(ScrollPointerRequest),
    ClickButton(ClickButtonRequest),
    SetTextField(SetTextFieldRequest),
    ActivateTab(ActivateTabRequest),
    ActivateLink(ActivateLinkRequest),
    ToggleCheck(ToggleCheckRequest),
    SetValue(SetValueRequest),
    SelectMenu(SelectMenuRequest),
    JournalTail(JournalTailRequest),
    FocusWindow(FocusWindowRequest),
}

impl DaemonRequest {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Capabilities => "capabilities",
            Self::PolicyStatus => "policy_status",
            Self::PanicStopStatus => "panic_stop_status",
            Self::SetPanicStop(_) => "set_panic_stop",
            Self::KwinBridgeStatus => "kwin_bridge_status",
            Self::UinputStatus => "uinput_status",
            Self::InputBackendStatus => "input_backend_status",
            Self::PointerCalibration => "pointer_calibration",
            Self::ListMonitors => "list_monitors",
            Self::ListWindows => "list_windows",
            Self::ActiveWindow => "active_window",
            Self::Observe(_) => "observe",
            Self::Screenshot(_) => "screenshot",
            Self::ScreenshotTile(_) => "screenshot_tile",
            Self::WaitForChange(_) => "wait_for_change",
            Self::ClipboardGet(_) => "clipboard_get",
            Self::ClipboardSet(_) => "clipboard_set",
            Self::FocusedAccessibilityTree(_) => "focused_accessibility_tree",
            Self::AccessibilityFind(_) => "accessibility_find",
            Self::AccessibilityInvoke(_) => "accessibility_invoke",
            Self::AccessibilitySetText(_) => "accessibility_set_text",
            Self::AccessibilityInsertText(_) => "accessibility_insert_text",
            Self::AccessibilityDeleteText(_) => "accessibility_delete_text",
            Self::AccessibilityCopyText(_) => "accessibility_copy_text",
            Self::AccessibilityCutText(_) => "accessibility_cut_text",
            Self::AccessibilityPasteText(_) => "accessibility_paste_text",
            Self::TypeText(_) => "type_text",
            Self::KeyCombo(_) => "key_combo",
            Self::MovePointer(_) => "move_pointer",
            Self::ClickPointer(_) => "click_pointer",
            Self::DragPointer(_) => "drag_pointer",
            Self::ScrollPointer(_) => "scroll_pointer",
            Self::ClickButton(_) => "click_button",
            Self::SetTextField(_) => "set_text_field",
            Self::ActivateTab(_) => "activate_tab",
            Self::ActivateLink(_) => "activate_link",
            Self::ToggleCheck(_) => "toggle_check",
            Self::SetValue(_) => "set_value",
            Self::SelectMenu(_) => "select_menu",
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
    PanicStop(PanicStopStatus),
    KwinBridgeStatus(KwinBridgeStatus),
    UinputStatus(UinputStatus),
    InputBackendStatus(InputBackendStatus),
    PointerCalibration(PointerCalibrationStatus),
    Monitors(Vec<MonitorInfo>),
    Windows(Vec<WindowInfo>),
    ActiveWindow(Option<WindowInfo>),
    Observation(Box<DesktopObservation>),
    Screenshot(ScreenshotInfo),
    WaitForChange(Box<WaitForChangeResult>),
    ClipboardText(ClipboardText),
    AccessibilityTree(Option<AccessibilityNode>),
    AccessibilityMatches(Vec<AccessibilityNode>),
    Journal(Vec<JournalEntry>),
    Action(Box<ActionResult>),
    Error { message: String },
}

impl DaemonResponse {
    pub fn response_type(&self) -> &'static str {
        match self {
            Self::Health(_) => "health",
            Self::Capabilities(_) => "capabilities",
            Self::PolicyStatus(_) => "policy_status",
            Self::PanicStop(_) => "panic_stop",
            Self::KwinBridgeStatus(_) => "kwin_bridge_status",
            Self::UinputStatus(_) => "uinput_status",
            Self::InputBackendStatus(_) => "input_backend_status",
            Self::PointerCalibration(_) => "pointer_calibration",
            Self::Monitors(_) => "monitors",
            Self::Windows(_) => "windows",
            Self::ActiveWindow(_) => "active_window",
            Self::Observation(_) => "observation",
            Self::Screenshot(_) => "screenshot",
            Self::WaitForChange(_) => "wait_for_change",
            Self::ClipboardText(_) => "clipboard_text",
            Self::AccessibilityTree(_) => "accessibility_tree",
            Self::AccessibilityMatches(_) => "accessibility_matches",
            Self::Journal(_) => "journal",
            Self::Action(_) => "action",
            Self::Error { .. } => "error",
        }
    }

    pub fn ok(&self) -> bool {
        !matches!(self, Self::Error { .. })
    }
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
    fn serializes_wait_for_change_request() {
        let request = DaemonRequest::WaitForChange(WaitForChangeRequest {
            output: PathBuf::from("/tmp/plasma-pilot-wait.png"),
            max_edge: Some(1200),
            timeout_ms: DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS,
            interval_ms: DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS,
            threshold: DEFAULT_WAIT_FOR_CHANGE_THRESHOLD,
        });
        let encoded = serde_json::to_string(&request).expect("wait request serializes");
        assert!(encoded.contains(r#""method":"wait_for_change""#));
        assert!(encoded.contains(r#""timeout_ms":5000"#));
        assert!(encoded.contains(r#""interval_ms":250"#));
        assert!(encoded.contains(r#""threshold":0.01"#));
    }

    #[test]
    fn serializes_wait_for_change_response_with_type_tag() {
        let response = DaemonResponse::WaitForChange(Box::new(WaitForChangeResult {
            changed: true,
            captures: 3,
            elapsed_ms: 500,
            score: 0.05,
            threshold: 0.01,
            screenshot: ScreenshotInfo {
                path: PathBuf::from("/tmp/plasma-pilot-wait.png"),
                backend: "spectacle".to_string(),
                source_width: 7680,
                source_height: 4320,
                output_width: 1600,
                output_height: 900,
                transform: ScreenshotTransform {
                    source_coordinate_space: CoordinateSpace::PhysicalPixel,
                    output_coordinate_space: CoordinateSpace::PhysicalPixel,
                    source_origin_x: 0,
                    source_origin_y: 0,
                    scale_x: 1600.0 / 7680.0,
                    scale_y: 900.0 / 4320.0,
                },
                coordinate_space: CoordinateSpace::PhysicalPixel,
                monitors: Vec::new(),
            },
        }));
        let encoded = serde_json::to_string(&response).expect("wait response serializes");
        assert!(encoded.contains(r#""type":"wait_for_change""#));
        assert!(encoded.contains(r#""changed":true"#));
        assert_eq!(response.response_type(), "wait_for_change");
    }

    #[test]
    fn screenshot_transform_maps_8k_preview_to_source_pixels() {
        let transform = ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: 1600.0 / 7680.0,
            scale_y: 900.0 / 4320.0,
        };

        let point = transform
            .output_to_source_point(800.0, 450.0)
            .expect("positive scale maps output point");
        assert_close(point.x, 3840.0);
        assert_close(point.y, 2160.0);
        assert_eq!(point.space, CoordinateSpace::PhysicalPixel);
    }

    #[test]
    fn screenshot_transform_maps_tile_preview_to_source_pixels() {
        let transform = ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x: 3200,
            source_origin_y: 1600,
            scale_x: 0.5,
            scale_y: 0.5,
        };

        let point = transform
            .output_to_source_point(400.0, 300.0)
            .expect("positive scale maps output point");
        assert_close(point.x, 4000.0);
        assert_close(point.y, 2200.0);
        assert_eq!(point.space, CoordinateSpace::PhysicalPixel);
    }

    #[test]
    fn screenshot_transform_rejects_zero_scale_mapping() {
        let transform = ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: 0.0,
            scale_y: 1.0,
        };

        assert_eq!(transform.output_to_source_point(1.0, 1.0), None);
    }

    #[test]
    fn serializes_journal_tail_request() {
        let request = DaemonRequest::JournalTail(JournalTailRequest {
            limit: 10,
            method_filter: None,
            ok: None,
        });
        let encoded = serde_json::to_string(&request).expect("journal request serializes");
        assert_eq!(encoded, r#"{"method":"journal_tail","limit":10}"#);
    }

    #[test]
    fn serializes_filtered_journal_tail_request() {
        let request = DaemonRequest::JournalTail(JournalTailRequest {
            limit: 10,
            method_filter: Some("focus_window".to_string()),
            ok: Some(false),
        });
        let encoded = serde_json::to_string(&request).expect("filtered journal request serializes");
        assert!(encoded.contains(r#""method":"journal_tail""#));
        assert!(encoded.contains(r#""limit":10"#));
        assert!(encoded.contains(r#""method_filter":"focus_window""#));
        assert!(encoded.contains(r#""ok":false"#));
    }

    #[test]
    fn parses_legacy_journal_entry_without_context() {
        let entry = serde_json::from_str::<JournalEntry>(
            r#"{"sequence":1,"unix_time_ms":1000,"method":"health","ok":true,"summary":"ok"}"#,
        )
        .expect("legacy journal entry parses");
        assert_eq!(entry.safety_class, None);
        assert!(!entry.guard_present);
        assert_eq!(entry.active_window_before, None);
    }

    #[test]
    fn serializes_journal_entry_context() {
        let entry = JournalEntry {
            sequence: 2,
            unix_time_ms: 1001,
            method: "focus_window".to_string(),
            safety_class: Some(SafetyClass::ControlSemantic),
            guard_present: true,
            active_window_before: Some(JournalWindowContext {
                id: "window-1".to_string(),
                app_id: Some("org.kde.kate".to_string()),
                title: "main.rs".to_string(),
                monitor_id: Some("main".to_string()),
            }),
            ok: false,
            summary: "policy denied".to_string(),
        };
        let encoded = serde_json::to_string(&entry).expect("journal context serializes");
        assert!(encoded.contains(r#""safety_class":"control_semantic""#));
        assert!(encoded.contains(r#""guard_present":true"#));
        assert!(encoded.contains(r#""active_window_before""#));
        assert!(encoded.contains(r#""app_id":"org.kde.kate""#));
    }

    #[test]
    fn serializes_panic_stop_requests() {
        let status = DaemonRequest::PanicStopStatus;
        assert_eq!(
            serde_json::to_string(&status).expect("panic status serializes"),
            r#"{"method":"panic_stop_status"}"#
        );

        let set = DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: true });
        assert_eq!(
            serde_json::to_string(&set).expect("panic set serializes"),
            r#"{"method":"set_panic_stop","enabled":true}"#
        );
    }

    #[test]
    fn serializes_pointer_control_requests() {
        let move_pointer = DaemonRequest::MovePointer(MovePointerRequest {
            point: Point {
                x: 3840.0,
                y: 2160.0,
                space: CoordinateSpace::PhysicalPixel,
            },
            guard: None,
        });
        let encoded =
            serde_json::to_string(&move_pointer).expect("move pointer request serializes");
        assert!(encoded.contains(r#""method":"move_pointer""#));
        assert!(encoded.contains(r#""space":"physical_pixel""#));

        let click_pointer = DaemonRequest::ClickPointer(ClickPointerRequest {
            point: Point {
                x: 100.0,
                y: 200.0,
                space: CoordinateSpace::PhysicalPixel,
            },
            button: PointerButton::Left,
            clicks: 2,
            guard: Some(ActiveWindowGuard {
                expected_window_id: Some("current-window".to_string()),
                expected_app_id: None,
                title_contains: None,
            }),
        });
        let encoded =
            serde_json::to_string(&click_pointer).expect("click pointer request serializes");
        assert!(encoded.contains(r#""method":"click_pointer""#));
        assert!(encoded.contains(r#""button":"left""#));
        assert!(encoded.contains(r#""clicks":2"#));
        assert!(encoded.contains(r#""expected_window_id":"current-window""#));

        let drag_pointer = DaemonRequest::DragPointer(DragPointerRequest {
            from: Point {
                x: 100.0,
                y: 200.0,
                space: CoordinateSpace::PhysicalPixel,
            },
            to: Point {
                x: 500.0,
                y: 600.0,
                space: CoordinateSpace::PhysicalPixel,
            },
            button: PointerButton::Left,
            duration_ms: 250,
            guard: None,
        });
        let encoded =
            serde_json::to_string(&drag_pointer).expect("drag pointer request serializes");
        assert!(encoded.contains(r#""method":"drag_pointer""#));
        assert!(encoded.contains(r#""from":{"x":100.0"#));
        assert!(encoded.contains(r#""to":{"x":500.0"#));
        assert!(encoded.contains(r#""duration_ms":250"#));

        let scroll_pointer = DaemonRequest::ScrollPointer(ScrollPointerRequest {
            vertical: -3,
            horizontal: 1,
            guard: None,
        });
        assert_eq!(
            serde_json::to_string(&scroll_pointer).expect("scroll pointer request serializes"),
            r#"{"method":"scroll_pointer","vertical":-3,"horizontal":1}"#
        );
    }

    #[test]
    fn serializes_kwin_bridge_status() {
        let request = DaemonRequest::KwinBridgeStatus;
        assert_eq!(
            serde_json::to_string(&request).expect("bridge status request serializes"),
            r#"{"method":"kwin_bridge_status"}"#
        );

        let response = DaemonResponse::KwinBridgeStatus(KwinBridgeStatus {
            dbus_service_registered: true,
            active_window_update_seen: false,
            active_window: None,
            package_dir: PathBuf::from("/home/user/.local/share/kwin/scripts/plasma-pilot-bridge"),
            package_installed: true,
            config_path: PathBuf::from("/home/user/.config/kwinrc"),
            script_enabled: Some(true),
        });
        let encoded = serde_json::to_string(&response).expect("bridge status response serializes");
        assert!(encoded.contains(r#""type":"kwin_bridge_status""#));
        assert!(encoded.contains(r#""dbus_service_registered":true"#));
        assert_eq!(response.response_type(), "kwin_bridge_status");
    }

    #[test]
    fn serializes_uinput_status() {
        let request = DaemonRequest::UinputStatus;
        assert_eq!(
            serde_json::to_string(&request).expect("uinput status request serializes"),
            r#"{"method":"uinput_status"}"#
        );

        let response = DaemonResponse::UinputStatus(UinputStatus {
            path: PathBuf::from("/dev/uinput"),
            available: true,
            exists: true,
            is_char_device: true,
            mode: Some(0o660),
            owner_uid: Some(0),
            owner_gid: Some(985),
            process_uid: 1000,
            process_gid: 1000,
            setup_hint: "uinput available to daemon process".to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("uinput status serializes");
        assert!(encoded.contains(r#""type":"uinput_status""#));
        assert!(encoded.contains(r#""available":true"#));
        assert_eq!(response.response_type(), "uinput_status");
    }

    #[test]
    fn serializes_input_backend_status() {
        let request = DaemonRequest::InputBackendStatus;
        assert_eq!(
            serde_json::to_string(&request).expect("input backend status request serializes"),
            r#"{"method":"input_backend_status"}"#
        );

        let response = DaemonResponse::InputBackendStatus(InputBackendStatus {
            uinput_available: true,
            remote_desktop_portal: RemoteDesktopPortalStatus {
                busctl_available: true,
                portal_service_available: true,
                remote_desktop_interface_available: true,
                kde_portal_service_available: true,
                setup_hint: "portal remote desktop interface is visible".to_string(),
            },
            libei: LibeiStatus {
                pkg_config_available: true,
                client_library_available: true,
                socket_env_present: false,
                setup_hint: "libei client library is available".to_string(),
            },
            preferred_available_backend: Some("portal_remote_desktop".to_string()),
            setup_hint: "prefer portal RemoteDesktop/libei before uinput".to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("input backend status serializes");
        assert!(encoded.contains(r#""type":"input_backend_status""#));
        assert!(encoded.contains(r#""preferred_available_backend":"portal_remote_desktop""#));
        assert_eq!(response.response_type(), "input_backend_status");
    }

    #[test]
    fn serializes_pointer_calibration() {
        let request = DaemonRequest::PointerCalibration;
        assert_eq!(
            serde_json::to_string(&request).expect("pointer calibration request serializes"),
            r#"{"method":"pointer_calibration"}"#
        );

        let response = DaemonResponse::PointerCalibration(PointerCalibrationStatus {
            coordinate_space: CoordinateSpace::PhysicalPixel,
            bounds: PointerPhysicalBounds {
                min_x: 0,
                min_y: 0,
                max_x: 7679,
                max_y: 4319,
                width: 7680,
                height: 4320,
            },
            monitors: vec![PointerMonitorCalibration {
                id: "HDMI-A-2".to_string(),
                name: Some("HDMI-A-2".to_string()),
                logical_origin_x: 0,
                logical_origin_y: 0,
                logical_width: 5120,
                logical_height: 2880,
                physical_origin_x: 0,
                physical_origin_y: 0,
                physical_width: 7680,
                physical_height: 4320,
                scale_factor: 1.5,
                transform: None,
            }],
            sample_points: vec![PointerCalibrationPoint {
                label: "center".to_string(),
                x: 3840,
                y: 2160,
            }],
            setup_hint:
                "physical_pixel pointer coordinates are calibrated from KWin monitor metadata"
                    .to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("pointer calibration serializes");
        assert!(encoded.contains(r#""type":"pointer_calibration""#));
        assert!(encoded.contains(r#""coordinate_space":"physical_pixel""#));
        assert_eq!(response.response_type(), "pointer_calibration");
    }

    #[test]
    fn serializes_replay_trace_with_daemon_requests() {
        let trace = ReplayTrace {
            version: 1,
            description: Some("status smoke".to_string()),
            steps: vec![
                TraceStep {
                    label: Some("health".to_string()),
                    request: DaemonRequest::Health,
                    expect_response_type: Some("health".to_string()),
                    expect_ok: Some(true),
                },
                TraceStep {
                    label: Some("policy".to_string()),
                    request: DaemonRequest::PolicyStatus,
                    expect_response_type: Some("policy_status".to_string()),
                    expect_ok: Some(true),
                },
            ],
        };

        let encoded = serde_json::to_string(&trace).expect("trace serializes");
        assert!(encoded.contains(r#""version":1"#));
        assert!(encoded.contains(r#""method":"health""#));
        let decoded: ReplayTrace = serde_json::from_str(&encoded).expect("trace deserializes");
        assert_eq!(decoded, trace);
    }

    #[test]
    fn daemon_response_reports_stable_type_and_ok_state() {
        let health = DaemonResponse::Health(HealthStatus {
            service: "plasma-pilotd".to_string(),
            version: "0.1.0".to_string(),
            status: "ok".to_string(),
        });
        assert_eq!(health.response_type(), "health");
        assert!(health.ok());

        let error = DaemonResponse::Error {
            message: "denied".to_string(),
        };
        assert_eq!(error.response_type(), "error");
        assert!(!error.ok());
    }

    #[test]
    fn serializes_focus_window_request() {
        let request = DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("focus request serializes");
        assert!(encoded.contains(r#""method":"focus_window""#));
        assert!(encoded.contains(r#""window_id":"{96d3c5da-75ec-4a2a-b75f-05c4c077153b}""#));
        assert!(!encoded.contains("guard"));
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

        let response = DaemonResponse::ClipboardText(ClipboardText {
            text: "hello".to_string(),
            truncated: false,
            original_bytes: 5,
            backend: "wl-clipboard".to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("clipboard response serializes");
        assert!(encoded.contains(r#""type":"clipboard_text""#));
        assert!(encoded.contains(r#""backend":"wl-clipboard""#));
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

    #[test]
    fn serializes_accessibility_invoke_request() {
        let request = DaemonRequest::AccessibilityInvoke(AccessibilityInvokeRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            action: AccessibilityAction::Press,
            destructive: false,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y invoke request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_invoke","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","action":"press"}"#
        );
    }

    #[test]
    fn serializes_accessibility_set_text_request() {
        let request = DaemonRequest::AccessibilitySetText(AccessibilitySetTextRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            text: "hello".to_string(),
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y set-text request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_set_text","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","text":"hello"}"#
        );
    }

    #[test]
    fn serializes_accessibility_insert_text_request() {
        let request = DaemonRequest::AccessibilityInsertText(AccessibilityInsertTextRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            offset: 5,
            text: "hello".to_string(),
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y insert-text request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_insert_text","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","offset":5,"text":"hello"}"#
        );
    }

    #[test]
    fn serializes_accessibility_delete_text_request() {
        let request = DaemonRequest::AccessibilityDeleteText(AccessibilityDeleteTextRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            start_offset: 2,
            end_offset: 5,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y delete-text request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_delete_text","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","start_offset":2,"end_offset":5}"#
        );
    }

    #[test]
    fn serializes_accessibility_copy_text_request() {
        let request = DaemonRequest::AccessibilityCopyText(AccessibilityCopyTextRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            start_offset: 2,
            end_offset: 5,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y copy-text request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_copy_text","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","start_offset":2,"end_offset":5}"#
        );
    }

    #[test]
    fn serializes_accessibility_cut_text_request() {
        let request = DaemonRequest::AccessibilityCutText(AccessibilityCutTextRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            start_offset: 2,
            end_offset: 5,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y cut-text request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_cut_text","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","start_offset":2,"end_offset":5}"#
        );
    }

    #[test]
    fn serializes_accessibility_paste_text_request() {
        let request = DaemonRequest::AccessibilityPasteText(AccessibilityPasteTextRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            offset: 5,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y paste-text request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_paste_text","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","offset":5}"#
        );
    }

    #[test]
    fn serializes_click_button_request() {
        let request = DaemonRequest::ClickButton(ClickButtonRequest {
            name: "OK".to_string(),
            destructive: false,
            app: Some("kate".to_string()),
            window_name_contains: Some("settings".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("click button request serializes");
        assert!(encoded.contains(r#""method":"click_button""#));
        assert!(encoded.contains(r#""name":"OK""#));
        assert!(encoded.contains(r#""app":"kate""#));
        assert!(encoded.contains(r#""window_name_contains":"settings""#));
    }

    #[test]
    fn serializes_set_text_field_request() {
        let request = DaemonRequest::SetTextField(SetTextFieldRequest {
            name: "Search".to_string(),
            text: "query".to_string(),
            app: Some("kate".to_string()),
            window_name_contains: Some("settings".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("set text field request serializes");
        assert!(encoded.contains(r#""method":"set_text_field""#));
        assert!(encoded.contains(r#""name":"Search""#));
        assert!(encoded.contains(r#""text":"query""#));
        assert!(encoded.contains(r#""app":"kate""#));
    }

    #[test]
    fn serializes_activate_tab_request() {
        let request = DaemonRequest::ActivateTab(ActivateTabRequest {
            name: "General".to_string(),
            app: Some("settings".to_string()),
            window_name_contains: Some("preferences".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("activate tab request serializes");
        assert!(encoded.contains(r#""method":"activate_tab""#));
        assert!(encoded.contains(r#""name":"General""#));
        assert!(encoded.contains(r#""app":"settings""#));
        assert!(encoded.contains(r#""window_name_contains":"preferences""#));
    }

    #[test]
    fn serializes_toggle_check_request() {
        let request = DaemonRequest::ToggleCheck(ToggleCheckRequest {
            name: "Enable feature".to_string(),
            checked: Some(true),
            app: Some("settings".to_string()),
            window_name_contains: Some("preferences".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("toggle check request serializes");
        assert!(encoded.contains(r#""method":"toggle_check""#));
        assert!(encoded.contains(r#""name":"Enable feature""#));
        assert!(encoded.contains(r#""checked":true"#));
        assert!(encoded.contains(r#""app":"settings""#));
    }

    #[test]
    fn serializes_set_value_request() {
        let request = DaemonRequest::SetValue(SetValueRequest {
            name: "Volume".to_string(),
            value: 0.75,
            app: Some("settings".to_string()),
            window_name_contains: Some("sound".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("set value request serializes");
        assert!(encoded.contains(r#""method":"set_value""#));
        assert!(encoded.contains(r#""name":"Volume""#));
        assert!(encoded.contains(r#""value":0.75"#));
        assert!(encoded.contains(r#""app":"settings""#));
    }

    #[test]
    fn serializes_activate_link_request() {
        let request = DaemonRequest::ActivateLink(ActivateLinkRequest {
            name: "Release notes".to_string(),
            app: Some("firefox".to_string()),
            window_name_contains: Some("docs".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("activate link request serializes");
        assert!(encoded.contains(r#""method":"activate_link""#));
        assert!(encoded.contains(r#""name":"Release notes""#));
        assert!(encoded.contains(r#""app":"firefox""#));
    }

    #[test]
    fn serializes_select_menu_request() {
        let request = DaemonRequest::SelectMenu(SelectMenuRequest {
            path: vec!["File".to_string(), "Open".to_string()],
            destructive: false,
            app: Some("kate".to_string()),
            window_name_contains: Some("editor".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("select menu request serializes");
        assert!(encoded.contains(r#""method":"select_menu""#));
        assert!(encoded.contains(r#""path":["File","Open"]"#));
        assert!(encoded.contains(r#""app":"kate""#));
    }

    #[test]
    fn serializes_control_request_with_active_window_guard() {
        let request = DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: "target-window".to_string(),
            guard: Some(ActiveWindowGuard {
                expected_window_id: Some("current-window".to_string()),
                expected_app_id: Some("org.kde.kate".to_string()),
                title_contains: Some("main.rs".to_string()),
            }),
        });
        let encoded = serde_json::to_string(&request).expect("guarded focus request serializes");
        assert!(encoded.contains(r#""method":"focus_window""#));
        assert!(encoded.contains(r#""expected_window_id":"current-window""#));
        assert!(encoded.contains(r#""expected_app_id":"org.kde.kate""#));
        assert!(encoded.contains(r#""title_contains":"main.rs""#));
    }

    #[test]
    fn serializes_keyboard_control_requests() {
        let type_text = DaemonRequest::TypeText(TypeTextRequest {
            text: "hello".to_string(),
            guard: None,
        });
        let encoded = serde_json::to_string(&type_text).expect("type text request serializes");
        assert_eq!(encoded, r#"{"method":"type_text","text":"hello"}"#);

        let key_combo = DaemonRequest::KeyCombo(KeyComboRequest {
            combo: "Ctrl+L".to_string(),
            guard: None,
        });
        let encoded = serde_json::to_string(&key_combo).expect("key combo request serializes");
        assert_eq!(encoded, r#"{"method":"key_combo","combo":"Ctrl+L"}"#);
    }

    #[test]
    fn parses_accessibility_action_names() {
        assert_eq!(
            "click"
                .parse::<AccessibilityAction>()
                .expect("click parses as press"),
            AccessibilityAction::Press
        );
        assert_eq!(
            "set-text"
                .parse::<AccessibilityAction>()
                .expect("hyphenated set-text parses"),
            AccessibilityAction::SetText
        );
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {actual} to be close to {expected}"
        );
    }
}
