use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Error, Result, bail};
use clap::Parser;
use image::{GenericImageView, imageops::FilterType};
use libplasma_pilot::{
    AccessibilityFindRequest, AccessibilityInvokeRequest, AccessibilitySetTextRequest,
    ActionResult, ActivateTabRequest, BackendCapability, CapabilitySet, ClickButtonRequest,
    ClipboardGetRequest, ClipboardText, CoordinateSpace, DaemonRequest, DaemonResponse,
    DesktopObservation, FocusWindowRequest, FocusedAccessibilityTreeRequest, HealthStatus,
    JournalEntry, ObserveRequest, PolicyStatus, SafetyClass, ScreenshotInfo, ScreenshotRequest,
    ScreenshotTileRequest, ScreenshotTransform, SelectMenuRequest, SetTextFieldRequest,
    ToolApprovalLevel, WindowGeometry, WindowInfo, current_euid, default_journal_path,
    default_socket_path,
};
use plasma_pilot_policy::{PolicyConfig, PolicyEngine};
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};
use tracing::{error, info, warn};
use uuid::Uuid;

static SCREENSHOT_CAPTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone)]
struct ActionJournal {
    path: PathBuf,
    sequence: Arc<Mutex<u64>>,
}

impl ActionJournal {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            sequence: Arc::new(Mutex::new(0)),
        }
    }

    fn record(&self, method: &str, response: &DaemonResponse) -> Result<()> {
        let entry = JournalEntry {
            sequence: self.next_sequence()?,
            unix_time_ms: unix_time_ms()?,
            method: method.to_string(),
            ok: !matches!(response, DaemonResponse::Error { .. }),
            summary: summarize_response(response),
        };
        append_journal_entry(&self.path, &entry)
    }

    fn tail(&self, limit: usize) -> Result<Vec<JournalEntry>> {
        tail_journal_entries(&self.path, limit)
    }

    fn next_sequence(&self) -> Result<u64> {
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| anyhow::anyhow!("journal sequence lock is poisoned"))?;
        *sequence += 1;
        Ok(*sequence)
    }
}

const KWIN_BRIDGE_SERVICE: &str = "org.plasmapilot.KWinBridge";
const KWIN_BRIDGE_PATH: &str = "/org/plasmapilot/KWinBridge1";
const KWIN_BRIDGE_INTERFACE: &str = "org.plasmapilot.KWinBridge1";

#[derive(Debug, Clone, Default)]
struct ActiveWindowState {
    inner: Arc<Mutex<ActiveWindowSnapshot>>,
}

