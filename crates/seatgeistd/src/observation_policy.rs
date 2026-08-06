use anyhow::{Context, Result};
use libseatgeist::{
    CoordinateSpace, DaemonRequest, JournalWindowContext, MonitorInfo, WindowGeometry, WindowInfo,
};
use seatgeist_backend::WindowBackend;

use crate::{
    config::{AppPolicy, RedactRegion},
    pointer_coordinates::scaled_physical_origin,
    window_safety::app_id_matches,
};

pub(crate) fn app_is_protected(app_policy: &AppPolicy, app_id: Option<&str>) -> bool {
    let Some(app_id) = app_id.map(str::trim).filter(|app_id| !app_id.is_empty()) else {
        return false;
    };
    app_policy
        .deny
        .iter()
        .any(|denied| app_id_matches(denied, app_id))
}

pub(crate) fn observable_window(
    app_policy: &AppPolicy,
    window: Option<WindowInfo>,
) -> Option<WindowInfo> {
    window.filter(|window| !app_is_protected(app_policy, window.app_id.as_deref()))
}

pub(crate) fn observable_windows(
    app_policy: &AppPolicy,
    windows: Vec<WindowInfo>,
) -> Vec<WindowInfo> {
    windows
        .into_iter()
        .filter(|window| !app_is_protected(app_policy, window.app_id.as_deref()))
        .collect()
}

pub(crate) fn observable_journal_window(
    app_policy: &AppPolicy,
    window: Option<JournalWindowContext>,
) -> Option<JournalWindowContext> {
    window.filter(|window| !app_is_protected(app_policy, window.app_id.as_deref()))
}

pub(crate) async fn require_observable_active_window(
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    context: &str,
) -> Result<()> {
    let active = window_backend
        .active_window()
        .await
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("protected-app observation could not read {context}"))?;
    if active
        .as_ref()
        .is_some_and(|window| app_is_protected(app_policy, window.app_id.as_deref()))
    {
        anyhow::bail!(
            "app policy denied observation of a protected application for {context}; \
             do not retry through accessibility, screenshot targeting, or another backend"
        );
    }
    Ok(())
}

