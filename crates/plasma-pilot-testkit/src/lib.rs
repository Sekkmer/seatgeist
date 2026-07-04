use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use libplasma_pilot::{
    AccessibilityAction, AccessibilityNode, CoordinateSpace, MonitorInfo, PilotError, Point,
    ScreenshotTarget, WindowGeometry, WindowId, WindowInfo,
};
use plasma_pilot_backend::{
    AccessibilityBackend, ClipboardBackend, InputBackend, Result, ScreenBackend, Screenshot,
    WindowBackend,
};

pub fn sample_monitor() -> MonitorInfo {
    MonitorInfo {
        id: "monitor-1".to_string(),
        name: Some("Sample Monitor".to_string()),
        physical_width: 7680,
        physical_height: 4320,
        logical_width: 3840,
        logical_height: 2160,
        scale_factor: 2.0,
        logical_origin_x: 0,
        logical_origin_y: 0,
        transform: None,
    }
}

pub fn sample_coordinate_space() -> CoordinateSpace {
    CoordinateSpace::LogicalPixel
}

pub fn sample_window() -> WindowInfo {
    WindowInfo {
        id: "window-1".to_string(),
        app_id: Some("org.kde.Sample".to_string()),
        title: "Sample Window".to_string(),
        pid: Some(42),
        monitor_id: Some("monitor-1".to_string()),
        geometry: Some(WindowGeometry {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
            space: CoordinateSpace::LogicalPixel,
        }),
    }
}

