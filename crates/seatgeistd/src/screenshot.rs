use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use libseatgeist::{
    CoordinateSpace, PortalScreenshotTarget, ScreenshotInfo, ScreenshotPortalStatus,
    ScreenshotRequest, ScreenshotTileRequest, ScreenshotTransform, WaitForChangeRequest,
    WaitForChangeResult,
};
use tracing::warn;
use uuid::Uuid;

use super::{
    capture_diagnostics::{screenshot_portal_status, tile_capture_backend},
    commands::exists as command_exists,
    compatibility_capture_backend,
    config::SafetySettings,
    kwin_bridge::WindowListState,
    screenshot_image::{
        apply_screenshot_redactions, prepare_screenshot_output, read_png_dimensions_with_retry,
        temporary_capture_path, validate_tile_bounds, validate_tile_request, write_preview_or_copy,
        write_tile_preview,
    },
    window_backend::list_monitors,
};

static SCREENSHOT_CAPTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const PORTAL_SCREENSHOT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) async fn capture_screenshot(
    request: ScreenshotRequest,
    safety_settings: &SafetySettings,
    window_list_state: &WindowListState,
) -> Result<ScreenshotInfo> {
    if !request.full_resolution && request.max_edge == Some(0) {
        bail!("max_edge must be greater than zero");
    }
    validate_screenshot_source_selection(&request)?;
    prepare_screenshot_output(&request.output)?;

    if request.visible_window_crop_id.is_some() {
        return compatibility_capture_backend::capture_visible_window_crop(
            request,
            safety_settings,
            window_list_state,
        )
        .await;
    }

    let screenshot_portal = screenshot_portal_status();
    validate_portal_screenshot_target_request(&request, &screenshot_portal)?;

    if request.portal_target.is_some() {
        return compatibility_capture_backend::capture_portal_target(request, safety_settings)
            .await;
    }

    if screenshot_portal.screenshot_interface_available {
        match capture_screenshot_portal(request.clone(), safety_settings).await {
            Ok(Some(info)) => return Ok(info),
            Ok(None) => {
                bail!(
                    "portal screenshot request was cancelled or ended without a screenshot; not falling back to Spectacle"
                );
            }
            Err(err) => {
                if !command_exists("spectacle") {
                    return Err(err)
                        .context("portal screenshot backend failed and Spectacle is unavailable");
                }
                warn!(
                    error = %err,
                    "portal screenshot backend failed; falling back to Spectacle"
                );
            }
        }
    }

    capture_screenshot_spectacle(request, safety_settings)
}

fn validate_screenshot_source_selection(request: &ScreenshotRequest) -> Result<()> {
    if request.portal_target.is_some() && request.visible_window_crop_id.is_some() {
        bail!("portal_target and visible_window_crop_id are mutually exclusive");
    }
    if let Some(window_id) = request.visible_window_crop_id.as_deref() {
        if window_id.trim().is_empty() {
            bail!("visible_window_crop_id must not be blank");
        }
        if request.portal_interactive {
            bail!(
                "visible_window_crop_id cannot be combined with portal_interactive; the exact KWin crop target must remain stable"
            );
        }
    }
    Ok(())
}

pub(crate) async fn capture_screenshot_portal(
    request: ScreenshotRequest,
    safety_settings: &SafetySettings,
) -> Result<Option<ScreenshotInfo>> {
    let handle_token = format!("seatgeist_{}", Uuid::new_v4().simple());
    let mut options = seatgeist_portal::PortalScreenshotOptions::new(handle_token);
    options.interactive = request.portal_interactive;
    options.target = request.portal_target.map(portal_screenshot_target_to_xdg);
    let Some(capture) =
        seatgeist_portal::request_screenshot_zbus(&options, PORTAL_SCREENSHOT_RESPONSE_TIMEOUT)
            .await
            .map_err(|err| anyhow::anyhow!(err))?
    else {
        return Ok(None);
    };
    let (source_width, source_height) = read_png_dimensions_with_retry(&capture.path)
        .with_context(|| {
            format!(
                "read portal screenshot dimensions from {}",
                capture.path.display()
            )
        })?;
    let (output_width, output_height) = if request.full_resolution {
        fs::copy(&capture.path, &request.output).with_context(|| {
            format!(
                "copy portal screenshot from {} to {}",
                capture.path.display(),
                request.output.display()
            )
        })?;
        (source_width, source_height)
    } else {
        write_preview_or_copy(
            &capture.path,
            &request.output,
            source_width,
            source_height,
            request.max_edge.unwrap_or(safety_settings.preview_max_edge),
        )?
    };
    Ok(Some(screenshot_info_from_capture(
        request.output,
        "portal_screenshot",
        source_width,
        source_height,
        output_width,
        output_height,
        0,
        0,
        source_width,
        source_height,
        safety_settings,
    )?))
}

