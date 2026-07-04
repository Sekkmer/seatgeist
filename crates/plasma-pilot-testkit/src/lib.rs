use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use libplasma_pilot::{
    AccessibilityAction, AccessibilityFindRequest, AccessibilityNode, CoordinateSpace, MonitorInfo,
    PilotError, Point, PointerButton, ScreenshotTarget, WindowGeometry, WindowId, WindowInfo,
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
    scaled_screenshots: Arc<Mutex<Vec<(ScreenshotTarget, u32)>>>,
}

impl MockScreenBackend {
    pub fn new(monitors: Vec<MonitorInfo>, screenshot: Screenshot) -> Self {
        Self {
            monitors,
            screenshot,
            screenshots: Arc::new(Mutex::new(Vec::new())),
            scaled_screenshots: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn screenshots(&self) -> Result<Vec<ScreenshotTarget>> {
        Ok(lock(&self.screenshots)?.clone())
    }

    pub fn scaled_screenshots(&self) -> Result<Vec<(ScreenshotTarget, u32)>> {
        Ok(lock(&self.scaled_screenshots)?.clone())
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

    async fn screenshot_scaled(
        &self,
        target: ScreenshotTarget,
        max_edge: u32,
    ) -> Result<Screenshot> {
        lock(&self.scaled_screenshots)?.push((target, max_edge));
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
    Click {
        point: Point,
        button: PointerButton,
        clicks: u8,
    },
    Drag {
        from: Point,
        to: Point,
        button: PointerButton,
        duration_ms: u64,
    },
    Scroll {
        vertical: i32,
        horizontal: i32,
    },
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

    async fn click(&self, point: Point, button: PointerButton, clicks: u8) -> Result<()> {
        lock(&self.events)?.push(MockInputEvent::Click {
            point,
            button,
            clicks,
        });
        Ok(())
    }

    async fn drag(
        &self,
        from: Point,
        to: Point,
        button: PointerButton,
        duration_ms: u64,
    ) -> Result<()> {
        lock(&self.events)?.push(MockInputEvent::Drag {
            from,
            to,
            button,
            duration_ms,
        });
        Ok(())
    }

    async fn scroll(&self, vertical: i32, horizontal: i32) -> Result<()> {
        lock(&self.events)?.push(MockInputEvent::Scroll {
            vertical,
            horizontal,
        });
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityTextSet {
    pub node_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityTextInsert {
    pub node_id: String,
    pub offset: i32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityTextDelete {
    pub node_id: String,
    pub start_offset: i32,
    pub end_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityTextCopy {
    pub node_id: String,
    pub start_offset: i32,
    pub end_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityTextCut {
    pub node_id: String,
    pub start_offset: i32,
    pub end_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityTextPaste {
    pub node_id: String,
    pub offset: i32,
}

#[derive(Debug, Clone)]
pub struct MockAccessibilityBackend {
    focused_tree: AccessibilityNode,
    find_matches: Vec<AccessibilityNode>,
    find_requests: Arc<Mutex<Vec<AccessibilityFindRequest>>>,
    invocations: Arc<Mutex<Vec<MockAccessibilityInvocation>>>,
    text_sets: Arc<Mutex<Vec<MockAccessibilityTextSet>>>,
    text_inserts: Arc<Mutex<Vec<MockAccessibilityTextInsert>>>,
    text_deletes: Arc<Mutex<Vec<MockAccessibilityTextDelete>>>,
    text_copies: Arc<Mutex<Vec<MockAccessibilityTextCopy>>>,
    text_cuts: Arc<Mutex<Vec<MockAccessibilityTextCut>>>,
    text_pastes: Arc<Mutex<Vec<MockAccessibilityTextPaste>>>,
}

impl MockAccessibilityBackend {
    pub fn new(focused_tree: AccessibilityNode) -> Self {
        Self {
            focused_tree,
            find_matches: Vec::new(),
            find_requests: Arc::new(Mutex::new(Vec::new())),
            invocations: Arc::new(Mutex::new(Vec::new())),
            text_sets: Arc::new(Mutex::new(Vec::new())),
            text_inserts: Arc::new(Mutex::new(Vec::new())),
            text_deletes: Arc::new(Mutex::new(Vec::new())),
            text_copies: Arc::new(Mutex::new(Vec::new())),
            text_cuts: Arc::new(Mutex::new(Vec::new())),
            text_pastes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn with_find_matches(mut self, find_matches: Vec<AccessibilityNode>) -> Self {
        self.find_matches = find_matches;
        self
    }

    pub fn find_requests(&self) -> Result<Vec<AccessibilityFindRequest>> {
        Ok(lock(&self.find_requests)?.clone())
    }

    pub fn invocations(&self) -> Result<Vec<MockAccessibilityInvocation>> {
        Ok(lock(&self.invocations)?.clone())
    }

    pub fn text_sets(&self) -> Result<Vec<MockAccessibilityTextSet>> {
        Ok(lock(&self.text_sets)?.clone())
    }

    pub fn text_inserts(&self) -> Result<Vec<MockAccessibilityTextInsert>> {
        Ok(lock(&self.text_inserts)?.clone())
    }

    pub fn text_deletes(&self) -> Result<Vec<MockAccessibilityTextDelete>> {
        Ok(lock(&self.text_deletes)?.clone())
    }

    pub fn text_copies(&self) -> Result<Vec<MockAccessibilityTextCopy>> {
        Ok(lock(&self.text_copies)?.clone())
    }

    pub fn text_cuts(&self) -> Result<Vec<MockAccessibilityTextCut>> {
        Ok(lock(&self.text_cuts)?.clone())
    }

    pub fn text_pastes(&self) -> Result<Vec<MockAccessibilityTextPaste>> {
        Ok(lock(&self.text_pastes)?.clone())
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

    async fn find(&self, request: AccessibilityFindRequest) -> Result<Vec<AccessibilityNode>> {
        lock(&self.find_requests)?.push(request);
        Ok(self.find_matches.clone())
    }

    async fn invoke(&self, node_id: &str, action: AccessibilityAction) -> Result<()> {
        lock(&self.invocations)?.push(MockAccessibilityInvocation {
            node_id: node_id.to_string(),
            action,
        });
        Ok(())
    }

    async fn set_text(&self, node_id: &str, text: &str) -> Result<()> {
        lock(&self.text_sets)?.push(MockAccessibilityTextSet {
            node_id: node_id.to_string(),
            text: text.to_string(),
        });
        Ok(())
    }

    async fn insert_text(&self, node_id: &str, offset: i32, text: &str) -> Result<()> {
        lock(&self.text_inserts)?.push(MockAccessibilityTextInsert {
            node_id: node_id.to_string(),
            offset,
            text: text.to_string(),
        });
        Ok(())
    }

    async fn delete_text(&self, node_id: &str, start_offset: i32, end_offset: i32) -> Result<()> {
        lock(&self.text_deletes)?.push(MockAccessibilityTextDelete {
            node_id: node_id.to_string(),
            start_offset,
            end_offset,
        });
        Ok(())
    }

    async fn copy_text(&self, node_id: &str, start_offset: i32, end_offset: i32) -> Result<()> {
        lock(&self.text_copies)?.push(MockAccessibilityTextCopy {
            node_id: node_id.to_string(),
            start_offset,
            end_offset,
        });
        Ok(())
    }

    async fn cut_text(&self, node_id: &str, start_offset: i32, end_offset: i32) -> Result<()> {
        lock(&self.text_cuts)?.push(MockAccessibilityTextCut {
            node_id: node_id.to_string(),
            start_offset,
            end_offset,
        });
        Ok(())
    }

    async fn paste_text(&self, node_id: &str, offset: i32) -> Result<()> {
        lock(&self.text_pastes)?.push(MockAccessibilityTextPaste {
            node_id: node_id.to_string(),
            offset,
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
    use libplasma_pilot::{
        AccessibilityAction, AccessibilityFindRequest, CoordinateSpace, Point, ScreenshotTarget,
    };
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

        let scaled = backend
            .screenshot_scaled(ScreenshotTarget::ActiveWindow, 800)
            .await?;
        assert_eq!(scaled.height, 900);
        assert_eq!(
            backend.scaled_screenshots()?,
            vec![(ScreenshotTarget::ActiveWindow, 800)]
        );
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
        backend.click(point, PointerButton::Right, 2).await?;
        backend.drag(point, point, PointerButton::Left, 250).await?;
        backend.scroll(-3, 1).await?;
        backend.type_text("hello").await?;
        backend.key_combo("Ctrl+L").await?;

        assert_eq!(
            backend.events()?,
            vec![
                MockInputEvent::MovePointer(point),
                MockInputEvent::Click {
                    point,
                    button: PointerButton::Right,
                    clicks: 2,
                },
                MockInputEvent::Drag {
                    from: point,
                    to: point,
                    button: PointerButton::Left,
                    duration_ms: 250,
                },
                MockInputEvent::Scroll {
                    vertical: -3,
                    horizontal: 1,
                },
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
        let match_node = AccessibilityNode {
            id: "atspi://sample/button".to_string(),
            role: "button".to_string(),
            name: Some("OK".to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: vec!["enabled".to_string()],
            bounds: None,
            available_actions: vec!["press".to_string()],
            actions: vec![AccessibilityAction::Press],
            children: Vec::new(),
        };
        let backend =
            MockAccessibilityBackend::default().with_find_matches(vec![match_node.clone()]);
        let find_request = AccessibilityFindRequest {
            role: Some("button".to_string()),
            name_contains: Some("OK".to_string()),
            app: None,
            window_name_contains: None,
            depth: 4,
            max_results: 8,
            max_nodes: 128,
        };

        assert_eq!(backend.focused_tree(1).await?, sample_accessibility_node());
        assert_eq!(backend.find(find_request.clone()).await?, vec![match_node]);
        assert_eq!(backend.find_requests()?, vec![find_request]);
        backend
            .invoke("atspi://sample/root", AccessibilityAction::Press)
            .await?;
        backend.set_text("atspi://sample/text", "hello").await?;
        backend
            .insert_text("atspi://sample/text", 5, " world")
            .await?;
        backend.delete_text("atspi://sample/text", 1, 3).await?;
        backend.copy_text("atspi://sample/text", 2, 4).await?;
        backend.cut_text("atspi://sample/text", 3, 5).await?;
        backend.paste_text("atspi://sample/text", 4).await?;
        assert_eq!(
            backend.invocations()?,
            vec![MockAccessibilityInvocation {
                node_id: "atspi://sample/root".to_string(),
                action: AccessibilityAction::Press,
            }]
        );
        assert_eq!(
            backend.text_sets()?,
            vec![MockAccessibilityTextSet {
                node_id: "atspi://sample/text".to_string(),
                text: "hello".to_string(),
            }]
        );
        assert_eq!(
            backend.text_inserts()?,
            vec![MockAccessibilityTextInsert {
                node_id: "atspi://sample/text".to_string(),
                offset: 5,
                text: " world".to_string(),
            }]
        );
        assert_eq!(
            backend.text_deletes()?,
            vec![MockAccessibilityTextDelete {
                node_id: "atspi://sample/text".to_string(),
                start_offset: 1,
                end_offset: 3,
            }]
        );
        assert_eq!(
            backend.text_copies()?,
            vec![MockAccessibilityTextCopy {
                node_id: "atspi://sample/text".to_string(),
                start_offset: 2,
                end_offset: 4,
            }]
        );
        assert_eq!(
            backend.text_cuts()?,
            vec![MockAccessibilityTextCut {
                node_id: "atspi://sample/text".to_string(),
                start_offset: 3,
                end_offset: 5,
            }]
        );
        assert_eq!(
            backend.text_pastes()?,
            vec![MockAccessibilityTextPaste {
                node_id: "atspi://sample/text".to_string(),
                offset: 4,
            }]
        );
        Ok(())
    }
}
