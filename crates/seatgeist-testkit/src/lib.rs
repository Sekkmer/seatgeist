use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use libseatgeist::{
    AccessibilityAction, AccessibilityFindRequest, AccessibilityNode, AccessibilityTextAttributes,
    CoordinateSpace, MonitorInfo, Point, PointerButton, SeatgeistError, TextAttribute,
    WindowGeometry, WindowId, WindowInfo,
};
use seatgeist_backend::{
    AccessibilityBackend, CaptureCapabilities, CaptureSession, CaptureSessionMetadata,
    CaptureSessionRequest, CaptureSource, CaptureSourceType, CapturedFrame, ClipboardBackend,
    FrameRequest, FrameWaitRequest, FrameWaitResult, InputBackend, Result, ScreenBackend,
    Screenshot, WindowBackend,
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

pub fn sample_text_attributes() -> AccessibilityTextAttributes {
    AccessibilityTextAttributes {
        node_id: "atspi://sample/text".to_string(),
        start_offset: 0,
        end_offset: 5,
        attributes: vec![TextAttribute {
            name: "weight".to_string(),
            value: "bold".to_string(),
        }],
    }
}

#[derive(Debug, Clone)]
pub struct MockScreenBackend {
    monitors: Vec<MonitorInfo>,
    frames: Vec<CapturedFrame>,
    capture_requests: Arc<Mutex<Vec<CaptureSessionRequest>>>,
    next_session: Arc<Mutex<u64>>,
    snapshot_requests: Arc<Mutex<Vec<(String, FrameRequest)>>>,
    wait_requests: Arc<Mutex<Vec<(String, FrameWaitRequest)>>>,
    closed_sessions: Arc<Mutex<Vec<String>>>,
}

impl MockScreenBackend {
    pub fn new(monitors: Vec<MonitorInfo>, screenshot: Screenshot) -> Self {
        Self::with_frames(
            monitors,
            vec![CapturedFrame {
                screenshot,
                revision: "mock-frame-1".to_string(),
                sequence: 1,
                complete: true,
                damage_present: false,
            }],
        )
    }

    pub fn with_frames(monitors: Vec<MonitorInfo>, frames: Vec<CapturedFrame>) -> Self {
        Self {
            monitors,
            frames,
            capture_requests: Arc::new(Mutex::new(Vec::new())),
            next_session: Arc::new(Mutex::new(1)),
            snapshot_requests: Arc::new(Mutex::new(Vec::new())),
            wait_requests: Arc::new(Mutex::new(Vec::new())),
            closed_sessions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn capture_requests(&self) -> Result<Vec<CaptureSessionRequest>> {
        Ok(lock(&self.capture_requests)?.clone())
    }

    pub fn snapshot_requests(&self) -> Result<Vec<(String, FrameRequest)>> {
        Ok(lock(&self.snapshot_requests)?.clone())
    }

    pub fn wait_requests(&self) -> Result<Vec<(String, FrameWaitRequest)>> {
        Ok(lock(&self.wait_requests)?.clone())
    }

    pub fn closed_sessions(&self) -> Result<Vec<String>> {
        Ok(lock(&self.closed_sessions)?.clone())
    }
}

impl Default for MockScreenBackend {
    fn default() -> Self {
        Self::new(
            vec![sample_monitor()],
            Screenshot {
                path: "mock-screen.png".to_string(),
                source_width: 1600,
                source_height: 900,
                width: 1600,
                height: 900,
            },
        )
    }
}

#[async_trait]
impl ScreenBackend for MockScreenBackend {
    async fn capabilities(&self) -> Result<CaptureCapabilities> {
        Ok(CaptureCapabilities {
            backend: "mock_capture_session".to_string(),
            source_types: vec![
                CaptureSourceType::Window,
                CaptureSourceType::Monitor,
                CaptureSourceType::VirtualOutput,
                CaptureSourceType::DesktopCompatibility,
            ],
            retained_sessions: true,
            wait_for_frame: true,
            restore_tokens: true,
            damage_tracking: true,
        })
    }

    async fn list_monitors(&self) -> Result<Vec<MonitorInfo>> {
        Ok(self.monitors.clone())
    }

    async fn open_capture(
        &self,
        request: CaptureSessionRequest,
    ) -> Result<Box<dyn CaptureSession>> {
        if self.frames.is_empty() {
            return Err(SeatgeistError::BackendUnavailable(
                "mock capture has no frames".to_string(),
            ));
        }
        lock(&self.capture_requests)?.push(request.clone());
        let id = {
            let mut next = lock(&self.next_session)?;
            let id = format!("mock-capture-{}", *next);
            *next = next.saturating_add(1);
            id
        };
        let source_id = match &request.source {
            CaptureSource::Window {
                requested_window_id,
            } => requested_window_id.clone(),
            CaptureSource::Monitor {
                requested_monitor_id,
            } => requested_monitor_id.clone(),
            CaptureSource::VirtualOutput => None,
            CaptureSource::DesktopCompatibility {
                requested_window_id,
            } => requested_window_id.clone(),
        };
        Ok(Box::new(MockCaptureSession {
            metadata: CaptureSessionMetadata {
                id,
                backend: "mock_capture_session".to_string(),
                source_type: request.source.source_type(),
                source_id,
                restore_token_reference: request.restore_token_reference,
                consent_required: false,
                occlusion_possible: matches!(
                    request.source,
                    CaptureSource::DesktopCompatibility { .. }
                ),
            },
            frames: self.frames.clone(),
            cursor: Mutex::new(0),
            closed: Mutex::new(false),
            snapshot_requests: Arc::clone(&self.snapshot_requests),
            wait_requests: Arc::clone(&self.wait_requests),
            closed_sessions: Arc::clone(&self.closed_sessions),
        }))
    }
}

#[derive(Debug)]
struct MockCaptureSession {
    metadata: CaptureSessionMetadata,
    frames: Vec<CapturedFrame>,
    cursor: Mutex<usize>,
    closed: Mutex<bool>,
    snapshot_requests: Arc<Mutex<Vec<(String, FrameRequest)>>>,
    wait_requests: Arc<Mutex<Vec<(String, FrameWaitRequest)>>>,
    closed_sessions: Arc<Mutex<Vec<String>>>,
}

impl MockCaptureSession {
    fn ensure_open(&self) -> Result<()> {
        if *lock(&self.closed)? {
            return Err(SeatgeistError::BackendUnavailable(format!(
                "capture session {} is closed",
                self.metadata.id
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl CaptureSession for MockCaptureSession {
    fn metadata(&self) -> CaptureSessionMetadata {
        self.metadata.clone()
    }

    async fn snapshot(&self, request: FrameRequest) -> Result<CapturedFrame> {
        self.ensure_open()?;
        lock(&self.snapshot_requests)?.push((self.metadata.id.clone(), request));
        let cursor = *lock(&self.cursor)?;
        Ok(self.frames[cursor].clone())
    }

    async fn wait_for_frame(&self, request: FrameWaitRequest) -> Result<FrameWaitResult> {
        self.ensure_open()?;
        lock(&self.wait_requests)?.push((self.metadata.id.clone(), request.clone()));
        let mut cursor = lock(&self.cursor)?;
        let current = self.frames[*cursor].clone();
        let after_revision = request.after_revision.as_deref();
        if after_revision
            .map(|revision| current.revision != revision)
            .unwrap_or(true)
        {
            return Ok(FrameWaitResult {
                frame: current,
                changed: true,
                timed_out: false,
                elapsed_ms: 0,
            });
        }
        let next_index = (*cursor + 1..self.frames.len()).find(|index| {
            after_revision
                .map(|revision| self.frames[*index].revision != revision)
                .unwrap_or(true)
        });
        match next_index {
            Some(index) => {
                *cursor = index;
                Ok(FrameWaitResult {
                    frame: self.frames[index].clone(),
                    changed: true,
                    timed_out: false,
                    elapsed_ms: 0,
                })
            }
            None => Ok(FrameWaitResult {
                frame: current,
                changed: false,
                timed_out: true,
                elapsed_ms: request.timeout_ms,
            }),
        }
    }

    async fn close(&self) -> Result<()> {
        let mut closed = lock(&self.closed)?;
        if !*closed {
            *closed = true;
            lock(&self.closed_sessions)?.push(self.metadata.id.clone());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct MockWindowBackend {
    windows: Vec<WindowInfo>,
    active_window: Arc<Mutex<Option<WindowInfo>>>,
    active_window_reads: Arc<Mutex<u64>>,
    focused_windows: Arc<Mutex<Vec<WindowId>>>,
    resized_windows: Arc<Mutex<Vec<(WindowId, u32, u32)>>>,
}

impl MockWindowBackend {
    pub fn new(windows: Vec<WindowInfo>, active_window: Option<WindowInfo>) -> Self {
        Self {
            windows,
            active_window: Arc::new(Mutex::new(active_window)),
            active_window_reads: Arc::new(Mutex::new(0)),
            focused_windows: Arc::new(Mutex::new(Vec::new())),
            resized_windows: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn focused_windows(&self) -> Result<Vec<WindowId>> {
        Ok(lock(&self.focused_windows)?.clone())
    }

    pub fn resized_windows(&self) -> Result<Vec<(WindowId, u32, u32)>> {
        Ok(lock(&self.resized_windows)?.clone())
    }

    pub fn set_active_window(&self, window: Option<WindowInfo>) -> Result<()> {
        *lock(&self.active_window)? = window;
        Ok(())
    }

    pub fn active_window_reads(&self) -> Result<u64> {
        Ok(*lock(&self.active_window_reads)?)
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
    fn backend_name(&self) -> &'static str {
        "mock-window"
    }

    async fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(self.windows.clone())
    }

    async fn active_window(&self) -> Result<Option<WindowInfo>> {
        *lock(&self.active_window_reads)? += 1;
        Ok(lock(&self.active_window)?.clone())
    }

    async fn focus_window(&self, id: WindowId) -> Result<()> {
        lock(&self.focused_windows)?.push(id);
        Ok(())
    }

    async fn move_window(&self, id: WindowId, x: i32, y: i32) -> Result<WindowGeometry> {
        let mut geometry = self
            .windows
            .iter()
            .find(|window| window.id == id)
            .and_then(|window| window.geometry.clone())
            .ok_or_else(|| SeatgeistError::BackendUnavailable("mock window not found".into()))?;
        geometry.x = x;
        geometry.y = y;
        Ok(geometry)
    }

    async fn resize_window(&self, id: WindowId, width: u32, height: u32) -> Result<WindowGeometry> {
        lock(&self.resized_windows)?.push((id.clone(), width, height));
        let mut geometry = self
            .windows
            .iter()
            .find(|window| window.id == id)
            .and_then(|window| window.geometry.clone())
            .ok_or_else(|| SeatgeistError::BackendUnavailable("mock window not found".into()))?;
        geometry.width = width;
        geometry.height = height;
        Ok(geometry)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityCaretSet {
    pub node_id: String,
    pub offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilitySelectionSet {
    pub node_id: String,
    pub selection_num: i32,
    pub start_offset: i32,
    pub end_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockAccessibilityTextAttributesRequest {
    pub node_id: String,
    pub offset: i32,
    pub include_defaults: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MockAccessibilityValueSet {
    pub node_id: String,
    pub value: f64,
}

#[derive(Debug, Clone)]
pub struct MockAccessibilityBackend {
    focused_tree: AccessibilityNode,
    find_matches: Vec<AccessibilityNode>,
    text_attributes: AccessibilityTextAttributes,
    find_requests: Arc<Mutex<Vec<AccessibilityFindRequest>>>,
    text_attribute_requests: Arc<Mutex<Vec<MockAccessibilityTextAttributesRequest>>>,
    invocations: Arc<Mutex<Vec<MockAccessibilityInvocation>>>,
    text_sets: Arc<Mutex<Vec<MockAccessibilityTextSet>>>,
    text_inserts: Arc<Mutex<Vec<MockAccessibilityTextInsert>>>,
    text_deletes: Arc<Mutex<Vec<MockAccessibilityTextDelete>>>,
    text_copies: Arc<Mutex<Vec<MockAccessibilityTextCopy>>>,
    text_cuts: Arc<Mutex<Vec<MockAccessibilityTextCut>>>,
    text_pastes: Arc<Mutex<Vec<MockAccessibilityTextPaste>>>,
    caret_sets: Arc<Mutex<Vec<MockAccessibilityCaretSet>>>,
    selection_sets: Arc<Mutex<Vec<MockAccessibilitySelectionSet>>>,
    value_sets: Arc<Mutex<Vec<MockAccessibilityValueSet>>>,
}

impl MockAccessibilityBackend {
    pub fn new(focused_tree: AccessibilityNode) -> Self {
        Self {
            focused_tree,
            find_matches: Vec::new(),
            text_attributes: sample_text_attributes(),
            find_requests: Arc::new(Mutex::new(Vec::new())),
            text_attribute_requests: Arc::new(Mutex::new(Vec::new())),
            invocations: Arc::new(Mutex::new(Vec::new())),
            text_sets: Arc::new(Mutex::new(Vec::new())),
            text_inserts: Arc::new(Mutex::new(Vec::new())),
            text_deletes: Arc::new(Mutex::new(Vec::new())),
            text_copies: Arc::new(Mutex::new(Vec::new())),
            text_cuts: Arc::new(Mutex::new(Vec::new())),
            text_pastes: Arc::new(Mutex::new(Vec::new())),
            caret_sets: Arc::new(Mutex::new(Vec::new())),
            selection_sets: Arc::new(Mutex::new(Vec::new())),
            value_sets: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn with_find_matches(mut self, find_matches: Vec<AccessibilityNode>) -> Self {
        self.find_matches = find_matches;
        self
    }

    pub fn with_text_attributes(mut self, text_attributes: AccessibilityTextAttributes) -> Self {
        self.text_attributes = text_attributes;
        self
    }

    pub fn find_requests(&self) -> Result<Vec<AccessibilityFindRequest>> {
        Ok(lock(&self.find_requests)?.clone())
    }

    pub fn text_attribute_requests(&self) -> Result<Vec<MockAccessibilityTextAttributesRequest>> {
        Ok(lock(&self.text_attribute_requests)?.clone())
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

    pub fn caret_sets(&self) -> Result<Vec<MockAccessibilityCaretSet>> {
        Ok(lock(&self.caret_sets)?.clone())
    }

    pub fn selection_sets(&self) -> Result<Vec<MockAccessibilitySelectionSet>> {
        Ok(lock(&self.selection_sets)?.clone())
    }

    pub fn value_sets(&self) -> Result<Vec<MockAccessibilityValueSet>> {
        Ok(lock(&self.value_sets)?.clone())
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

    async fn text_attributes(
        &self,
        node_id: &str,
        offset: i32,
        include_defaults: bool,
    ) -> Result<AccessibilityTextAttributes> {
        lock(&self.text_attribute_requests)?.push(MockAccessibilityTextAttributesRequest {
            node_id: node_id.to_string(),
            offset,
            include_defaults,
        });
        Ok(self.text_attributes.clone())
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

    async fn set_caret(&self, node_id: &str, offset: i32) -> Result<()> {
        lock(&self.caret_sets)?.push(MockAccessibilityCaretSet {
            node_id: node_id.to_string(),
            offset,
        });
        Ok(())
    }

    async fn set_selection(
        &self,
        node_id: &str,
        selection_num: i32,
        start_offset: i32,
        end_offset: i32,
    ) -> Result<()> {
        lock(&self.selection_sets)?.push(MockAccessibilitySelectionSet {
            node_id: node_id.to_string(),
            selection_num,
            start_offset,
            end_offset,
        });
        Ok(())
    }

    async fn set_value(&self, node_id: &str, value: f64) -> Result<()> {
        lock(&self.value_sets)?.push(MockAccessibilityValueSet {
            node_id: node_id.to_string(),
            value,
        });
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| SeatgeistError::Io("mock backend lock poisoned".to_string()))
}

#[cfg(test)]
mod tests {
    use libseatgeist::{AccessibilityAction, AccessibilityFindRequest, CoordinateSpace, Point};
    use seatgeist_backend::{
        AccessibilityBackend, ClipboardBackend, InputBackend, ScreenBackend, WindowBackend,
    };

    use super::*;

    #[tokio::test]
    async fn mock_screen_retains_capture_session_and_advances_frames() -> Result<()> {
        let first = CapturedFrame {
            screenshot: Screenshot {
                path: "mock-frame-1.png".to_string(),
                source_width: 1600,
                source_height: 900,
                width: 1600,
                height: 900,
            },
            revision: "revision-1".to_string(),
            sequence: 1,
            complete: true,
            damage_present: false,
        };
        let second = CapturedFrame {
            screenshot: Screenshot {
                path: "mock-frame-2.png".to_string(),
                source_width: 1600,
                source_height: 900,
                width: 1600,
                height: 900,
            },
            revision: "revision-2".to_string(),
            sequence: 2,
            complete: true,
            damage_present: true,
        };
        let backend = MockScreenBackend::with_frames(
            vec![sample_monitor()],
            vec![first.clone(), second.clone()],
        );

        let capabilities = backend.capabilities().await?;
        assert!(capabilities.retained_sessions);
        assert!(capabilities.wait_for_frame);
        assert_eq!(backend.list_monitors().await?, vec![sample_monitor()]);

        let request = CaptureSessionRequest {
            source: CaptureSource::Window {
                requested_window_id: Some("window-1".to_string()),
            },
            restore_token_reference: Some("restore-ref-1".to_string()),
            persist: true,
            consent_parent_window: String::new(),
            open_timeout_ms: 30_000,
            default_max_edge: 1_600,
        };
        let session = backend.open_capture(request.clone()).await?;
        assert_eq!(session.metadata().id, "mock-capture-1");
        assert_eq!(session.metadata().source_type, CaptureSourceType::Window);
        assert_eq!(session.metadata().source_id.as_deref(), Some("window-1"));
        assert_eq!(backend.capture_requests()?, vec![request]);

        let snapshot_request = FrameRequest {
            output: "mock-snapshot.png".to_string(),
            max_edge: Some(800),
            timeout_ms: 1_500,
        };
        assert_eq!(session.snapshot(snapshot_request.clone()).await?, first);
        assert_eq!(
            backend.snapshot_requests()?,
            vec![("mock-capture-1".to_string(), snapshot_request)]
        );

        let wait_request = FrameWaitRequest {
            after_revision: Some("revision-1".to_string()),
            timeout_ms: 1_000,
            frame: FrameRequest {
                output: "mock-wait.png".to_string(),
                max_edge: Some(800),
                timeout_ms: 1_000,
            },
        };
        let changed = session.wait_for_frame(wait_request.clone()).await?;
        assert!(changed.changed);
        assert!(!changed.timed_out);
        assert_eq!(changed.frame, second);
        assert_eq!(
            backend.wait_requests()?,
            vec![("mock-capture-1".to_string(), wait_request)]
        );

        let timed_out = session
            .wait_for_frame(FrameWaitRequest {
                after_revision: Some("revision-2".to_string()),
                timeout_ms: 250,
                frame: FrameRequest {
                    output: "mock-timeout.png".to_string(),
                    max_edge: Some(800),
                    timeout_ms: 250,
                },
            })
            .await?;
        assert!(!timed_out.changed);
        assert!(timed_out.timed_out);
        assert_eq!(timed_out.elapsed_ms, 250);

        session.close().await?;
        session.close().await?;
        assert_eq!(
            backend.closed_sessions()?,
            vec!["mock-capture-1".to_string()]
        );
        assert!(
            session
                .snapshot(FrameRequest {
                    output: "closed.png".to_string(),
                    max_edge: Some(800),
                    timeout_ms: 100,
                })
                .await
                .is_err()
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
        assert_eq!(
            backend
                .text_attributes("atspi://sample/text", 2, true)
                .await?,
            sample_text_attributes()
        );
        assert_eq!(backend.find_requests()?, vec![find_request]);
        assert_eq!(
            backend.text_attribute_requests()?,
            vec![MockAccessibilityTextAttributesRequest {
                node_id: "atspi://sample/text".to_string(),
                offset: 2,
                include_defaults: true,
            }]
        );
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
        backend.set_caret("atspi://sample/text", 6).await?;
        backend
            .set_selection("atspi://sample/text", 0, 2, 6)
            .await?;
        backend.set_value("atspi://sample/value", 0.75).await?;
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
        assert_eq!(
            backend.caret_sets()?,
            vec![MockAccessibilityCaretSet {
                node_id: "atspi://sample/text".to_string(),
                offset: 6,
            }]
        );
        assert_eq!(
            backend.selection_sets()?,
            vec![MockAccessibilitySelectionSet {
                node_id: "atspi://sample/text".to_string(),
                selection_num: 0,
                start_offset: 2,
                end_offset: 6,
            }]
        );
        assert_eq!(
            backend.value_sets()?,
            vec![MockAccessibilityValueSet {
                node_id: "atspi://sample/value".to_string(),
                value: 0.75,
            }]
        );
        Ok(())
    }
}
