use async_trait::async_trait;
use libplasma_pilot::{
    AccessibilityAction, AccessibilityFindRequest, AccessibilityNode, MonitorInfo, PilotError,
    Point, ScreenshotTarget, WindowId, WindowInfo,
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
    async fn click(&self, point: Point) -> Result<()>;
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
    async fn invoke(&self, node_id: &str, action: AccessibilityAction) -> Result<()>;
    async fn set_text(&self, node_id: &str, text: &str) -> Result<()>;
}
