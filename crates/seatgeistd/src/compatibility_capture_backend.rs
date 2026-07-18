use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use libseatgeist::{
    CoordinateSpace, MonitorInfo, PortalScreenshotTarget, ScreenshotInfo, ScreenshotPortalStatus,
    ScreenshotRequest, ScreenshotTransform, SeatgeistError, WindowInfo,
};
use seatgeist_backend::{
    CaptureCapabilities, CaptureSession, CaptureSessionLifecycle, CaptureSessionMetadata,
    CaptureSessionRequest, CaptureSource, CaptureSourceType, CapturedFrame, FrameRequest,
    FrameWaitRequest, FrameWaitResult, Result as BackendResult, ScreenBackend, Screenshot,
};
use uuid::Uuid;

use super::{SafetySettings, WindowListState};

const ONE_SHOT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const ONE_SHOT_TIMEOUT_MS: u64 = 120_000;

#[async_trait]
trait PortalTargetCapture: std::fmt::Debug + Send + Sync {
    async fn capture(
        &self,
        target: PortalScreenshotTarget,
        interactive: bool,
        full_resolution: bool,
        request: &FrameRequest,
    ) -> BackendResult<ScreenshotInfo>;
}

#[async_trait]
pub(super) trait OneShotFrameCapture: std::fmt::Debug + Send + Sync {
    async fn capture(&self, request: &FrameRequest) -> BackendResult<ScreenshotInfo>;
}

#[derive(Debug, Clone)]
struct ProductionPortalTargetCapture {
    safety_settings: SafetySettings,
}

#[async_trait]
impl PortalTargetCapture for ProductionPortalTargetCapture {
    async fn capture(
        &self,
        target: PortalScreenshotTarget,
        interactive: bool,
        full_resolution: bool,
        request: &FrameRequest,
    ) -> BackendResult<ScreenshotInfo> {
        let output = PathBuf::from(&request.output);
        super::screenshot::capture_screenshot_portal(
            ScreenshotRequest {
                output,
                max_edge: request.max_edge,
                full_resolution,
                portal_interactive: interactive,
                portal_target: Some(target),
                visible_window_crop_id: None,
            },
            &self.safety_settings,
        )
        .await
        .map_err(|error| {
            SeatgeistError::BackendUnavailable(format!(
                "portal Screenshot v3 {} capture failed: {error}",
                target.as_str()
            ))
        })?
        .ok_or_else(|| {
            SeatgeistError::BackendUnavailable(format!(
                "portal Screenshot v3 {} request was cancelled",
                target.as_str()
            ))
        })
    }
}

#[derive(Debug)]
struct PortalTargetFrameCapture {
    target: PortalScreenshotTarget,
    interactive: bool,
    full_resolution: bool,
    capture: Arc<dyn PortalTargetCapture>,
}