pub(crate) fn validate_portal_screenshot_target_request(
    request: &ScreenshotRequest,
    screenshot_portal: &ScreenshotPortalStatus,
) -> Result<()> {
    let Some(target) = request.portal_target else {
        return Ok(());
    };
    if !screenshot_portal.screenshot_interface_available {
        bail!(
            "portal screenshot target {target} requires xdg-desktop-portal Screenshot; no portal screenshot backend is visible"
        );
    }
    if !screenshot_portal.screenshot_target_option_supported {
        let version = screenshot_portal
            .screenshot_interface_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        bail!(
            "portal screenshot target {target} requires xdg-desktop-portal Screenshot v3/AvailableTargets; current Screenshot interface version is {version}"
        );
    }
    if let Some(mask) = screenshot_portal.screenshot_available_targets_mask {
        let target_mask = portal_screenshot_target_to_xdg(target).value();
        if mask & target_mask == 0 {
            bail!(
                "portal screenshot target {target} is not advertised by AvailableTargets mask {mask}"
            );
        }
    }
    Ok(())
}

fn portal_screenshot_target_to_xdg(
    target: PortalScreenshotTarget,
) -> seatgeist_portal::PortalScreenshotTarget {
    match target {
        PortalScreenshotTarget::Screen => seatgeist_portal::PortalScreenshotTarget::Screen,
        PortalScreenshotTarget::Window => seatgeist_portal::PortalScreenshotTarget::Window,
        PortalScreenshotTarget::Area => seatgeist_portal::PortalScreenshotTarget::Area,
        PortalScreenshotTarget::ActiveWindow => {
            seatgeist_portal::PortalScreenshotTarget::ActiveWindow
        }
    }
}

fn capture_screenshot_spectacle(
    request: ScreenshotRequest,
    safety_settings: &SafetySettings,
) -> Result<ScreenshotInfo> {
    let _guard = SCREENSHOT_CAPTURE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("screenshot capture lock is poisoned"))?;
    if !command_exists("spectacle") {
        bail!("spectacle command is not available for KDE screenshot capture");
    }

    let capture_output = if request.full_resolution {
        request.output.clone()
    } else {
        temporary_capture_path(&request.output)
    };
    prepare_screenshot_output(&capture_output)?;

    let status = Command::new("spectacle")
        .args(["-b", "-f", "-n", "-o"])
        .arg(&capture_output)
        .status()
        .context("run spectacle screenshot backend")?;
    if !status.success() {
        bail!("spectacle screenshot backend exited with status {status}");
    }

    let (source_width, source_height) = read_png_dimensions_with_retry(&capture_output)
        .with_context(|| {
            format!(
                "read screenshot dimensions from {}",
                capture_output.display()
            )
        })?;

    let (output_width, output_height) = if request.full_resolution {
        (source_width, source_height)
    } else {
        write_preview_or_copy(
            &capture_output,
            &request.output,
            source_width,
            source_height,
            request.max_edge.unwrap_or(safety_settings.preview_max_edge),
        )?
    };

    let info = screenshot_info_from_capture(
        request.output,
        "spectacle",
        source_width,
        source_height,
        output_width,
        output_height,
        0,
        0,
        source_width,
        source_height,
        safety_settings,
    )?;

    if capture_output != info.path {
        fs::remove_file(&capture_output).ok();
    }

    Ok(info)
}

