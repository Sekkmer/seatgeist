use std::{collections::BTreeMap, path::PathBuf};

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
pub const DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS: u64 = 120_000;

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
pub struct SafetyStatus {
    pub require_focus_guard: bool,
    pub pause_on_human_input: bool,
    pub human_input_activity_file: Option<PathBuf>,
    pub human_input_quiet_ms: u64,
    pub human_input_signal_fresh: bool,
    pub human_input_signal_age_ms: Option<u64>,
    pub control_rate_limit_per_minute: Option<u32>,
    pub preview_max_edge: u32,
    pub tile_max_edge: u32,
    pub screenshot_redaction_count: usize,
    #[serde(default)]
    pub journal_artifact_metadata_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopSessionStatus {
    pub xdg_session_type: Option<String>,
    pub xdg_current_desktop: Option<String>,
    pub desktop_session: Option<String>,
    pub kde_full_session: Option<String>,
    pub kde_session_version: Option<String>,
    pub wayland_display: Option<String>,
    pub display: Option<String>,
    pub dbus_session_bus_address_present: bool,
    pub xdg_runtime_dir_present: bool,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerUseReadinessStatus {
    pub ready_for_observe: bool,
    pub ready_for_screenshot: bool,
    pub ready_for_window_control: bool,
    pub ready_for_keyboard_input: bool,
    pub ready_for_pointer_input: bool,
    pub ready_for_semantic_actions: bool,
    pub ready_for_clipboard_read: bool,
    pub ready_for_clipboard_write: bool,
    pub focus_guard_required: bool,
    pub panic_stop_enabled: bool,
    pub human_input_pause_enabled: bool,
    pub human_input_signal_fresh: bool,
    pub desktop_session_ready: bool,
    pub dbus_session_bus_present: bool,
    pub runtime_dir_present: bool,
    pub capture_backend: Option<String>,
    pub input_backend: Option<String>,
    pub clipboard_read_backend: Option<String>,
    pub clipboard_write_backend: Option<String>,
    pub accessibility_backend: String,
    pub issues: Vec<String>,
    pub next_steps: Vec<String>,
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
    pub window_list_update_seen: bool,
    pub window_count: usize,
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
    pub eis_keymap: XkbKeymapStatus,
    pub configured_backend: String,
    pub preferred_available_backend: Option<String>,
    pub implemented_available_backend: Option<String>,
    pub setup_hint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct XkbKeymapStatus {
    pub source: String,
    pub rules: Option<String>,
    pub model: Option<String>,
    pub layout: Option<String>,
    pub variant: Option<String>,
    pub options: Option<String>,
    pub kde_current_layout: Option<String>,
    pub kde_config_layouts: Option<String>,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDesktopPersistMode {
    DoNotPersist,
    ApplicationLifetime,
    ExplicitlyRevoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopSessionProbeRequest {
    #[serde(default = "default_true")]
    pub keyboard: bool,
    #[serde(default = "default_true")]
    pub pointer: bool,
    #[serde(default)]
    pub touchscreen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist_mode: Option<RemoteDesktopPersistMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_window: Option<String>,
    #[serde(default = "default_remote_desktop_session_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopSessionProbe {
    pub started: bool,
    pub requested_devices: Vec<String>,
    pub selected_devices: Vec<String>,
    pub clipboard_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_request_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_request_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_request_path: Option<String>,
    pub transient_session_closed: bool,
    pub setup_hint: String,
}

pub type RemoteDesktopEisProbeRequest = RemoteDesktopSessionProbeRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopEisProbe {
    pub started: bool,
    pub eis_connected: bool,
    #[serde(default)]
    pub eis_runtime_connected: bool,
    #[serde(default)]
    pub eis_event_count: usize,
    #[serde(default)]
    pub eis_bound_capabilities: Vec<String>,
    #[serde(default)]
    pub eis_resumed_device_count: usize,
    pub requested_devices: Vec<String>,
    pub selected_devices: Vec<String>,
    pub clipboard_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_request_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_request_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_request_path: Option<String>,
    pub eis_fd_closed: bool,
    pub transient_session_closed: bool,
    pub setup_hint: String,
}

pub type RemoteDesktopEisStartRequest = RemoteDesktopSessionProbeRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopEisSessionStatus {
    pub active: bool,
    #[serde(default)]
    pub runtime_connected: bool,
    #[serde(default)]
    pub bound_capabilities: Vec<String>,
    #[serde(default)]
    pub resumed_device_count: usize,
    pub selected_devices: Vec<String>,
    pub clipboard_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_request_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_request_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_request_path: Option<String>,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureBackendStatus {
    pub screenshot_portal: ScreenshotPortalStatus,
    pub kwin_metadata: KwinMetadataStatus,
    pub spectacle: SpectacleStatus,
    pub preferred_available_backend: Option<String>,
    pub implemented_available_backend: Option<String>,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotPortalStatus {
    pub busctl_available: bool,
    pub portal_service_available: bool,
    pub screenshot_interface_available: bool,
    pub screencast_interface_available: bool,
    pub kde_portal_service_available: bool,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KwinMetadataStatus {
    pub busctl_available: bool,
    pub kwin_service_available: bool,
    pub support_information_available: bool,
    pub setup_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpectacleStatus {
    pub command_available: bool,
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
    pub client: Option<JournalClientContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_class: Option<SafetyClass>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub guard_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_window_before: Option<JournalWindowContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_window_after: Option<JournalWindowContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<JournalControlContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<JournalArtifactContext>,
    pub ok: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalClientContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalArtifactContext {
    pub kind: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonClientIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonRequestEnvelope {
    pub request: DaemonRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<DaemonClientIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalControlContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_target: Option<JournalRequestedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRequestedTarget {
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
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
    pub expect_error_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect_json: Vec<TraceJsonExpectation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceJsonExpectation {
    pub pointer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
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
    #[serde(default)]
    pub portal_interactive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotTileRequest {
    pub output: PathBuf,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub max_edge: Option<u32>,
    #[serde(default)]
    pub portal_interactive: bool,
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
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub timeout_ms: u64,
    #[serde(default)]
    pub interval_ms: u64,
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
pub struct ClipboardBackendStatus {
    pub wl_paste_available: bool,
    pub wl_copy_available: bool,
    pub kde_klipper_available: bool,
    pub read_backend: Option<String>,
    pub write_backend: Option<String>,
    pub setup_hint: String,
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
pub struct AccessibilityQualityStatus {
    pub atspi_available: bool,
    pub focused_node_present: bool,
    pub sample_depth: usize,
    pub sample_max_nodes: usize,
    pub sampled_node_count: usize,
    pub named_node_count: usize,
    pub actionable_node_count: usize,
    pub text_node_count: usize,
    pub sensitive_node_count: usize,
    pub generic_role_count: usize,
    pub max_depth_seen: usize,
    pub tree_flat: bool,
    pub semantic_targeting_reliable: bool,
    pub recommended_fallback: String,
    pub setup_hint: String,
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
pub struct AccessibilityTextAttributesRequest {
    pub node_id: String,
    pub offset: i32,
    #[serde(default)]
    pub include_defaults: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextAttribute {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityTextAttributes {
    pub node_id: String,
    pub start_offset: i32,
    pub end_offset: i32,
    pub attributes: Vec<TextAttribute>,
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
pub struct AccessibilitySetCaretRequest {
    pub node_id: String,
    pub offset: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilitySetSelectionRequest {
    pub node_id: String,
    pub selection_num: i32,
    pub start_offset: i32,
    pub end_offset: i32,
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
pub struct FocusTextFieldRequest {
    pub name: String,
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
pub struct SelectItemRequest {
    pub name: String,
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

fn default_true() -> bool {
    true
}

fn default_remote_desktop_session_timeout_ms() -> u64 {
    DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS
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
    SafetyStatus,
    DesktopSessionStatus,
    ComputerUseReadiness,
    PanicStopStatus,
    SetPanicStop(SetPanicStopRequest),
    KwinBridgeStatus,
    UinputStatus,
    InputBackendStatus,
    RemoteDesktopSessionProbe(RemoteDesktopSessionProbeRequest),
    RemoteDesktopEisProbe(RemoteDesktopEisProbeRequest),
    RemoteDesktopEisStart(RemoteDesktopEisStartRequest),
    RemoteDesktopEisSessionStatus,
    RemoteDesktopEisStop,
    CaptureBackendStatus,
    PointerCalibration,
    ListMonitors,
    ListWindows,
    ActiveWindow,
    Observe(ObserveRequest),
    Screenshot(ScreenshotRequest),
    ScreenshotTile(ScreenshotTileRequest),
    WaitForChange(WaitForChangeRequest),
    ClipboardBackendStatus,
    ClipboardGet(ClipboardGetRequest),
    ClipboardSet(ClipboardSetRequest),
    AccessibilityQualityStatus,
    FocusedAccessibilityTree(FocusedAccessibilityTreeRequest),
    AccessibilityFind(AccessibilityFindRequest),
    AccessibilityTextAttributes(AccessibilityTextAttributesRequest),
    AccessibilityInvoke(AccessibilityInvokeRequest),
    AccessibilitySetText(AccessibilitySetTextRequest),
    AccessibilityInsertText(AccessibilityInsertTextRequest),
    AccessibilityDeleteText(AccessibilityDeleteTextRequest),
    AccessibilityCopyText(AccessibilityCopyTextRequest),
    AccessibilityCutText(AccessibilityCutTextRequest),
    AccessibilityPasteText(AccessibilityPasteTextRequest),
    AccessibilitySetCaret(AccessibilitySetCaretRequest),
    AccessibilitySetSelection(AccessibilitySetSelectionRequest),
    TypeText(TypeTextRequest),
    KeyCombo(KeyComboRequest),
    MovePointer(MovePointerRequest),
    ClickPointer(ClickPointerRequest),
    DragPointer(DragPointerRequest),
    ScrollPointer(ScrollPointerRequest),
    ClickButton(ClickButtonRequest),
    SetTextField(SetTextFieldRequest),
    FocusTextField(FocusTextFieldRequest),
    ActivateTab(ActivateTabRequest),
    ActivateLink(ActivateLinkRequest),
    ToggleCheck(ToggleCheckRequest),
    SetValue(SetValueRequest),
    SelectItem(SelectItemRequest),
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
            Self::SafetyStatus => "safety_status",
            Self::DesktopSessionStatus => "desktop_session_status",
            Self::ComputerUseReadiness => "computer_use_readiness",
            Self::PanicStopStatus => "panic_stop_status",
            Self::SetPanicStop(_) => "set_panic_stop",
            Self::KwinBridgeStatus => "kwin_bridge_status",
            Self::UinputStatus => "uinput_status",
            Self::InputBackendStatus => "input_backend_status",
            Self::RemoteDesktopSessionProbe(_) => "remote_desktop_session_probe",
            Self::RemoteDesktopEisProbe(_) => "remote_desktop_eis_probe",
            Self::RemoteDesktopEisStart(_) => "remote_desktop_eis_start",
            Self::RemoteDesktopEisSessionStatus => "remote_desktop_eis_session_status",
            Self::RemoteDesktopEisStop => "remote_desktop_eis_stop",
            Self::CaptureBackendStatus => "capture_backend_status",
            Self::PointerCalibration => "pointer_calibration",
            Self::ListMonitors => "list_monitors",
            Self::ListWindows => "list_windows",
            Self::ActiveWindow => "active_window",
            Self::Observe(_) => "observe",
            Self::Screenshot(_) => "screenshot",
            Self::ScreenshotTile(_) => "screenshot_tile",
            Self::WaitForChange(_) => "wait_for_change",
            Self::ClipboardBackendStatus => "clipboard_backend_status",
            Self::ClipboardGet(_) => "clipboard_get",
            Self::ClipboardSet(_) => "clipboard_set",
            Self::AccessibilityQualityStatus => "accessibility_quality_status",
            Self::FocusedAccessibilityTree(_) => "focused_accessibility_tree",
            Self::AccessibilityFind(_) => "accessibility_find",
            Self::AccessibilityTextAttributes(_) => "accessibility_text_attributes",
            Self::AccessibilityInvoke(_) => "accessibility_invoke",
            Self::AccessibilitySetText(_) => "accessibility_set_text",
            Self::AccessibilityInsertText(_) => "accessibility_insert_text",
            Self::AccessibilityDeleteText(_) => "accessibility_delete_text",
            Self::AccessibilityCopyText(_) => "accessibility_copy_text",
            Self::AccessibilityCutText(_) => "accessibility_cut_text",
            Self::AccessibilityPasteText(_) => "accessibility_paste_text",
            Self::AccessibilitySetCaret(_) => "accessibility_set_caret",
            Self::AccessibilitySetSelection(_) => "accessibility_set_selection",
            Self::TypeText(_) => "type_text",
            Self::KeyCombo(_) => "key_combo",
            Self::MovePointer(_) => "move_pointer",
            Self::ClickPointer(_) => "click_pointer",
            Self::DragPointer(_) => "drag_pointer",
            Self::ScrollPointer(_) => "scroll_pointer",
            Self::ClickButton(_) => "click_button",
            Self::SetTextField(_) => "set_text_field",
            Self::FocusTextField(_) => "focus_text_field",
            Self::ActivateTab(_) => "activate_tab",
            Self::ActivateLink(_) => "activate_link",
            Self::ToggleCheck(_) => "toggle_check",
            Self::SetValue(_) => "set_value",
            Self::SelectItem(_) => "select_item",
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
    SafetyStatus(SafetyStatus),
    DesktopSessionStatus(DesktopSessionStatus),
    ComputerUseReadiness(ComputerUseReadinessStatus),
    PanicStop(PanicStopStatus),
    KwinBridgeStatus(KwinBridgeStatus),
    UinputStatus(UinputStatus),
    InputBackendStatus(InputBackendStatus),
    RemoteDesktopSessionProbe(RemoteDesktopSessionProbe),
    RemoteDesktopEisProbe(RemoteDesktopEisProbe),
    RemoteDesktopEisSessionStatus(RemoteDesktopEisSessionStatus),
    CaptureBackendStatus(CaptureBackendStatus),
    PointerCalibration(PointerCalibrationStatus),
    Monitors(Vec<MonitorInfo>),
    Windows(Vec<WindowInfo>),
    ActiveWindow(Option<WindowInfo>),
    Observation(Box<DesktopObservation>),
    Screenshot(ScreenshotInfo),
    WaitForChange(Box<WaitForChangeResult>),
    ClipboardBackendStatus(ClipboardBackendStatus),
    ClipboardText(ClipboardText),
    AccessibilityQualityStatus(AccessibilityQualityStatus),
    AccessibilityTree(Option<AccessibilityNode>),
    AccessibilityMatches(Vec<AccessibilityNode>),
    AccessibilityTextAttributes(AccessibilityTextAttributes),
    Journal(Vec<JournalEntry>),
    Action(Box<ActionResult>),
    Error { kind: ErrorKind, message: String },
}

impl DaemonResponse {
    pub fn response_type(&self) -> &'static str {
        match self {
            Self::Health(_) => "health",
            Self::Capabilities(_) => "capabilities",
            Self::PolicyStatus(_) => "policy_status",
            Self::SafetyStatus(_) => "safety_status",
            Self::DesktopSessionStatus(_) => "desktop_session_status",
            Self::ComputerUseReadiness(_) => "computer_use_readiness",
            Self::PanicStop(_) => "panic_stop",
            Self::KwinBridgeStatus(_) => "kwin_bridge_status",
            Self::UinputStatus(_) => "uinput_status",
            Self::InputBackendStatus(_) => "input_backend_status",
            Self::RemoteDesktopSessionProbe(_) => "remote_desktop_session_probe",
            Self::RemoteDesktopEisProbe(_) => "remote_desktop_eis_probe",
            Self::RemoteDesktopEisSessionStatus(_) => "remote_desktop_eis_session_status",
            Self::CaptureBackendStatus(_) => "capture_backend_status",
            Self::PointerCalibration(_) => "pointer_calibration",
            Self::Monitors(_) => "monitors",
            Self::Windows(_) => "windows",
            Self::ActiveWindow(_) => "active_window",
            Self::Observation(_) => "observation",
            Self::Screenshot(_) => "screenshot",
            Self::WaitForChange(_) => "wait_for_change",
            Self::ClipboardBackendStatus(_) => "clipboard_backend_status",
            Self::ClipboardText(_) => "clipboard_text",
            Self::AccessibilityQualityStatus(_) => "accessibility_quality_status",
            Self::AccessibilityTree(_) => "accessibility_tree",
            Self::AccessibilityMatches(_) => "accessibility_matches",
            Self::AccessibilityTextAttributes(_) => "accessibility_text_attributes",
            Self::Journal(_) => "journal",
            Self::Action(_) => "action",
            Self::Error { .. } => "error",
        }
    }

    pub fn ok(&self) -> bool {
        !matches!(self, Self::Error { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    PolicyPromptRequired,
    PolicyDenied,
    AppDenied,
    FocusGuard,
    HumanInputPause,
    PanicStop,
    RateLimited,
    PortalUnavailable,
    BackendUnavailable,
    BackendFailed,
    AccessibilityUnavailable,
    AccessibilityWeakTree,
    Validation,
    Unknown,
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
            output: PathBuf::from("/tmp/seatgeist.png"),
            max_edge: Some(1600),
            full_resolution: false,
            portal_interactive: true,
        });
        let encoded = serde_json::to_string(&request).expect("screenshot request serializes");
        assert!(encoded.contains(r#""method":"screenshot""#));
        assert!(encoded.contains(r#"/tmp/seatgeist.png"#));
        assert!(encoded.contains(r#""max_edge":1600"#));
        assert!(encoded.contains(r#""portal_interactive":true"#));
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
            output: PathBuf::from("/tmp/seatgeist-tile.png"),
            x: 100,
            y: 200,
            width: 800,
            height: 600,
            max_edge: Some(400),
            portal_interactive: true,
        });
        let encoded = serde_json::to_string(&request).expect("tile request serializes");
        assert!(encoded.contains(r#""method":"screenshot_tile""#));
        assert!(encoded.contains(r#""x":100"#));
        assert!(encoded.contains(r#""max_edge":400"#));
        assert!(encoded.contains(r#""portal_interactive":true"#));
    }

    #[test]
    fn serializes_wait_for_change_request() {
        let request = DaemonRequest::WaitForChange(WaitForChangeRequest {
            output: PathBuf::from("/tmp/seatgeist-wait.png"),
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
            timed_out: false,
            timeout_ms: DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS,
            interval_ms: DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS,
            captures: 3,
            elapsed_ms: 500,
            score: 0.05,
            threshold: 0.01,
            screenshot: ScreenshotInfo {
                path: PathBuf::from("/tmp/seatgeist-wait.png"),
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
        assert!(encoded.contains(r#""timed_out":false"#));
        assert!(encoded.contains(r#""timeout_ms":5000"#));
        assert!(encoded.contains(r#""interval_ms":250"#));
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
        assert_eq!(entry.client, None);
        assert!(!entry.guard_present);
        assert_eq!(entry.active_window_before, None);
        assert_eq!(entry.active_window_after, None);
        assert_eq!(entry.control, None);
        assert!(entry.artifacts.is_empty());
    }

    #[test]
    fn serializes_journal_entry_context() {
        let entry = JournalEntry {
            sequence: 2,
            unix_time_ms: 1001,
            method: "focus_window".to_string(),
            client: Some(JournalClientContext {
                tool: Some("seatgeist-mcp".to_string()),
                pid: Some(4242),
                process_name: Some("seatgeist-cl".to_string()),
            }),
            safety_class: Some(SafetyClass::ControlSemantic),
            guard_present: true,
            active_window_before: Some(JournalWindowContext {
                id: "window-1".to_string(),
                app_id: Some("org.kde.kate".to_string()),
                title: "main.rs".to_string(),
                monitor_id: Some("main".to_string()),
            }),
            active_window_after: Some(JournalWindowContext {
                id: "window-2".to_string(),
                app_id: Some("org.kde.konsole".to_string()),
                title: "shell".to_string(),
                monitor_id: Some("main".to_string()),
            }),
            control: Some(JournalControlContext {
                action_id: Some(Uuid::nil()),
                policy: Some("allow".to_string()),
                backend: Some("kwin".to_string()),
                requested_target: Some(JournalRequestedTarget {
                    kind: "window".to_string(),
                    fields: BTreeMap::from([("window_id".to_string(), "window-1".to_string())]),
                }),
            }),
            artifacts: vec![JournalArtifactContext {
                kind: "screenshot".to_string(),
                path: PathBuf::from("/tmp/seatgeist-preview.png"),
                sha256: Some("a".repeat(64)),
                bytes: Some(1024),
            }],
            ok: false,
            summary: "policy denied".to_string(),
        };
        let encoded = serde_json::to_string(&entry).expect("journal context serializes");
        assert!(encoded.contains(r#""safety_class":"control_semantic""#));
        assert!(encoded.contains(r#""client""#));
        assert!(encoded.contains(r#""tool":"seatgeist-mcp""#));
        assert!(encoded.contains(r#""pid":4242"#));
        assert!(encoded.contains(r#""process_name":"seatgeist-cl""#));
        assert!(encoded.contains(r#""guard_present":true"#));
        assert!(encoded.contains(r#""active_window_before""#));
        assert!(encoded.contains(r#""active_window_after""#));
        assert!(encoded.contains(r#""app_id":"org.kde.kate""#));
        assert!(encoded.contains(r#""app_id":"org.kde.konsole""#));
        assert!(encoded.contains(r#""control""#));
        assert!(encoded.contains(r#""policy":"allow""#));
        assert!(encoded.contains(r#""backend":"kwin""#));
        assert!(encoded.contains(r#""requested_target""#));
        assert!(encoded.contains(r#""artifacts""#));
        assert!(encoded.contains(r#""kind":"screenshot""#));
        assert!(encoded.contains(r#""sha256""#));
        assert!(encoded.contains(r#""bytes":1024"#));
    }

    #[test]
    fn serializes_daemon_request_envelope_with_client_identity() {
        let envelope = DaemonRequestEnvelope {
            request: DaemonRequest::Health,
            client: Some(DaemonClientIdentity {
                tool: Some("seatgeist-mcp".to_string()),
            }),
        };
        let encoded = serde_json::to_string(&envelope).expect("envelope serializes");
        assert!(encoded.contains(r#""request":{"method":"health"}"#));
        assert!(encoded.contains(r#""client":{"tool":"seatgeist-mcp"}"#));

        let decoded: DaemonRequestEnvelope =
            serde_json::from_str(&encoded).expect("envelope deserializes");
        assert_eq!(decoded.request, DaemonRequest::Health);
        assert_eq!(
            decoded.client.and_then(|client| client.tool),
            Some("seatgeist-mcp".to_string())
        );
    }

    #[test]
    fn serializes_panic_stop_requests() {
        let safety = DaemonRequest::SafetyStatus;
        assert_eq!(
            serde_json::to_string(&safety).expect("safety status serializes"),
            r#"{"method":"safety_status"}"#
        );

        let response = DaemonResponse::SafetyStatus(SafetyStatus {
            require_focus_guard: true,
            pause_on_human_input: true,
            human_input_activity_file: Some(PathBuf::from("/run/user/1000/seatgeist/human")),
            human_input_quiet_ms: 1500,
            human_input_signal_fresh: false,
            human_input_signal_age_ms: Some(3000),
            control_rate_limit_per_minute: Some(120),
            preview_max_edge: 1600,
            tile_max_edge: 1600,
            screenshot_redaction_count: 2,
            journal_artifact_metadata_enabled: true,
        });
        let encoded = serde_json::to_string(&response).expect("safety response serializes");
        assert!(encoded.contains(r#""type":"safety_status""#));
        assert!(encoded.contains(r#""require_focus_guard":true"#));
        assert!(encoded.contains(r#""control_rate_limit_per_minute":120"#));
        assert!(encoded.contains(r#""preview_max_edge":1600"#));
        assert!(encoded.contains(r#""tile_max_edge":1600"#));
        assert!(encoded.contains(r#""journal_artifact_metadata_enabled":true"#));
        assert_eq!(response.response_type(), "safety_status");

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
            window_list_update_seen: false,
            window_count: 0,
            active_window: None,
            package_dir: PathBuf::from("/home/user/.local/share/kwin/scripts/seatgeist-bridge"),
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
            eis_keymap: XkbKeymapStatus {
                source: "kde_current_layout".to_string(),
                rules: None,
                model: Some("pc105".to_string()),
                layout: Some("us".to_string()),
                variant: None,
                options: Some("".to_string()),
                kde_current_layout: Some("us".to_string()),
                kde_config_layouts: Some("us,de".to_string()),
                setup_hint: "using KDE current keyboard layout for EIS key combos".to_string(),
            },
            configured_backend: "portal_remote_desktop".to_string(),
            preferred_available_backend: Some("portal_remote_desktop".to_string()),
            implemented_available_backend: Some("uinput".to_string()),
            setup_hint: "prefer portal RemoteDesktop/libei before uinput".to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("input backend status serializes");
        assert!(encoded.contains(r#""type":"input_backend_status""#));
        assert!(encoded.contains(r#""configured_backend":"portal_remote_desktop""#));
        assert!(encoded.contains(r#""preferred_available_backend":"portal_remote_desktop""#));
        assert!(encoded.contains(r#""implemented_available_backend":"uinput""#));
        assert!(encoded.contains(r#""source":"kde_current_layout""#));
        assert!(encoded.contains(r#""layout":"us""#));
        assert_eq!(response.response_type(), "input_backend_status");
    }

    #[test]
    fn serializes_remote_desktop_session_probe() {
        let request = DaemonRequest::RemoteDesktopSessionProbe(RemoteDesktopSessionProbeRequest {
            keyboard: true,
            pointer: true,
            touchscreen: false,
            restore_token: Some("restore_once".to_string()),
            persist_mode: Some(RemoteDesktopPersistMode::ApplicationLifetime),
            parent_window: Some("wayland:app-window".to_string()),
            timeout_ms: 30_000,
            guard: Some(ActiveWindowGuard {
                expected_window_id: Some("current-window".to_string()),
                expected_app_id: None,
                title_contains: None,
            }),
        });
        let encoded =
            serde_json::to_string(&request).expect("remote desktop probe request serializes");
        assert!(encoded.contains(r#""method":"remote_desktop_session_probe""#));
        assert!(encoded.contains(r#""persist_mode":"application_lifetime""#));
        assert!(encoded.contains(r#""expected_window_id":"current-window""#));

        let response = DaemonResponse::RemoteDesktopSessionProbe(RemoteDesktopSessionProbe {
            started: true,
            requested_devices: vec!["keyboard".to_string(), "pointer".to_string()],
            selected_devices: vec!["pointer".to_string()],
            clipboard_enabled: false,
            restore_token: Some("restore_next".to_string()),
            session_handle: Some("/org/freedesktop/portal/desktop/session/1_42/p".to_string()),
            create_request_path: None,
            select_request_path: None,
            start_request_path: None,
            transient_session_closed: true,
            setup_hint: "transient probe completed".to_string(),
        });
        let encoded =
            serde_json::to_string(&response).expect("remote desktop probe response serializes");
        assert!(encoded.contains(r#""type":"remote_desktop_session_probe""#));
        assert!(encoded.contains(r#""selected_devices":["pointer"]"#));
        assert_eq!(response.response_type(), "remote_desktop_session_probe");
    }

    #[test]
    fn serializes_remote_desktop_eis_probe() {
        let request = DaemonRequest::RemoteDesktopEisProbe(RemoteDesktopEisProbeRequest {
            keyboard: true,
            pointer: true,
            touchscreen: false,
            restore_token: None,
            persist_mode: Some(RemoteDesktopPersistMode::DoNotPersist),
            parent_window: None,
            timeout_ms: 30_000,
            guard: Some(ActiveWindowGuard {
                expected_window_id: None,
                expected_app_id: Some("org.kde.kwrite".to_string()),
                title_contains: Some("scratch".to_string()),
            }),
        });
        let encoded =
            serde_json::to_string(&request).expect("remote desktop EIS probe request serializes");
        assert!(encoded.contains(r#""method":"remote_desktop_eis_probe""#));
        assert!(encoded.contains(r#""persist_mode":"do_not_persist""#));
        assert!(encoded.contains(r#""expected_app_id":"org.kde.kwrite""#));

        let response = DaemonResponse::RemoteDesktopEisProbe(RemoteDesktopEisProbe {
            started: true,
            eis_connected: true,
            eis_runtime_connected: true,
            eis_event_count: 3,
            eis_bound_capabilities: vec!["text".to_string()],
            eis_resumed_device_count: 1,
            requested_devices: vec!["keyboard".to_string(), "pointer".to_string()],
            selected_devices: vec!["keyboard".to_string(), "pointer".to_string()],
            clipboard_enabled: false,
            restore_token: None,
            session_handle: Some("/org/freedesktop/portal/desktop/session/1_42/p".to_string()),
            create_request_path: None,
            select_request_path: None,
            start_request_path: None,
            eis_fd_closed: true,
            transient_session_closed: true,
            setup_hint: "EIS probe completed without input".to_string(),
        });
        let encoded =
            serde_json::to_string(&response).expect("remote desktop EIS probe response serializes");
        assert!(encoded.contains(r#""type":"remote_desktop_eis_probe""#));
        assert!(encoded.contains(r#""eis_connected":true"#));
        assert!(encoded.contains(r#""eis_runtime_connected":true"#));
        assert!(encoded.contains(r#""eis_event_count":3"#));
        assert!(encoded.contains(r#""eis_bound_capabilities":["text"]"#));
        assert!(encoded.contains(r#""eis_resumed_device_count":1"#));
        assert!(encoded.contains(r#""eis_fd_closed":true"#));
        assert_eq!(response.response_type(), "remote_desktop_eis_probe");
    }

    #[test]
    fn serializes_remote_desktop_eis_session_lifecycle() {
        let start = DaemonRequest::RemoteDesktopEisStart(RemoteDesktopEisStartRequest {
            keyboard: true,
            pointer: true,
            touchscreen: false,
            restore_token: Some("restore".to_string()),
            persist_mode: Some(RemoteDesktopPersistMode::ApplicationLifetime),
            parent_window: None,
            timeout_ms: 30_000,
            guard: None,
        });
        let encoded =
            serde_json::to_string(&start).expect("remote desktop EIS start request serializes");
        assert!(encoded.contains(r#""method":"remote_desktop_eis_start""#));
        assert!(encoded.contains(r#""persist_mode":"application_lifetime""#));
        assert_eq!(start.method_name(), "remote_desktop_eis_start");

        assert_eq!(
            serde_json::to_string(&DaemonRequest::RemoteDesktopEisSessionStatus)
                .expect("remote desktop EIS status request serializes"),
            r#"{"method":"remote_desktop_eis_session_status"}"#
        );
        assert_eq!(
            serde_json::to_string(&DaemonRequest::RemoteDesktopEisStop)
                .expect("remote desktop EIS stop request serializes"),
            r#"{"method":"remote_desktop_eis_stop"}"#
        );

        let response =
            DaemonResponse::RemoteDesktopEisSessionStatus(RemoteDesktopEisSessionStatus {
                active: true,
                runtime_connected: true,
                bound_capabilities: vec!["text".to_string()],
                resumed_device_count: 1,
                selected_devices: vec!["keyboard".to_string()],
                clipboard_enabled: false,
                restore_token: Some("restore-next".to_string()),
                session_handle: Some("/org/freedesktop/portal/desktop/session/1_42/p".to_string()),
                create_request_path: None,
                select_request_path: None,
                start_request_path: None,
                setup_hint: "stored session active".to_string(),
            });
        let encoded =
            serde_json::to_string(&response).expect("remote desktop EIS status serializes");
        assert!(encoded.contains(r#""type":"remote_desktop_eis_session_status""#));
        assert!(encoded.contains(r#""active":true"#));
        assert!(encoded.contains(r#""bound_capabilities":["text"]"#));
        assert_eq!(
            response.response_type(),
            "remote_desktop_eis_session_status"
        );
    }

    #[test]
    fn serializes_desktop_session_status() {
        let request = DaemonRequest::DesktopSessionStatus;
        assert_eq!(
            serde_json::to_string(&request).expect("desktop session status request serializes"),
            r#"{"method":"desktop_session_status"}"#
        );

        let response = DaemonResponse::DesktopSessionStatus(DesktopSessionStatus {
            xdg_session_type: Some("wayland".to_string()),
            xdg_current_desktop: Some("KDE".to_string()),
            desktop_session: Some("plasma".to_string()),
            kde_full_session: Some("true".to_string()),
            kde_session_version: Some("6".to_string()),
            wayland_display: Some("wayland-0".to_string()),
            display: Some(":0".to_string()),
            dbus_session_bus_address_present: true,
            xdg_runtime_dir_present: true,
            setup_hint: "KDE Wayland session detected".to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("desktop status serializes");
        assert!(encoded.contains(r#""type":"desktop_session_status""#));
        assert!(encoded.contains(r#""xdg_session_type":"wayland""#));
        assert_eq!(response.response_type(), "desktop_session_status");
    }

    #[test]
    fn serializes_computer_use_readiness_status() {
        let request = DaemonRequest::ComputerUseReadiness;
        assert_eq!(
            serde_json::to_string(&request).expect("readiness request serializes"),
            r#"{"method":"computer_use_readiness"}"#
        );
        assert_eq!(request.method_name(), "computer_use_readiness");

        let response = DaemonResponse::ComputerUseReadiness(ComputerUseReadinessStatus {
            ready_for_observe: true,
            ready_for_screenshot: true,
            ready_for_window_control: false,
            ready_for_keyboard_input: false,
            ready_for_pointer_input: false,
            ready_for_semantic_actions: true,
            ready_for_clipboard_read: false,
            ready_for_clipboard_write: true,
            focus_guard_required: true,
            panic_stop_enabled: false,
            human_input_pause_enabled: true,
            human_input_signal_fresh: false,
            desktop_session_ready: true,
            dbus_session_bus_present: true,
            runtime_dir_present: true,
            capture_backend: Some("portal_screenshot".to_string()),
            input_backend: None,
            clipboard_read_backend: None,
            clipboard_write_backend: Some("wl-clipboard".to_string()),
            accessibility_backend: "atspi_semantic".to_string(),
            issues: vec!["input backend is not executable".to_string()],
            next_steps: vec!["check seatgeist.input_backend_status".to_string()],
        });
        let encoded = serde_json::to_string(&response).expect("readiness response serializes");
        assert!(encoded.contains(r#""type":"computer_use_readiness""#));
        assert!(encoded.contains(r#""ready_for_observe":true"#));
        assert!(encoded.contains(r#""input backend is not executable""#));
        assert_eq!(response.response_type(), "computer_use_readiness");
    }

    #[test]
    fn serializes_capture_backend_status() {
        let request = DaemonRequest::CaptureBackendStatus;
        assert_eq!(
            serde_json::to_string(&request).expect("capture backend status request serializes"),
            r#"{"method":"capture_backend_status"}"#
        );

        let response = DaemonResponse::CaptureBackendStatus(CaptureBackendStatus {
            screenshot_portal: ScreenshotPortalStatus {
                busctl_available: true,
                portal_service_available: true,
                screenshot_interface_available: true,
                screencast_interface_available: true,
                kde_portal_service_available: true,
                setup_hint: "portal screenshot interface is visible".to_string(),
            },
            kwin_metadata: KwinMetadataStatus {
                busctl_available: true,
                kwin_service_available: true,
                support_information_available: true,
                setup_hint: "KWin support information is available".to_string(),
            },
            spectacle: SpectacleStatus {
                command_available: true,
                setup_hint: "Spectacle command backend is available".to_string(),
            },
            preferred_available_backend: Some("portal_screenshot".to_string()),
            implemented_available_backend: Some("spectacle".to_string()),
            setup_hint: "prefer portal Screenshot before Spectacle fallback".to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("capture backend status serializes");
        assert!(encoded.contains(r#""type":"capture_backend_status""#));
        assert!(encoded.contains(r#""preferred_available_backend":"portal_screenshot""#));
        assert!(encoded.contains(r#""implemented_available_backend":"spectacle""#));
        assert_eq!(response.response_type(), "capture_backend_status");
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
                    expect_error_contains: None,
                    expect_json: vec![TraceJsonExpectation {
                        pointer: "/type".to_string(),
                        equals: Some(serde_json::json!("health")),
                        value_type: None,
                        value_types: Vec::new(),
                        exists: None,
                    }],
                },
                TraceStep {
                    label: Some("policy".to_string()),
                    request: DaemonRequest::PolicyStatus,
                    expect_response_type: Some("policy_status".to_string()),
                    expect_ok: Some(true),
                    expect_error_contains: None,
                    expect_json: Vec::new(),
                },
            ],
        };

        let encoded = serde_json::to_string(&trace).expect("trace serializes");
        assert!(encoded.contains(r#""version":1"#));
        assert!(encoded.contains(r#""method":"health""#));
        assert!(encoded.contains(r#""pointer":"/type""#));
        let decoded: ReplayTrace = serde_json::from_str(&encoded).expect("trace deserializes");
        assert_eq!(decoded, trace);
    }

    #[test]
    fn daemon_response_reports_stable_type_and_ok_state() {
        let health = DaemonResponse::Health(HealthStatus {
            service: "seatgeistd".to_string(),
            version: "0.1.0".to_string(),
            status: "ok".to_string(),
        });
        assert_eq!(health.response_type(), "health");
        assert!(health.ok());

        let error = DaemonResponse::Error {
            kind: ErrorKind::PolicyDenied,
            message: "denied".to_string(),
        };
        assert_eq!(error.response_type(), "error");
        assert!(!error.ok());
        let encoded = serde_json::to_string(&error).expect("error response serializes");
        assert!(encoded.contains(r#""type":"error""#));
        assert!(encoded.contains(r#""kind":"policy_denied""#));
        assert!(encoded.contains(r#""message":"denied""#));
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
                portal_interactive: false,
            }),
        });
        let encoded = serde_json::to_string(&request).expect("observe request serializes");
        assert!(encoded.contains(r#""method":"observe""#));
        assert!(encoded.contains(r#"/tmp/observe.png"#));
        assert!(encoded.contains(r#""max_edge":1200"#));
    }

    #[test]
    fn serializes_clipboard_requests() {
        let status = DaemonRequest::ClipboardBackendStatus;
        let encoded = serde_json::to_string(&status).expect("clipboard status request serializes");
        assert_eq!(encoded, r#"{"method":"clipboard_backend_status"}"#);

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

        let response = DaemonResponse::ClipboardBackendStatus(ClipboardBackendStatus {
            wl_paste_available: true,
            wl_copy_available: false,
            kde_klipper_available: true,
            read_backend: Some("wl-clipboard".to_string()),
            write_backend: Some("kde-klipper".to_string()),
            setup_hint: "clipboard text read/write backends are available".to_string(),
        });
        let encoded =
            serde_json::to_string(&response).expect("clipboard status response serializes");
        assert!(encoded.contains(r#""type":"clipboard_backend_status""#));
        assert!(encoded.contains(r#""wl_paste_available":true"#));
        assert!(encoded.contains(r#""write_backend":"kde-klipper""#));
        assert_eq!(response.response_type(), "clipboard_backend_status");
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
    fn serializes_accessibility_quality_status() {
        let request = DaemonRequest::AccessibilityQualityStatus;
        let encoded =
            serde_json::to_string(&request).expect("a11y quality status request serializes");
        assert_eq!(encoded, r#"{"method":"accessibility_quality_status"}"#);

        let response = DaemonResponse::AccessibilityQualityStatus(AccessibilityQualityStatus {
            atspi_available: true,
            focused_node_present: true,
            sample_depth: 4,
            sample_max_nodes: 512,
            sampled_node_count: 9,
            named_node_count: 4,
            actionable_node_count: 2,
            text_node_count: 1,
            sensitive_node_count: 0,
            generic_role_count: 1,
            max_depth_seen: 3,
            tree_flat: false,
            semantic_targeting_reliable: true,
            recommended_fallback: "atspi_semantic".to_string(),
            setup_hint: "prefer semantic actions".to_string(),
        });
        let encoded = serde_json::to_string(&response).expect("a11y quality response serializes");
        assert!(encoded.contains(r#""type":"accessibility_quality_status""#));
        assert!(encoded.contains(r#""semantic_targeting_reliable":true"#));
        assert_eq!(response.response_type(), "accessibility_quality_status");
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
    fn serializes_accessibility_text_attributes_request() {
        let request =
            DaemonRequest::AccessibilityTextAttributes(AccessibilityTextAttributesRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 4,
                include_defaults: true,
            });
        let encoded =
            serde_json::to_string(&request).expect("a11y text attributes request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_text_attributes","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","offset":4,"include_defaults":true}"#
        );
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
    fn serializes_accessibility_set_caret_request() {
        let request = DaemonRequest::AccessibilitySetCaret(AccessibilitySetCaretRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            offset: 5,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("a11y set-caret request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_set_caret","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","offset":5}"#
        );
    }

    #[test]
    fn serializes_accessibility_set_selection_request() {
        let request = DaemonRequest::AccessibilitySetSelection(AccessibilitySetSelectionRequest {
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
            selection_num: 0,
            start_offset: 2,
            end_offset: 8,
            guard: None,
        });
        let encoded =
            serde_json::to_string(&request).expect("a11y set-selection request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"accessibility_set_selection","node_id":"atspi://:1.42/org/a11y/atspi/accessible/7","selection_num":0,"start_offset":2,"end_offset":8}"#
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
    fn serializes_focus_text_field_request() {
        let request = DaemonRequest::FocusTextField(FocusTextFieldRequest {
            name: "Search".to_string(),
            app: Some("kate".to_string()),
            window_name_contains: Some("settings".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("focus text field request serializes");
        assert!(encoded.contains(r#""method":"focus_text_field""#));
        assert!(encoded.contains(r#""name":"Search""#));
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
    fn serializes_select_item_request() {
        let request = DaemonRequest::SelectItem(SelectItemRequest {
            name: "Printer".to_string(),
            app: Some("systemsettings".to_string()),
            window_name_contains: Some("devices".to_string()),
            max_nodes: 512,
            guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("select item request serializes");
        assert!(encoded.contains(r#""method":"select_item""#));
        assert!(encoded.contains(r#""name":"Printer""#));
        assert!(encoded.contains(r#""app":"systemsettings""#));
        assert!(encoded.contains(r#""window_name_contains":"devices""#));
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
