use async_trait::async_trait;
use libseatgeist::{
    AccessibilityAction, AccessibilityFindRequest, AccessibilityNode, AccessibilityTextAttributes,
    MonitorInfo, Point, PointerButton, SeatgeistError, WindowGeometry, WindowId, WindowInfo,
};
use std::time::Duration;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, SeatgeistError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub path: String,
    pub source_width: u32,
    pub source_height: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSourceType {
    Window,
    Monitor,
    VirtualOutput,
    DesktopCompatibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSource {
    Window {
        requested_window_id: Option<WindowId>,
    },
    Monitor {
        requested_monitor_id: Option<String>,
    },
    VirtualOutput,
    DesktopCompatibility {
        requested_window_id: Option<WindowId>,
    },
}

impl CaptureSource {
    pub fn source_type(&self) -> CaptureSourceType {
        match self {
            Self::Window { .. } => CaptureSourceType::Window,
            Self::Monitor { .. } => CaptureSourceType::Monitor,
            Self::VirtualOutput => CaptureSourceType::VirtualOutput,
            Self::DesktopCompatibility { .. } => CaptureSourceType::DesktopCompatibility,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureCapabilities {
    pub backend: String,
    pub source_types: Vec<CaptureSourceType>,
    pub retained_sessions: bool,
    pub wait_for_frame: bool,
    pub restore_tokens: bool,
    pub damage_tracking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSessionRequest {
    pub source: CaptureSource,
    pub restore_token_reference: Option<String>,
    pub persist: bool,
    pub consent_parent_window: String,
    pub open_timeout_ms: u64,
    pub default_max_edge: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSessionMetadata {
    pub id: String,
    pub backend: String,
    pub source_type: CaptureSourceType,
    pub source_id: Option<String>,
    pub restore_token_reference: Option<String>,
    pub consent_required: bool,
    pub occlusion_possible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSessionLifecycle {
    Open,
    ClientClosed,
    PortalClosed,
    MonitorFailed,
}

impl CaptureSessionLifecycle {
    pub const fn end_reason(self) -> Option<&'static str> {
        match self {
            Self::Open => None,
            Self::ClientClosed => Some("client_closed"),
            Self::PortalClosed => Some("portal_closed"),
            Self::MonitorFailed => Some("portal_monitor_failed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRequest {
    pub output: String,
    pub max_edge: Option<u32>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameWaitRequest {
    pub after_revision: Option<String>,
    pub timeout_ms: u64,
    pub frame: FrameRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub screenshot: Screenshot,
    pub revision: String,
    pub sequence: u64,
    pub complete: bool,
    pub damage_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameWaitResult {
    pub frame: CapturedFrame,
    pub changed: bool,
    pub timed_out: bool,
    pub elapsed_ms: u64,
}

#[async_trait]
pub trait ScreenBackend: std::fmt::Debug + Send + Sync {
    async fn capabilities(&self) -> Result<CaptureCapabilities>;
    async fn list_monitors(&self) -> Result<Vec<MonitorInfo>>;
    async fn open_capture(&self, request: CaptureSessionRequest)
    -> Result<Box<dyn CaptureSession>>;
}

#[async_trait]
pub trait CaptureSession: Send + Sync {
    fn metadata(&self) -> CaptureSessionMetadata;
    async fn lifecycle(&self) -> CaptureSessionLifecycle {
        CaptureSessionLifecycle::Open
    }
    async fn snapshot(&self, request: FrameRequest) -> Result<CapturedFrame>;
    async fn wait_for_frame(&self, request: FrameWaitRequest) -> Result<FrameWaitResult>;
    async fn close(&self) -> Result<()>;
}

#[async_trait]
pub trait WindowBackend: std::fmt::Debug + Send + Sync {
    fn backend_name(&self) -> &'static str;
    async fn list_windows(&self) -> Result<Vec<WindowInfo>>;
    async fn active_window(&self) -> Result<Option<WindowInfo>>;
    async fn focus_window(&self, id: WindowId) -> Result<()>;
    async fn close_window(&self, id: WindowId) -> Result<()>;
    async fn move_window(&self, id: WindowId, x: i32, y: i32) -> Result<WindowGeometry>;
    async fn resize_window(&self, id: WindowId, width: u32, height: u32) -> Result<WindowGeometry>;
}

#[async_trait]
pub trait InputBackend: Send + Sync {
    async fn move_pointer(&self, point: Point) -> Result<()>;
    async fn click(&self, point: Point, button: PointerButton, clicks: u8) -> Result<()>;
    async fn drag(
        &self,
        from: Point,
        to: Point,
        button: PointerButton,
        duration_ms: u64,
    ) -> Result<()>;
    async fn scroll(&self, vertical: i32, horizontal: i32) -> Result<()>;
    async fn type_text(&self, text: &str) -> Result<()>;
    async fn key_combo(&self, combo: &str) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetedInputDelivery {
    pub action_id: Uuid,
    pub backend: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetedInputContext {
    pub lane_id: String,
}

#[async_trait]
pub trait TargetedInputBackend: std::fmt::Debug + Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn ready(&self) -> bool;
    async fn key_combo(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        keycodes: &[u16],
    ) -> Result<TargetedInputDelivery>;
    async fn key_sequence(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        chords: &[Vec<u16>],
    ) -> Result<TargetedInputDelivery>;
    async fn move_pointer(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        point: Point,
    ) -> Result<TargetedInputDelivery>;
    async fn click(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        point: Point,
        button: PointerButton,
        clicks: u8,
    ) -> Result<TargetedInputDelivery>;
    async fn drag(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        from: Point,
        to: Point,
        button: PointerButton,
    ) -> Result<TargetedInputDelivery>;
    async fn scroll(
        &self,
        context: &TargetedInputContext,
        target: &WindowInfo,
        vertical: i32,
        horizontal: i32,
    ) -> Result<TargetedInputDelivery>;
}

#[async_trait]
pub trait ClipboardBackend: Send + Sync {
    async fn get_text(&self) -> Result<Option<String>>;
    async fn set_text(&self, text: &str) -> Result<()>;
}

#[async_trait]
pub trait AccessibilityBackend: Send + Sync {
    async fn focused_tree(&self, depth: usize) -> Result<AccessibilityNode>;
    async fn find(&self, request: AccessibilityFindRequest) -> Result<Vec<AccessibilityNode>>;
    async fn text_attributes(
        &self,
        node_id: &str,
        offset: i32,
        include_defaults: bool,
    ) -> Result<AccessibilityTextAttributes>;
    async fn invoke(&self, node_id: &str, action: AccessibilityAction) -> Result<()>;
    async fn set_text(&self, node_id: &str, text: &str) -> Result<()>;
    async fn insert_text(&self, node_id: &str, offset: i32, text: &str) -> Result<()>;
    async fn delete_text(&self, node_id: &str, start_offset: i32, end_offset: i32) -> Result<()>;
    async fn copy_text(&self, node_id: &str, start_offset: i32, end_offset: i32) -> Result<()>;
    async fn cut_text(&self, node_id: &str, start_offset: i32, end_offset: i32) -> Result<()>;
    async fn paste_text(&self, node_id: &str, offset: i32) -> Result<()>;
    async fn set_caret(&self, node_id: &str, offset: i32) -> Result<()>;
    async fn set_selection(
        &self,
        node_id: &str,
        selection_num: i32,
        start_offset: i32,
        end_offset: i32,
    ) -> Result<()>;
    async fn set_value(&self, node_id: &str, value: f64) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityEventTarget {
    pub application_bus_name: String,
    pub node_id: String,
    pub window_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityEvent {
    pub interface: String,
    pub member: String,
    pub source_node_id: String,
}

#[async_trait]
pub trait AccessibilityEventSubscription: Send {
    async fn wait_for_event(&mut self, timeout: Duration) -> Result<Option<AccessibilityEvent>>;
    async fn close(self: Box<Self>) -> Result<()>;
}

#[async_trait]
pub trait AccessibilityEventBackend: Send + Sync {
    async fn subscribe(
        &self,
        target: AccessibilityEventTarget,
    ) -> Result<Box<dyn AccessibilityEventSubscription>>;
}
