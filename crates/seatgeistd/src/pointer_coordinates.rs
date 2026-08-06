use anyhow::{Context, Result, bail};
use libseatgeist::{
    CoordinateSpace, MonitorInfo, Point, PointerCalibrationPoint, PointerCalibrationStatus,
    PointerMonitorCalibration, PointerPhysicalBounds, WindowGeometry, WindowInfo,
};
use seatgeist_backend::{ScreenBackend, WindowBackend};

#[derive(Debug, Clone)]
pub(crate) struct ResolutionContext {
    space: CoordinateSpace,
    monitors: Vec<MonitorInfo>,
    active_window: Option<WindowInfo>,
    bounds: seatgeist_uinput::PointerBounds,
}

impl ResolutionContext {
    pub(crate) async fn load(
        space: CoordinateSpace,
        window_backend: &dyn WindowBackend,
        screen_backend: &dyn ScreenBackend,
    ) -> Result<Self> {
        if matches!(
            space,
            CoordinateSpace::AccessibilityNode | CoordinateSpace::CaptureOutput
        ) {
            bail!(
                "pointer coordinate resolution requires capture_output coordinates to be resolved from their capture session first; got {:?}",
                space
            );
        }
        let monitors = screen_backend
            .list_monitors()
            .await
            .map_err(anyhow::Error::msg)
            .context("pointer coordinate resolution could not list monitors")?;
        let bounds = physical_pointer_bounds_from_monitors(&monitors)?;
        let active_window = if space == CoordinateSpace::WindowLocal {
            window_backend
                .active_window()
                .await
                .map_err(anyhow::Error::msg)
                .context("window_local pointer coordinates could not read active window")?
        } else {
            None
        };
        Ok(Self {
            space,
            monitors,
            active_window,
            bounds,
        })
    }

    pub(crate) fn resolve(&self, point: Point) -> Result<Point> {
        if point.space != self.space {
            bail!(
                "pointer coordinate context is {:?}, got {:?}",
                self.space,
                point.space
            );
        }
        let point = match point.space {
            CoordinateSpace::PhysicalPixel => point,
            CoordinateSpace::LogicalPixel => logical_to_physical_point(point, &self.monitors)?,
            CoordinateSpace::WindowLocal => active_window_local_to_physical_point(
                point,
                self.active_window.as_ref(),
                &self.monitors,
            )?,
            CoordinateSpace::AccessibilityNode | CoordinateSpace::CaptureOutput => {
                unreachable!("rejected while loading context")
            }
        };
        validate_physical_pointer_point(point, self.bounds)?;
        Ok(point)
    }

    pub(crate) const fn bounds(&self) -> seatgeist_uinput::PointerBounds {
        self.bounds
    }
}

pub(crate) async fn calibration(
    screen_backend: &dyn ScreenBackend,
) -> Result<PointerCalibrationStatus> {
    let monitors = screen_backend
        .list_monitors()
        .await
        .map_err(anyhow::Error::msg)
        .context("pointer calibration could not list monitors")?;
    calibration_from_monitors(&monitors)
}

pub(crate) async fn physical_bounds(
    screen_backend: &dyn ScreenBackend,
) -> Result<seatgeist_uinput::PointerBounds> {
    let monitors = screen_backend
        .list_monitors()
        .await
        .map_err(anyhow::Error::msg)
        .context("pointer bounds could not list monitors")?;
    physical_pointer_bounds_from_monitors(&monitors)
}

pub(crate) fn calibration_from_monitors(
    monitors: &[MonitorInfo],
) -> Result<PointerCalibrationStatus> {
    let bounds = physical_pointer_bounds_from_monitors(monitors)?;
    let monitors = pointer_monitor_calibrations(monitors)?;
    let physical_bounds = PointerPhysicalBounds {
        min_x: bounds.min_x,
        min_y: bounds.min_y,
        max_x: bounds.min_x + i32::try_from(bounds.width)? - 1,
        max_y: bounds.min_y + i32::try_from(bounds.height)? - 1,
        width: bounds.width,
        height: bounds.height,
    };
    let center_x = bounds.min_x + i32::try_from(bounds.width / 2)?;
    let center_y = bounds.min_y + i32::try_from(bounds.height / 2)?;
    Ok(PointerCalibrationStatus {
        coordinate_space: CoordinateSpace::PhysicalPixel,
        bounds: physical_bounds,
        monitors,
        sample_points: vec![
            PointerCalibrationPoint {
                label: "top_left".to_string(),
                x: bounds.min_x,
                y: bounds.min_y,
            },
            PointerCalibrationPoint {
                label: "center".to_string(),
                x: center_x,
                y: center_y,
            },
            PointerCalibrationPoint {
                label: "bottom_right".to_string(),
                x: bounds.min_x + i32::try_from(bounds.width)? - 1,
                y: bounds.min_y + i32::try_from(bounds.height)? - 1,
            },
        ],
        setup_hint: "physical_pixel pointer coordinates are derived from backend monitor logical origins, scale factors, and physical sizes; verify with a guarded disposable test window before production click use".to_string(),
    })
}