impl ActiveWindowState {
    fn update_from_payload(&self, payload: &str) -> Result<()> {
        let payload = serde_json::from_str::<KwinActiveWindowPayload>(payload)
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

    fn snapshot(&self) -> Result<Option<Option<WindowInfo>>> {
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

#[derive(Debug, Clone)]
struct KwinBridge {
    active_window_state: ActiveWindowState,
}

#[zbus::interface(name = "org.plasmapilot.KWinBridge1")]
impl KwinBridge {
    async fn update_active_window(&self, payload: &str) -> zbus::fdo::Result<()> {
        self.active_window_state
            .update_from_payload(payload)
            .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct KwinActiveWindowPayload {
    active: bool,
    id: Option<String>,
    title: Option<String>,
    app_id: Option<String>,
    pid: Option<u32>,
    geometry: Option<KwinActiveWindowGeometry>,
}

impl KwinActiveWindowPayload {
    fn into_window(self) -> Result<Option<WindowInfo>> {
        if !self.active {
            return Ok(None);
        }
        let id = self
            .id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("KWin active-window payload missing id"))?;
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
struct KwinActiveWindowGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl From<KwinActiveWindowGeometry> for WindowGeometry {
    fn from(geometry: KwinActiveWindowGeometry) -> Self {
        Self {
            x: geometry.x,
            y: geometry.y,
            width: geometry.width.max(1),
            height: geometry.height.max(1),
            space: CoordinateSpace::LogicalPixel,
        }
    }
}

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot local desktop-control daemon")]
struct Args {
    #[arg(long, env = "PLASMA_PILOT_SOCKET")]
    socket: Option<PathBuf>,

    #[arg(long, env = "PLASMA_PILOT_JOURNAL")]
    journal: Option<PathBuf>,

    #[arg(long, env = "PLASMA_PILOT_ALLOW_CONTROL")]
    allow_control: bool,

    #[arg(long, env = "PLASMA_PILOT_ALLOW_CLIPBOARD_READ")]
    allow_clipboard_read: bool,

    #[arg(long)]
    print_capabilities: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    if args.print_capabilities {
        println!("{}", serde_json::to_string_pretty(&capabilities())?);
        return Ok(());
    }

    let socket = match args.socket {
        Some(path) => path,
        None => default_socket_path().context("resolve default socket path")?,
    };
    let journal = match args.journal {
        Some(path) => path,
        None => default_journal_path().context("resolve default journal path")?,
    };

    let policy_config = policy_config(args.allow_control, args.allow_clipboard_read);

    run(socket, journal, policy_config).await
}

async fn run(socket: PathBuf, journal_path: PathBuf, policy_config: PolicyConfig) -> Result<()> {
    let journal = ActionJournal::new(journal_path);
    let policy = PolicyEngine::new(policy_config);
    let active_window_state = ActiveWindowState::default();
    let _kwin_bridge_connection = match start_kwin_bridge(active_window_state.clone()).await {
        Ok(connection) => Some(connection),
        Err(err) => {
            warn!(error = %err, "KWin bridge DBus service is unavailable");
            None
        }
    };

    prepare_socket_path(&socket)?;
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("bind daemon socket at {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("set socket permissions on {}", socket.display()))?;
    validate_socket_permissions(&socket)?;

    info!(socket = %socket.display(), "plasma-pilotd listening");

    loop {
        let (stream, _addr) = listener.accept().await.context("accept client")?;
        let active_window_state = active_window_state.clone();
        let journal = journal.clone();
        let policy = policy.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_client(stream, active_window_state, journal, policy).await {
                warn!(error = %err, "client request failed");
            }
        });
    }
}

async fn start_kwin_bridge(active_window_state: ActiveWindowState) -> Result<zbus::Connection> {
    let connection = zbus::connection::Builder::session()
        .context("connect to session bus for KWin bridge")?
        .name(KWIN_BRIDGE_SERVICE)
        .context("request KWin bridge DBus service name")?
        .serve_at(
            KWIN_BRIDGE_PATH,
            KwinBridge {
                active_window_state,
            },
        )
        .context("serve KWin bridge DBus object")?
        .build()
        .await
        .context("build KWin bridge DBus connection")?;
    info!(
        service = KWIN_BRIDGE_SERVICE,
        path = KWIN_BRIDGE_PATH,
        interface = KWIN_BRIDGE_INTERFACE,
        "KWin bridge DBus service registered"
    );
    Ok(connection)
}

async fn handle_client(
    stream: UnixStream,
    active_window_state: ActiveWindowState,
    journal: ActionJournal,
    policy: PolicyEngine,
) -> Result<()> {
    validate_peer_uid(&stream)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .await
        .context("read request line")?;
    if bytes == 0 {
        bail!("empty request");
    }

    let request = serde_json::from_str::<DaemonRequest>(&line).context("parse daemon request")?;
    let method = request.method_name();
    let response = handle_request(request, &active_window_state, &journal, &policy);
    journal
        .record(method, &response)
        .context("record request in action journal")?;
    let mut stream = reader.into_inner();
    let response_line = serde_json::to_string(&response).context("serialize daemon response")?;
    stream
        .write_all(response_line.as_bytes())
        .await
        .context("write response")?;
    stream.write_all(b"\n").await.context("write newline")?;
    Ok(())
}

fn handle_request(
    request: DaemonRequest,
    active_window_state: &ActiveWindowState,
    journal: &ActionJournal,
    policy: &PolicyEngine,
) -> DaemonResponse {
    if let Err(err) = enforce_policy(policy, &request) {
        return DaemonResponse::Error {
            message: format_error_chain(&err),
        };
    }

    match request {
        DaemonRequest::Health => DaemonResponse::Health(health()),
        DaemonRequest::Capabilities => DaemonResponse::Capabilities(capabilities()),
        DaemonRequest::PolicyStatus => {
            DaemonResponse::PolicyStatus(policy_status_from_config(policy.config()))
        }
        DaemonRequest::ListMonitors => match list_monitors() {
            Ok(monitors) => DaemonResponse::Monitors(monitors),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ListWindows => match list_windows() {
            Ok(windows) => DaemonResponse::Windows(windows),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ActiveWindow => match active_window(active_window_state) {
            Ok(window) => DaemonResponse::ActiveWindow(window),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::Observe(request) => match observe_desktop(request, active_window_state) {
            Ok(observation) => DaemonResponse::Observation(Box::new(observation)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::Screenshot(request) => match capture_screenshot(request) {
            Ok(info) => DaemonResponse::Screenshot(info),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ScreenshotTile(request) => match capture_screenshot_tile(request) {
            Ok(info) => DaemonResponse::Screenshot(info),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ClipboardGet(request) => match clipboard_get_text(request) {
            Ok(text) => DaemonResponse::ClipboardText(text),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ClipboardSet(request) => match clipboard_set_text(&request.text) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::FocusedAccessibilityTree(request) => {
            match focused_accessibility_tree(request) {
                Ok(tree) => DaemonResponse::AccessibilityTree(tree),
                Err(err) => DaemonResponse::Error {
                    message: format_error_chain(&err),
                },
            }
        }
        DaemonRequest::AccessibilityFind(request) => match accessibility_find(request) {
            Ok(matches) => DaemonResponse::AccessibilityMatches(matches),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::AccessibilityInvoke(request) => match accessibility_invoke(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::AccessibilitySetText(request) => match accessibility_set_text(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ClickButton(request) => match click_button(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::SetTextField(request) => match set_text_field(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ActivateTab(request) => match activate_tab(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::SelectMenu(request) => match select_menu(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::JournalTail(request) => match journal.tail(request.limit) {
            Ok(entries) => DaemonResponse::Journal(entries),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::FocusWindow(request) => match focus_window(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
    }
}

fn health() -> HealthStatus {
    HealthStatus {
        service: "plasma-pilotd".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        status: "ok".to_string(),
    }
}

fn capabilities() -> CapabilitySet {
    CapabilitySet {
        capabilities: current_capabilities(),
    }
}

fn policy_status_from_config(config: &PolicyConfig) -> PolicyStatus {
    PolicyStatus {
        default_observe: config.default_observe.clone(),
        default_control: config.default_control.clone(),
        default_clipboard_read: config.default_clipboard_read.clone(),
        default_clipboard_write: config.default_clipboard_write.clone(),
    }
}

fn policy_config(allow_control: bool, allow_clipboard_read: bool) -> PolicyConfig {
    let mut config = PolicyConfig::default();
    if allow_control {
        config.default_control = ToolApprovalLevel::Allow;
    }
    if allow_clipboard_read {
        config.default_clipboard_read = ToolApprovalLevel::Allow;
    }
    config
}

fn enforce_policy(policy: &PolicyEngine, request: &DaemonRequest) -> Result<()> {
    let safety_class = safety_class_for_request(request);
    let decision = policy.decide(&safety_class);
    match decision.level {
        ToolApprovalLevel::Allow => Ok(()),
        ToolApprovalLevel::Prompt => bail!(
            "policy prompt required for {safety_class:?}, but no approval channel is available"
        ),
        ToolApprovalLevel::Deny => bail!("policy denied {safety_class:?}: {}", decision.reason),
    }
}

fn safety_class_for_request(request: &DaemonRequest) -> SafetyClass {
    match request {
        DaemonRequest::Health
        | DaemonRequest::Capabilities
        | DaemonRequest::PolicyStatus
        | DaemonRequest::JournalTail(_) => SafetyClass::Policy,
        DaemonRequest::ListMonitors
        | DaemonRequest::ListWindows
        | DaemonRequest::ActiveWindow
        | DaemonRequest::Observe(_)
        | DaemonRequest::Screenshot(_)
        | DaemonRequest::ScreenshotTile(_)
        | DaemonRequest::FocusedAccessibilityTree(_)
        | DaemonRequest::AccessibilityFind(_) => SafetyClass::Observe,
        DaemonRequest::ClipboardGet(_) => SafetyClass::ClipboardRead,
        DaemonRequest::ClipboardSet(_) => SafetyClass::ClipboardWrite,
        DaemonRequest::FocusWindow(_)
        | DaemonRequest::AccessibilityInvoke(_)
        | DaemonRequest::AccessibilitySetText(_)
        | DaemonRequest::ClickButton(_)
        | DaemonRequest::SetTextField(_)
        | DaemonRequest::ActivateTab(_)
        | DaemonRequest::SelectMenu(_) => SafetyClass::ControlSemantic,
    }
}

fn current_capabilities() -> Vec<BackendCapability> {
    let mut capabilities = vec![
        BackendCapability::DaemonHealth,
        BackendCapability::DaemonPolicyStatus,
    ];
    if command_exists("spectacle") {
        capabilities.push(BackendCapability::Screenshot);
    }
    if command_exists("qdbus6") {
        capabilities.push(BackendCapability::MonitorMetadata);
        capabilities.push(BackendCapability::WindowList);
        capabilities.push(BackendCapability::WindowFocus);
    }
    if command_exists("wl-copy") && command_exists("wl-paste") {
        capabilities.push(BackendCapability::ClipboardText);
    }
    if command_exists("busctl") && plasma_pilot_atspi::available() {
        capabilities.push(BackendCapability::AccessibilityTree);
        capabilities.push(BackendCapability::SemanticActions);
    }
    capabilities
}

fn command_exists(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(command);
        candidate.is_file()
    })
}

fn capture_screenshot(request: ScreenshotRequest) -> Result<ScreenshotInfo> {
    let _guard = SCREENSHOT_CAPTURE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("screenshot capture lock is poisoned"))?;
    if !request.full_resolution && request.max_edge == Some(0) {
        bail!("max_edge must be greater than zero");
    }
    prepare_screenshot_output(&request.output)?;
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
            request.max_edge.unwrap_or(1600),
        )?
    };

    if capture_output != request.output {
        fs::remove_file(&capture_output).ok();
    }
    let monitors = list_monitors().unwrap_or_default();

    Ok(ScreenshotInfo {
        path: request.output,
        backend: "spectacle".to_string(),
        source_width,
        source_height,
        output_width,
        output_height,
        transform: ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: f64::from(output_width) / f64::from(source_width),
            scale_y: f64::from(output_height) / f64::from(source_height),
        },
        coordinate_space: CoordinateSpace::PhysicalPixel,
        monitors,
    })
}

fn capture_screenshot_tile(request: ScreenshotTileRequest) -> Result<ScreenshotInfo> {
    let _guard = SCREENSHOT_CAPTURE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("screenshot capture lock is poisoned"))?;
    validate_tile_request(&request)?;
    prepare_screenshot_output(&request.output)?;
    if !command_exists("spectacle") {
        bail!("spectacle command is not available for KDE screenshot capture");
    }

    let capture_output = temporary_capture_path(&request.output);
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
    validate_tile_bounds(&request, source_width, source_height)?;
    let (output_width, output_height) =
        write_tile_preview(&capture_output, &request, request.max_edge.unwrap_or(1600))?;

    fs::remove_file(&capture_output).ok();
    let monitors = list_monitors().unwrap_or_default();

    Ok(ScreenshotInfo {
        path: request.output,
        backend: "spectacle".to_string(),
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
    })
}

fn list_monitors() -> Result<Vec<libplasma_pilot::MonitorInfo>> {
    plasma_pilot_kwin::list_monitors().map_err(|err| anyhow::anyhow!(err))
}

fn list_windows() -> Result<Vec<libplasma_pilot::WindowInfo>> {
    plasma_pilot_kwin::list_windows().map_err(|err| anyhow::anyhow!(err))
}

fn active_window(active_window_state: &ActiveWindowState) -> Result<Option<WindowInfo>> {
    if let Some(window) = active_window_state.snapshot()? {
        return Ok(window);
    }
    plasma_pilot_kwin::active_window().map_err(|err| anyhow::anyhow!(err))
}

fn observe_desktop(
    request: ObserveRequest,
    active_window_state: &ActiveWindowState,
) -> Result<DesktopObservation> {
    let monitors = list_monitors().unwrap_or_default();
    let windows = list_windows().unwrap_or_default();
    let active_window = active_window(active_window_state).unwrap_or_default();
    let screenshot = match request.screenshot {
        Some(request) => Some(capture_screenshot(request)?),
        None => None,
    };

    Ok(DesktopObservation {
        active_window,
        windows,
        monitors,
        screenshot,
    })
}

fn focus_window(request: FocusWindowRequest) -> Result<ActionResult> {
    if request.window_id.trim().is_empty() {
        bail!("window id must not be empty");
    }
    plasma_pilot_kwin::focus_window(&request.window_id).map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!("focused window {}", request.window_id)),
    })
}

fn clipboard_get_text(request: ClipboardGetRequest) -> Result<ClipboardText> {
    if request.max_bytes == Some(0) {
        bail!("clipboard max_bytes must be greater than zero");
    }
    if !command_exists("wl-paste") {
        bail!("wl-paste command is not available for Wayland clipboard reads");
    }

    let output = Command::new("wl-paste")
        .arg("--no-newline")
        .output()
        .context("run wl-paste clipboard backend")?;
    if !output.status.success() {
        bail!(
            "wl-paste clipboard backend exited with status {}",
            output.status
        );
    }

    let text = String::from_utf8(output.stdout).context("clipboard text is not valid UTF-8")?;
    Ok(bound_clipboard_text(text, request.max_bytes))
}

fn bound_clipboard_text(mut text: String, max_bytes: Option<usize>) -> ClipboardText {
    let original_bytes = text.len();
    let Some(max_bytes) = max_bytes else {
        return ClipboardText {
            text,
            truncated: false,
            original_bytes,
        };
    };
    if original_bytes <= max_bytes {
        return ClipboardText {
            text,
            truncated: false,
            original_bytes,
        };
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    ClipboardText {
        text,
        truncated: true,
        original_bytes,
    }
}

fn clipboard_set_text(text: &str) -> Result<ActionResult> {
    if !command_exists("wl-copy") {
        bail!("wl-copy command is not available for Wayland clipboard writes");
    }

    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg("text/plain;charset=utf-8")
        .stdin(Stdio::piped())
        .spawn()
        .context("start wl-copy clipboard backend")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("wl-copy stdin is unavailable"))?;
        stdin
            .write_all(text.as_bytes())
            .context("write text to wl-copy")?;
    }
    let status = child.wait().context("wait for wl-copy clipboard backend")?;
    if !status.success() {
        bail!("wl-copy clipboard backend exited with status {status}");
    }

    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!("set clipboard text length={}", text.len())),
    })
}

fn focused_accessibility_tree(
    request: FocusedAccessibilityTreeRequest,
) -> Result<Option<libplasma_pilot::AccessibilityNode>> {
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }
    plasma_pilot_atspi::focused_tree(request.depth, request.max_nodes)
        .map_err(|err| anyhow::anyhow!(err))
}

fn accessibility_find(
    request: AccessibilityFindRequest,
) -> Result<Vec<libplasma_pilot::AccessibilityNode>> {
    plasma_pilot_atspi::find(request).map_err(|err| anyhow::anyhow!(err))
}

fn accessibility_invoke(request: AccessibilityInvokeRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    plasma_pilot_atspi::invoke(&request.node_id, request.action.clone())
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "invoked accessibility action={} node={}",
            request.action.as_str(),
            request.node_id
        )),
    })
}

fn accessibility_set_text(request: AccessibilitySetTextRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    plasma_pilot_atspi::set_text(&request.node_id, &request.text)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "set accessibility text length={} node={}",
            request.text.chars().count(),
            request.node_id
        )),
    })
}