#[async_trait]
impl OneShotFrameCapture for PortalTargetFrameCapture {
    async fn capture(&self, request: &FrameRequest) -> BackendResult<ScreenshotInfo> {
        self.capture
            .capture(self.target, self.interactive, self.full_resolution, request)
            .await
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PortalScreenshotScreenBackend {
    target: PortalScreenshotTarget,
    interactive: bool,
    full_resolution: bool,
    portal_status: ScreenshotPortalStatus,
    capture: Arc<dyn PortalTargetCapture>,
}

impl PortalScreenshotScreenBackend {
    pub(crate) fn new(
        target: PortalScreenshotTarget,
        interactive: bool,
        full_resolution: bool,
        safety_settings: &SafetySettings,
    ) -> Self {
        Self {
            target,
            interactive,
            full_resolution,
            portal_status: super::screenshot_portal_status(),
            capture: Arc::new(ProductionPortalTargetCapture {
                safety_settings: safety_settings.clone(),
            }),
        }
    }

    fn source_type(&self) -> CaptureSourceType {
        match self.target {
            PortalScreenshotTarget::Window | PortalScreenshotTarget::ActiveWindow => {
                CaptureSourceType::Window
            }
            PortalScreenshotTarget::Screen | PortalScreenshotTarget::Area => {
                CaptureSourceType::DesktopCompatibility
            }
        }
    }

    fn validate_request(&self, request: &CaptureSessionRequest) -> BackendResult<()> {
        if request.open_timeout_ms == 0 {
            return Err(SeatgeistError::InvalidRequest(
                "one-shot capture timeout_ms must be greater than zero".to_string(),
            ));
        }
        if request.default_max_edge == 0 {
            return Err(SeatgeistError::InvalidRequest(
                "one-shot capture default_max_edge must be greater than zero".to_string(),
            ));
        }
        if request.persist || request.restore_token_reference.is_some() {
            return Err(SeatgeistError::InvalidRequest(
                "portal Screenshot v3 is one-shot and cannot persist or restore a session"
                    .to_string(),
            ));
        }
        match (&request.source, self.source_type()) {
            (
                CaptureSource::Window {
                    requested_window_id: None,
                },
                CaptureSourceType::Window,
            )
            | (
                CaptureSource::DesktopCompatibility {
                    requested_window_id: None,
                },
                CaptureSourceType::DesktopCompatibility,
            ) => {}
            (
                CaptureSource::Window {
                    requested_window_id: Some(_),
                },
                CaptureSourceType::Window,
            ) => {
                return Err(SeatgeistError::InvalidRequest(
                    "portal Screenshot v3 cannot bind an arbitrary KWin window id; the portal target remains authoritative"
                        .to_string(),
                ));
            }
            _ => {
                return Err(SeatgeistError::InvalidRequest(format!(
                    "portal Screenshot v3 {} target does not match the requested capture source",
                    self.target.as_str()
                )));
            }
        }
        super::screenshot::validate_portal_screenshot_target_request(
            &ScreenshotRequest {
                output: PathBuf::new(),
                max_edge: Some(request.default_max_edge),
                full_resolution: self.full_resolution,
                portal_interactive: self.interactive,
                portal_target: Some(self.target),
                visible_window_crop_id: None,
            },
            &self.portal_status,
        )
        .map_err(|error| SeatgeistError::BackendUnavailable(error.to_string()))
    }

    fn backend_name(&self) -> String {
        format!("portal_screenshot_v3_{}", self.target.as_str())
    }
}

#[async_trait]
impl ScreenBackend for PortalScreenshotScreenBackend {
    async fn capabilities(&self) -> BackendResult<CaptureCapabilities> {
        Ok(CaptureCapabilities {
            backend: self.backend_name(),
            source_types: vec![self.source_type()],
            retained_sessions: false,
            wait_for_frame: true,
            restore_tokens: false,
            damage_tracking: false,
        })
    }

    async fn list_monitors(&self) -> BackendResult<Vec<libseatgeist::MonitorInfo>> {
        super::list_monitors().map_err(|error| {
            SeatgeistError::BackendUnavailable(format!(
                "KWin monitor discovery failed for Screenshot v3: {error}"
            ))
        })
    }

    async fn open_capture(
        &self,
        request: CaptureSessionRequest,
    ) -> BackendResult<Box<dyn CaptureSession>> {
        self.validate_request(&request)?;
        Ok(Box::new(OneShotCaptureSession::new(
            CaptureSessionMetadata {
                id: format!("screenshot-v3-{}", Uuid::new_v4().simple()),
                backend: self.backend_name(),
                source_type: self.source_type(),
                source_id: Some(self.target.as_str().to_string()),
                restore_token_reference: None,
                consent_required: true,
                occlusion_possible: false,
            },
            Arc::new(PortalTargetFrameCapture {
                target: self.target,
                interactive: self.interactive,
                full_resolution: self.full_resolution,
                capture: Arc::clone(&self.capture),
            }),
        )))
    }
}

#[derive(Debug)]
pub(super) struct OneShotCaptureSession {
    metadata: CaptureSessionMetadata,
    capture: Arc<dyn OneShotFrameCapture>,
    sequence: AtomicU64,
    closed: AtomicBool,
}

impl OneShotCaptureSession {
    pub(super) fn new(
        metadata: CaptureSessionMetadata,
        capture: Arc<dyn OneShotFrameCapture>,
    ) -> Self {
        Self {
            metadata,
            capture,
            sequence: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        }
    }

    fn ensure_open(&self) -> BackendResult<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SeatgeistError::BackendUnavailable(
                "one-shot capture session is closed".to_string(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl CaptureSession for OneShotCaptureSession {
    fn metadata(&self) -> CaptureSessionMetadata {
        self.metadata.clone()
    }

    async fn lifecycle(&self) -> CaptureSessionLifecycle {
        if self.closed.load(Ordering::Acquire) {
            CaptureSessionLifecycle::ClientClosed
        } else {
            CaptureSessionLifecycle::Open
        }
    }

    async fn snapshot(&self, request: FrameRequest) -> BackendResult<CapturedFrame> {
        self.ensure_open()?;
        let info = self.capture.capture(&request).await?;
        let revision = super::sha256_file(&info.path).map_err(|error| {
            SeatgeistError::Io(format!("hash Screenshot v3 output failed: {error}"))
        })?;
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        Ok(CapturedFrame {
            screenshot: Screenshot {
                path: info.path.display().to_string(),
                source_width: info.source_width,
                source_height: info.source_height,
                width: info.output_width,
                height: info.output_height,
            },
            revision,
            sequence,
            complete: true,
            damage_present: false,
        })
    }

    async fn wait_for_frame(&self, request: FrameWaitRequest) -> BackendResult<FrameWaitResult> {
        self.ensure_open()?;
        let started = Instant::now();
        loop {
            let frame = self.snapshot(request.frame.clone()).await?;
            let changed = request
                .after_revision
                .as_deref()
                .is_none_or(|revision| revision != frame.revision);
            if changed || started.elapsed() >= Duration::from_millis(request.timeout_ms) {
                return Ok(FrameWaitResult {
                    frame,
                    changed,
                    timed_out: !changed,
                    elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                });
            }
            let remaining =
                Duration::from_millis(request.timeout_ms).saturating_sub(started.elapsed());
            tokio::time::sleep(ONE_SHOT_POLL_INTERVAL.min(remaining)).await;
        }
    }

    async fn close(&self) -> BackendResult<()> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CropRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

trait WindowCropResolver: std::fmt::Debug + Send + Sync {
    fn resolve(&self, window_id: &str) -> BackendResult<(WindowInfo, Vec<MonitorInfo>)>;
    fn monitors(&self) -> BackendResult<Vec<MonitorInfo>>;
}

#[derive(Debug, Clone)]
struct ProductionWindowCropResolver {
    window_list_state: WindowListState,
}

impl WindowCropResolver for ProductionWindowCropResolver {
    fn resolve(&self, window_id: &str) -> BackendResult<(WindowInfo, Vec<MonitorInfo>)> {
        let monitors = super::list_monitors().map_err(|error| {
            SeatgeistError::BackendUnavailable(format!(
                "KWin monitor discovery failed for visible crop: {error}"
            ))
        })?;
        let windows = super::list_windows_with_monitors(&self.window_list_state, &monitors)
            .map_err(|error| {
                SeatgeistError::BackendUnavailable(format!(
                    "KWin window discovery failed for visible crop: {error}"
                ))
            })?;
        let window = windows
            .into_iter()
            .find(|window| window.id == window_id)
            .ok_or_else(|| {
                SeatgeistError::InvalidRequest(
                    "visible-window crop target no longer exists".to_string(),
                )
            })?;
        Ok((window, monitors))
    }

    fn monitors(&self) -> BackendResult<Vec<MonitorInfo>> {
        super::list_monitors().map_err(|error| {
            SeatgeistError::BackendUnavailable(format!(
                "KWin monitor discovery failed for visible crop: {error}"
            ))
        })
    }
}

#[derive(Debug, Clone)]
struct ProductionDesktopCapture {
    safety_settings: SafetySettings,
    window_list_state: WindowListState,
}

#[async_trait]
impl OneShotFrameCapture for ProductionDesktopCapture {
    async fn capture(&self, request: &FrameRequest) -> BackendResult<ScreenshotInfo> {
        super::capture_screenshot(
            ScreenshotRequest {
                output: PathBuf::from(&request.output),
                max_edge: None,
                full_resolution: true,
                portal_interactive: false,
                portal_target: None,
                visible_window_crop_id: None,
            },
            &self.safety_settings,
            &self.window_list_state,
        )
        .await
        .map_err(|error| {
            SeatgeistError::BackendUnavailable(format!(
                "composed desktop source for visible-window crop failed: {error}"
            ))
        })
    }
}

#[derive(Debug)]
struct VisibleWindowCropFrameCapture {
    window: WindowInfo,
    monitors: Vec<MonitorInfo>,
    desktop_capture: Arc<dyn OneShotFrameCapture>,
}

#[async_trait]
impl OneShotFrameCapture for VisibleWindowCropFrameCapture {
    async fn capture(&self, request: &FrameRequest) -> BackendResult<ScreenshotInfo> {
        let output = PathBuf::from(&request.output);
        let source = output.with_file_name(format!(
            ".seatgeist-visible-crop-{}.png",
            Uuid::new_v4().simple()
        ));
        let desktop = self
            .desktop_capture
            .capture(&FrameRequest {
                output: source.display().to_string(),
                max_edge: None,
                timeout_ms: request.timeout_ms,
            })
            .await;
        let desktop = match desktop {
            Ok(desktop) => desktop,
            Err(error) => {
                std::fs::remove_file(&source).ok();
                return Err(error);
            }
        };
        let result = crop_visible_window(
            &desktop,
            &output,
            &self.window,
            &self.monitors,
            request.max_edge,
        );
        std::fs::remove_file(&source).ok();
        result
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VisibleWindowCropScreenBackend {
    resolver: Arc<dyn WindowCropResolver>,
    desktop_capture: Arc<dyn OneShotFrameCapture>,
}

impl VisibleWindowCropScreenBackend {
    pub(crate) fn new(
        safety_settings: &SafetySettings,
        window_list_state: &WindowListState,
    ) -> Self {
        Self {
            resolver: Arc::new(ProductionWindowCropResolver {
                window_list_state: window_list_state.clone(),
            }),
            desktop_capture: Arc::new(ProductionDesktopCapture {
                safety_settings: safety_settings.clone(),
                window_list_state: window_list_state.clone(),
            }),
        }
    }

    fn requested_window_id(request: &CaptureSessionRequest) -> BackendResult<&str> {
        if request.open_timeout_ms == 0 || request.default_max_edge == 0 {
            return Err(SeatgeistError::InvalidRequest(
                "visible-window crop timeouts and bounds must be greater than zero".to_string(),
            ));
        }
        if request.persist || request.restore_token_reference.is_some() {
            return Err(SeatgeistError::InvalidRequest(
                "visible-window crop is one-shot and cannot persist or restore a session"
                    .to_string(),
            ));
        }
        let CaptureSource::DesktopCompatibility {
            requested_window_id: Some(window_id),
        } = &request.source
        else {
            return Err(SeatgeistError::InvalidRequest(
                "visible-window crop requires an explicit KWin window id and desktop_compatibility source"
                    .to_string(),
            ));
        };
        let window_id = window_id.trim();
        if window_id.is_empty() {
            return Err(SeatgeistError::InvalidRequest(
                "visible-window crop id must not be blank".to_string(),
            ));
        }
        Ok(window_id)
    }
}

#[async_trait]
impl ScreenBackend for VisibleWindowCropScreenBackend {
    async fn capabilities(&self) -> BackendResult<CaptureCapabilities> {
        Ok(CaptureCapabilities {
            backend: "visible_window_crop".to_string(),
            source_types: vec![CaptureSourceType::DesktopCompatibility],
            retained_sessions: false,
            wait_for_frame: true,
            restore_tokens: false,
            damage_tracking: false,
        })
    }

    async fn list_monitors(&self) -> BackendResult<Vec<MonitorInfo>> {
        self.resolver.monitors()
    }

    async fn open_capture(
        &self,
        request: CaptureSessionRequest,
    ) -> BackendResult<Box<dyn CaptureSession>> {
        let window_id = Self::requested_window_id(&request)?;
        let (window, monitors) = self.resolver.resolve(window_id)?;
        validate_visible_window(&window, &monitors)?;
        Ok(Box::new(OneShotCaptureSession::new(
            CaptureSessionMetadata {
                id: format!("visible-crop-{}", Uuid::new_v4().simple()),
                backend: "visible_window_crop".to_string(),
                source_type: CaptureSourceType::DesktopCompatibility,
                source_id: Some(window.id.clone()),
                restore_token_reference: None,
                consent_required: true,
                occlusion_possible: true,
            },
            Arc::new(VisibleWindowCropFrameCapture {
                window,
                monitors,
                desktop_capture: Arc::clone(&self.desktop_capture),
            }),
        )))
    }
}

fn validate_visible_window(window: &WindowInfo, monitors: &[MonitorInfo]) -> BackendResult<()> {
    let geometry = window.geometry.as_ref().ok_or_else(|| {
        SeatgeistError::InvalidRequest("visible-window crop target has no geometry".to_string())
    })?;
    if geometry.space != CoordinateSpace::LogicalPixel
        || geometry.width == 0
        || geometry.height == 0
    {
        return Err(SeatgeistError::InvalidRequest(
            "visible-window crop requires non-empty logical KWin geometry".to_string(),
        ));
    }
    monitor_containing_window(window, monitors)?;
    Ok(())
}

fn monitor_containing_window<'a>(
    window: &WindowInfo,
    monitors: &'a [MonitorInfo],
) -> BackendResult<&'a MonitorInfo> {
    let geometry = window.geometry.as_ref().ok_or_else(|| {
        SeatgeistError::InvalidRequest("visible-window crop target has no geometry".to_string())
    })?;
    let left = i64::from(geometry.x);
    let top = i64::from(geometry.y);
    let right = left + i64::from(geometry.width);
    let bottom = top + i64::from(geometry.height);
    monitors
        .iter()
        .find(|monitor| {
            let monitor_left = i64::from(monitor.logical_origin_x);
            let monitor_top = i64::from(monitor.logical_origin_y);
            let monitor_right = monitor_left + i64::from(monitor.logical_width);
            let monitor_bottom = monitor_top + i64::from(monitor.logical_height);
            left >= monitor_left
                && top >= monitor_top
                && right <= monitor_right
                && bottom <= monitor_bottom
                && monitor
                    .transform
                    .as_deref()
                    .is_none_or(|transform| transform.eq_ignore_ascii_case("normal"))
        })
        .ok_or_else(|| {
            SeatgeistError::InvalidRequest(
                "visible-window crop currently requires one unrotated monitor; spanning, off-screen, or transformed windows fail closed"
                    .to_string(),
            )
        })
}

fn crop_visible_window(
    desktop: &ScreenshotInfo,
    output: &std::path::Path,
    window: &WindowInfo,
    monitors: &[MonitorInfo],
    max_edge: Option<u32>,
) -> BackendResult<ScreenshotInfo> {
    let rect = visible_window_crop_rect(
        window,
        monitors,
        desktop.output_width,
        desktop.output_height,
    )?;
    let image = image::open(&desktop.path).map_err(|error| {
        SeatgeistError::Io(format!(
            "open composed desktop for visible crop failed: {error}"
        ))
    })?;
    if image.width() != desktop.output_width || image.height() != desktop.output_height {
        return Err(SeatgeistError::Io(
            "composed desktop dimensions changed before visible crop".to_string(),
        ));
    }
    let cropped = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
    let output_image = match max_edge {
        Some(0) => {
            return Err(SeatgeistError::InvalidRequest(
                "visible-window crop max_edge must be greater than zero".to_string(),
            ));
        }
        Some(max_edge) if rect.width.max(rect.height) > max_edge => {
            let scale = f64::from(max_edge) / f64::from(rect.width.max(rect.height));
            cropped.resize(
                super::screenshot_image::scaled_dimension(rect.width, scale),
                super::screenshot_image::scaled_dimension(rect.height, scale),
                image::imageops::FilterType::Lanczos3,
            )
        }
        _ => cropped,
    };
    let output_width = output_image.width();
    let output_height = output_image.height();
    output_image.save(output).map_err(|error| {
        SeatgeistError::Io(format!("write visible-window crop failed: {error}"))
    })?;
    Ok(ScreenshotInfo {
        path: output.to_path_buf(),
        backend: "visible_window_crop".to_string(),
        occlusion_possible: true,
        source_width: rect.width,
        source_height: rect.height,
        output_width,
        output_height,
        transform: ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::WindowLocal,
            output_coordinate_space: CoordinateSpace::WindowLocal,
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: f64::from(output_width) / f64::from(rect.width),
            scale_y: f64::from(output_height) / f64::from(rect.height),
        },
        coordinate_space: CoordinateSpace::WindowLocal,
        monitors: monitors.to_vec(),
    })
}

fn visible_window_crop_rect(
    window: &WindowInfo,
    monitors: &[MonitorInfo],
    desktop_width: u32,
    desktop_height: u32,
) -> BackendResult<CropRect> {
    let geometry = window.geometry.as_ref().ok_or_else(|| {
        SeatgeistError::InvalidRequest("visible-window crop target has no geometry".to_string())
    })?;
    let monitor = monitor_containing_window(window, monitors)?;
    let physical_bounds = monitors
        .iter()
        .map(|monitor| {
            let x = super::pointer_coordinates::scaled_physical_origin(
                monitor.logical_origin_x,
                monitor.scale_factor,
            )
            .map_err(|error| SeatgeistError::InvalidRequest(error.to_string()))?;
            let y = super::pointer_coordinates::scaled_physical_origin(
                monitor.logical_origin_y,
                monitor.scale_factor,
            )
            .map_err(|error| SeatgeistError::InvalidRequest(error.to_string()))?;
            Ok((
                i64::from(x),
                i64::from(y),
                i64::from(x) + i64::from(monitor.physical_width),
                i64::from(y) + i64::from(monitor.physical_height),
            ))
        })
        .collect::<BackendResult<Vec<_>>>()?;
    let min_x = physical_bounds
        .iter()
        .map(|bounds| bounds.0)
        .min()
        .ok_or_else(|| {
            SeatgeistError::BackendUnavailable(
                "visible-window crop has no monitor metadata".to_string(),
            )
        })?;
    let min_y = physical_bounds
        .iter()
        .map(|bounds| bounds.1)
        .min()
        .unwrap_or(0);
    let max_x = physical_bounds
        .iter()
        .map(|bounds| bounds.2)
        .max()
        .unwrap_or(0);
    let max_y = physical_bounds
        .iter()
        .map(|bounds| bounds.3)
        .max()
        .unwrap_or(0);
    if max_x - min_x != i64::from(desktop_width) || max_y - min_y != i64::from(desktop_height) {
        return Err(SeatgeistError::BackendUnavailable(format!(
            "composed desktop {}x{} does not match KWin physical bounds {}x{}; refusing an uncertain crop",
            desktop_width,
            desktop_height,
            max_x - min_x,
            max_y - min_y
        )));
    }
    let monitor_x = super::pointer_coordinates::scaled_physical_origin(
        monitor.logical_origin_x,
        monitor.scale_factor,
    )
    .map_err(|error| SeatgeistError::InvalidRequest(error.to_string()))?;
    let monitor_y = super::pointer_coordinates::scaled_physical_origin(
        monitor.logical_origin_y,
        monitor.scale_factor,
    )
    .map_err(|error| SeatgeistError::InvalidRequest(error.to_string()))?;
    let global_x = f64::from(monitor_x)
        + f64::from(geometry.x - monitor.logical_origin_x) * monitor.scale_factor;
    let global_y = f64::from(monitor_y)
        + f64::from(geometry.y - monitor.logical_origin_y) * monitor.scale_factor;
    let width = (f64::from(geometry.width) * monitor.scale_factor).round();
    let height = (f64::from(geometry.height) * monitor.scale_factor).round();
    let x = (global_x - min_x as f64).round();
    let y = (global_y - min_y as f64).round();
    if x < 0.0 || y < 0.0 || width <= 0.0 || height <= 0.0 {
        return Err(SeatgeistError::InvalidRequest(
            "visible-window crop resolved outside the composed desktop".to_string(),
        ));
    }
    let rect = CropRect {
        x: x as u32,
        y: y as u32,
        width: width as u32,
        height: height as u32,
    };
    if rect.x.saturating_add(rect.width) > desktop_width
        || rect.y.saturating_add(rect.height) > desktop_height
    {
        return Err(SeatgeistError::InvalidRequest(
            "visible-window crop exceeds the composed desktop".to_string(),
        ));
    }
    Ok(rect)
}

pub(crate) async fn capture_visible_window_crop(
    request: ScreenshotRequest,
    safety_settings: &SafetySettings,
    window_list_state: &WindowListState,
) -> anyhow::Result<ScreenshotInfo> {
    if request.portal_interactive {
        anyhow::bail!(
            "visible_window_crop_id cannot be combined with portal_interactive; the exact KWin crop target must remain stable"
        );
    }
    let window_id = request
        .visible_window_crop_id
        .as_deref()
        .map(str::trim)
        .filter(|window_id| !window_id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("visible_window_crop_id must not be blank"))?;
    let backend = VisibleWindowCropScreenBackend::new(safety_settings, window_list_state);
    let session = backend
        .open_capture(CaptureSessionRequest {
            source: CaptureSource::DesktopCompatibility {
                requested_window_id: Some(window_id.to_string()),
            },
            restore_token_reference: None,
            persist: false,
            consent_parent_window: String::new(),
            open_timeout_ms: ONE_SHOT_TIMEOUT_MS,
            default_max_edge: request.max_edge.unwrap_or(safety_settings.preview_max_edge),
        })
        .await
        .map_err(anyhow::Error::new)?;
    let metadata = session.metadata();
    let max_edge = if request.full_resolution {
        None
    } else {
        request.max_edge.or(Some(safety_settings.preview_max_edge))
    };
    let frame = session
        .snapshot(FrameRequest {
            output: request.output.display().to_string(),
            max_edge,
            timeout_ms: ONE_SHOT_TIMEOUT_MS,
        })
        .await;
    let _ = session.close().await;
    let frame = frame.map_err(anyhow::Error::new)?;
    let screenshot = frame.screenshot;
    let monitors = backend.list_monitors().await.unwrap_or_default();
    Ok(ScreenshotInfo {
        path: PathBuf::from(screenshot.path),
        backend: metadata.backend,
        occlusion_possible: metadata.occlusion_possible,
        source_width: screenshot.source_width,
        source_height: screenshot.source_height,
        output_width: screenshot.width,
        output_height: screenshot.height,
        transform: ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::WindowLocal,
            output_coordinate_space: CoordinateSpace::WindowLocal,
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: f64::from(screenshot.width) / f64::from(screenshot.source_width.max(1)),
            scale_y: f64::from(screenshot.height) / f64::from(screenshot.source_height.max(1)),
        },
        coordinate_space: CoordinateSpace::WindowLocal,
        monitors,
    })
}

pub(crate) async fn capture_portal_target(
    request: ScreenshotRequest,
    safety_settings: &SafetySettings,
) -> anyhow::Result<ScreenshotInfo> {
    let target = request
        .portal_target
        .ok_or_else(|| anyhow::anyhow!("portal Screenshot v3 target is required"))?;
    let backend = PortalScreenshotScreenBackend::new(
        target,
        request.portal_interactive,
        request.full_resolution,
        safety_settings,
    );
    let source = match target {
        PortalScreenshotTarget::Window | PortalScreenshotTarget::ActiveWindow => {
            CaptureSource::Window {
                requested_window_id: None,
            }
        }
        PortalScreenshotTarget::Screen | PortalScreenshotTarget::Area => {
            CaptureSource::DesktopCompatibility {
                requested_window_id: None,
            }
        }
    };
    let session = backend
        .open_capture(CaptureSessionRequest {
            source,
            restore_token_reference: None,
            persist: false,
            consent_parent_window: String::new(),
            open_timeout_ms: ONE_SHOT_TIMEOUT_MS,
            default_max_edge: request.max_edge.unwrap_or(safety_settings.preview_max_edge),
        })
        .await
        .map_err(anyhow::Error::new)?;
    let metadata = session.metadata();
    let frame = session
        .snapshot(FrameRequest {
            output: request.output.display().to_string(),
            max_edge: request.max_edge,
            timeout_ms: ONE_SHOT_TIMEOUT_MS,
        })
        .await;
    let _ = session.close().await;
    let frame = frame.map_err(anyhow::Error::new)?;
    let screenshot = frame.screenshot;
    let monitors = backend.list_monitors().await.unwrap_or_default();
    Ok(ScreenshotInfo {
        path: PathBuf::from(screenshot.path),
        backend: metadata.backend,
        occlusion_possible: metadata.occlusion_possible,
        source_width: screenshot.source_width,
        source_height: screenshot.source_height,
        output_width: screenshot.width,
        output_height: screenshot.height,
        transform: ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: f64::from(screenshot.width) / f64::from(screenshot.source_width.max(1)),
            scale_y: f64::from(screenshot.height) / f64::from(screenshot.source_height.max(1)),
        },
        coordinate_space: CoordinateSpace::PhysicalPixel,
        monitors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct FakeCapture {
        calls: Mutex<Vec<PortalScreenshotTarget>>,
    }

    #[derive(Debug, Clone)]
    struct FakeResolver {
        window: WindowInfo,
        monitors: Vec<MonitorInfo>,
    }

    impl WindowCropResolver for FakeResolver {
        fn resolve(&self, window_id: &str) -> BackendResult<(WindowInfo, Vec<MonitorInfo>)> {
            if self.window.id != window_id {
                return Err(SeatgeistError::InvalidRequest(
                    "fake visible-window target missing".to_string(),
                ));
            }
            Ok((self.window.clone(), self.monitors.clone()))
        }

        fn monitors(&self) -> BackendResult<Vec<MonitorInfo>> {
            Ok(self.monitors.clone())
        }
    }

    #[derive(Debug)]
    struct FakeDesktopCapture;

    #[async_trait]
    impl OneShotFrameCapture for FakeDesktopCapture {
        async fn capture(&self, request: &FrameRequest) -> BackendResult<ScreenshotInfo> {
            let path = PathBuf::from(&request.output);
            image::RgbaImage::from_pixel(100, 100, image::Rgba([7, 8, 9, 255]))
                .save(&path)
                .expect("fake desktop PNG writes");
            Ok(ScreenshotInfo {
                path,
                backend: "fake_desktop".to_string(),
                occlusion_possible: false,
                source_width: 100,
                source_height: 100,
                output_width: 100,
                output_height: 100,
                transform: ScreenshotTransform {
                    source_coordinate_space: CoordinateSpace::PhysicalPixel,
                    output_coordinate_space: CoordinateSpace::PhysicalPixel,
                    source_origin_x: 0,
                    source_origin_y: 0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
                coordinate_space: CoordinateSpace::PhysicalPixel,
                monitors: Vec::new(),
            })
        }
    }

    #[async_trait]
    impl PortalTargetCapture for FakeCapture {
        async fn capture(
            &self,
            target: PortalScreenshotTarget,
            _interactive: bool,
            _full_resolution: bool,
            request: &FrameRequest,
        ) -> BackendResult<ScreenshotInfo> {
            self.calls.lock().expect("calls lock").push(target);
            let path = PathBuf::from(&request.output);
            image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255]))
                .save(&path)
                .expect("fake PNG writes");
            Ok(ScreenshotInfo {
                path,
                backend: "provider-internal".to_string(),
                occlusion_possible: false,
                source_width: 4,
                source_height: 2,
                output_width: 4,
                output_height: 2,
                transform: ScreenshotTransform {
                    source_coordinate_space: CoordinateSpace::PhysicalPixel,
                    output_coordinate_space: CoordinateSpace::PhysicalPixel,
                    source_origin_x: 0,
                    source_origin_y: 0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
                coordinate_space: CoordinateSpace::PhysicalPixel,
                monitors: Vec::new(),
            })
        }
    }

    fn portal_status() -> ScreenshotPortalStatus {
        ScreenshotPortalStatus {
            busctl_available: true,
            portal_service_available: true,
            screenshot_interface_available: true,
            screenshot_interface_version: Some(3),
            screenshot_available_targets_mask: Some(1 | 2 | 4 | 8),
            screenshot_available_targets: vec![
                "screen".to_string(),
                "window".to_string(),
                "area".to_string(),
                "active_window".to_string(),
            ],
            screenshot_target_option_supported: true,
            screencast_interface_available: true,
            kde_portal_service_available: true,
            setup_hint: String::new(),
        }
    }

    fn backend(
        target: PortalScreenshotTarget,
        capture: Arc<dyn PortalTargetCapture>,
    ) -> PortalScreenshotScreenBackend {
        PortalScreenshotScreenBackend {
            target,
            interactive: false,
            full_resolution: false,
            portal_status: portal_status(),
            capture,
        }
    }

    fn open_request(source: CaptureSource) -> CaptureSessionRequest {
        CaptureSessionRequest {
            source,
            restore_token_reference: None,
            persist: false,
            consent_parent_window: String::new(),
            open_timeout_ms: 1_000,
            default_max_edge: 1_600,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "seatgeist-compat-{}-{name}",
            Uuid::new_v4().simple()
        ))
    }

    fn crop_fixture() -> (WindowInfo, Vec<MonitorInfo>) {
        (
            WindowInfo {
                id: "kwin-window-1".to_string(),
                app_id: Some("org.example.App".to_string()),
                title: "must-not-enter-capture-metadata".to_string(),
                pid: Some(4242),
                monitor_id: Some("monitor-1".to_string()),
                geometry: Some(libseatgeist::WindowGeometry {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 20,
                    space: CoordinateSpace::LogicalPixel,
                }),
            },
            vec![MonitorInfo {
                id: "monitor-1".to_string(),
                name: Some("test".to_string()),
                physical_width: 100,
                physical_height: 100,
                logical_width: 100,
                logical_height: 100,
                scale_factor: 1.0,
                logical_origin_x: 0,
                logical_origin_y: 0,
                transform: Some("normal".to_string()),
            }],
        )
    }

    #[tokio::test]
    async fn active_window_target_runs_through_one_shot_screen_backend() {
        let capture = Arc::new(FakeCapture {
            calls: Mutex::new(Vec::new()),
        });
        let backend = backend(PortalScreenshotTarget::ActiveWindow, capture.clone());
        let capabilities = backend.capabilities().await.expect("capabilities");
        assert_eq!(capabilities.source_types, vec![CaptureSourceType::Window]);
        assert!(!capabilities.retained_sessions);
        let session = backend
            .open_capture(open_request(CaptureSource::Window {
                requested_window_id: None,
            }))
            .await
            .expect("one-shot session opens");
        assert_eq!(
            session.metadata().backend,
            "portal_screenshot_v3_active_window"
        );
        let output = temp_path("screenshot-v3-adapter.png");
        let frame = session
            .snapshot(FrameRequest {
                output: output.display().to_string(),
                max_edge: Some(1_600),
                timeout_ms: 1_000,
            })
            .await
            .expect("snapshot succeeds");
        assert_eq!(frame.sequence, 1);
        assert!(!frame.revision.is_empty());
        assert_eq!(
            capture.calls.lock().expect("calls lock").as_slice(),
            &[PortalScreenshotTarget::ActiveWindow]
        );
        std::fs::remove_file(output).ok();
    }

    #[tokio::test]
    async fn arbitrary_kwin_id_and_restore_state_fail_before_capture() {
        let capture = Arc::new(FakeCapture {
            calls: Mutex::new(Vec::new()),
        });
        let backend = backend(PortalScreenshotTarget::Window, capture.clone());
        let error = backend
            .open_capture(open_request(CaptureSource::Window {
                requested_window_id: Some("kwin-window-1".to_string()),
            }))
            .await
            .err()
            .expect("arbitrary KWin id is not portal authority");
        assert!(error.to_string().contains("cannot bind an arbitrary KWin"));

        let mut persistent = open_request(CaptureSource::Window {
            requested_window_id: None,
        });
        persistent.persist = true;
        let error = backend
            .open_capture(persistent)
            .await
            .err()
            .expect("one-shot persistence is rejected");
        assert!(error.to_string().contains("cannot persist"));
        assert!(capture.calls.lock().expect("calls lock").is_empty());
    }

    #[tokio::test]
    async fn visible_window_crop_is_explicit_occlusion_labeled_screen_backend() {
        let (window, monitors) = crop_fixture();
        let backend = VisibleWindowCropScreenBackend {
            resolver: Arc::new(FakeResolver { window, monitors }),
            desktop_capture: Arc::new(FakeDesktopCapture),
        };
        let capabilities = backend.capabilities().await.expect("capabilities");
        assert_eq!(
            capabilities.source_types,
            vec![CaptureSourceType::DesktopCompatibility]
        );
        assert!(!capabilities.retained_sessions);
        let session = backend
            .open_capture(open_request(CaptureSource::DesktopCompatibility {
                requested_window_id: Some("kwin-window-1".to_string()),
            }))
            .await
            .expect("visible crop session opens");
        let metadata = session.metadata();
        assert_eq!(metadata.backend, "visible_window_crop");
        assert_eq!(
            metadata.source_type,
            CaptureSourceType::DesktopCompatibility
        );
        assert_eq!(metadata.source_id.as_deref(), Some("kwin-window-1"));
        assert!(metadata.occlusion_possible);
        let output = temp_path("visible-window-crop.png");
        let frame = session
            .snapshot(FrameRequest {
                output: output.display().to_string(),
                max_edge: Some(15),
                timeout_ms: 1_000,
            })
            .await
            .expect("visible crop succeeds");
        assert_eq!(frame.screenshot.source_width, 30);
        assert_eq!(frame.screenshot.source_height, 20);
        assert_eq!(frame.screenshot.width, 15);
        assert_eq!(frame.screenshot.height, 10);
        let crop = image::open(&output).expect("crop opens");
        assert_eq!((crop.width(), crop.height()), (15, 10));
        assert!(!format!("{metadata:?}").contains("must-not-enter-capture-metadata"));
        std::fs::remove_file(output).ok();
    }

    #[test]
    fn visible_crop_fails_closed_for_spanning_or_uncertain_geometry() {
        let (mut window, monitors) = crop_fixture();
        window.geometry.as_mut().expect("geometry").width = 101;
        let error = visible_window_crop_rect(&window, &monitors, 100, 100)
            .expect_err("spanning window is rejected");
        assert!(error.to_string().contains("requires one unrotated monitor"));
    }
}
