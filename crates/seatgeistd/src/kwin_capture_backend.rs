use std::{
    collections::HashMap,
    io::Read,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use libseatgeist::{CoordinateSpace, ScreenshotInfo, ScreenshotTransform, SeatgeistError};
use seatgeist_backend::{
    CaptureCapabilities, CaptureSession, CaptureSessionMetadata, CaptureSessionRequest,
    CaptureSource, CaptureSourceType, FrameRequest, Result as BackendResult, ScreenBackend,
};
use uuid::Uuid;
use zbus::zvariant::{Fd, OwnedValue, Value};

use crate::{
    compatibility_capture_backend::{OneShotCaptureSession, OneShotFrameCapture},
    screenshot_image::prepare_screenshot_output,
};

const KWIN_SCREENSHOT_SERVICE: &str = "org.kde.KWin.ScreenShot2";
const KWIN_SCREENSHOT_PATH: &str = "/org/kde/KWin/ScreenShot2";
const KWIN_SCREENSHOT_INTERFACE: &str = "org.kde.KWin.ScreenShot2";
const KWIN_ARGB32_PREMULTIPLIED: u32 = 6;
const MAX_RAW_CAPTURE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub(crate) struct KwinWindowScreenBackend;

#[derive(Debug)]
struct KwinWindowFrameCapture {
    window_id: String,
}

#[async_trait]
impl OneShotFrameCapture for KwinWindowFrameCapture {
    async fn capture(&self, request: &FrameRequest) -> BackendResult<ScreenshotInfo> {
        capture_kwin_window(&self.window_id, request)
            .await
            .map_err(|error| {
                SeatgeistError::BackendUnavailable(format!(
                    "KWin exact-window screenshot failed: {error}"
                ))
            })
    }
}

#[async_trait]
impl ScreenBackend for KwinWindowScreenBackend {
    async fn capabilities(&self) -> BackendResult<CaptureCapabilities> {
        Ok(CaptureCapabilities {
            backend: "kwin_screenshot2_window".to_string(),
            source_types: vec![CaptureSourceType::Window],
            retained_sessions: false,
            wait_for_frame: true,
            restore_tokens: false,
            damage_tracking: false,
        })
    }

    async fn list_monitors(&self) -> BackendResult<Vec<libseatgeist::MonitorInfo>> {
        seatgeist_kwin::list_monitors().map_err(|error| {
            SeatgeistError::BackendUnavailable(format!("KWin monitor discovery failed: {error}"))
        })
    }

    async fn open_capture(
        &self,
        request: CaptureSessionRequest,
    ) -> BackendResult<Box<dyn CaptureSession>> {
        if request.open_timeout_ms == 0 || request.default_max_edge == 0 {
            return Err(SeatgeistError::InvalidRequest(
                "KWin exact-window capture timeouts and bounds must be greater than zero"
                    .to_string(),
            ));
        }
        if request.persist || request.restore_token_reference.is_some() {
            return Err(SeatgeistError::InvalidRequest(
                "KWin exact-window capture is direct and does not use portal persistence"
                    .to_string(),
            ));
        }
        let CaptureSource::Window {
            requested_window_id: Some(window_id),
        } = request.source
        else {
            return Err(SeatgeistError::InvalidRequest(
                "KWin exact-window capture requires one explicit KWin window id".to_string(),
            ));
        };
        let window_id = window_id.trim();
        if Uuid::parse_str(window_id).is_err() {
            return Err(SeatgeistError::InvalidRequest(
                "KWin exact-window capture requires a valid UUID window id".to_string(),
            ));
        }
        let window_id = window_id.to_string();
        Ok(Box::new(OneShotCaptureSession::new(
            CaptureSessionMetadata {
                id: format!("kwin-window-{}", Uuid::new_v4().simple()),
                backend: "kwin_screenshot2_window".to_string(),
                source_type: CaptureSourceType::Window,
                source_id: Some(window_id.clone()),
                restore_token_reference: None,
                consent_required: false,
                occlusion_possible: false,
            },
            Arc::new(KwinWindowFrameCapture { window_id }),
        )))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RoutedScreenBackend {
    exact_window: Arc<dyn ScreenBackend>,
    portal: Arc<dyn ScreenBackend>,
}

impl RoutedScreenBackend {
    pub(crate) fn new(portal: Arc<dyn ScreenBackend>) -> Self {
        Self {
            exact_window: Arc::new(KwinWindowScreenBackend),
            portal,
        }
    }

    #[cfg(test)]
    fn with_backends(exact_window: Arc<dyn ScreenBackend>, portal: Arc<dyn ScreenBackend>) -> Self {
        Self {
            exact_window,
            portal,
        }
    }

    fn uses_exact_window(request: &CaptureSessionRequest) -> bool {
        matches!(
            request.source,
            CaptureSource::Window {
                requested_window_id: Some(_)
            }
        )
    }
}

#[async_trait]
impl ScreenBackend for RoutedScreenBackend {
    async fn capabilities(&self) -> BackendResult<CaptureCapabilities> {
        let mut portal = self.portal.capabilities().await?;
        portal.backend = "kwin_screenshot2_window+portal_screencast_pipewire".to_string();
        portal.wait_for_frame = true;
        Ok(portal)
    }

    async fn list_monitors(&self) -> BackendResult<Vec<libseatgeist::MonitorInfo>> {
        self.portal.list_monitors().await
    }

    async fn open_capture(
        &self,
        mut request: CaptureSessionRequest,
    ) -> BackendResult<Box<dyn CaptureSession>> {
        if Self::uses_exact_window(&request) {
            request.persist = false;
            request.restore_token_reference = None;
            self.exact_window.open_capture(request).await
        } else {
            self.portal.open_capture(request).await
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RawCaptureMetadata {
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
    scale: f64,
    window_id: Option<String>,
}

async fn capture_kwin_window(
    window_id: &str,
    request: &FrameRequest,
) -> anyhow::Result<ScreenshotInfo> {
    if request.timeout_ms == 0 {
        anyhow::bail!("capture timeout must be greater than zero");
    }
    let output = PathBuf::from(&request.output);
    prepare_screenshot_output(&output)?;
    let connection = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        KWIN_SCREENSHOT_SERVICE,
        KWIN_SCREENSHOT_PATH,
        KWIN_SCREENSHOT_INTERFACE,
    )
    .await?;

    let (reader, writer) = UnixStream::pair()?;
    reader.set_read_timeout(Some(Duration::from_millis(request.timeout_ms)))?;
    let read_task = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        reader
            .take(MAX_RAW_CAPTURE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RAW_CAPTURE_BYTES {
            anyhow::bail!("KWin raw screenshot exceeded the 512 MiB safety bound");
        }
        Ok(bytes)
    });

    let mut options = HashMap::<&str, Value<'_>>::new();
    options.insert("include-cursor", Value::from(false));
    options.insert("include-decoration", Value::from(false));
    options.insert("include-shadow", Value::from(false));
    options.insert("native-resolution", Value::from(true));
    let pipe = Fd::from(&writer);
    let results: HashMap<String, OwnedValue> = tokio::time::timeout(
        Duration::from_millis(request.timeout_ms),
        proxy.call("CaptureWindow", &(window_id, options, pipe)),
    )
    .await
    .map_err(|_| anyhow::anyhow!("KWin CaptureWindow timed out"))??;
    drop(writer);
    let bytes = read_task.await??;
    let metadata = decode_metadata(&results)?;
    verify_window_id(window_id, metadata.window_id.as_deref())?;
    let rgba = decode_argb32_premultiplied(&bytes, &metadata)?;
    write_window_png(&rgba, &output, request.max_edge)?;
    let (output_width, output_height) = image::image_dimensions(&output)?;
    let logical_width = logical_capture_extent(metadata.width, metadata.scale);
    let logical_height = logical_capture_extent(metadata.height, metadata.scale);
    Ok(ScreenshotInfo {
        path: output,
        backend: "kwin_screenshot2_window".to_string(),
        occlusion_possible: false,
        source_width: metadata.width,
        source_height: metadata.height,
        output_width,
        output_height,
        transform: ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::WindowLocal,
            output_coordinate_space: CoordinateSpace::CaptureOutput,
            source_extent_width: Some(logical_width),
            source_extent_height: Some(logical_height),
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: f64::from(output_width) / f64::from(logical_width),
            scale_y: f64::from(output_height) / f64::from(logical_height),
        },
        coordinate_space: CoordinateSpace::WindowLocal,
        monitors: Vec::new(),
    })
}

fn logical_capture_extent(native_extent: u32, scale: f64) -> u32 {
    (f64::from(native_extent) / scale)
        .round()
        .clamp(1.0, f64::from(u32::MAX)) as u32
}

fn decode_metadata(results: &HashMap<String, OwnedValue>) -> anyhow::Result<RawCaptureMetadata> {
    let image_type = string_result(results, "type")?;
    if image_type != "raw" {
        anyhow::bail!("KWin returned unsupported screenshot type {image_type:?}");
    }
    let metadata = RawCaptureMetadata {
        width: u32_result(results, "width")?,
        height: u32_result(results, "height")?,
        stride: u32_result(results, "stride")?,
        format: u32_result(results, "format")?,
        scale: f64_result(results, "scale")?.unwrap_or(1.0),
        window_id: optional_string_result(results, "windowId")?,
    };
    if metadata.width == 0 || metadata.height == 0 || metadata.stride == 0 {
        anyhow::bail!("KWin returned empty exact-window screenshot metadata");
    }
    if !metadata.scale.is_finite() || metadata.scale <= 0.0 {
        anyhow::bail!("KWin returned invalid screenshot scale");
    }
    Ok(metadata)
}

fn string_result(results: &HashMap<String, OwnedValue>, key: &str) -> anyhow::Result<String> {
    optional_string_result(results, key)?
        .ok_or_else(|| anyhow::anyhow!("KWin screenshot result omitted {key}"))
}

fn optional_string_result(
    results: &HashMap<String, OwnedValue>,
    key: &str,
) -> anyhow::Result<Option<String>> {
    results
        .get(key)
        .map(|value| String::try_from(&**value))
        .transpose()
        .map_err(Into::into)
}

fn u32_result(results: &HashMap<String, OwnedValue>, key: &str) -> anyhow::Result<u32> {
    results
        .get(key)
        .ok_or_else(|| anyhow::anyhow!("KWin screenshot result omitted {key}"))
        .and_then(|value| u32::try_from(&**value).map_err(Into::into))
}

fn f64_result(results: &HashMap<String, OwnedValue>, key: &str) -> anyhow::Result<Option<f64>> {
    results
        .get(key)
        .map(|value| f64::try_from(&**value))
        .transpose()
        .map_err(Into::into)
}

fn verify_window_id(requested: &str, returned: Option<&str>) -> anyhow::Result<()> {
    let Some(returned) = returned else {
        return Ok(());
    };
    let requested = Uuid::parse_str(requested)?;
    let returned = Uuid::parse_str(returned)?;
    if requested != returned {
        anyhow::bail!("KWin captured a different window than requested");
    }
    Ok(())
}

fn decode_argb32_premultiplied(
    bytes: &[u8],
    metadata: &RawCaptureMetadata,
) -> anyhow::Result<RgbaImage> {
    if metadata.format != KWIN_ARGB32_PREMULTIPLIED {
        anyhow::bail!(
            "KWin returned unsupported QImage format {} (expected ARGB32 premultiplied)",
            metadata.format
        );
    }
    let row_bytes = metadata
        .width
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("KWin screenshot row size overflow"))?;
    if metadata.stride < row_bytes {
        anyhow::bail!("KWin screenshot stride is smaller than one pixel row");
    }
    let expected = u64::from(metadata.stride)
        .checked_mul(u64::from(metadata.height))
        .ok_or_else(|| anyhow::anyhow!("KWin screenshot byte size overflow"))?;
    if expected > MAX_RAW_CAPTURE_BYTES || bytes.len() as u64 != expected {
        anyhow::bail!(
            "KWin screenshot byte count mismatch: expected {expected}, got {}",
            bytes.len()
        );
    }
    let pixel_count = u64::from(metadata.width)
        .checked_mul(u64::from(metadata.height))
        .ok_or_else(|| anyhow::anyhow!("KWin screenshot pixel count overflow"))?;
    let output_bytes = pixel_count
        .checked_mul(4)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("KWin screenshot output size overflow"))?;
    let mut rgba = Vec::with_capacity(output_bytes);
    for y in 0..metadata.height {
        let start = usize::try_from(u64::from(y) * u64::from(metadata.stride))?;
        let end = start + usize::try_from(row_bytes)?;
        for pixel in bytes[start..end].chunks_exact(4) {
            let alpha = pixel[3];
            rgba.extend_from_slice(&[
                unpremultiply(pixel[2], alpha),
                unpremultiply(pixel[1], alpha),
                unpremultiply(pixel[0], alpha),
                alpha,
            ]);
        }
    }
    RgbaImage::from_raw(metadata.width, metadata.height, rgba)
        .ok_or_else(|| anyhow::anyhow!("could not construct decoded KWin screenshot"))
}

fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    match alpha {
        0 => 0,
        255 => channel,
        alpha => {
            ((u16::from(channel) * 255 + u16::from(alpha) / 2) / u16::from(alpha)).min(255) as u8
        }
    }
}