fn click_button(request: ClickButtonRequest) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("button name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find(AccessibilityFindRequest {
        role: Some("button".to_string()),
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 5,
        max_nodes: request.max_nodes,
    })?;
    let target = resolve_click_button_match(name, matches)?;
    plasma_pilot_atspi::invoke(&target.id, libplasma_pilot::AccessibilityAction::Press)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "clicked button name={} node={}",
            target.name.as_deref().unwrap_or(name),
            target.id
        )),
    })
}

fn set_text_field(request: SetTextFieldRequest) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("text field name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let target = resolve_text_field_match(name, matches)?;
    plasma_pilot_atspi::set_text(&target.id, &request.text).map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "set text field name={} length={} node={}",
            target.name.as_deref().unwrap_or(name),
            request.text.chars().count(),
            target.id
        )),
    })
}

fn activate_tab(request: ActivateTabRequest) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("tab name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let (target, action) = resolve_tab_match(name, matches)?;
    plasma_pilot_atspi::invoke(&target.id, action.clone()).map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "activated tab name={} action={} node={}",
            target.name.as_deref().unwrap_or(name),
            action.as_str(),
            target.id
        )),
    })
}

fn select_menu(request: SelectMenuRequest) -> Result<ActionResult> {
    let path = normalize_semantic_path(&request.path);
    if path.is_empty() {
        bail!("menu path must contain at least one non-empty segment");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }
    let first = path[0].clone();
    let search_depth = path.len().saturating_add(2);
    let matches = accessibility_find(AccessibilityFindRequest {
        role: None,
        name_contains: Some(first),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: search_depth,
        max_results: 20,
        max_nodes: request.max_nodes,
    })?;
    let (target, action) = resolve_menu_path_match(&path, matches)?;
    plasma_pilot_atspi::invoke(&target.id, action.clone()).map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "selected menu path={} action={} node={}",
            path.join("/"),
            action.as_str(),
            target.id
        )),
    })
}

