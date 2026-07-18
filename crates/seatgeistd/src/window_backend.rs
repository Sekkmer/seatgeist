use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use libseatgeist::{
    CoordinateSpace, MonitorInfo, SeatgeistError, WindowGeometry, WindowId, WindowInfo,
};
use seatgeist_backend::WindowBackend;

use crate::{
    interaction::FocusBackend,
    kwin_bridge::WindowActionQueue,
    kwin_bridge::{ActiveWindowState, WindowListState},
};

#[derive(Debug, Clone)]
pub(crate) struct KwinWindowBackend {
    active_window_state: ActiveWindowState,
    window_list_state: WindowListState,
    focus_backend: Arc<dyn FocusBackend>,
    window_action_queue: WindowActionQueue,
}

impl KwinWindowBackend {
    pub(crate) fn new(
        active_window_state: ActiveWindowState,
        window_list_state: WindowListState,
        focus_backend: Arc<dyn FocusBackend>,
        window_action_queue: WindowActionQueue,
    ) -> Self {
        Self {
            active_window_state,
            window_list_state,
            focus_backend,
            window_action_queue,
        }
    }
}

#[async_trait]
impl WindowBackend for KwinWindowBackend {
    fn backend_name(&self) -> &'static str {
        self.focus_backend.name()
    }

    async fn list_windows(&self) -> seatgeist_backend::Result<Vec<WindowInfo>> {
        list_windows(&self.window_list_state).map_err(backend_error)
    }

    async fn active_window(&self) -> seatgeist_backend::Result<Option<WindowInfo>> {
        active_window(&self.active_window_state).map_err(backend_error)
    }

    async fn focus_window(&self, id: WindowId) -> seatgeist_backend::Result<()> {
        self.focus_backend.focus(&id).map_err(backend_error)
    }

    async fn move_window(
        &self,
        id: WindowId,
        x: i32,
        y: i32,
    ) -> seatgeist_backend::Result<WindowGeometry> {
        self.window_action_queue
            .move_window(&id, x, y)
            .await
            .map_err(backend_error)
    }

    async fn resize_window(
        &self,
        id: WindowId,
        width: u32,
        height: u32,
    ) -> seatgeist_backend::Result<WindowGeometry> {
        self.window_action_queue
            .resize_window(&id, width, height)
            .await
            .map_err(backend_error)
    }
}

fn backend_error(error: anyhow::Error) -> SeatgeistError {
    SeatgeistError::BackendUnavailable(error.to_string())
}

pub(crate) fn list_monitors() -> Result<Vec<MonitorInfo>> {
    seatgeist_kwin::list_monitors().map_err(anyhow::Error::msg)
}

pub(crate) fn list_windows(window_list_state: &WindowListState) -> Result<Vec<WindowInfo>> {
    let monitors = list_monitors().unwrap_or_default();
    list_windows_with_monitors(window_list_state, &monitors)
}

pub(crate) fn list_windows_with_monitors(
    window_list_state: &WindowListState,
    monitors: &[MonitorInfo],
) -> Result<Vec<WindowInfo>> {
    let bridge_windows = window_list_state.snapshot()?;
    let mut windows = match seatgeist_kwin::list_windows() {
        Ok(windows) => windows,
        Err(err) => match bridge_windows {
            Some(mut windows) => {
                assign_monitor_ids(&mut windows, monitors);
                return Ok(windows);
            }
            None => return Err(anyhow::Error::msg(err)),
        },
    };
    if let Some(bridge_windows) = bridge_windows {
        merge_bridge_windows(&mut windows, bridge_windows);
    }
    assign_monitor_ids(&mut windows, monitors);
    Ok(windows)
}

pub(crate) fn active_window(active_window_state: &ActiveWindowState) -> Result<Option<WindowInfo>> {
    let monitors = list_monitors().unwrap_or_default();
    active_window_with_monitors(active_window_state, &monitors)
}

pub(crate) fn active_window_with_monitors(
    active_window_state: &ActiveWindowState,
    monitors: &[MonitorInfo],
) -> Result<Option<WindowInfo>> {
    if let Some(window) = active_window_state.snapshot()? {
        return Ok(window.map(|mut window| {
            assign_monitor_id(&mut window, monitors);
            window
        }));
    }
    let mut window = seatgeist_kwin::active_window().map_err(anyhow::Error::msg)?;
    if let Some(window) = window.as_mut() {
        assign_monitor_id(window, monitors);
    }
    Ok(window)
}

pub(crate) fn merge_bridge_windows(windows: &mut Vec<WindowInfo>, bridge_windows: Vec<WindowInfo>) {
    for bridge_window in bridge_windows {
        match windows
            .iter_mut()
            .find(|window| window.id == bridge_window.id)
        {
            Some(window) => merge_bridge_window(window, bridge_window),
            None => windows.push(bridge_window),
        }
    }
}

fn merge_bridge_window(window: &mut WindowInfo, bridge_window: WindowInfo) {
    if let Some(app_id) = bridge_window.app_id {
        window.app_id = Some(app_id);
    }
    if !bridge_window.title.trim().is_empty() {
        window.title = bridge_window.title;
    }
    if bridge_window.pid.is_some() {
        window.pid = bridge_window.pid;
    }
    if bridge_window.geometry.is_some() {
        window.geometry = bridge_window.geometry;
    }
    if bridge_window.monitor_id.is_some() {
        window.monitor_id = bridge_window.monitor_id;
    }
}

fn assign_monitor_ids(windows: &mut [WindowInfo], monitors: &[MonitorInfo]) {
    for window in windows {
        assign_monitor_id(window, monitors);
    }
}

pub(crate) fn assign_monitor_id(window: &mut WindowInfo, monitors: &[MonitorInfo]) {
    if window.monitor_id.is_none() {
        window.monitor_id = window_monitor_id(window, monitors);
    }
}

fn window_monitor_id(window: &WindowInfo, monitors: &[MonitorInfo]) -> Option<String> {
    let geometry = window.geometry.as_ref()?;
    if geometry.space != CoordinateSpace::LogicalPixel {
        return None;
    }
    monitors
        .iter()
        .filter_map(|monitor| {
            let area = logical_overlap_area(geometry, monitor);
            (area > 0).then(|| (area, monitor.id.clone()))
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, id)| id)
}

fn logical_overlap_area(geometry: &WindowGeometry, monitor: &MonitorInfo) -> i64 {
    let window_left = i64::from(geometry.x);
    let window_top = i64::from(geometry.y);
    let window_right = window_left + i64::from(geometry.width);
    let window_bottom = window_top + i64::from(geometry.height);
    let monitor_left = i64::from(monitor.logical_origin_x);
    let monitor_top = i64::from(monitor.logical_origin_y);
    let monitor_right = monitor_left + i64::from(monitor.logical_width);
    let monitor_bottom = monitor_top + i64::from(monitor.logical_height);

    let overlap_width = (window_right.min(monitor_right) - window_left.max(monitor_left)).max(0);
    let overlap_height = (window_bottom.min(monitor_bottom) - window_top.max(monitor_top)).max(0);
    overlap_width * overlap_height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_shared_backend<T: WindowBackend>() {}

    #[test]
    fn production_adapter_implements_shared_window_backend() {
        assert_shared_backend::<KwinWindowBackend>();
    }
}
