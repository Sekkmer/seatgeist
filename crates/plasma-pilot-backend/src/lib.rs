use async_trait::async_trait;
use libplasma_pilot::{
    AccessibilityAction, AccessibilityFindRequest, AccessibilityNode, AccessibilityTextAttributes,
    MonitorInfo, PilotError, Point, PointerButton, ScreenshotTarget, WindowId, WindowInfo,
};

pub type Result<T> = std::result::Result<T, PilotError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

#[async_trait]
pub trait ScreenBackend: Send + Sync {
    async fn list_monitors(&self) -> Result<Vec<MonitorInfo>>;
    async fn screenshot(&self, target: ScreenshotTarget) -> Result<Screenshot>;
    async fn screenshot_scaled(
        &self,
        target: ScreenshotTarget,
        max_edge: u32,
    ) -> Result<Screenshot>;
}

#[async_trait]
pub trait WindowBackend: Send + Sync {
    async fn list_windows(&self) -> Result<Vec<WindowInfo>>;
    async fn active_window(&self) -> Result<Option<WindowInfo>>;
    async fn focus_window(&self, id: WindowId) -> Result<()>;
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
    async fn set_value(&self, node_id: &str, value: f64) -> Result<()>;
}