fn resolve_click_button_match(
    name: &str,
    matches: Vec<libplasma_pilot::AccessibilityNode>,
) -> Result<libplasma_pilot::AccessibilityNode> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(|node| {
            node.actions
                .contains(&libplasma_pilot::AccessibilityAction::Press)
        })
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive pressable button matched name={name}");
    }

    let exact = viable
        .iter()
        .filter(|node| {
            node.name
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        viable = exact;
    }

    if viable.len() == 1 {
        return Ok(viable.remove(0));
    }

    let choices = viable
        .iter()
        .take(5)
        .map(|node| {
            format!(
                "{}:{}",
                node.id,
                node.name.as_deref().unwrap_or("<unnamed>")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "ambiguous button match for name={name}: {} candidates: {choices}",
        viable.len()
    );
}

fn resolve_menu_path_match(
    path: &[String],
    matches: Vec<libplasma_pilot::AccessibilityNode>,
) -> Result<(
    libplasma_pilot::AccessibilityNode,
    libplasma_pilot::AccessibilityAction,
)> {
    if path.is_empty() {
        bail!("menu path must contain at least one segment");
    }
    let mut candidates = Vec::new();
    for node in &matches {
        collect_menu_path_candidates(node, path, 0, &mut candidates);
    }
    candidates.retain(|(node, _)| !node.sensitive);

    if candidates.is_empty() {
        bail!(
            "no visible non-sensitive menu item matched path={}",
            path.join("/")
        );
    }
    if candidates.len() == 1 {
        return Ok(candidates.remove(0));
    }

    let choices = candidates
        .iter()
        .take(5)
        .map(|(node, _)| {
            format!(
                "{}:{}",
                node.id,
                node.name.as_deref().unwrap_or("<unnamed>")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "ambiguous menu path={} matched {} candidates: {choices}",
        path.join("/"),
        candidates.len()
    );
}

fn resolve_tab_match(
    name: &str,
    matches: Vec<libplasma_pilot::AccessibilityNode>,
) -> Result<(
    libplasma_pilot::AccessibilityNode,
    libplasma_pilot::AccessibilityAction,
)> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(is_tab_candidate)
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive activatable tab matched name={name}");
    }

    let exact = viable
        .iter()
        .filter(|node| {
            node.name
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        viable = exact;
    }

    if viable.len() == 1 {
        let node = viable.remove(0);
        let action = tab_activation_action(&node)
            .ok_or_else(|| anyhow::anyhow!("tab has no select or press action"))?;
        return Ok((node, action));
    }

    let choices = viable
        .iter()
        .take(5)
        .map(|node| {
            format!(
                "{}:{}",
                node.id,
                node.name.as_deref().unwrap_or("<unnamed>")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "ambiguous tab match for name={name}: {} candidates: {choices}",
        viable.len()
    );
}

fn collect_menu_path_candidates(
    node: &libplasma_pilot::AccessibilityNode,
    path: &[String],
    index: usize,
    candidates: &mut Vec<(
        libplasma_pilot::AccessibilityNode,
        libplasma_pilot::AccessibilityAction,
    )>,
) {
    if node_name_matches(node, &path[index]) {
        if index + 1 == path.len() {
            if let Some(action) = menu_activation_action(node) {
                candidates.push((node.clone(), action));
            }
        } else {
            for child in &node.children {
                collect_menu_path_candidates(child, path, index + 1, candidates);
            }
        }
    }

    for child in &node.children {
        collect_menu_path_candidates(child, path, index, candidates);
    }
}

fn normalize_semantic_path(path: &[String]) -> Vec<String> {
    path.iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn node_name_matches(node: &libplasma_pilot::AccessibilityNode, name: &str) -> bool {
    node.name
        .as_deref()
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
}

fn menu_activation_action(
    node: &libplasma_pilot::AccessibilityNode,
) -> Option<libplasma_pilot::AccessibilityAction> {
    if !is_menu_item_candidate(node) {
        return None;
    }
    if node
        .actions
        .contains(&libplasma_pilot::AccessibilityAction::Select)
    {
        Some(libplasma_pilot::AccessibilityAction::Select)
    } else if node
        .actions
        .contains(&libplasma_pilot::AccessibilityAction::Press)
    {
        Some(libplasma_pilot::AccessibilityAction::Press)
    } else {
        None
    }
}

fn resolve_text_field_match(
    name: &str,
    matches: Vec<libplasma_pilot::AccessibilityNode>,
) -> Result<libplasma_pilot::AccessibilityNode> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(is_text_field_candidate)
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive text field matched name={name}");
    }

    let exact = viable
        .iter()
        .filter(|node| {
            node.name
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        viable = exact;
    }

    if viable.len() == 1 {
        return Ok(viable.remove(0));
    }

    let choices = viable
        .iter()
        .take(5)
        .map(|node| {
            format!(
                "{}:{}",
                node.id,
                node.name.as_deref().unwrap_or("<unnamed>")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "ambiguous text field match for name={name}: {} candidates: {choices}",
        viable.len()
    );
}

fn is_menu_item_candidate(node: &libplasma_pilot::AccessibilityNode) -> bool {
    matches!(
        node.role.to_ascii_lowercase().as_str(),
        "menu item" | "check menu item" | "radio menu item"
    )
}

fn is_tab_candidate(node: &libplasma_pilot::AccessibilityNode) -> bool {
    let role = node.role.to_ascii_lowercase();
    matches!(
        role.as_str(),
        "page tab" | "tab" | "tab item" | "page tab list item"
    ) && tab_activation_action(node).is_some()
}

fn tab_activation_action(
    node: &libplasma_pilot::AccessibilityNode,
) -> Option<libplasma_pilot::AccessibilityAction> {
    if node
        .actions
        .contains(&libplasma_pilot::AccessibilityAction::Select)
    {
        Some(libplasma_pilot::AccessibilityAction::Select)
    } else if node
        .actions
        .contains(&libplasma_pilot::AccessibilityAction::Press)
    {
        Some(libplasma_pilot::AccessibilityAction::Press)
    } else {
        None
    }
}

fn is_text_field_candidate(node: &libplasma_pilot::AccessibilityNode) -> bool {
    let role = node.role.to_ascii_lowercase();
    role == "text"
        || role == "entry"
        || role == "text input"
        || role == "editable text"
        || node
            .actions
            .contains(&libplasma_pilot::AccessibilityAction::SetText)
}

fn temporary_capture_path(output: &Path) -> PathBuf {
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("screenshot.png");
    let temp_name = format!(".plasma-pilot-full-{}-{file_name}", std::process::id());
    output.with_file_name(temp_name)
}

fn write_preview_or_copy(
    source: &Path,
    output: &Path,
    source_width: u32,
    source_height: u32,
    max_edge: u32,
) -> Result<(u32, u32)> {
    if max_edge == 0 {
        bail!("max_edge must be greater than zero");
    }

    let largest_edge = source_width.max(source_height);
    if largest_edge <= max_edge {
        fs::copy(source, output)
            .with_context(|| format!("copy screenshot preview to {}", output.display()))?;
        return Ok((source_width, source_height));
    }

    let scale = f64::from(max_edge) / f64::from(largest_edge);
    let output_width = scaled_dimension(source_width, scale);
    let output_height = scaled_dimension(source_height, scale);
    let image =
        image::open(source).with_context(|| format!("open screenshot {}", source.display()))?;
    let resized = image.resize(output_width, output_height, FilterType::Lanczos3);
    resized
        .save(output)
        .with_context(|| format!("write screenshot preview {}", output.display()))?;
    Ok((output_width, output_height))
}

fn write_tile_preview(
    source: &Path,
    request: &ScreenshotTileRequest,
    max_edge: u32,
) -> Result<(u32, u32)> {
    if max_edge == 0 {
        bail!("max_edge must be greater than zero");
    }

    let image =
        image::open(source).with_context(|| format!("open screenshot {}", source.display()))?;
    let cropped = image.crop_imm(request.x, request.y, request.width, request.height);
    let largest_edge = request.width.max(request.height);
    let output_image = if largest_edge > max_edge {
        let scale = f64::from(max_edge) / f64::from(largest_edge);
        let output_width = scaled_dimension(request.width, scale);
        let output_height = scaled_dimension(request.height, scale);
        cropped.resize(output_width, output_height, FilterType::Lanczos3)
    } else {
        cropped
    };

    let (output_width, output_height) = output_image.dimensions();
    output_image
        .save(&request.output)
        .with_context(|| format!("write screenshot tile {}", request.output.display()))?;
    Ok((output_width, output_height))
}

fn scaled_dimension(value: u32, scale: f64) -> u32 {
    (f64::from(value) * scale).round().max(1.0) as u32
}

fn validate_tile_request(request: &ScreenshotTileRequest) -> Result<()> {
    if request.width == 0 || request.height == 0 {
        bail!("tile width and height must be greater than zero");
    }
    if request.max_edge == Some(0) {
        bail!("max_edge must be greater than zero");
    }
    Ok(())
}

fn validate_tile_bounds(
    request: &ScreenshotTileRequest,
    source_width: u32,
    source_height: u32,
) -> Result<()> {
    let Some(end_x) = request.x.checked_add(request.width) else {
        bail!("tile x + width overflows u32");
    };
    let Some(end_y) = request.y.checked_add(request.height) else {
        bail!("tile y + height overflows u32");
    };

    if end_x > source_width || end_y > source_height {
        bail!(
            "tile {}x{} at {},{} is outside source screenshot {}x{}",
            request.width,
            request.height,
            request.x,
            request.y,
            source_width,
            source_height
        );
    }

    Ok(())
}

fn format_error_chain(err: &Error) -> String {
    err.chain()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

fn append_journal_entry(path: &Path, entry: &JournalEntry) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("journal path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create journal dir {}", parent.display()))?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("set journal dir permissions {}", parent.display()))?;
    validate_dir_permissions(parent)?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open journal {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("set journal permissions {}", path.display()))?;
    serde_json::to_writer(&mut file, entry).context("serialize journal entry")?;
    file.write_all(b"\n").context("write journal newline")?;
    file.flush().context("flush journal")?;
    Ok(())
}

fn tail_journal_entries(path: &Path, limit: usize) -> Result<Vec<JournalEntry>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read journal {}", path.display())),
    };

    let mut entries = Vec::new();
    for line in content.lines().rev().filter(|line| !line.trim().is_empty()) {
        if entries.len() >= limit {
            break;
        }
        let entry = serde_json::from_str::<JournalEntry>(line)
            .with_context(|| format!("parse journal line in {}", path.display()))?;
        entries.push(entry);
    }
    entries.reverse();
    Ok(entries)
}

fn summarize_response(response: &DaemonResponse) -> String {
    match response {
        DaemonResponse::Health(status) => format!("{} {}", status.service, status.status),
        DaemonResponse::Capabilities(capabilities) => {
            format!("{} capabilities", capabilities.capabilities.len())
        }
        DaemonResponse::PolicyStatus(_) => "policy status".to_string(),
        DaemonResponse::Monitors(monitors) => format!("{} monitors", monitors.len()),
        DaemonResponse::Windows(windows) => format!("{} windows", windows.len()),
        DaemonResponse::Observation(observation) => format!(
            "observe {} monitors {} windows active={} screenshot={}",
            observation.monitors.len(),
            observation.windows.len(),
            observation.active_window.is_some(),
            observation.screenshot.is_some()
        ),
        DaemonResponse::ActiveWindow(Some(window)) => {
            format!(
                "active window app={}",
                window.app_id.as_deref().unwrap_or("")
            )
        }
        DaemonResponse::ActiveWindow(None) => "no active window".to_string(),
        DaemonResponse::Screenshot(info) => format!(
            "screenshot {}x{} from {}x{} path={}",
            info.output_width,
            info.output_height,
            info.source_width,
            info.source_height,
            info.path.display()
        ),
        DaemonResponse::ClipboardText(text) => format!(
            "clipboard text length={} truncated={} original_bytes={}",
            text.text.len(),
            text.truncated,
            text.original_bytes
        ),
        DaemonResponse::AccessibilityTree(Some(node)) => format!(
            "accessibility focused role={} name={} children={}",
            node.role,
            node.name.as_deref().unwrap_or(""),
            node.children.len()
        ),
        DaemonResponse::AccessibilityTree(None) => "no focused accessibility node".to_string(),
        DaemonResponse::AccessibilityMatches(matches) => {
            format!("{} accessibility matches", matches.len())
        }
        DaemonResponse::Journal(entries) => format!("{} journal entries", entries.len()),
        DaemonResponse::Action(result) => result
            .message
            .clone()
            .unwrap_or_else(|| format!("action {}", result.id)),
        DaemonResponse::Error { message } => format!("error: {message}"),
    }
}

fn unix_time_ms() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before Unix epoch")?;
    Ok(duration.as_millis().try_into().unwrap_or(u64::MAX))
}

fn prepare_screenshot_output(output: &Path) -> Result<()> {
    if output.extension().and_then(|ext| ext.to_str()) != Some("png") {
        bail!(
            "screenshot output must be a .png path: {}",
            output.display()
        );
    }

    if let Ok(metadata) = fs::symlink_metadata(output) {
        if metadata.file_type().is_symlink() {
            bail!(
                "refusing to write screenshot through symlink {}",
                output.display()
            );
        }
        if metadata.is_dir() {
            bail!("screenshot output is a directory: {}", output.display());
        }
    }

    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("screenshot output has no parent: {}", output.display()))?;
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create screenshot output dir {}", parent.display()))?;
    }
    Ok(())
}

