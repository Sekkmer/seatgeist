use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use libseatgeist::{
    CoordinateSpace, KwinBridgeStatus, WindowActivationMode, WindowGeometry, WindowInfo,
    WindowPlacementAnchor,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::info;
use uuid::Uuid;

use crate::{activity, xdg};

const SERVICE: &str = "org.seatgeist.KWinBridge";
const PATH: &str = "/org/seatgeist/KWinBridge1";
const INTERFACE: &str = "org.seatgeist.KWinBridge1";
const WINDOW_ACTION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default)]
pub(super) struct WindowActionQueue {
    registered: Arc<AtomicBool>,
    script_ready: Arc<AtomicBool>,
    move_ready: Arc<AtomicBool>,
    launch_ready: Arc<AtomicBool>,
    inner: Arc<Mutex<WindowActionState>>,
}

#[derive(Debug, Default)]
struct WindowActionState {
    pending: VecDeque<PendingWindowAction>,
    waiters: HashMap<String, oneshot::Sender<WindowActionResult>>,
    acknowledgements: HashMap<String, oneshot::Sender<()>>,
}

#[derive(Debug)]
struct PendingWindowAction {
    id: String,
    payload: String,
}

#[derive(Debug, Serialize)]
struct ResizeWindowAction<'a> {
    id: &'a str,
    action: &'static str,
    window_id: &'a str,
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize)]
struct MoveWindowAction<'a> {
    id: &'a str,
    action: &'static str,
    window_id: &'a str,
    x: i32,
    y: i32,
}

#[derive(Debug, Serialize)]
struct LaunchWindowAction<'a> {
    id: &'a str,
    action: &'static str,
    desktop_entry: &'a str,
    anchor: WindowPlacementAnchor,
    monitor_id: Option<&'a str>,
    width: Option<u32>,
    height: Option<u32>,
    margin: u32,
    activation: WindowActivationMode,
    timeout_ms: u64,
}

#[derive(Debug, Serialize)]
struct CancelLaunchWindowAction<'a> {
    id: &'a str,
    action: &'static str,
    launch_id: &'a str,
}

#[derive(Debug)]
pub(super) struct LaunchWindowTicket {
    id: String,
    receiver: oneshot::Receiver<WindowActionResult>,
}

#[derive(Debug)]
pub(super) struct LaunchWindowOutcome {
    pub window: WindowInfo,
    pub focus_preserved: bool,
}

impl LaunchWindowTicket {
    pub(super) fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Deserialize)]
struct WindowActionResult {
    id: String,
    ok: bool,
    error: Option<String>,
    geometry: Option<ActiveWindowGeometry>,
    window_id: Option<String>,
    app_id: Option<String>,
    title: Option<String>,
    pid: Option<u32>,
    monitor_id: Option<String>,
    focus_preserved: Option<bool>,
}

impl WindowActionQueue {
    pub(super) fn set_registered(&self, registered: bool) {
        self.registered.store(registered, Ordering::Release);
    }

    pub(super) fn register_script_capabilities(&self, capabilities: &str) {
        let resize = capabilities
            .split(',')
            .map(str::trim)
            .any(|capability| capability == "resize_window");
        self.script_ready.store(resize, Ordering::Release);
        let move_window = capabilities
            .split(',')
            .map(str::trim)
            .any(|capability| capability == "move_window");
        self.move_ready.store(move_window, Ordering::Release);
        let launch_window = capabilities
            .split(',')
            .map(str::trim)
            .any(|capability| capability == "launch_window");
        self.launch_ready.store(launch_window, Ordering::Release);
    }

    pub(super) fn resize_ready(&self) -> bool {
        self.registered.load(Ordering::Acquire) && self.script_ready.load(Ordering::Acquire)
    }

    pub(super) fn move_ready(&self) -> bool {
        self.registered.load(Ordering::Acquire) && self.move_ready.load(Ordering::Acquire)
    }