pub(crate) async fn enforce_observation_app_policy(
    request: &DaemonRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
) -> Result<()> {
    match request {
        DaemonRequest::FocusedAccessibilityTree(_)
        | DaemonRequest::AccessibilityTextAttributes(_) => {
            require_observable_active_window(window_backend, app_policy, "accessibility read").await
        }
        DaemonRequest::AccessibilityFind(request) => {
            if app_is_protected(app_policy, request.app.as_deref()) {
                anyhow::bail!(
                    "app policy denied observation of a protected application for accessibility \
                     search; do not retry through another accessibility or screenshot backend"
                );
            }
            if request.app.is_none() {
                require_observable_active_window(
                    window_backend,
                    app_policy,
                    "unscoped accessibility search",
                )
                .await?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) fn protected_window_redactions(
    app_policy: &AppPolicy,
    windows: &[WindowInfo],
    monitors: &[MonitorInfo],
) -> Result<Vec<RedactRegion>> {
    let mut redactions = Vec::new();
    let Some((desktop_min_x, desktop_min_y)) = physical_desktop_origin(monitors)? else {
        return Ok(redactions);
    };
    for window in windows
        .iter()
        .filter(|window| app_is_protected(app_policy, window.app_id.as_deref()))
    {
        let Some(geometry) = window.geometry.as_ref() else {
            continue;
        };
        match geometry.space {
            CoordinateSpace::LogicalPixel => {
                for monitor in monitors {
                    if let Some(region) =
                        logical_window_region(geometry, monitor, desktop_min_x, desktop_min_y)?
                    {
                        redactions.push(region);
                    }
                }
            }
            CoordinateSpace::PhysicalPixel => {
                if let Some(region) = physical_window_region(geometry, desktop_min_x, desktop_min_y)
                {
                    redactions.push(region);
                }
            }
            CoordinateSpace::WindowLocal
            | CoordinateSpace::AccessibilityNode
            | CoordinateSpace::CaptureOutput => {}
        }
    }
    Ok(redactions)
}

fn physical_desktop_origin(monitors: &[MonitorInfo]) -> Result<Option<(i32, i32)>> {
    let mut origins = monitors
        .iter()
        .map(|monitor| {
            Ok((
                scaled_physical_origin(monitor.logical_origin_x, monitor.scale_factor)?,
                scaled_physical_origin(monitor.logical_origin_y, monitor.scale_factor)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    if origins.is_empty() {
        return Ok(None);
    }
    let min_x = origins.iter().map(|(x, _)| *x).min().unwrap_or_default();
    let min_y = origins.iter().map(|(_, y)| *y).min().unwrap_or_default();
    origins.clear();
    Ok(Some((min_x, min_y)))
}

fn logical_window_region(
    geometry: &WindowGeometry,
    monitor: &MonitorInfo,
    desktop_min_x: i32,
    desktop_min_y: i32,
) -> Result<Option<RedactRegion>> {
    let monitor_right = i64::from(monitor.logical_origin_x) + i64::from(monitor.logical_width);
    let monitor_bottom = i64::from(monitor.logical_origin_y) + i64::from(monitor.logical_height);
    let window_right = i64::from(geometry.x) + i64::from(geometry.width);
    let window_bottom = i64::from(geometry.y) + i64::from(geometry.height);
    let left = i64::from(geometry.x).max(i64::from(monitor.logical_origin_x));
    let top = i64::from(geometry.y).max(i64::from(monitor.logical_origin_y));
    let right = window_right.min(monitor_right);
    let bottom = window_bottom.min(monitor_bottom);
    if right <= left || bottom <= top {
        return Ok(None);
    }

    let monitor_physical_x =
        scaled_physical_origin(monitor.logical_origin_x, monitor.scale_factor)?;
    let monitor_physical_y =
        scaled_physical_origin(monitor.logical_origin_y, monitor.scale_factor)?;
    let physical_left = f64::from(monitor_physical_x)
        + (left as f64 - f64::from(monitor.logical_origin_x)) * monitor.scale_factor;
    let physical_top = f64::from(monitor_physical_y)
        + (top as f64 - f64::from(monitor.logical_origin_y)) * monitor.scale_factor;
    let physical_right = f64::from(monitor_physical_x)
        + (right as f64 - f64::from(monitor.logical_origin_x)) * monitor.scale_factor;
    let physical_bottom = f64::from(monitor_physical_y)
        + (bottom as f64 - f64::from(monitor.logical_origin_y)) * monitor.scale_factor;
    normalized_region(
        physical_left.floor() as i64,
        physical_top.floor() as i64,
        physical_right.ceil() as i64,
        physical_bottom.ceil() as i64,
        desktop_min_x,
        desktop_min_y,
    )
}

fn physical_window_region(
    geometry: &WindowGeometry,
    desktop_min_x: i32,
    desktop_min_y: i32,
) -> Option<RedactRegion> {
    normalized_region(
        i64::from(geometry.x),
        i64::from(geometry.y),
        i64::from(geometry.x) + i64::from(geometry.width),
        i64::from(geometry.y) + i64::from(geometry.height),
        desktop_min_x,
        desktop_min_y,
    )
    .ok()
    .flatten()
}

fn normalized_region(
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
    desktop_min_x: i32,
    desktop_min_y: i32,
) -> Result<Option<RedactRegion>> {
    if right <= left || bottom <= top {
        return Ok(None);
    }
    let x = left - i64::from(desktop_min_x);
    let y = top - i64::from(desktop_min_y);
    if x < 0 || y < 0 {
        return Ok(None);
    }
    Ok(Some(RedactRegion {
        x: u32::try_from(x).context("protected-window redaction x overflows u32")?,
        y: u32::try_from(y).context("protected-window redaction y overflows u32")?,
        width: u32::try_from(right - left)
            .context("protected-window redaction width overflows u32")?,
        height: u32::try_from(bottom - top)
            .context("protected-window redaction height overflows u32")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AppPolicy {
        AppPolicy {
            allow: Vec::new(),
            deny: vec!["org.keepassxc.KeePassXC".to_string()],
        }
    }

    fn monitor(id: &str, logical_x: i32, scale: f64) -> MonitorInfo {
        MonitorInfo {
            id: id.to_string(),
            name: Some(id.to_string()),
            physical_width: (1000.0 * scale) as u32,
            physical_height: (800.0 * scale) as u32,
            logical_width: 1000,
            logical_height: 800,
            scale_factor: scale,
            logical_origin_x: logical_x,
            logical_origin_y: 0,
            transform: None,
        }
    }

    fn window(app_id: &str, geometry: WindowGeometry) -> WindowInfo {
        WindowInfo {
            id: app_id.to_string(),
            app_id: Some(app_id.to_string()),
            title: "database name".to_string(),
            pid: Some(42),
            monitor_id: None,
            geometry: Some(geometry),
        }
    }

    #[test]
    fn protected_windows_are_omitted_from_model_facing_metadata() {
        let windows = vec![
            window(
                "org.keepassxc.KeePassXC",
                WindowGeometry {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                    space: CoordinateSpace::LogicalPixel,
                },
            ),
            window(
                "org.kde.konsole",
                WindowGeometry {
                    x: 50,
                    y: 60,
                    width: 70,
                    height: 80,
                    space: CoordinateSpace::LogicalPixel,
                },
            ),
        ];
        let visible = observable_windows(&policy(), windows);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].app_id.as_deref(), Some("org.kde.konsole"));
    }

    #[test]
    fn protected_window_geometry_becomes_physical_redactions() {
        let windows = vec![window(
            "org.keepassxc.KeePassXC",
            WindowGeometry {
                x: 100,
                y: 50,
                width: 200,
                height: 100,
                space: CoordinateSpace::LogicalPixel,
            },
        )];
        let redactions =
            protected_window_redactions(&policy(), &windows, &[monitor("main", 0, 1.5)])
                .expect("redactions resolve");
        assert_eq!(
            redactions,
            vec![RedactRegion {
                x: 150,
                y: 75,
                width: 300,
                height: 150,
            }]
        );
    }

    #[test]
    fn spanning_window_is_split_and_normalized_across_negative_origin_monitors() {
        let windows = vec![window(
            "org.keepassxc.KeePassXC",
            WindowGeometry {
                x: -100,
                y: 10,
                width: 200,
                height: 50,
                space: CoordinateSpace::LogicalPixel,
            },
        )];
        let redactions = protected_window_redactions(
            &policy(),
            &windows,
            &[monitor("left", -1000, 1.0), monitor("right", 0, 2.0)],
        )
        .expect("redactions resolve");
        assert_eq!(
            redactions,
            vec![
                RedactRegion {
                    x: 900,
                    y: 10,
                    width: 100,
                    height: 50,
                },
                RedactRegion {
                    x: 1000,
                    y: 20,
                    width: 200,
                    height: 100,
                },
            ]
        );
    }
}
