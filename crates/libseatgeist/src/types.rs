use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type WindowId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSpace {
    PhysicalPixel,
    LogicalPixel,
    WindowLocal,
    AccessibilityNode,
}

impl FromStr for CoordinateSpace {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "physical_pixel" | "physical" | "pixel" => Ok(Self::PhysicalPixel),
            "logical_pixel" | "logical" => Ok(Self::LogicalPixel),
            "window_local" | "window" => Ok(Self::WindowLocal),
            "accessibility_node" | "accessibility" | "a11y" => Ok(Self::AccessibilityNode),
            other => Err(format!("unsupported coordinate space: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub space: CoordinateSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

impl FromStr for PointerButton {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "left" | "primary" => Ok(Self::Left),
            "middle" => Ok(Self::Middle),
            "right" | "secondary" => Ok(Self::Right),
            other => Err(format!("unsupported pointer button: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotTarget {
    ActiveWindow,
    Monitor(String),
    AllMonitors,
    Region {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        space: CoordinateSpace,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: String,
    pub name: Option<String>,
    pub physical_width: u32,
    pub physical_height: u32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub scale_factor: f64,
    pub logical_origin_x: i32,
    pub logical_origin_y: i32,
    pub transform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub app_id: Option<String>,
    pub title: String,
    pub pid: Option<u32>,
    pub monitor_id: Option<String>,
    pub geometry: Option<WindowGeometry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub space: CoordinateSpace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityNode {
    pub id: String,
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub value_truncated: bool,
    pub sensitive: bool,
    pub states: Vec<String>,
    pub bounds: Option<AccessibilityBounds>,
    pub available_actions: Vec<String>,
    pub actions: Vec<AccessibilityAction>,
    pub children: Vec<AccessibilityNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub space: CoordinateSpace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityAction {
    Press,
    SetText,
    Focus,
    Select,
}

impl AccessibilityAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::SetText => "set_text",
            Self::Focus => "focus",
            Self::Select => "select",
        }
    }
}

impl FromStr for AccessibilityAction {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "press" | "click" | "default" | "activate" => Ok(Self::Press),
            "set_text" | "settext" => Ok(Self::SetText),
            "focus" => Ok(Self::Focus),
            "select" => Ok(Self::Select),
            other => Err(format!("unsupported accessibility action: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalLevel {
    Allow,
    Prompt,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyClass {
    Observe,
    FullResolutionScreenshot,
    ClipboardRead,
    ClipboardWrite,
    ControlPointer,
    ControlKeyboard,
    ControlSemantic,
    DestructiveAction,
    SecretField,
    Policy,
}

impl FromStr for SafetyClass {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "observe" => Ok(Self::Observe),
            "full_resolution_screenshot" | "full_resolution" | "screenshot_full" => {
                Ok(Self::FullResolutionScreenshot)
            }
            "clipboard_read" => Ok(Self::ClipboardRead),
            "clipboard_write" => Ok(Self::ClipboardWrite),
            "control_pointer" | "pointer" => Ok(Self::ControlPointer),
            "control_keyboard" | "keyboard" => Ok(Self::ControlKeyboard),
            "control_semantic" | "semantic" | "control" => Ok(Self::ControlSemantic),
            "destructive_action" | "destructive" => Ok(Self::DestructiveAction),
            "secret_field" | "secret" => Ok(Self::SecretField),
            "policy" => Ok(Self::Policy),
            other => Err(format!("unsupported safety class: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendCapability {
    DaemonHealth,
    DaemonPolicyStatus,
    DaemonSafetyStatus,
    DaemonDesktopSessionStatus,
    DaemonComputerUseReadiness,
    Screenshot,
    MonitorMetadata,
    WindowList,
    WindowFocus,
    WindowLaunch,
    WindowMove,
    WindowResize,
    PointerInput,
    KeyboardInput,
    ClipboardText,
    AccessibilityTree,
    SemanticActions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub level: ToolApprovalLevel,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub active_window: Option<WindowInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_window: Option<WindowInfo>,
    pub windows: Vec<WindowInfo>,
    pub monitors: Vec<MonitorInfo>,
    pub focused_accessibility: Option<AccessibilityNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_accessibility: Option<AccessibilityNode>,
    pub screenshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle: Option<ActionSettleResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSettleCondition {
    None,
    Auto,
    Stable,
    ActiveWindowChange,
    AccessibilityChange,
    AnyChange,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSettleBackend {
    #[default]
    Polling,
    AtspiEvent,
    TargetRead,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionConfirmation {
    Confirmed,
    UnconfirmedTimeout,
    #[default]
    NotRequested,
}

impl ActionConfirmation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::UnconfirmedTimeout => "unconfirmed_timeout",
            Self::NotRequested => "not_requested",
        }
    }
}

impl FromStr for ActionSettleCondition {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "none" => Ok(Self::None),
            "auto" => Ok(Self::Auto),
            "stable" => Ok(Self::Stable),
            "active_window_change" | "window_change" => Ok(Self::ActiveWindowChange),
            "accessibility_change" | "a11y_change" => Ok(Self::AccessibilityChange),
            "any_change" | "change" => Ok(Self::AnyChange),
            other => Err(format!("unsupported action settle condition: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSettleResult {
    #[serde(default)]
    pub confirmation: ActionConfirmation,
    pub condition: ActionSettleCondition,
    #[serde(default)]
    pub backend: ActionSettleBackend,
    #[serde(default)]
    pub target_scoped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    pub settled: bool,
    pub timed_out: bool,
    pub timeout_ms: u64,
    pub interval_ms: u64,
    pub samples: u32,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_revision: Option<String>,
    pub after_revision: String,
}

#[derive(Debug, Error)]
pub enum SeatgeistError {
    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),
    #[error("policy denied action: {0}")]
    PolicyDenied(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("I/O error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionId(pub Uuid);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settle_results_default_to_polling_scope() {
        let result: ActionSettleResult = serde_json::from_str(
            r#"{"condition":"stable","settled":true,"timed_out":false,"timeout_ms":1500,"interval_ms":100,"samples":2,"elapsed_ms":100,"after_revision":"after"}"#,
        )
        .expect("legacy settle result parses");

        assert_eq!(result.backend, ActionSettleBackend::Polling);
        assert!(!result.target_scoped);
        assert_eq!(result.event, None);
    }
}