fn read_png_dimensions(path: &Path) -> Result<(u32, u32)> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        bail!("screenshot is not a valid PNG: {}", path.display());
    }

    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((width, height))
}

fn read_png_dimensions_with_retry(path: &Path) -> Result<(u32, u32)> {
    let mut last_error = None;
    for _ in 0..10 {
        match read_png_dimensions(path) {
            Ok(dimensions) => return Ok(dimensions),
            Err(err) => {
                last_error = Some(err);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    match last_error {
        Some(err) => Err(err),
        None => bail!("could not read screenshot dimensions"),
    }
}

fn prepare_socket_path(socket: &Path) -> Result<()> {
    let dir = socket
        .parent()
        .ok_or_else(|| anyhow::anyhow!("socket path has no parent: {}", socket.display()))?;
    fs::create_dir_all(dir).with_context(|| format!("create socket dir {}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("set socket dir permissions {}", dir.display()))?;
    validate_dir_permissions(dir)?;

    match fs::symlink_metadata(socket) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(socket)
                .with_context(|| format!("remove stale socket {}", socket.display()))?;
        }
        Ok(_) => bail!("refusing to replace non-socket path {}", socket.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("stat {}", socket.display())),
    }

    Ok(())
}

fn validate_dir_permissions(dir: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(dir).with_context(|| format!("stat {}", dir.display()))?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "refusing unsafe socket directory permissions {mode:o} on {}",
            dir.display()
        );
    }
    Ok(())
}

fn validate_socket_permissions(socket: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(socket).with_context(|| format!("stat {}", socket.display()))?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "refusing unsafe socket permissions {mode:o} on {}",
            socket.display()
        );
    }
    Ok(())
}

