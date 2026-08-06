use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{
    AccessibilityAction, AccessibilityNode, ActionSettleCondition, ActionSettleResult,
    BackendCapability, CoordinateSpace, MonitorInfo, Observation, Point, PointerButton,
    SafetyClass, ToolApprovalLevel, WindowInfo,
};

pub const DEFAULT_CLIPBOARD_MAX_BYTES: usize = 64 * 1024;
pub const DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS: u64 = 250;
pub const DEFAULT_WAIT_FOR_CHANGE_THRESHOLD: f64 = 0.01;
pub const DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_POST_ACTION_SETTLE_TIMEOUT_MS: u64 = 1_500;
pub const DEFAULT_POST_ACTION_SETTLE_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStatus {
    pub service: String,
    pub version: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resident_memory_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resident_memory_peak_bytes: Option<u64>,
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
    #[serde(default)]
    pub human_input_activity_backend: Option<String>,
    #[serde(default)]
    pub human_input_activity_trusted: bool,
    #[serde(default)]
    pub human_input_last_class: Option<String>,
    #[serde(default)]
    pub human_input_last_provenance: Option<String>,
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
#[serde(rename_all = "snake_case")]
pub enum ActionReadiness {
    Ready,
    NeedsGuard,
    NeedsApproval,
    Blocked,
    Unavailable,
}

impl ActionReadiness {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsGuard => "needs_guard",
            Self::NeedsApproval => "needs_approval",
            Self::Blocked => "blocked",
            Self::Unavailable => "unavailable",
        }
    }
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
    pub observe_state: ActionReadiness,
    pub screenshot_state: ActionReadiness,
    pub window_control_state: ActionReadiness,
    pub keyboard_input_state: ActionReadiness,
    pub pointer_input_state: ActionReadiness,
    pub semantic_action_state: ActionReadiness,
    pub clipboard_read_state: ActionReadiness,
    pub clipboard_write_state: ActionReadiness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_revision: Option<String>,
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
    #[serde(default)]
    pub ownership_retry_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership_retry_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership_last_error: Option<String>,
    #[serde(default)]
    pub window_resize_supported: bool,
    #[serde(default)]
    pub window_move_supported: bool,
    #[serde(default)]
    pub window_launch_supported: bool,
    #[serde(default)]
    pub window_close_supported: bool,
    pub active_window_update_seen: bool,
    pub window_list_update_seen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_window_update_age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_list_update_age_ms: Option<u64>,
    #[serde(default)]
    pub snapshot_stale: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_interface_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_available_targets_mask: Option<u32>,
    #[serde(default)]
    pub screenshot_available_targets: Vec<String>,
    #[serde(default)]
    pub screenshot_target_option_supported: bool,
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
    pub run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_options: Option<DaemonResponseOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonResponseOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_action: Option<PostActionOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostActionOptions {
    #[serde(default)]
    pub observe_after: bool,
    #[serde(default = "default_post_action_settle_condition")]
    pub settle_condition: ActionSettleCondition,
    #[serde(default = "default_post_action_settle_timeout_ms")]
    pub settle_timeout_ms: u64,
    #[serde(default = "default_post_action_settle_interval_ms")]
    pub settle_interval_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<PostActionImageOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostActionImageOptions {
    pub session_id: String,
    pub output: PathBuf,
    pub max_edge: Option<u32>,
    pub timeout_ms: u64,
}

fn default_post_action_settle_condition() -> ActionSettleCondition {
    ActionSettleCondition::Auto
}

fn default_post_action_settle_timeout_ms() -> u64 {
    DEFAULT_POST_ACTION_SETTLE_TIMEOUT_MS
}

fn default_post_action_settle_interval_ms() -> u64 {
    DEFAULT_POST_ACTION_SETTLE_INTERVAL_MS
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub occlusion_possible: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_extent_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_extent_height: Option<u32>,
    pub source_origin_x: u32,
    pub source_origin_y: u32,
    pub scale_x: f64,
    pub scale_y: f64,
}

impl ScreenshotTransform {
    pub fn output_to_source_point(&self, output_x: f64, output_y: f64) -> Option<Point> {
        if !output_x.is_finite()
            || !output_y.is_finite()
            || !self.scale_x.is_finite()
            || !self.scale_y.is_finite()
            || self.scale_x <= 0.0
            || self.scale_y <= 0.0
        {
            return None;
        }
        let point = Point {
            x: f64::from(self.source_origin_x) + output_x / self.scale_x,
            y: f64::from(self.source_origin_y) + output_y / self.scale_y,
            space: self.source_coordinate_space,
        };
        (point.x.is_finite() && point.y.is_finite()).then_some(point)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotRequest {
    pub output: PathBuf,
    pub max_edge: Option<u32>,
    pub full_resolution: bool,
    #[serde(default)]
    pub portal_interactive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_target: Option<PortalScreenshotTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_window_crop_id: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalScreenshotTarget {
    Screen,
    Window,
    Area,
    ActiveWindow,
}

impl PortalScreenshotTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::Window => "window",
            Self::Area => "area",
            Self::ActiveWindow => "active_window",
        }
    }
}

impl fmt::Display for PortalScreenshotTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PortalScreenshotTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "screen" => Ok(Self::Screen),
            "window" => Ok(Self::Window),
            "area" | "region" => Ok(Self::Area),
            "active_window" | "active" => Ok(Self::ActiveWindow),
            other => Err(format!("unsupported portal screenshot target: {other}")),
        }
    }
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
pub struct WindowCaptureOpenRequest {
    pub requested_window_id: Option<String>,
    pub parent_window: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSourceKind {
    Window,
    Monitor,
    VirtualOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureOpenRequest {
    pub source: CaptureSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_source_id: Option<String>,
    pub parent_window: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSnapshotRequest {
    pub session_id: String,
    pub output: PathBuf,
    pub max_edge: Option<u32>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureWaitRequest {
    pub session_id: String,
    pub after_revision: Option<String>,
    pub output: PathBuf,
    pub max_edge: Option<u32>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSessionStatus {
    pub active: bool,
    pub opening: bool,
    pub session_id: Option<String>,
    pub backend: Option<String>,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_token_reference: Option<String>,
    pub requested_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_scope: Option<String>,
    pub latest_revision: Option<String>,
    pub consent_required: bool,
    pub occlusion_possible: bool,
    #[serde(default)]
    pub sticky_target_bound: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_expires_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_end_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Box<SessionExecutionStatus>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFocusLeaseStatus {
    pub lease_id: Uuid,
    pub focus_reacquired: bool,
    pub focus_restored: bool,
    pub restoration: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExecutionStatus {
    pub capture_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action_safety_class: Option<SafetyClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_policy_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_policy_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooperative_focus_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_backend: Option<String>,
    pub activity_trusted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_provenance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_lease: Option<SessionFocusLeaseStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle: Option<ActionSettleResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureFrameResult {
    pub session_id: String,
    pub screenshot: ScreenshotInfo,
    pub revision: String,
    pub sequence: u64,
    pub complete: bool,
    pub damage_present: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureWaitResult {
    pub frame: CaptureFrameResult,
    pub changed: bool,
    pub timed_out: bool,
    pub timeout_ms: u64,
    pub elapsed_ms: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_process_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_registry_process_count: Option<usize>,
    #[serde(default)]
    pub target_event_settle_available: bool,
    #[serde(default)]
    pub event_backend: String,
    #[serde(default)]
    pub target_event_classes: Vec<String>,
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
    pub desktop_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetWindowGuard {
    pub expected_window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeTextRequest {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyComboRequest {
    pub combo: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovePointerRequest {
    pub point: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClickPointerRequest {
    pub point: Point,
    pub button: PointerButton,
    pub clicks: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DragPointerRequest {
    pub from: Point,
    pub to: Point,
    pub button: PointerButton,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollPointerRequest {
    pub vertical: i32,
    pub horizontal: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusTextFieldRequest {
    pub name: String,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateTabRequest {
    pub name: String,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateLinkRequest {
    pub name: String,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectItemRequest {
    pub name: String,
    pub app: Option<String>,
    pub window_name_contains: Option<String>,
    pub max_nodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_guard: Option<TargetWindowGuard>,
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
    pub desktop_revision: String,
    pub active_window: Option<WindowInfo>,
    pub windows: Vec<WindowInfo>,
    pub monitors: Vec<MonitorInfo>,
    pub screenshot: Option<ScreenshotInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowInventory {
    pub revision: String,
    pub active_window: Option<WindowInfo>,
    pub windows: Vec<WindowInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_handles: Vec<SemanticWindowHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticWindowHandle {
    pub handle: String,
    pub window_id: String,
    pub expires_in_ms: u64,
    pub one_shot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInventoryWaitRequest {
    pub after_revision: String,
    #[serde(default = "default_wait_for_change_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_wait_for_change_timeout_ms() -> u64 {
    DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowInventoryWaitResult {
    pub changed: bool,
    pub timed_out: bool,
    pub elapsed_ms: u64,
    pub inventory: WindowInventory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusWindowRequest {
    pub window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseWindowRequest {
    pub window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeWindowRequest {
    pub window_id: String,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveWindowRequest {
    pub window_id: String,
    pub x: i32,
    pub y: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPlacementAnchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowActivationMode {
    PreserveFocus,
    Activate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchWindowRequest {
    pub desktop_entry: String,
    pub anchor: WindowPlacementAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default)]
    pub margin: u32,
    #[serde(default = "default_window_activation_mode")]
    pub activation: WindowActivationMode,
    #[serde(default = "default_launch_window_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<ActiveWindowGuard>,
}

fn default_window_activation_mode() -> WindowActivationMode {
    WindowActivationMode::PreserveFocus
}

fn default_launch_window_timeout_ms() -> u64 {
    10_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageZoomOperation {
    In,
    Out,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageZoomRequest {
    pub operation: PageZoomOperation,
    #[serde(default = "default_page_zoom_steps")]
    pub steps: u8,
    pub guard: ActiveWindowGuard,
}

fn default_page_zoom_steps() -> u8 {
    1
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
    CaptureOpen(CaptureOpenRequest),
    WindowCaptureOpen(WindowCaptureOpenRequest),
    CaptureSessionStatus,
    CaptureSessionRenew(CaptureSessionRequest),
    CaptureSnapshot(CaptureSnapshotRequest),
    CaptureWait(CaptureWaitRequest),
    CaptureSessionClose(CaptureSessionRequest),
    PointerCalibration,
    ListMonitors,
    ListWindows,
    ActiveWindow,
    WindowInventory,
    WindowInventoryWait(WindowInventoryWaitRequest),
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
    CloseWindow(CloseWindowRequest),
    MoveWindow(MoveWindowRequest),
    LaunchWindow(LaunchWindowRequest),
    ResizeWindow(ResizeWindowRequest),
    PageZoom(PageZoomRequest),
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
            Self::CaptureOpen(_) => "capture_open",
            Self::WindowCaptureOpen(_) => "window_capture_open",
            Self::CaptureSessionStatus => "capture_session_status",
            Self::CaptureSessionRenew(_) => "capture_session_renew",
            Self::CaptureSnapshot(_) => "capture_snapshot",
            Self::CaptureWait(_) => "capture_wait",
            Self::CaptureSessionClose(_) => "capture_session_close",
            Self::PointerCalibration => "pointer_calibration",
            Self::ListMonitors => "list_monitors",
            Self::ListWindows => "list_windows",
            Self::ActiveWindow => "active_window",
            Self::WindowInventory => "window_inventory",
            Self::WindowInventoryWait(_) => "window_inventory_wait",
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
            Self::CloseWindow(_) => "close_window",
            Self::MoveWindow(_) => "move_window",
            Self::LaunchWindow(_) => "launch_window",
            Self::ResizeWindow(_) => "resize_window",
            Self::PageZoom(_) => "page_zoom",
        }
    }

    pub fn returns_action(&self) -> bool {
        matches!(
            self,
            Self::ClipboardSet(_)
                | Self::AccessibilityInvoke(_)
                | Self::AccessibilitySetText(_)
                | Self::AccessibilityInsertText(_)
                | Self::AccessibilityDeleteText(_)
                | Self::AccessibilityCopyText(_)
                | Self::AccessibilityCutText(_)
                | Self::AccessibilityPasteText(_)
                | Self::AccessibilitySetCaret(_)
                | Self::AccessibilitySetSelection(_)
                | Self::TypeText(_)
                | Self::KeyCombo(_)
                | Self::MovePointer(_)
                | Self::ClickPointer(_)
                | Self::DragPointer(_)
                | Self::ScrollPointer(_)
                | Self::ClickButton(_)
                | Self::SetTextField(_)
                | Self::FocusTextField(_)
                | Self::ActivateTab(_)
                | Self::ActivateLink(_)
                | Self::ToggleCheck(_)
                | Self::SetValue(_)
                | Self::SelectItem(_)
                | Self::SelectMenu(_)
                | Self::FocusWindow(_)
                | Self::CloseWindow(_)
                | Self::MoveWindow(_)
                | Self::LaunchWindow(_)
                | Self::ResizeWindow(_)
                | Self::PageZoom(_)
        )
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
    CaptureSessionStatus(CaptureSessionStatus),
    CaptureFrame(CaptureFrameResult),
    CaptureWait(Box<CaptureWaitResult>),
    PointerCalibration(PointerCalibrationStatus),
    Monitors(Vec<MonitorInfo>),
    Windows(Vec<WindowInfo>),
    ActiveWindow(Option<WindowInfo>),
    WindowInventory(WindowInventory),
    WindowInventoryWait(WindowInventoryWaitResult),
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
    Error {
        kind: ErrorKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason_code: Option<String>,
        message: String,
    },
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
            Self::CaptureSessionStatus(_) => "capture_session_status",
            Self::CaptureFrame(_) => "capture_frame",
            Self::CaptureWait(_) => "capture_wait",
            Self::PointerCalibration(_) => "pointer_calibration",
            Self::Monitors(_) => "monitors",
            Self::Windows(_) => "windows",
            Self::ActiveWindow(_) => "active_window",
            Self::WindowInventory(_) => "window_inventory",
            Self::WindowInventoryWait(_) => "window_inventory_wait",
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
        match self {
            Self::Error { .. } => false,
            Self::Action(action) => action.ok,
            _ => true,
        }
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
    ConsentCancelled,
    PortalUnavailable,
    BackendUnavailable,
    BackendFailed,
    AccessibilityUnavailable,
    AccessibilityWeakTree,
    TargetMismatch,
    TargetLost,
    SessionOwnerMismatch,
    FocusLeaseConflict,
    Validation,
    Unknown,
}

impl ErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PolicyPromptRequired => "policy_prompt_required",
            Self::PolicyDenied => "policy_denied",
            Self::AppDenied => "app_denied",
            Self::FocusGuard => "focus_guard",
            Self::HumanInputPause => "human_input_pause",
            Self::PanicStop => "panic_stop",
            Self::RateLimited => "rate_limited",
            Self::ConsentCancelled => "consent_cancelled",
            Self::PortalUnavailable => "portal_unavailable",
            Self::BackendUnavailable => "backend_unavailable",
            Self::BackendFailed => "backend_failed",
            Self::AccessibilityUnavailable => "accessibility_unavailable",
            Self::AccessibilityWeakTree => "accessibility_weak_tree",
            Self::TargetMismatch => "target_mismatch",
            Self::TargetLost => "target_lost",
            Self::SessionOwnerMismatch => "session_owner_mismatch",
            Self::FocusLeaseConflict => "focus_lease_conflict",
            Self::Validation => "validation",
            Self::Unknown => "unknown",
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<ScreenshotInfo>,
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
            portal_target: Some(PortalScreenshotTarget::ActiveWindow),
            visible_window_crop_id: None,
        });
        let encoded = serde_json::to_string(&request).expect("screenshot request serializes");
        assert!(encoded.contains(r#""method":"screenshot""#));
        assert!(encoded.contains(r#"/tmp/seatgeist.png"#));
        assert!(encoded.contains(r#""max_edge":1600"#));
        assert!(encoded.contains(r#""portal_interactive":true"#));
        assert!(encoded.contains(r#""portal_target":"active_window""#));

        let crop = DaemonRequest::Screenshot(ScreenshotRequest {
            output: PathBuf::from("/tmp/visible-window.png"),
            max_edge: Some(1200),
            full_resolution: false,
            portal_interactive: false,
            portal_target: None,
            visible_window_crop_id: Some("kwin-window-7".to_string()),
        });
        let encoded = serde_json::to_string(&crop).expect("visible crop request serializes");
        assert!(encoded.contains(r#""visible_window_crop_id":"kwin-window-7""#));
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
    fn serializes_retained_window_capture_lifecycle_requests() {
        let open = DaemonRequest::WindowCaptureOpen(WindowCaptureOpenRequest {
            requested_window_id: Some("kwin-window-7".to_string()),
            parent_window: "wayland:parent".to_string(),
            timeout_ms: 30_000,
        });
        let encoded = serde_json::to_string(&open).expect("capture open serializes");
        assert!(encoded.contains(r#""method":"window_capture_open""#));
        assert!(encoded.contains(r#""requested_window_id":"kwin-window-7""#));

        let monitor = DaemonRequest::CaptureOpen(CaptureOpenRequest {
            source: CaptureSourceKind::Monitor,
            requested_source_id: Some("DP-1".to_string()),
            parent_window: String::new(),
            timeout_ms: 30_000,
        });
        let encoded = serde_json::to_string(&monitor).expect("monitor capture open serializes");
        assert_eq!(monitor.method_name(), "capture_open");
        assert!(encoded.contains(r#""method":"capture_open""#));
        assert!(encoded.contains(r#""source":"monitor""#));
        assert!(encoded.contains(r#""requested_source_id":"DP-1""#));

        let snapshot = DaemonRequest::CaptureSnapshot(CaptureSnapshotRequest {
            session_id: "capture-1".to_string(),
            output: PathBuf::from("/tmp/window.png"),
            max_edge: Some(1200),
            timeout_ms: 1_500,
        });
        let encoded = serde_json::to_string(&snapshot).expect("capture snapshot serializes");
        assert!(encoded.contains(r#""method":"capture_snapshot""#));
        assert!(encoded.contains(r#""session_id":"capture-1""#));
        assert!(encoded.contains(r#""max_edge":1200"#));

        let wait = DaemonRequest::CaptureWait(CaptureWaitRequest {
            session_id: "capture-1".to_string(),
            after_revision: Some("revision-4".to_string()),
            output: PathBuf::from("/tmp/window-wait.png"),
            max_edge: None,
            timeout_ms: 5_000,
        });
        let encoded = serde_json::to_string(&wait).expect("capture wait serializes");
        assert!(encoded.contains(r#""method":"capture_wait""#));
        assert!(encoded.contains(r#""after_revision":"revision-4""#));

        let renew = DaemonRequest::CaptureSessionRenew(CaptureSessionRequest {
            session_id: "capture-1".to_string(),
        });
        let encoded = serde_json::to_string(&renew).expect("capture renew serializes");
        assert_eq!(
            encoded,
            r#"{"method":"capture_session_renew","session_id":"capture-1"}"#
        );
        assert_eq!(renew.method_name(), "capture_session_renew");

        let close = DaemonRequest::CaptureSessionClose(CaptureSessionRequest {
            session_id: "capture-1".to_string(),
        });
        let encoded = serde_json::to_string(&close).expect("capture close serializes");
        assert_eq!(
            encoded,
            r#"{"method":"capture_session_close","session_id":"capture-1"}"#
        );
    }

    #[test]
    fn serializes_retained_capture_response_types() {
        let status = DaemonResponse::CaptureSessionStatus(CaptureSessionStatus {
            active: true,
            opening: false,
            session_id: Some("capture-1".to_string()),
            backend: Some("portal_screencast_pipewire".to_string()),
            source_type: Some("window".to_string()),
            source_id: Some("opaque-source".to_string()),
            restore_token_reference: Some("screencast-a1b2c3d4".to_string()),
            requested_window_id: Some("kwin-window-7".to_string()),
            requested_source_type: Some("window".to_string()),
            requested_source_id: Some("kwin-window-7".to_string()),
            owner_tool: Some("seatgeist-mcp".to_string()),
            owner_pid: Some(4242),
            owner_scope: Some("process".to_string()),
            latest_revision: Some("revision-4".to_string()),
            consent_required: true,
            occlusion_possible: false,
            sticky_target_bound: true,
            target_window_id: Some("kwin-window-7".to_string()),
            target_app_id: Some("org.mozilla.firefox".to_string()),
            target_pid: Some(4242),
            target_expires_in_ms: Some(60_000),
            last_end_reason: None,
            execution: Some(Box::new(SessionExecutionStatus {
                capture_backend: "portal_screencast_pipewire".to_string(),
                semantic_backend: Some("atspi".to_string()),
                raw_input_backend: Some("uinput".to_string()),
                last_action_backend: Some("atspi".to_string()),
                last_action_method: Some("set_text_field".to_string()),
                last_action_safety_class: Some(SafetyClass::ControlKeyboard),
                last_action_id: Some(Uuid::nil()),
                last_action_unix_ms: Some(1_725_000_000_000),
                target_policy_result: Some("allow".to_string()),
                last_policy_result: Some("allow".to_string()),
                cooperative_focus_policy: Some(
                    "reacquire_verify_inject_restore_if_safe".to_string(),
                ),
                activity_backend: Some("kwin_input_spy_v1".to_string()),
                activity_trusted: true,
                last_activity_class: Some("keyboard".to_string()),
                last_activity_provenance: Some("seatgeist_injected".to_string()),
                focus_lease: Some(SessionFocusLeaseStatus {
                    lease_id: Uuid::nil(),
                    focus_reacquired: true,
                    focus_restored: true,
                    restoration: "restored".to_string(),
                }),
                settle: Some(ActionSettleResult {
                    confirmation: crate::types::ActionConfirmation::Confirmed,
                    condition: ActionSettleCondition::AccessibilityChange,
                    backend: crate::types::ActionSettleBackend::AtspiEvent,
                    target_scoped: true,
                    event: Some("object:text-changed".to_string()),
                    settled: true,
                    timed_out: false,
                    timeout_ms: 1_000,
                    interval_ms: 100,
                    samples: 1,
                    elapsed_ms: 12,
                    before_revision: Some("before".to_string()),
                    after_revision: "after".to_string(),
                }),
            })),
        });
        let encoded = serde_json::to_string(&status).expect("capture status serializes");
        assert!(encoded.contains(r#""type":"capture_session_status""#));
        assert!(encoded.contains(r#""restore_token_reference":"screencast-a1b2c3d4""#));
        assert!(encoded.contains(r#""owner_tool":"seatgeist-mcp""#));
        assert!(encoded.contains(r#""owner_scope":"process""#));
        assert!(encoded.contains(r#""owner_pid":4242"#));
        assert!(encoded.contains(r#""capture_backend":"portal_screencast_pipewire""#));
        assert!(encoded.contains(r#""semantic_backend":"atspi""#));
        assert!(encoded.contains(r#""raw_input_backend":"uinput""#));
        assert!(encoded.contains(r#""last_policy_result":"allow""#));
        assert!(encoded.contains(r#""backend":"atspi_event""#));
        assert!(!encoded.contains("last_end_reason"));
        assert!(!encoded.contains("private-restore-token"));
        assert!(!encoded.contains("window_title"));
        assert!(!encoded.contains("input_text"));
        assert_eq!(status.response_type(), "capture_session_status");

        let frame = CaptureFrameResult {
            session_id: "capture-1".to_string(),
            screenshot: ScreenshotInfo {
                path: PathBuf::from("/tmp/window.png"),
                backend: "portal_screencast_pipewire".to_string(),
                occlusion_possible: false,
                source_width: 1280,
                source_height: 720,
                output_width: 640,
                output_height: 360,
                transform: ScreenshotTransform {
                    source_coordinate_space: CoordinateSpace::PhysicalPixel,
                    output_coordinate_space: CoordinateSpace::CaptureOutput,
                    source_extent_width: Some(1280),
                    source_extent_height: Some(720),
                    source_origin_x: 0,
                    source_origin_y: 0,
                    scale_x: 0.5,
                    scale_y: 0.5,
                },
                coordinate_space: CoordinateSpace::PhysicalPixel,
                monitors: Vec::new(),
            },
            revision: "revision-5".to_string(),
            sequence: 5,
            complete: true,
            damage_present: true,
        };
        let response = DaemonResponse::CaptureFrame(frame.clone());
        assert_eq!(response.response_type(), "capture_frame");
        let wait = DaemonResponse::CaptureWait(Box::new(CaptureWaitResult {
            frame,
            changed: true,
            timed_out: false,
            timeout_ms: 5_000,
            elapsed_ms: 20,
        }));
        assert_eq!(wait.response_type(), "capture_wait");
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
                occlusion_possible: false,
                source_width: 7680,
                source_height: 4320,
                output_width: 1600,
                output_height: 900,
                transform: ScreenshotTransform {
                    source_coordinate_space: CoordinateSpace::PhysicalPixel,
                    output_coordinate_space: CoordinateSpace::CaptureOutput,
                    source_extent_width: Some(7680),
                    source_extent_height: Some(4320),
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
            output_coordinate_space: CoordinateSpace::CaptureOutput,
            source_extent_width: Some(7680),
            source_extent_height: Some(4320),
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
            output_coordinate_space: CoordinateSpace::CaptureOutput,
            source_extent_width: Some(1600),
            source_extent_height: Some(1200),
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
            output_coordinate_space: CoordinateSpace::CaptureOutput,
            source_extent_width: Some(1),
            source_extent_height: Some(1),
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
            run_id: None,
            build_id: None,
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
            response_options: Some(DaemonResponseOptions {
                post_action: Some(PostActionOptions {
                    observe_after: true,
                    settle_condition: ActionSettleCondition::Stable,
                    settle_timeout_ms: 1_000,
                    settle_interval_ms: 100,
                    image: None,
                }),
            }),
        };
        let encoded = serde_json::to_string(&envelope).expect("envelope serializes");
        assert!(encoded.contains(r#""request":{"method":"health"}"#));
        assert!(encoded.contains(r#""client":{"tool":"seatgeist-mcp"}"#));
        assert!(encoded.contains(r#""settle_condition":"stable""#));

        let decoded: DaemonRequestEnvelope =
            serde_json::from_str(&encoded).expect("envelope deserializes");
        assert_eq!(decoded.request, DaemonRequest::Health);
        assert_eq!(
            decoded
                .client
                .as_ref()
                .and_then(|client| client.tool.clone()),
            Some("seatgeist-mcp".to_string())
        );
        assert_eq!(
            decoded
                .response_options
                .and_then(|options| options.post_action)
                .map(|options| options.settle_timeout_ms),
            Some(1_000)
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
            human_input_activity_backend: Some("kwin_input_spy_v1".to_string()),
            human_input_activity_trusted: true,
            human_input_last_class: Some("keyboard".to_string()),
            human_input_last_provenance: Some("trusted_physical".to_string()),
            control_rate_limit_per_minute: Some(120),
            preview_max_edge: 1600,
            tile_max_edge: 1600,
            screenshot_redaction_count: 2,
            journal_artifact_metadata_enabled: true,
        });
        let encoded = serde_json::to_string(&response).expect("safety response serializes");
        assert!(encoded.contains(r#""type":"safety_status""#));
        assert!(encoded.contains(r#""require_focus_guard":true"#));
        assert!(encoded.contains(r#""human_input_activity_backend":"kwin_input_spy_v1""#));
        assert!(encoded.contains(r#""human_input_activity_trusted":true"#));
        assert!(encoded.contains(r#""human_input_last_provenance":"trusted_physical""#));
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
            capture_revision: None,
            guard: None,
            session_id: None,
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
            capture_revision: None,
            guard: Some(ActiveWindowGuard {
                desktop_revision: None,
                expected_window_id: Some("current-window".to_string()),
                expected_app_id: None,
                title_contains: None,
            }),
            session_id: None,
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
            capture_revision: None,
            guard: None,
            session_id: None,
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
            session_id: None,
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
            ownership_retry_count: 0,
            ownership_retry_in_ms: None,
            ownership_last_error: None,
            window_resize_supported: true,
            window_move_supported: true,
            window_launch_supported: true,
            window_close_supported: true,
            active_window_update_seen: false,
            window_list_update_seen: false,
            active_window_update_age_ms: None,
            window_list_update_age_ms: None,
            snapshot_stale: true,
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
                desktop_revision: None,
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
                desktop_revision: None,
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
            observe_state: ActionReadiness::Ready,
            screenshot_state: ActionReadiness::Ready,
            window_control_state: ActionReadiness::NeedsGuard,
            keyboard_input_state: ActionReadiness::Unavailable,
            pointer_input_state: ActionReadiness::Unavailable,
            semantic_action_state: ActionReadiness::Ready,
            clipboard_read_state: ActionReadiness::Unavailable,
            clipboard_write_state: ActionReadiness::Ready,
            desktop_revision: Some("aw1:test".to_string()),
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
                screenshot_interface_version: Some(3),
                screenshot_available_targets_mask: Some(15),
                screenshot_available_targets: vec![
                    "screen".to_string(),
                    "window".to_string(),
                    "area".to_string(),
                    "active_window".to_string(),
                ],
                screenshot_target_option_supported: true,
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
            protocol_version: None,
            run_id: None,
            git_sha: None,
            build_unix_ms: None,
            binary_sha256: None,
            config_fingerprint: None,
            resident_memory_bytes: None,
            resident_memory_peak_bytes: None,
        });
        assert_eq!(health.response_type(), "health");
        assert!(health.ok());

        let unconfirmed_action = DaemonResponse::Action(Box::new(ActionResult {
            id: Uuid::nil(),
            ok: false,
            observation: None,
            screenshot: None,
            message: Some("dispatch accepted but postcondition failed".to_string()),
        }));
        assert_eq!(unconfirmed_action.response_type(), "action");
        assert!(!unconfirmed_action.ok());

        let error = DaemonResponse::Error {
            kind: ErrorKind::PolicyDenied,
            reason_code: Some("policy_denied".to_string()),
            message: "denied".to_string(),
        };
        assert_eq!(error.response_type(), "error");
        assert!(!error.ok());
        let encoded = serde_json::to_string(&error).expect("error response serializes");
        assert!(encoded.contains(r#""type":"error""#));
        assert!(encoded.contains(r#""kind":"policy_denied""#));
        assert!(encoded.contains(r#""reason_code":"policy_denied""#));
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
    fn serializes_window_resize_and_page_zoom_requests() {
        let launch = DaemonRequest::LaunchWindow(LaunchWindowRequest {
            desktop_entry: "org.kde.kcalc".to_string(),
            anchor: WindowPlacementAnchor::TopRight,
            monitor_id: Some("DP-1".to_string()),
            width: Some(400),
            height: Some(300),
            margin: 20,
            activation: WindowActivationMode::PreserveFocus,
            timeout_ms: 10_000,
            guard: None,
        });
        let encoded = serde_json::to_string(&launch).expect("launch request serializes");
        assert!(encoded.contains(r#""method":"launch_window""#));
        assert!(encoded.contains(r#""anchor":"top_right""#));
        assert!(encoded.contains(r#""activation":"preserve_focus""#));
        assert_eq!(
            serde_json::from_str::<DaemonRequest>(&encoded).expect("launch request deserializes"),
            launch
        );

        let resize = DaemonRequest::ResizeWindow(ResizeWindowRequest {
            window_id: "window-1".to_string(),
            width: 1280,
            height: 720,
            guard: None,
        });
        let encoded = serde_json::to_string(&resize).expect("resize request serializes");
        assert!(encoded.contains(r#""method":"resize_window""#));
        assert!(encoded.contains(r#""width":1280"#));
        assert_eq!(
            serde_json::from_str::<DaemonRequest>(&encoded).expect("resize request deserializes"),
            resize
        );

        let zoom = DaemonRequest::PageZoom(PageZoomRequest {
            operation: PageZoomOperation::Out,
            steps: 2,
            guard: ActiveWindowGuard {
                desktop_revision: None,
                expected_window_id: Some("window-1".to_string()),
                expected_app_id: Some("org.mozilla.firefox".to_string()),
                title_contains: None,
            },
        });
        let encoded = serde_json::to_string(&zoom).expect("page zoom request serializes");
        assert!(encoded.contains(r#""method":"page_zoom""#));
        assert!(encoded.contains(r#""operation":"out""#));
        assert_eq!(
            serde_json::from_str::<DaemonRequest>(&encoded)
                .expect("page zoom request deserializes"),
            zoom
        );
    }

    #[test]
    fn serializes_observe_request_with_optional_screenshot() {
        let request = DaemonRequest::Observe(ObserveRequest {
            screenshot: Some(ScreenshotRequest {
                output: PathBuf::from("/tmp/observe.png"),
                max_edge: Some(1200),
                full_resolution: false,
                portal_interactive: false,
                portal_target: None,
                visible_window_crop_id: None,
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
            registry_process_count: Some(1),
            extra_registry_process_count: Some(0),
            target_event_settle_available: true,
            event_backend: "atspi_registry".to_string(),
            target_event_classes: vec![
                "object".to_string(),
                "window".to_string(),
                "focus".to_string(),
            ],
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
        assert!(encoded.contains(r#""event_backend":"atspi_registry""#));
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
            target_guard: None,
        });
        let encoded = serde_json::to_string(&request).expect("click button request serializes");
        assert!(encoded.contains(r#""method":"click_button""#));
        assert!(encoded.contains(r#""name":"OK""#));
        assert!(encoded.contains(r#""app":"kate""#));
        assert!(encoded.contains(r#""window_name_contains":"settings""#));
    }

    #[test]
    fn serializes_semantic_target_window_guard_separately_from_active_guard() {
        let request = DaemonRequest::ClickButton(ClickButtonRequest {
            name: "Continue".to_string(),
            destructive: false,
            app: Some("Firefox".to_string()),
            window_name_contains: Some("Example".to_string()),
            max_nodes: 512,
            guard: None,
            target_guard: Some(TargetWindowGuard {
                expected_window_id: "kwin-firefox-1".to_string(),
                expected_app_id: Some("org.mozilla.firefox".to_string()),
                expected_pid: Some(4242),
                title_contains: Some("Example".to_string()),
            }),
        });
        let encoded = serde_json::to_string(&request).expect("target guard serializes");
        assert!(encoded.contains(r#""target_guard""#));
        assert!(encoded.contains(r#""expected_window_id":"kwin-firefox-1""#));
        assert!(encoded.contains(r#""expected_pid":4242"#));
        assert!(!encoded.contains(r#""guard""#));
        let decoded: DaemonRequest = serde_json::from_str(&encoded).expect("target guard parses");
        assert_eq!(decoded, request);
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
            target_guard: None,
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
            target_guard: None,
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
            target_guard: None,
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
            target_guard: None,
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
            target_guard: None,
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
            target_guard: None,
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
            target_guard: None,
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
            target_guard: None,
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
                desktop_revision: None,
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
            session_id: None,
        });
        let encoded = serde_json::to_string(&type_text).expect("type text request serializes");
        assert_eq!(encoded, r#"{"method":"type_text","text":"hello"}"#);

        let key_combo = DaemonRequest::KeyCombo(KeyComboRequest {
            combo: "Ctrl+L".to_string(),
            destructive: false,
            guard: None,
            session_id: Some("capture-1".to_string()),
        });
        let encoded = serde_json::to_string(&key_combo).expect("key combo request serializes");
        assert_eq!(
            encoded,
            r#"{"method":"key_combo","combo":"Ctrl+L","session_id":"capture-1"}"#
        );

        let destructive = DaemonRequest::KeyCombo(KeyComboRequest {
            combo: "Ctrl+Shift+W".to_string(),
            destructive: true,
            guard: None,
            session_id: Some("capture-1".to_string()),
        });
        assert_eq!(
            serde_json::to_string(&destructive).expect("destructive combo serializes"),
            r#"{"method":"key_combo","combo":"Ctrl+Shift+W","destructive":true,"session_id":"capture-1"}"#
        );

        let close = DaemonRequest::CloseWindow(CloseWindowRequest {
            window_id: "{d9ba63dd-1081-42c7-90cf-6f7c1c26e009}".to_string(),
            session_id: Some("capture-1".to_string()),
            guard: None,
        });
        assert_eq!(
            serde_json::to_string(&close).expect("exact close serializes"),
            r#"{"method":"close_window","window_id":"{d9ba63dd-1081-42c7-90cf-6f7c1c26e009}","session_id":"capture-1"}"#
        );
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