fn pointer_monitor_calibrations(
    monitors: &[MonitorInfo],
) -> Result<Vec<PointerMonitorCalibration>> {
    monitors
        .iter()
        .map(|monitor| {
            Ok(PointerMonitorCalibration {
                id: monitor.id.clone(),
                name: monitor.name.clone(),
                logical_origin_x: monitor.logical_origin_x,
                logical_origin_y: monitor.logical_origin_y,
                logical_width: monitor.logical_width,
                logical_height: monitor.logical_height,
                physical_origin_x: scaled_physical_origin(
                    monitor.logical_origin_x,
                    monitor.scale_factor,
                )?,
                physical_origin_y: scaled_physical_origin(
                    monitor.logical_origin_y,
                    monitor.scale_factor,
                )?,
                physical_width: monitor.physical_width,
                physical_height: monitor.physical_height,
                scale_factor: monitor.scale_factor,
                transform: monitor.transform.clone(),
            })
        })
        .collect()
}

pub(crate) fn logical_to_physical_point(point: Point, monitors: &[MonitorInfo]) -> Result<Point> {
    if !point.x.is_finite() || !point.y.is_finite() {
        bail!("logical_pixel pointer coordinates must be finite");
    }
    let monitor = monitor_for_global_logical_point(point.x, point.y, monitors)?;
    logical_point_on_monitor_to_physical(point.x, point.y, monitor)
}

pub(crate) fn active_window_local_to_physical_point(
    point: Point,
    active_window: Option<&WindowInfo>,
    monitors: &[MonitorInfo],
) -> Result<Point> {
    if !point.x.is_finite() || !point.y.is_finite() {
        bail!("window-local pointer coordinates must be finite");
    }
    let window = active_window.ok_or_else(|| {
        anyhow::anyhow!("window_local pointer coordinates require an active window")
    })?;
    let geometry = window.geometry.as_ref().ok_or_else(|| {
        anyhow::anyhow!("active window has no geometry for window_local pointer coordinates")
    })?;
    if geometry.space != CoordinateSpace::LogicalPixel {
        bail!(
            "active window geometry must be logical_pixel for window_local pointer coordinates, got {:?}",
            geometry.space
        );
    }
    if geometry.width == 0 || geometry.height == 0 {
        bail!("active window geometry has invalid size");
    }
    if point.x < 0.0
        || point.y < 0.0
        || point.x >= f64::from(geometry.width)
        || point.y >= f64::from(geometry.height)
    {
        bail!(
            "window_local pointer coordinate {},{} is outside active window {} {}x{}",
            point.x,
            point.y,
            window.id,
            geometry.width,
            geometry.height
        );
    }
    let monitor = monitor_for_window_point(window, geometry, point, monitors)?;
    let global_logical_x = f64::from(geometry.x) + point.x;
    let global_logical_y = f64::from(geometry.y) + point.y;
    logical_point_on_monitor_to_physical(global_logical_x, global_logical_y, monitor)
}

fn monitor_for_window_point<'a>(
    window: &WindowInfo,
    geometry: &WindowGeometry,
    point: Point,
    monitors: &'a [MonitorInfo],
) -> Result<&'a MonitorInfo> {
    if let Some(monitor_id) = window.monitor_id.as_deref()
        && let Some(monitor) = monitors.iter().find(|monitor| monitor.id == monitor_id)
    {
        return Ok(monitor);
    }
    let global_x = f64::from(geometry.x) + point.x;
    let global_y = f64::from(geometry.y) + point.y;
    monitor_for_global_logical_point(global_x, global_y, monitors).map_err(|_| {
        anyhow::anyhow!("window_local pointer coordinate does not map to a known monitor")
    })
}

fn monitor_for_global_logical_point(
    x: f64,
    y: f64,
    monitors: &[MonitorInfo],
) -> Result<&MonitorInfo> {
    monitors
        .iter()
        .find(|monitor| {
            let left = f64::from(monitor.logical_origin_x);
            let top = f64::from(monitor.logical_origin_y);
            let right = left + f64::from(monitor.logical_width);
            let bottom = top + f64::from(monitor.logical_height);
            x >= left && x < right && y >= top && y < bottom
        })
        .ok_or_else(|| {
            anyhow::anyhow!("logical_pixel pointer coordinate does not map to a known monitor")
        })
}