pub fn sample_accessibility_node() -> AccessibilityNode {
    AccessibilityNode {
        id: "atspi://sample/root".to_string(),
        role: "application".to_string(),
        name: Some("Sample Application".to_string()),
        value: None,
        value_truncated: false,
        sensitive: false,
        states: vec!["enabled".to_string()],
        bounds: None,
        available_actions: Vec::new(),
        actions: Vec::new(),
        children: Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub struct MockScreenBackend {
    monitors: Vec<MonitorInfo>,
    screenshot: Screenshot,
    screenshots: Arc<Mutex<Vec<ScreenshotTarget>>>,
}

impl MockScreenBackend {
    pub fn new(monitors: Vec<MonitorInfo>, screenshot: Screenshot) -> Self {
        Self {
            monitors,
            screenshot,
            screenshots: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn screenshots(&self) -> Result<Vec<ScreenshotTarget>> {
        Ok(lock(&self.screenshots)?.clone())
    }
}

impl Default for MockScreenBackend {
    fn default() -> Self {
        Self::new(
            vec![sample_monitor()],
            Screenshot {
                path: "mock-screen.png".to_string(),
                width: 1600,
                height: 900,
            },
        )
    }
}

#[async_trait]
impl ScreenBackend for MockScreenBackend {
    async fn list_monitors(&self) -> Result<Vec<MonitorInfo>> {
        Ok(self.monitors.clone())
    }

    async fn screenshot(&self, target: ScreenshotTarget) -> Result<Screenshot> {
        lock(&self.screenshots)?.push(target);
        Ok(self.screenshot.clone())
    }
}

#[derive(Debug, Clone)]
pub struct MockWindowBackend {
    windows: Vec<WindowInfo>,
    active_window: Option<WindowInfo>,
    focused_windows: Arc<Mutex<Vec<WindowId>>>,
}

impl MockWindowBackend {
    pub fn new(windows: Vec<WindowInfo>, active_window: Option<WindowInfo>) -> Self {
        Self {
            windows,
            active_window,
            focused_windows: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn focused_windows(&self) -> Result<Vec<WindowId>> {
        Ok(lock(&self.focused_windows)?.clone())
    }
}

impl Default for MockWindowBackend {
    fn default() -> Self {
        let window = sample_window();
        Self::new(vec![window.clone()], Some(window))
    }
}

#[async_trait]
impl WindowBackend for MockWindowBackend {
    async fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(self.windows.clone())
    }

    async fn active_window(&self) -> Result<Option<WindowInfo>> {
        Ok(self.active_window.clone())
    }

    async fn focus_window(&self, id: WindowId) -> Result<()> {
        lock(&self.focused_windows)?.push(id);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MockInputEvent {
    MovePointer(Point),
    Click(Point),
    TypeText(String),
    KeyCombo(String),
}

#[derive(Debug, Clone, Default)]
pub struct MockInputBackend {
    events: Arc<Mutex<Vec<MockInputEvent>>>,
}

impl MockInputBackend {
    pub fn events(&self) -> Result<Vec<MockInputEvent>> {
        Ok(lock(&self.events)?.clone())
    }
}

#[async_trait]
impl InputBackend for MockInputBackend {
    async fn move_pointer(&self, point: Point) -> Result<()> {
        lock(&self.events)?.push(MockInputEvent::MovePointer(point));
        Ok(())
    }

    async fn click(&self, point: Point) -> Result<()> {
        lock(&self.events)?.push(MockInputEvent::Click(point));
        Ok(())
    }

    async fn type_text(&self, text: &str) -> Result<()> {
        lock(&self.events)?.push(MockInputEvent::TypeText(text.to_string()));
        Ok(())
    }

    async fn key_combo(&self, combo: &str) -> Result<()> {
        lock(&self.events)?.push(MockInputEvent::KeyCombo(combo.to_string()));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct MockClipboardBackend {
    text: Arc<Mutex<Option<String>>>,
}

impl MockClipboardBackend {
    pub fn new(text: Option<String>) -> Self {
        Self {
            text: Arc::new(Mutex::new(text)),
        }
    }
}

impl Default for MockClipboardBackend {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait]
impl ClipboardBackend for MockClipboardBackend {
    async fn get_text(&self) -> Result<Option<String>> {
        Ok(lock(&self.text)?.clone())
    }

    async fn set_text(&self, text: &str) -> Result<()> {
        *lock(&self.text)? = Some(text.to_string());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityInvocation {
    pub node_id: String,
    pub action: AccessibilityAction,
}

#[derive(Debug, Clone)]
pub struct MockAccessibilityBackend {
    focused_tree: AccessibilityNode,
    invocations: Arc<Mutex<Vec<MockAccessibilityInvocation>>>,
}

impl MockAccessibilityBackend {
    pub fn new(focused_tree: AccessibilityNode) -> Self {
        Self {
            focused_tree,
            invocations: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn invocations(&self) -> Result<Vec<MockAccessibilityInvocation>> {
        Ok(lock(&self.invocations)?.clone())
    }
}

impl Default for MockAccessibilityBackend {
    fn default() -> Self {
        Self::new(sample_accessibility_node())
    }
}

#[async_trait]
impl AccessibilityBackend for MockAccessibilityBackend {
    async fn focused_tree(&self, _depth: usize) -> Result<AccessibilityNode> {
        Ok(self.focused_tree.clone())
    }

    async fn invoke(&self, node_id: &str, action: AccessibilityAction) -> Result<()> {
        lock(&self.invocations)?.push(MockAccessibilityInvocation {
            node_id: node_id.to_string(),
            action,
        });
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| PilotError::Io("mock backend lock poisoned".to_string()))
}

#[cfg(test)]
mod tests {
    use libplasma_pilot::{AccessibilityAction, CoordinateSpace, Point, ScreenshotTarget};
    use plasma_pilot_backend::{
        AccessibilityBackend, ClipboardBackend, InputBackend, ScreenBackend, WindowBackend,
    };

    use super::*;

    #[tokio::test]
    async fn mock_screen_returns_monitors_and_records_screenshot_targets() -> Result<()> {
        let backend = MockScreenBackend::default();

        assert_eq!(backend.list_monitors().await?, vec![sample_monitor()]);
        let screenshot = backend.screenshot(ScreenshotTarget::AllMonitors).await?;
        assert_eq!(screenshot.width, 1600);
        assert_eq!(backend.screenshots()?, vec![ScreenshotTarget::AllMonitors]);
        Ok(())
    }

    #[tokio::test]
    async fn mock_window_returns_state_and_records_focus_requests() -> Result<()> {
        let backend = MockWindowBackend::default();

        assert_eq!(backend.list_windows().await?, vec![sample_window()]);
        assert_eq!(backend.active_window().await?, Some(sample_window()));
        backend.focus_window("window-1".to_string()).await?;
        assert_eq!(backend.focused_windows()?, vec!["window-1".to_string()]);
        Ok(())
    }

    #[tokio::test]
    async fn mock_input_records_pointer_and_keyboard_events() -> Result<()> {
        let backend = MockInputBackend::default();
        let point = Point {
            x: 12.0,
            y: 34.0,
            space: CoordinateSpace::LogicalPixel,
        };

        backend.move_pointer(point).await?;
        backend.click(point).await?;
        backend.type_text("hello").await?;
        backend.key_combo("Ctrl+L").await?;

        assert_eq!(
            backend.events()?,
            vec![
                MockInputEvent::MovePointer(point),
                MockInputEvent::Click(point),
                MockInputEvent::TypeText("hello".to_string()),
                MockInputEvent::KeyCombo("Ctrl+L".to_string()),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn mock_clipboard_round_trips_text() -> Result<()> {
        let backend = MockClipboardBackend::default();

        assert_eq!(backend.get_text().await?, None);
        backend.set_text("hello").await?;
        assert_eq!(backend.get_text().await?, Some("hello".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn mock_accessibility_returns_tree_and_records_invocations() -> Result<()> {
        let backend = MockAccessibilityBackend::default();

        assert_eq!(backend.focused_tree(1).await?, sample_accessibility_node());
        backend
            .invoke("atspi://sample/root", AccessibilityAction::Press)
            .await?;
        assert_eq!(
            backend.invocations()?,
            vec![MockAccessibilityInvocation {
                node_id: "atspi://sample/root".to_string(),
                action: AccessibilityAction::Press,
            }]
        );
        Ok(())
    }
}