    pub(super) fn launch_ready(&self) -> bool {
        self.registered.load(Ordering::Acquire) && self.launch_ready.load(Ordering::Acquire)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn arm_launch_window(
        &self,
        desktop_entry: &str,
        anchor: WindowPlacementAnchor,
        monitor_id: Option<&str>,
        width: Option<u32>,
        height: Option<u32>,
        margin: u32,
        activation: WindowActivationMode,
        timeout_ms: u64,
    ) -> Result<LaunchWindowTicket> {
        if !self.registered.load(Ordering::Acquire) {
            bail!("KWin script bridge DBus receiver is unavailable");
        }
        if !self.launch_ready.load(Ordering::Acquire) {
            bail!(
                "installed KWin script has not registered launch_window support; install/reload the current seatgeist-bridge package"
            );
        }
        let id = Uuid::new_v4().to_string();
        let payload = serde_json::to_string(&LaunchWindowAction {
            id: &id,
            action: "launch_window",
            desktop_entry,
            anchor,
            monitor_id,
            width,
            height,
            margin,
            activation,
            timeout_ms,
        })
        .context("encode KWin launch intent")?;
        let (result_sender, result_receiver) = oneshot::channel();
        let (ack_sender, ack_receiver) = oneshot::channel();
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?;
            state.pending.push_back(PendingWindowAction {
                id: id.clone(),
                payload,
            });
            state.waiters.insert(id.clone(), result_sender);
            state.acknowledgements.insert(id.clone(), ack_sender);
        }
        match tokio::time::timeout(WINDOW_ACTION_TIMEOUT, ack_receiver).await {
            Ok(Ok(())) => Ok(LaunchWindowTicket {
                id,
                receiver: result_receiver,
            }),
            Ok(Err(_)) => {
                self.remove(&id)?;
                bail!("KWin launch-intent acknowledgement channel closed")
            }
            Err(_) => {
                self.remove(&id)?;
                bail!("KWin script bridge did not arm the launch intent in time")
            }
        }
    }

    pub(super) fn cancel_launch_window(&self, id: &str) -> Result<()> {
        self.remove(id)?;
        let cancel_id = Uuid::new_v4().to_string();
        let payload = serde_json::to_string(&CancelLaunchWindowAction {
            id: &cancel_id,
            action: "cancel_launch_window",
            launch_id: id,
        })
        .context("encode KWin launch-intent cancellation")?;
        self.inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?
            .pending
            .push_back(PendingWindowAction {
                id: cancel_id,
                payload,
            });
        Ok(())
    }

    pub(super) async fn finish_launch_window(
        &self,
        ticket: LaunchWindowTicket,
        timeout: Duration,
    ) -> Result<LaunchWindowOutcome> {
        let result = match tokio::time::timeout(timeout, ticket.receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => bail!("KWin launch result channel closed before confirmation"),
            Err(_) => {
                self.cancel_launch_window(&ticket.id)?;
                bail!("timed out waiting for the launched window")
            }
        };
        if !result.ok {
            bail!(
                "KWin launch placement failed: {}",
                result.error.as_deref().unwrap_or("unknown script error")
            );
        }
        let geometry = result
            .geometry
            .map(Into::into)
            .ok_or_else(|| anyhow::anyhow!("launch succeeded without geometry metadata"))?;
        let window_id = result
            .window_id
            .ok_or_else(|| anyhow::anyhow!("launch succeeded without a window id"))?;
        Ok(LaunchWindowOutcome {
            window: WindowInfo {
                id: window_id,
                app_id: result.app_id,
                title: result.title.unwrap_or_default(),
                pid: result.pid,
                monitor_id: result.monitor_id,
                geometry: Some(geometry),
            },
            focus_preserved: result.focus_preserved.unwrap_or(false),
        })
    }