fn logical_point_on_monitor_to_physical(x: f64, y: f64, monitor: &MonitorInfo) -> Result<Point> {
    if monitor.logical_width == 0 || monitor.logical_height == 0 {
        bail!("monitor {} has invalid logical dimensions", monitor.id);
    }
    let physical_origin_x = scaled_physical_origin(monitor.logical_origin_x, monitor.scale_factor)?;
    let physical_origin_y = scaled_physical_origin(monitor.logical_origin_y, monitor.scale_factor)?;
    Ok(Point {
        x: f64::from(physical_origin_x)
            + (x - f64::from(monitor.logical_origin_x)) * monitor.scale_factor,
        y: f64::from(physical_origin_y)
            + (y - f64::from(monitor.logical_origin_y)) * monitor.scale_factor,
        space: CoordinateSpace::PhysicalPixel,
    })
}

pub(crate) fn physical_pointer_bounds_from_monitors(
    monitors: &[MonitorInfo],
) -> Result<seatgeist_uinput::PointerBounds> {
    if monitors.is_empty() {
        bail!("no monitor metadata available for physical pointer bounds");
    }
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for monitor in monitors {
        if monitor.physical_width < 2 || monitor.physical_height < 2 {
            bail!("monitor {} has invalid physical dimensions", monitor.id);
        }
        let origin_x = scaled_physical_origin(monitor.logical_origin_x, monitor.scale_factor)?;
        let origin_y = scaled_physical_origin(monitor.logical_origin_y, monitor.scale_factor)?;
        let end_x = origin_x
            .checked_add(i32::try_from(monitor.physical_width)?)
            .ok_or_else(|| anyhow::anyhow!("monitor {} physical x range overflows", monitor.id))?;
        let end_y = origin_y
            .checked_add(i32::try_from(monitor.physical_height)?)
            .ok_or_else(|| anyhow::anyhow!("monitor {} physical y range overflows", monitor.id))?;
        min_x = min_x.min(origin_x);
        min_y = min_y.min(origin_y);
        max_x = max_x.max(end_x);
        max_y = max_y.max(end_y);
    }
    let width = u32::try_from(max_x - min_x).context("physical pointer width is invalid")?;
    let height = u32::try_from(max_y - min_y).context("physical pointer height is invalid")?;
    if width < 2 || height < 2 {
        bail!("physical pointer bounds must be at least 2x2 pixels");
    }
    Ok(seatgeist_uinput::PointerBounds {
        min_x,
        min_y,
        width,
        height,
    })
}

pub(crate) fn scaled_physical_origin(origin: i32, scale_factor: f64) -> Result<i32> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        bail!("monitor scale factor must be finite and positive");
    }
    let scaled = f64::from(origin) * scale_factor;
    if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        bail!("scaled monitor origin overflows i32");
    }
    Ok(scaled.round() as i32)
}

pub(crate) fn validate_physical_pointer_point(
    point: Point,
    bounds: seatgeist_uinput::PointerBounds,
) -> Result<()> {
    if point.space != CoordinateSpace::PhysicalPixel {
        bail!(
            "resolved pointer actions require physical_pixel coordinate space, got {:?}",
            point.space
        );
    }
    if !point.x.is_finite() || !point.y.is_finite() {
        bail!("pointer coordinates must be finite");
    }
    let max_x = f64::from(bounds.min_x) + f64::from(bounds.width - 1);
    let max_y = f64::from(bounds.min_y) + f64::from(bounds.height - 1);
    if point.x < f64::from(bounds.min_x)
        || point.x > max_x
        || point.y < f64::from(bounds.min_y)
        || point.y > max_y
    {
        bail!(
            "pointer coordinate {},{} is outside physical desktop bounds {},{} {}x{}",
            point.x,
            point.y,
            bounds.min_x,
            bounds.min_y,
            bounds.width,
            bounds.height
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn window_local_context_reads_one_injected_active_window() {
        let window_backend = seatgeist_testkit::MockWindowBackend::default();
        let screen_backend = seatgeist_testkit::MockScreenBackend::default();
        let context = ResolutionContext::load(
            CoordinateSpace::WindowLocal,
            &window_backend,
            &screen_backend,
        )
        .await
        .expect("window-local context resolves");
        let point = context
            .resolve(Point {
                x: 10.0,
                y: 20.0,
                space: CoordinateSpace::WindowLocal,
            })
            .expect("window-local point resolves");
        assert_eq!(point.space, CoordinateSpace::PhysicalPixel);
        assert_eq!(point.x, 40.0);
        assert_eq!(point.y, 80.0);
        assert_eq!(
            window_backend
                .active_window_reads()
                .expect("active-window read count is available"),
            1
        );

        context
            .resolve(Point {
                x: 30.0,
                y: 40.0,
                space: CoordinateSpace::WindowLocal,
            })
            .expect("second point uses the same context");
        assert_eq!(
            window_backend
                .active_window_reads()
                .expect("active-window read count is available"),
            1,
            "multiple points in one action share one active-window snapshot"
        );
    }
}