#[allow(clippy::too_many_arguments)]
fn screenshot_info_from_capture(
    path: PathBuf,
    backend: &str,
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
    source_origin_x: u32,
    source_origin_y: u32,
    transform_source_width: u32,
    transform_source_height: u32,
    safety_settings: &SafetySettings,
) -> Result<ScreenshotInfo> {
    let monitors = list_monitors().unwrap_or_default();
    let info = ScreenshotInfo {
        path,
        backend: backend.to_string(),
        occlusion_possible: false,
        source_width,
        source_height,
        output_width,
        output_height,
        transform: ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x,
            source_origin_y,
            scale_x: f64::from(output_width) / f64::from(transform_source_width),
            scale_y: f64::from(output_height) / f64::from(transform_source_height),
        },
        coordinate_space: CoordinateSpace::PhysicalPixel,
        monitors,
    };
    apply_screenshot_redactions(&info, &safety_settings.screenshot_redactions)?;
    Ok(info)
}

#[derive(Debug)]
struct TileCaptureSource {
    path: PathBuf,
    backend: &'static str,
    cleanup_after_use: bool,
}

pub(crate) async fn capture_screenshot_tile(
    request: ScreenshotTileRequest,
    safety_settings: &SafetySettings,
) -> Result<ScreenshotInfo> {
    validate_tile_request(&request)?;
    prepare_screenshot_output(&request.output)?;
    let capture = capture_tile_source(&request.output, request.portal_interactive).await?;

    let (source_width, source_height) = read_png_dimensions_with_retry(&capture.path)
        .with_context(|| format!("read screenshot dimensions from {}", capture.path.display()))?;
    validate_tile_bounds(&request, source_width, source_height)?;
    let (output_width, output_height) = write_tile_preview(
        &capture.path,
        &request,
        request.max_edge.unwrap_or(safety_settings.tile_max_edge),
    )?;

    let monitors = list_monitors().unwrap_or_default();

    let info = ScreenshotInfo {
        path: request.output,
        backend: capture.backend.to_string(),
        occlusion_possible: false,
        source_width,
        source_height,
        output_width,
        output_height,
        transform: ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x: request.x,
            source_origin_y: request.y,
            scale_x: f64::from(output_width) / f64::from(request.width),
            scale_y: f64::from(output_height) / f64::from(request.height),
        },
        coordinate_space: CoordinateSpace::PhysicalPixel,
        monitors,
    };
    apply_screenshot_redactions(&info, &safety_settings.screenshot_redactions)?;

    if capture.cleanup_after_use {
        fs::remove_file(&capture.path).ok();
    }
    Ok(info)
}

async fn capture_tile_source(output: &Path, portal_interactive: bool) -> Result<TileCaptureSource> {
    let screenshot_portal = screenshot_portal_status();
    let spectacle_available = command_exists("spectacle");
    if tile_capture_backend(&screenshot_portal, spectacle_available) == Some("portal_screenshot") {
        match capture_tile_source_portal(portal_interactive).await {
            Ok(Some(capture)) => return Ok(capture),
            Ok(None) => {
                bail!(
                    "portal screenshot request was cancelled or ended without a screenshot; not falling back to Spectacle"
                );
            }
            Err(err) => {
                if !spectacle_available {
                    return Err(err)
                        .context("portal screenshot backend failed and Spectacle is unavailable");
                }
                warn!(
                    error = %err,
                    "portal screenshot backend failed for tile capture; falling back to Spectacle"
                );
            }
        }
    }

    capture_tile_source_spectacle(output)
}

async fn capture_tile_source_portal(portal_interactive: bool) -> Result<Option<TileCaptureSource>> {
    let handle_token = format!("seatgeist_{}", Uuid::new_v4().simple());
    let mut options = seatgeist_portal::PortalScreenshotOptions::new(handle_token);
    options.interactive = portal_interactive;
    let Some(capture) =
        seatgeist_portal::request_screenshot_zbus(&options, PORTAL_SCREENSHOT_RESPONSE_TIMEOUT)
            .await
            .map_err(|err| anyhow::anyhow!(err))?
    else {
        return Ok(None);
    };

    Ok(Some(TileCaptureSource {
        path: capture.path,
        backend: "portal_screenshot",
        cleanup_after_use: false,
    }))
}