    pub(super) async fn move_window(
        &self,
        window_id: &str,
        x: i32,
        y: i32,
    ) -> Result<WindowGeometry> {
        if !self.registered.load(Ordering::Acquire) {
            bail!("KWin script bridge DBus receiver is unavailable");
        }
        if !self.move_ready.load(Ordering::Acquire) {
            bail!(
                "installed KWin script has not registered move_window support; install/reload the current seatgeist-bridge package"
            );
        }
        let id = Uuid::new_v4().to_string();
        let payload = serde_json::to_string(&MoveWindowAction {
            id: &id,
            action: "move_window",
            window_id,
            x,
            y,
        })
        .context("encode KWin move action")?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?;
            state.pending.push_back(PendingWindowAction {
                id: id.clone(),
                payload,
            });
            state.waiters.insert(id.clone(), sender);
        }
        let result = match tokio::time::timeout(WINDOW_ACTION_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => bail!("KWin move action channel closed before acknowledgement"),
            Err(_) => {
                self.remove(&id)?;
                bail!(
                    "KWin script bridge did not acknowledge move within {}ms",
                    WINDOW_ACTION_TIMEOUT.as_millis()
                );
            }
        };
        if !result.ok {
            bail!(
                "KWin move failed: {}",
                result.error.as_deref().unwrap_or("unknown script error")
            );
        }
        result
            .geometry
            .map(Into::into)
            .ok_or_else(|| anyhow::anyhow!("KWin move succeeded without geometry metadata"))
    }

    pub(super) async fn resize_window(
        &self,
        window_id: &str,
        width: u32,
        height: u32,
    ) -> Result<WindowGeometry> {
        if !self.registered.load(Ordering::Acquire) {
            bail!("KWin script bridge DBus receiver is unavailable");
        }
        if !self.script_ready.load(Ordering::Acquire) {
            bail!(
                "installed KWin script has not registered resize_window support; install/reload the current seatgeist-bridge package"
            );
        }
        let id = Uuid::new_v4().to_string();
        let payload = serde_json::to_string(&ResizeWindowAction {
            id: &id,
            action: "resize_window",
            window_id,
            width,
            height,
        })
        .context("encode KWin resize action")?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?;
            state.pending.push_back(PendingWindowAction {
                id: id.clone(),
                payload,
            });
            state.waiters.insert(id.clone(), sender);
        }

        let result = match tokio::time::timeout(WINDOW_ACTION_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => bail!("KWin resize action channel closed before acknowledgement"),
            Err(_) => {
                self.remove(&id)?;
                bail!(
                    "KWin script bridge did not acknowledge resize within {}ms; verify the packaged script is installed and enabled",
                    WINDOW_ACTION_TIMEOUT.as_millis()
                );
            }
        };
        if !result.ok {
            bail!(
                "KWin resize failed: {}",
                result.error.as_deref().unwrap_or("unknown script error")
            );
        }
        result
            .geometry
            .map(Into::into)
            .ok_or_else(|| anyhow::anyhow!("KWin resize succeeded without geometry metadata"))
    }

    fn take(&self) -> Result<String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?;
        Ok(state
            .pending
            .pop_front()
            .map(|action| action.payload)
            .unwrap_or_default())
    }

    fn complete(&self, payload: &str) -> Result<()> {
        let result: WindowActionResult =
            serde_json::from_str(payload).context("parse KWin action result")?;
        let sender = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?
            .waiters
            .remove(&result.id);
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
        Ok(())
    }

    fn remove(&self, id: &str) -> Result<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?;
        state.pending.retain(|action| action.id != id);
        state.waiters.remove(id);
        state.acknowledgements.remove(id);
        Ok(())
    }

    fn acknowledge(&self, id: &str) -> Result<()> {
        let sender = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("KWin action queue lock is poisoned"))?
            .acknowledgements
            .remove(id);
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ActiveWindowState {
    inner: Arc<Mutex<ActiveWindowSnapshot>>,
}