fn write_window_png(image: &RgbaImage, output: &Path, max_edge: Option<u32>) -> anyhow::Result<()> {
    let max_edge = max_edge.unwrap_or_else(|| image.width().max(image.height()));
    if max_edge == 0 {
        anyhow::bail!("max_edge must be greater than zero");
    }
    let dynamic = DynamicImage::ImageRgba8(image.clone());
    let largest = image.width().max(image.height());
    let output_image = if largest > max_edge {
        let scale = f64::from(max_edge) / f64::from(largest);
        let width = (f64::from(image.width()) * scale).round().max(1.0) as u32;
        let height = (f64::from(image.height()) * scale).round().max(1.0) as u32;
        dynamic.resize_exact(width, height, FilterType::Lanczos3)
    } else {
        dynamic
    };
    output_image.save(output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use seatgeist_backend::{
        CaptureSessionLifecycle, CapturedFrame, FrameWaitRequest, FrameWaitResult,
    };
    use std::sync::Mutex;

    #[derive(Debug)]
    struct RecordingBackend {
        name: &'static str,
        requests: Mutex<Vec<CaptureSessionRequest>>,
    }

    #[derive(Debug)]
    struct EmptySession;

    #[async_trait]
    impl CaptureSession for EmptySession {
        fn metadata(&self) -> CaptureSessionMetadata {
            CaptureSessionMetadata {
                id: "empty".to_string(),
                backend: "empty".to_string(),
                source_type: CaptureSourceType::Window,
                source_id: None,
                restore_token_reference: None,
                consent_required: false,
                occlusion_possible: false,
            }
        }

        async fn lifecycle(&self) -> CaptureSessionLifecycle {
            CaptureSessionLifecycle::Open
        }

        async fn snapshot(&self, _request: FrameRequest) -> BackendResult<CapturedFrame> {
            Err(SeatgeistError::BackendUnavailable("unused".to_string()))
        }

        async fn wait_for_frame(
            &self,
            _request: FrameWaitRequest,
        ) -> BackendResult<FrameWaitResult> {
            Err(SeatgeistError::BackendUnavailable("unused".to_string()))
        }

        async fn close(&self) -> BackendResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ScreenBackend for RecordingBackend {
        async fn capabilities(&self) -> BackendResult<CaptureCapabilities> {
            Ok(CaptureCapabilities {
                backend: self.name.to_string(),
                source_types: vec![CaptureSourceType::Window],
                retained_sessions: true,
                wait_for_frame: true,
                restore_tokens: true,
                damage_tracking: true,
            })
        }

        async fn list_monitors(&self) -> BackendResult<Vec<libseatgeist::MonitorInfo>> {
            Ok(Vec::new())
        }

        async fn open_capture(
            &self,
            request: CaptureSessionRequest,
        ) -> BackendResult<Box<dyn CaptureSession>> {
            self.requests.lock().expect("requests lock").push(request);
            Ok(Box::new(EmptySession))
        }
    }

    fn recording(name: &'static str) -> Arc<RecordingBackend> {
        Arc::new(RecordingBackend {
            name,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn request(source: CaptureSource) -> CaptureSessionRequest {
        CaptureSessionRequest {
            source,
            restore_token_reference: Some("portal-token".to_string()),
            persist: true,
            consent_parent_window: String::new(),
            open_timeout_ms: 1_500,
            default_max_edge: 1_600,
        }
    }

    #[tokio::test]
    async fn router_uses_kwin_for_exact_windows_without_portal_persistence() {
        let exact = recording("exact");
        let portal = recording("portal");
        let router = RoutedScreenBackend::with_backends(exact.clone(), portal.clone());
        router
            .open_capture(request(CaptureSource::Window {
                requested_window_id: Some("{20b626b8-3272-4494-8c2c-5eb27b73f361}".to_string()),
            }))
            .await
            .expect("exact route opens");
        let requests = exact.requests.lock().expect("exact requests");
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].persist);
        assert_eq!(requests[0].restore_token_reference, None);
        assert!(portal.requests.lock().expect("portal requests").is_empty());
    }

    #[tokio::test]
    async fn router_keeps_non_exact_windows_on_the_portal() {
        let exact = recording("exact");
        let portal = recording("portal");
        let router = RoutedScreenBackend::with_backends(exact.clone(), portal.clone());
        router
            .open_capture(request(CaptureSource::Window {
                requested_window_id: None,
            }))
            .await
            .expect("portal route opens");
        assert!(exact.requests.lock().expect("exact requests").is_empty());
        assert_eq!(portal.requests.lock().expect("portal requests").len(), 1);
    }

    #[tokio::test]
    async fn exact_backend_rejects_non_uuid_window_ids_before_dbus() {
        let result = KwinWindowScreenBackend
            .open_capture(CaptureSessionRequest {
                source: CaptureSource::Window {
                    requested_window_id: Some("not-a-kwin-uuid".to_string()),
                },
                restore_token_reference: None,
                persist: false,
                consent_parent_window: String::new(),
                open_timeout_ms: 1_500,
                default_max_edge: 1_600,
            })
            .await;
        let error = result
            .err()
            .expect("invalid ids must fail before KWin is contacted");
        assert!(error.to_string().contains("valid UUID"));
    }

    #[test]
    fn decodes_little_endian_argb32_premultiplied_with_stride() {
        let metadata = RawCaptureMetadata {
            width: 2,
            height: 1,
            stride: 12,
            format: KWIN_ARGB32_PREMULTIPLIED,
            scale: 1.0,
            window_id: None,
        };
        let decoded = decode_argb32_premultiplied(
            &[25, 50, 100, 128, 30, 20, 10, 255, 0, 0, 0, 0],
            &metadata,
        )
        .expect("raw image decodes");
        assert_eq!(decoded.get_pixel(0, 0).0, [199, 100, 50, 128]);
        assert_eq!(decoded.get_pixel(1, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn rejects_wrong_format_and_byte_count() {
        let mut metadata = RawCaptureMetadata {
            width: 1,
            height: 1,
            stride: 4,
            format: 17,
            scale: 1.0,
            window_id: None,
        };
        assert!(decode_argb32_premultiplied(&[0; 4], &metadata).is_err());
        metadata.format = KWIN_ARGB32_PREMULTIPLIED;
        assert!(decode_argb32_premultiplied(&[0; 3], &metadata).is_err());
    }

    #[test]
    fn native_capture_extent_maps_fractional_dpi_to_logical_surface() {
        assert_eq!(logical_capture_extent(1_920, 1.5), 1_280);
        assert_eq!(logical_capture_extent(1_620, 1.5), 1_080);
        assert_eq!(logical_capture_extent(1_279, 1.25), 1_023);
    }
}