fn capture_tile_source_spectacle(output: &Path) -> Result<TileCaptureSource> {
    let _guard = SCREENSHOT_CAPTURE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("screenshot capture lock is poisoned"))?;
    if !command_exists("spectacle") {
        bail!("spectacle command is not available for KDE screenshot capture");
    }

    let capture_output = temporary_capture_path(output);
    prepare_screenshot_output(&capture_output)?;
    let status = Command::new("spectacle")
        .args(["-b", "-f", "-n", "-o"])
        .arg(&capture_output)
        .status()
        .context("run spectacle screenshot backend")?;
    if !status.success() {
        bail!("spectacle screenshot backend exited with status {status}");
    }

    Ok(TileCaptureSource {
        path: capture_output,
        backend: "spectacle",
        cleanup_after_use: true,
    })
}

pub(crate) async fn wait_for_change(
    request: WaitForChangeRequest,
    safety_settings: &SafetySettings,
    window_list_state: &WindowListState,
) -> Result<WaitForChangeResult> {
    validate_wait_for_change_request(&request)?;
    let timeout = Duration::from_millis(request.timeout_ms);
    let interval = Duration::from_millis(request.interval_ms);
    let started = Instant::now();
    let screenshot_request = || ScreenshotRequest {
        output: request.output.clone(),
        max_edge: request.max_edge.or(Some(safety_settings.preview_max_edge)),
        full_resolution: false,
        portal_interactive: false,
        portal_target: None,
        visible_window_crop_id: None,
    };

    let baseline_info =
        capture_screenshot(screenshot_request(), safety_settings, window_list_state).await?;
    let baseline = read_image_sample(&baseline_info.path)?;
    let mut final_info = baseline_info;
    let mut captures = 1;
    let mut score = 0.0;
    let mut changed = false;

    while started.elapsed() < timeout {
        let remaining = timeout.saturating_sub(started.elapsed());
        tokio::time::sleep(interval.min(remaining)).await;
        final_info =
            capture_screenshot(screenshot_request(), safety_settings, window_list_state).await?;
        captures += 1;

        let candidate = read_image_sample(&final_info.path)?;
        score = normalized_image_difference(&baseline, &candidate)?;
        if score >= request.threshold {
            changed = true;
            break;
        }
    }

    Ok(WaitForChangeResult {
        changed,
        timed_out: !changed,
        timeout_ms: request.timeout_ms,
        interval_ms: request.interval_ms,
        captures,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        score,
        threshold: request.threshold,
        screenshot: final_info,
    })
}