impl ActiveWindowState {
    pub(super) fn update_from_payload(&self, payload: &str) -> Result<()> {
        let payload = serde_json::from_str::<ActiveWindowPayload>(payload)
            .context("parse KWin active-window payload")?;
        let window = payload.into_window()?;
        let mut snapshot = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("active-window state lock is poisoned"))?;
        snapshot.updated = true;
        snapshot.window = window;
        Ok(())
    }

    pub(super) fn snapshot(&self) -> Result<Option<Option<WindowInfo>>> {
        let snapshot = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("active-window state lock is poisoned"))?;
        if snapshot.updated {
            Ok(Some(snapshot.window.clone()))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ActiveWindowSnapshot {
    updated: bool,
    window: Option<WindowInfo>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct WindowListState {
    inner: Arc<Mutex<WindowListSnapshot>>,
}

impl WindowListState {
    pub(super) fn update_from_payload(&self, payload: &str) -> Result<()> {
        let payload = serde_json::from_str::<WindowListPayload>(payload)
            .context("parse KWin window-list payload")?;
        let mut windows = Vec::with_capacity(payload.windows.len());
        for window in payload.windows {
            if let Some(window) = window.into_window()? {
                windows.push(window);
            }
        }
        let mut snapshot = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("window-list state lock is poisoned"))?;
        snapshot.updated = true;
        snapshot.windows = windows;
        Ok(())
    }

    pub(super) fn snapshot(&self) -> Result<Option<Vec<WindowInfo>>> {
        let snapshot = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("window-list state lock is poisoned"))?;
        if snapshot.updated {
            Ok(Some(snapshot.windows.clone()))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, Default)]
struct WindowListSnapshot {
    updated: bool,
    windows: Vec<WindowInfo>,
}

#[derive(Debug, Clone)]
struct Bridge {
    active_window_state: ActiveWindowState,
    window_list_state: WindowListState,
    activity_tracker: activity::ActivityTracker,
    window_action_queue: WindowActionQueue,
}

#[zbus::interface(name = "org.seatgeist.KWinBridge1")]
impl Bridge {
    async fn update_active_window(&self, payload: &str) -> zbus::fdo::Result<()> {
        self.active_window_state
            .update_from_payload(payload)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?;
        Ok(())
    }

    async fn update_windows(&self, payload: &str) -> zbus::fdo::Result<()> {
        self.window_list_state
            .update_from_payload(payload)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?;
        Ok(())
    }

    async fn register_input_activity_backend(&self, backend: &str) -> zbus::fdo::Result<()> {
        self.activity_tracker
            .register_backend(backend)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
    }

    async fn update_input_activity(&self, payload: &str) -> zbus::fdo::Result<()> {
        self.activity_tracker
            .record_payload(payload)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
    }

    async fn take_pending_action(&self) -> zbus::fdo::Result<String> {
        self.window_action_queue
            .take()
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
    }

    async fn complete_action(&self, payload: &str) -> zbus::fdo::Result<()> {
        self.window_action_queue
            .complete(payload)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
    }

    async fn acknowledge_action(&self, id: &str) -> zbus::fdo::Result<()> {
        self.window_action_queue
            .acknowledge(id)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
    }

    async fn register_action_capabilities(&self, capabilities: &str) -> zbus::fdo::Result<()> {
        self.window_action_queue
            .register_script_capabilities(capabilities);
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct ActiveWindowPayload {
    active: bool,
    id: Option<String>,
    title: Option<String>,
    app_id: Option<String>,
    pid: Option<u32>,
    geometry: Option<ActiveWindowGeometry>,
}

impl ActiveWindowPayload {
    fn into_window(self) -> Result<Option<WindowInfo>> {
        if !self.active {
            return Ok(None);
        }
        BridgeWindowPayload {
            id: self.id,
            title: self.title,
            app_id: self.app_id,
            pid: self.pid,
            geometry: self.geometry,
        }
        .into_window()
    }
}

#[derive(Debug, Deserialize)]
struct WindowListPayload {
    windows: Vec<BridgeWindowPayload>,
}

#[derive(Debug, Deserialize)]
struct BridgeWindowPayload {
    id: Option<String>,
    title: Option<String>,
    app_id: Option<String>,
    pid: Option<u32>,
    geometry: Option<ActiveWindowGeometry>,
}

impl BridgeWindowPayload {
    fn into_window(self) -> Result<Option<WindowInfo>> {
        let id = self
            .id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("KWin window payload missing id"))?;
        Ok(Some(WindowInfo {
            id,
            app_id: self.app_id.filter(|app_id| !app_id.trim().is_empty()),
            title: self.title.unwrap_or_default(),
            pid: self.pid,
            monitor_id: None,
            geometry: self.geometry.map(Into::into),
        }))
    }
}

#[derive(Debug, Deserialize)]
struct ActiveWindowGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl From<ActiveWindowGeometry> for WindowGeometry {
    fn from(geometry: ActiveWindowGeometry) -> Self {
        Self {
            x: geometry.x,
            y: geometry.y,
            width: geometry.width.max(1),
            height: geometry.height.max(1),
            space: CoordinateSpace::LogicalPixel,
        }
    }
}

pub(super) fn status(
    active_window_state: &ActiveWindowState,
    window_list_state: &WindowListState,
    dbus_service_registered: bool,
    window_action_queue: &WindowActionQueue,
) -> Result<KwinBridgeStatus> {
    let package_dir = xdg::data_home().join("kwin/scripts/seatgeist-bridge");
    let config_path = xdg::config_home().join("kwinrc");
    let script_enabled = read_enabled(&config_path)?;
    status_with_installation(
        active_window_state,
        window_list_state,
        dbus_service_registered,
        window_action_queue.resize_ready(),
        window_action_queue.move_ready(),
        window_action_queue.launch_ready(),
        BridgeInstallation {
            package_dir,
            config_path,
            script_enabled,
        },
    )
}

struct BridgeInstallation {
    package_dir: PathBuf,
    config_path: PathBuf,
    script_enabled: Option<bool>,
}

fn status_with_installation(
    active_window_state: &ActiveWindowState,
    window_list_state: &WindowListState,
    dbus_service_registered: bool,
    window_resize_supported: bool,
    window_move_supported: bool,
    window_launch_supported: bool,
    installation: BridgeInstallation,
) -> Result<KwinBridgeStatus> {
    let active_window_snapshot = active_window_state.snapshot()?;
    let active_window_update_seen = active_window_snapshot.is_some();
    let active_window = active_window_snapshot.flatten();
    let window_list_snapshot = window_list_state.snapshot()?;
    let window_list_update_seen = window_list_snapshot.is_some();
    let window_count = window_list_snapshot.map_or(0, |windows| windows.len());

    Ok(KwinBridgeStatus {
        dbus_service_registered,
        window_resize_supported,
        window_move_supported,
        window_launch_supported,
        active_window_update_seen,
        window_list_update_seen,
        window_count,
        active_window,
        package_installed: installation.package_dir.join("metadata.json").is_file(),
        package_dir: installation.package_dir,
        config_path: installation.config_path,
        script_enabled: installation.script_enabled,
    })
}

fn read_enabled(config_path: &Path) -> Result<Option<bool>> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", config_path.display())),
    };
    Ok(parse_enabled(&content))
}