fn validate_peer_uid(stream: &UnixStream) -> Result<()> {
    let peer_uid = stream.peer_cred().context("read peer credentials")?.uid();
    let daemon_uid = current_euid().context("read daemon uid")?;
    if peer_uid != daemon_uid {
        error!(peer_uid, daemon_uid, "rejecting client from different uid");
        bail!("peer uid {peer_uid} does not match daemon uid {daemon_uid}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_window_state_accepts_kwin_payload() {
        let state = ActiveWindowState::default();
        state
            .update_from_payload(
                r#"{
                    "active": true,
                    "id": "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}",
                    "title": "Konsole",
                    "app_id": "org.kde.konsole",
                    "pid": 1234,
                    "geometry": {"x": 10, "y": 20, "width": 800, "height": 600}
                }"#,
            )
            .expect("payload updates active-window state");

        let window = state
            .snapshot()
            .expect("state snapshot succeeds")
            .expect("bridge reported")
            .expect("active window exists");
        assert_eq!(window.id, "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}");
        assert_eq!(window.app_id.as_deref(), Some("org.kde.konsole"));
        assert_eq!(window.pid, Some(1234));
        let geometry = window.geometry.expect("geometry is present");
        assert_eq!(geometry.x, 10);
        assert_eq!(geometry.y, 20);
        assert_eq!(geometry.width, 800);
        assert_eq!(geometry.height, 600);
        assert_eq!(geometry.space, CoordinateSpace::LogicalPixel);
    }

    #[test]
    fn active_window_state_accepts_no_active_window() {
        let state = ActiveWindowState::default();
        assert!(
            state
                .snapshot()
                .expect("initial snapshot succeeds")
                .is_none()
        );

        state
            .update_from_payload(r#"{"active": false}"#)
            .expect("payload updates active-window state");
        assert_eq!(
            state.snapshot().expect("state snapshot succeeds"),
            Some(None)
        );
    }

    #[test]
    fn journal_appends_and_tails_entries() {
        let path = std::env::temp_dir().join(format!(
            "plasma-pilot-journal-test-{}-{}.jsonl",
            std::process::id(),
            unix_time_ms().expect("time is available")
        ));
        let journal = ActionJournal::new(path.clone());

        journal
            .record(
                "health",
                &DaemonResponse::Health(HealthStatus {
                    service: "plasma-pilotd".to_string(),
                    version: "0.1.0".to_string(),
                    status: "ok".to_string(),
                }),
            )
            .expect("health record appends");
        journal
            .record(
                "capabilities",
                &DaemonResponse::Capabilities(CapabilitySet {
                    capabilities: vec![BackendCapability::DaemonHealth],
                }),
            )
            .expect("capabilities record appends");

        let entries = journal.tail(1).expect("journal tail succeeds");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sequence, 2);
        assert_eq!(entries[0].method, "capabilities");
        assert!(entries[0].ok);

        fs::remove_file(&path).ok();
    }

    #[test]
    fn observe_requests_pass_default_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(&policy, &DaemonRequest::ListWindows)
            .expect("observe requests are allowed by default");
    }

    #[test]
    fn prompt_policy_fails_closed_without_approval_channel() {
        let policy = PolicyEngine::new(PolicyConfig {
            default_observe: ToolApprovalLevel::Prompt,
            ..PolicyConfig::default()
        });
        let err = enforce_policy(&policy, &DaemonRequest::ListWindows)
            .expect_err("prompt requires approval channel");
        assert!(err.to_string().contains("no approval channel is available"));
    }

    #[test]
    fn focus_window_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
            }),
        )
        .expect_err("focus requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_invoke_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityInvoke(AccessibilityInvokeRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                action: libplasma_pilot::AccessibilityAction::Press,
            }),
        )
        .expect_err("accessibility invoke requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_set_text_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilitySetText(AccessibilitySetTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                text: "hello".to_string(),
            }),
        )
        .expect_err("accessibility set-text requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn click_button_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::ClickButton(ClickButtonRequest {
                name: "OK".to_string(),
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
            }),
        )
        .expect_err("click button requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn set_text_field_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::SetTextField(SetTextFieldRequest {
                name: "Search".to_string(),
                text: "query".to_string(),
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
            }),
        )
        .expect_err("set text field requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn activate_tab_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::ActivateTab(ActivateTabRequest {
                name: "General".to_string(),
                app: Some("settings".to_string()),
                window_name_contains: Some("preferences".to_string()),
                max_nodes: 256,
            }),
        )
        .expect_err("activate tab requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn select_menu_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::SelectMenu(SelectMenuRequest {
                path: vec!["File".to_string(), "Open".to_string()],
                app: Some("kate".to_string()),
                window_name_contains: Some("editor".to_string()),
                max_nodes: 256,
            }),
        )
        .expect_err("select menu requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn click_button_resolver_prefers_exact_match() {
        let target = resolve_click_button_match(
            "OK",
            vec![button_node("1", "OK-ish"), button_node("2", "OK")],
        )
        .expect("exact match resolves");
        assert_eq!(target.id, "2");
    }

    #[test]
    fn click_button_resolver_refuses_ambiguous_matches() {
        let err = resolve_click_button_match(
            "Open",
            vec![button_node("1", "Open"), button_node("2", "Open")],
        )
        .expect_err("multiple exact matches are ambiguous");
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn click_button_resolver_requires_pressable_non_sensitive_match() {
        let mut sensitive = button_node("1", "Delete");
        sensitive.sensitive = true;
        let err = resolve_click_button_match("Delete", vec![sensitive])
            .expect_err("sensitive buttons are not viable");
        assert!(err.to_string().contains("no non-sensitive"));
    }

    #[test]
    fn text_field_resolver_prefers_exact_match() {
        let target = resolve_text_field_match(
            "Search",
            vec![
                text_node("1", "Search everywhere"),
                text_node("2", "Search"),
            ],
        )
        .expect("exact text field resolves");
        assert_eq!(target.id, "2");
    }

    #[test]
    fn text_field_resolver_refuses_ambiguous_matches() {
        let err = resolve_text_field_match(
            "Search",
            vec![text_node("1", "Search"), text_node("2", "Search")],
        )
        .expect_err("multiple exact text fields are ambiguous");
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn text_field_resolver_requires_non_sensitive_text_field() {
        let mut sensitive = text_node("1", "Password");
        sensitive.sensitive = true;
        let err = resolve_text_field_match("Password", vec![sensitive])
            .expect_err("sensitive text fields are not viable");
        assert!(err.to_string().contains("no non-sensitive"));
    }

    #[test]
    fn tab_resolver_prefers_exact_match_and_select_action() {
        let (target, action) = resolve_tab_match(
            "General",
            vec![tab_node("1", "General settings"), tab_node("2", "General")],
        )
        .expect("exact tab resolves");
        assert_eq!(target.id, "2");
        assert_eq!(action, libplasma_pilot::AccessibilityAction::Select);
    }

    #[test]
    fn tab_resolver_uses_press_when_select_is_unavailable() {
        let (target, action) = resolve_tab_match("General", vec![press_tab_node("1", "General")])
            .expect("pressable tab resolves");
        assert_eq!(target.id, "1");
        assert_eq!(action, libplasma_pilot::AccessibilityAction::Press);
    }

    #[test]
    fn tab_resolver_refuses_ambiguous_matches() {
        let err = resolve_tab_match(
            "General",
            vec![tab_node("1", "General"), tab_node("2", "General")],
        )
        .expect_err("multiple exact tabs are ambiguous");
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn tab_resolver_requires_non_sensitive_tab() {
        let mut sensitive = tab_node("1", "Security");
        sensitive.sensitive = true;
        let err = resolve_tab_match("Security", vec![sensitive])
            .expect_err("sensitive tabs are not viable");
        assert!(err.to_string().contains("no non-sensitive"));
    }

    #[test]
    fn menu_resolver_matches_visible_path_and_select_action() {
        let (target, action) = resolve_menu_path_match(
            &["File".to_string(), "Open".to_string()],
            vec![menu_node(
                "menu",
                "File",
                vec![menu_item_node("open", "Open")],
            )],
        )
        .expect("visible menu path resolves");
        assert_eq!(target.id, "open");
        assert_eq!(action, libplasma_pilot::AccessibilityAction::Select);
    }

    #[test]
    fn menu_resolver_uses_press_when_select_is_unavailable() {
        let (target, action) = resolve_menu_path_match(
            &["File".to_string(), "Open".to_string()],
            vec![menu_node(
                "menu",
                "File",
                vec![press_menu_item_node("open", "Open")],
            )],
        )
        .expect("pressable menu item resolves");
        assert_eq!(target.id, "open");
        assert_eq!(action, libplasma_pilot::AccessibilityAction::Press);
    }

    #[test]
    fn menu_resolver_refuses_ambiguous_visible_paths() {
        let err = resolve_menu_path_match(
            &["File".to_string(), "Open".to_string()],
            vec![
                menu_node("menu1", "File", vec![menu_item_node("open1", "Open")]),
                menu_node("menu2", "File", vec![menu_item_node("open2", "Open")]),
            ],
        )
        .expect_err("duplicate visible menu paths are ambiguous");
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn menu_resolver_refuses_sensitive_menu_item() {
        let mut sensitive = menu_item_node("secret", "Secrets");
        sensitive.sensitive = true;
        let err = resolve_menu_path_match(
            &["File".to_string(), "Secrets".to_string()],
            vec![menu_node("menu", "File", vec![sensitive])],
        )
        .expect_err("sensitive menu items are not viable");
        assert!(err.to_string().contains("no visible non-sensitive"));
    }

    #[test]
    fn allow_control_config_allows_focus_policy() {
        let policy = PolicyEngine::new(policy_config(true, false));
        enforce_policy(
            &policy,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
            }),
        )
        .expect("explicit control override allows focus policy");
        assert_eq!(
            policy_status_from_config(policy.config()).default_control,
            ToolApprovalLevel::Allow
        );
    }

    #[test]
    fn clipboard_read_fails_closed_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: Some(1024),
            }),
        )
        .expect_err("clipboard reads require approval by default");
        assert!(err.to_string().contains("ClipboardRead"));
    }

    #[test]
    fn allow_clipboard_read_config_allows_clipboard_get_policy() {
        let policy = PolicyEngine::new(policy_config(false, true));
        enforce_policy(
            &policy,
            &DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: Some(1024),
            }),
        )
        .expect("explicit clipboard-read override allows clipboard get");
        assert_eq!(
            policy_status_from_config(policy.config()).default_clipboard_read,
            ToolApprovalLevel::Allow
        );
    }

    #[test]
    fn clipboard_text_is_bounded_on_utf8_boundary() {
        let bounded = bound_clipboard_text("abécd".to_string(), Some(4));
        assert_eq!(bounded.text, "abé");
        assert!(bounded.truncated);
        assert_eq!(bounded.original_bytes, 6);
    }

    #[test]
    fn clipboard_text_can_be_unbounded() {
        let bounded = bound_clipboard_text("hello".to_string(), None);
        assert_eq!(bounded.text, "hello");
        assert!(!bounded.truncated);
        assert_eq!(bounded.original_bytes, 5);
    }

    #[test]
    fn clipboard_write_is_allowed_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(
            &policy,
            &DaemonRequest::ClipboardSet(libplasma_pilot::ClipboardSetRequest {
                text: "hello".to_string(),
            }),
        )
        .expect("clipboard writes are allowed by default policy");
    }

    #[test]
    fn focused_accessibility_tree_is_observe_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(
            &policy,
            &DaemonRequest::FocusedAccessibilityTree(FocusedAccessibilityTreeRequest {
                depth: 1,
                max_nodes: 32,
            }),
        )
        .expect("accessibility tree reads are observe policy");
    }

    #[test]
    fn accessibility_find_is_observe_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityFind(AccessibilityFindRequest {
                role: Some("button".to_string()),
                name_contains: None,
                app: None,
                window_name_contains: None,
                depth: 0,
                max_results: 4,
                max_nodes: 128,
            }),
        )
        .expect("accessibility find is observe policy");
    }

    fn button_node(id: &str, name: &str) -> libplasma_pilot::AccessibilityNode {
        libplasma_pilot::AccessibilityNode {
            id: id.to_string(),
            role: "button".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["click".to_string()],
            actions: vec![libplasma_pilot::AccessibilityAction::Press],
            children: Vec::new(),
        }
    }

    fn text_node(id: &str, name: &str) -> libplasma_pilot::AccessibilityNode {
        libplasma_pilot::AccessibilityNode {
            id: id.to_string(),
            role: "text".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: Vec::new(),
            actions: vec![libplasma_pilot::AccessibilityAction::SetText],
            children: Vec::new(),
        }
    }

    fn tab_node(id: &str, name: &str) -> libplasma_pilot::AccessibilityNode {
        let mut node = press_tab_node(id, name);
        node.actions = vec![
            libplasma_pilot::AccessibilityAction::Press,
            libplasma_pilot::AccessibilityAction::Select,
        ];
        node.available_actions = vec!["press".to_string(), "select".to_string()];
        node
    }

    fn press_tab_node(id: &str, name: &str) -> libplasma_pilot::AccessibilityNode {
        libplasma_pilot::AccessibilityNode {
            id: id.to_string(),
            role: "page tab".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["press".to_string()],
            actions: vec![libplasma_pilot::AccessibilityAction::Press],
            children: Vec::new(),
        }
    }

    fn menu_node(
        id: &str,
        name: &str,
        children: Vec<libplasma_pilot::AccessibilityNode>,
    ) -> libplasma_pilot::AccessibilityNode {
        libplasma_pilot::AccessibilityNode {
            id: id.to_string(),
            role: "menu".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: Vec::new(),
            actions: Vec::new(),
            children,
        }
    }

    fn menu_item_node(id: &str, name: &str) -> libplasma_pilot::AccessibilityNode {
        let mut node = press_menu_item_node(id, name);
        node.actions = vec![
            libplasma_pilot::AccessibilityAction::Press,
            libplasma_pilot::AccessibilityAction::Select,
        ];
        node.available_actions = vec!["press".to_string(), "select".to_string()];
        node
    }

    fn press_menu_item_node(id: &str, name: &str) -> libplasma_pilot::AccessibilityNode {
        libplasma_pilot::AccessibilityNode {
            id: id.to_string(),
            role: "menu item".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["press".to_string()],
            actions: vec![libplasma_pilot::AccessibilityAction::Press],
            children: Vec::new(),
        }
    }
}