fn validate_wait_for_change_request(request: &WaitForChangeRequest) -> Result<()> {
    if request.timeout_ms == 0 {
        bail!("timeout_ms must be greater than zero");
    }
    if request.interval_ms == 0 {
        bail!("interval_ms must be greater than zero");
    }
    if request.max_edge == Some(0) {
        bail!("max_edge must be greater than zero");
    }
    if !request.threshold.is_finite() || request.threshold <= 0.0 || request.threshold > 1.0 {
        bail!("threshold must be greater than 0.0 and less than or equal to 1.0");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageSample {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn read_image_sample(path: &Path) -> Result<ImageSample> {
    let image = image::open(path)
        .with_context(|| format!("read wait_for_change image {}", path.display()))?
        .to_rgba8();
    Ok(ImageSample {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

fn normalized_image_difference(baseline: &ImageSample, candidate: &ImageSample) -> Result<f64> {
    if baseline.width != candidate.width || baseline.height != candidate.height {
        bail!(
            "wait_for_change image size changed from {}x{} to {}x{}",
            baseline.width,
            baseline.height,
            candidate.width,
            candidate.height
        );
    }
    if baseline.rgba.len() != candidate.rgba.len() {
        bail!("wait_for_change image buffers have different lengths");
    }

    let mut sum = 0u64;
    let mut channels = 0u64;
    for (baseline, candidate) in baseline
        .rgba
        .chunks_exact(4)
        .zip(candidate.rgba.chunks_exact(4))
    {
        for index in 0..3 {
            sum += u64::from(baseline[index].abs_diff(candidate[index]));
            channels += 1;
        }
    }
    if channels == 0 {
        return Ok(0.0);
    }
    Ok(sum as f64 / (channels as f64 * 255.0))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use libseatgeist::DEFAULT_WAIT_FOR_CHANGE_THRESHOLD;

    use super::*;

    fn screenshot_request(label: &str) -> ScreenshotRequest {
        ScreenshotRequest {
            output: PathBuf::from(format!("/tmp/{label}.png")),
            max_edge: Some(1600),
            full_resolution: false,
            portal_interactive: false,
            portal_target: None,
            visible_window_crop_id: None,
        }
    }

    fn screenshot_portal_status_fixture() -> ScreenshotPortalStatus {
        ScreenshotPortalStatus {
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
            setup_hint: String::new(),
        }
    }

    #[test]
    fn validates_wait_for_change_request() {
        validate_wait_for_change_request(&WaitForChangeRequest {
            output: PathBuf::from("/tmp/wait-valid.png"),
            max_edge: Some(1600),
            timeout_ms: 1000,
            interval_ms: 100,
            threshold: DEFAULT_WAIT_FOR_CHANGE_THRESHOLD,
        })
        .expect("valid wait request passes");

        let err = validate_wait_for_change_request(&WaitForChangeRequest {
            output: PathBuf::from("/tmp/wait-invalid.png"),
            max_edge: Some(1600),
            timeout_ms: 1000,
            interval_ms: 100,
            threshold: 0.0,
        })
        .expect_err("zero threshold is rejected");
        assert!(err.to_string().contains("threshold"));
    }

    #[test]
    fn image_difference_reports_normalized_rgb_delta() {
        let baseline = ImageSample {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        };
        let candidate = ImageSample {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        };

        let score =
            normalized_image_difference(&baseline, &candidate).expect("same dimensions compare");
        assert!((score - (1.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn portal_target_requires_advertised_v3_support() {
        let mut request = screenshot_request("portal-target");
        request.portal_target = Some(PortalScreenshotTarget::ActiveWindow);

        let mut portal = screenshot_portal_status_fixture();
        portal.screenshot_interface_version = Some(2);
        portal.screenshot_available_targets_mask = None;
        portal.screenshot_available_targets = Vec::new();
        portal.screenshot_target_option_supported = false;
        let err = validate_portal_screenshot_target_request(&request, &portal)
            .expect_err("v2 portal must reject target-specific capture");
        assert!(err.to_string().contains("Screenshot v3"));

        portal.screenshot_interface_version = Some(3);
        portal.screenshot_available_targets_mask =
            Some(seatgeist_portal::PortalScreenshotTarget::Screen.value());
        portal.screenshot_available_targets = vec!["screen".to_string()];
        portal.screenshot_target_option_supported = true;
        let err = validate_portal_screenshot_target_request(&request, &portal)
            .expect_err("missing AvailableTargets bit must reject target-specific capture");
        assert!(err.to_string().contains("not advertised"));

        portal.screenshot_available_targets_mask = Some(
            seatgeist_portal::PortalScreenshotTarget::Screen.value()
                | seatgeist_portal::PortalScreenshotTarget::ActiveWindow.value(),
        );
        validate_portal_screenshot_target_request(&request, &portal)
            .expect("advertised v3 target is accepted");
    }

    #[test]
    fn source_modes_are_explicit_and_mutually_exclusive() {
        let mut request = screenshot_request("visible-crop-validation");
        request.visible_window_crop_id = Some("kwin-window-7".to_string());
        validate_screenshot_source_selection(&request).expect("explicit crop is valid");

        request.portal_target = Some(PortalScreenshotTarget::Window);
        let error = validate_screenshot_source_selection(&request)
            .expect_err("two source modes are rejected");
        assert!(error.to_string().contains("mutually exclusive"));

        request.portal_target = None;
        request.portal_interactive = true;
        let error = validate_screenshot_source_selection(&request)
            .expect_err("interactive portal cannot change exact crop target");
        assert!(error.to_string().contains("cannot be combined"));
    }
}