fn parse_enabled(content: &str) -> Option<bool> {
    let mut in_plugins = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_plugins = line == "[Plugins]";
            continue;
        }
        if !in_plugins {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "seatgeist-bridgeEnabled" {
            return parse_bool(value.trim());
        }
    }
    None
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub(super) async fn start_kwin_bridge(
    active_window_state: ActiveWindowState,
    window_list_state: WindowListState,
    activity_tracker: activity::ActivityTracker,
    window_action_queue: WindowActionQueue,
) -> Result<zbus::Connection> {
    let connection = zbus::connection::Builder::session()
        .context("connect to session bus for KWin bridge")?
        .name(SERVICE)
        .context("request KWin bridge DBus service name")?
        .serve_at(
            PATH,
            Bridge {
                active_window_state,
                window_list_state,
                activity_tracker,
                window_action_queue,
            },
        )
        .context("serve KWin bridge DBus object")?
        .build()
        .await
        .context("build KWin bridge DBus connection")?;
    info!(
        service = SERVICE,
        path = PATH,
        interface = INTERFACE,
        "KWin bridge DBus service registered"
    );
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_enabled_state_only_from_plugins_group() {
        let config = r#"
            [Other]
            seatgeist-bridgeEnabled=false

            [Plugins]
            unrelated=true
            seatgeist-bridgeEnabled=true
        "#;
        assert_eq!(parse_enabled(config), Some(true));
        assert_eq!(
            parse_enabled("[Plugins]\nseatgeist-bridgeEnabled=OFF\n"),
            Some(false)
        );
        assert_eq!(parse_enabled("[Plugins]\nunrelated=true\n"), None);
    }

    #[test]
    fn status_reports_window_list_snapshot() {
        let active_window_state = ActiveWindowState::default();
        let window_list_state = WindowListState::default();
        window_list_state
            .update_from_payload(
                r#"{
                    "windows": [
                        {"id": "window-1", "title": "One", "app_id": "org.example.One"},
                        {"id": "window-2", "title": "Two", "app_id": "org.example.Two"}
                    ]
                }"#,
            )
            .expect("window list payload updates state");

        let status = status_with_installation(
            &active_window_state,
            &window_list_state,
            true,
            false,
            false,
            false,
            BridgeInstallation {
                package_dir: PathBuf::from("/missing/seatgeist-bridge"),
                config_path: PathBuf::from("/missing/kwinrc"),
                script_enabled: Some(true),
            },
        )
        .expect("bridge status succeeds");

        assert!(status.dbus_service_registered);
        assert!(!status.window_resize_supported);
        assert!(!status.window_move_supported);
        assert!(!status.window_launch_supported);
        assert!(!status.active_window_update_seen);
        assert!(status.window_list_update_seen);
        assert_eq!(status.window_count, 2);
        assert!(status.active_window.is_none());
        assert!(!status.package_installed);
        assert_eq!(status.script_enabled, Some(true));
    }

    #[tokio::test]
    async fn resize_actions_round_trip_through_the_bridge_queue() {
        let queue = WindowActionQueue::default();
        queue.set_registered(true);
        queue.register_script_capabilities("resize_window");
        let resize_queue = queue.clone();
        let resize =
            tokio::spawn(async move { resize_queue.resize_window("window-1", 1280, 720).await });

        tokio::task::yield_now().await;
        let payload = queue.take().expect("queued action is available");
        let action: serde_json::Value =
            serde_json::from_str(&payload).expect("queued action is valid JSON");
        assert_eq!(action["action"], "resize_window");
        assert_eq!(action["window_id"], "window-1");
        assert_eq!(action["width"], 1280);
        assert_eq!(action["height"], 720);

        queue
            .complete(
                &serde_json::json!({
                    "id": action["id"],
                    "ok": true,
                    "geometry": {"x": 10, "y": 20, "width": 1280, "height": 720}
                })
                .to_string(),
            )
            .expect("completion is accepted");
        let geometry = resize
            .await
            .expect("resize task joins")
            .expect("resize succeeds");
        assert_eq!(geometry.x, 10);
        assert_eq!(geometry.y, 20);
        assert_eq!(geometry.width, 1280);
        assert_eq!(geometry.height, 720);
    }

    #[tokio::test]
    async fn stale_script_fails_before_a_resize_is_queued() {
        let queue = WindowActionQueue::default();
        queue.set_registered(true);
        let error = queue
            .resize_window("window-1", 1280, 720)
            .await
            .expect_err("missing capability handshake fails closed");
        assert!(error.to_string().contains("has not registered"));
        assert!(queue.take().expect("queue remains readable").is_empty());
    }

    #[tokio::test]
    async fn launch_intent_is_acknowledged_before_completion() {
        let queue = WindowActionQueue::default();
        queue.set_registered(true);
        queue.register_script_capabilities("resize_window,move_window,launch_window");
        let arm_queue = queue.clone();
        let arm = tokio::spawn(async move {
            arm_queue
                .arm_launch_window(
                    "org.kde.kcalc",
                    WindowPlacementAnchor::TopRight,
                    Some("DP-1"),
                    Some(400),
                    Some(300),
                    20,
                    WindowActivationMode::PreserveFocus,
                    10_000,
                )
                .await
        });

        tokio::task::yield_now().await;
        let payload = queue.take().expect("launch intent is queued");
        let action: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        assert_eq!(action["action"], "launch_window");
        assert_eq!(action["desktop_entry"], "org.kde.kcalc");
        assert_eq!(action["anchor"], "top_right");
        assert_eq!(action["activation"], "preserve_focus");
        let id = action["id"].as_str().expect("action id");
        queue
            .acknowledge(id)
            .expect("intent acknowledgement succeeds");
        let ticket = arm.await.expect("arm task joins").expect("intent arms");

        let finish_queue = queue.clone();
        let finish = tokio::spawn(async move {
            finish_queue
                .finish_launch_window(ticket, Duration::from_secs(1))
                .await
        });
        queue
            .complete(
                &serde_json::json!({
                    "id": id,
                    "ok": true,
                    "geometry": {"x": 1500, "y": 20, "width": 400, "height": 300},
                    "window_id": "window-2",
                    "app_id": "org.kde.kcalc",
                    "title": "KCalc",
                    "pid": 84,
                    "focus_preserved": true
                })
                .to_string(),
            )
            .expect("launch completion is accepted");
        let outcome = finish
            .await
            .expect("finish task joins")
            .expect("launch confirms");
        assert_eq!(outcome.window.id, "window-2");
        assert_eq!(outcome.window.geometry.expect("geometry").x, 1500);
        assert!(outcome.focus_preserved);
    }
}
