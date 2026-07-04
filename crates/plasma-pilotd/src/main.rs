use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Error, Result, bail};
use clap::Parser;
use image::{GenericImageView, imageops::FilterType};
use libplasma_pilot::{
    AccessibilityFindRequest, AccessibilityInvokeRequest, AccessibilitySetTextRequest,
    ActionResult, ActivateTabRequest, ActiveWindowGuard, BackendCapability, CapabilitySet,
    ClickButtonRequest, ClickPointerRequest, ClipboardGetRequest, ClipboardText, CoordinateSpace,
    DaemonRequest, DaemonResponse, DesktopObservation, FocusWindowRequest,
    FocusedAccessibilityTreeRequest, HealthStatus, InputBackendStatus, JournalEntry,
    KeyComboRequest, KwinBridgeStatus, LibeiStatus, MovePointerRequest, ObserveRequest,
    PanicStopStatus, Point, PointerButton, PointerCalibrationPoint, PointerCalibrationStatus,
    PointerMonitorCalibration, PointerPhysicalBounds, PolicyStatus, RemoteDesktopPortalStatus,
    SafetyClass, ScreenshotInfo, ScreenshotRequest, ScreenshotTileRequest, ScreenshotTransform,
    ScrollPointerRequest, SelectMenuRequest, SetPanicStopRequest, SetTextFieldRequest,
    ToolApprovalLevel, TypeTextRequest, UinputStatus, WaitForChangeRequest, WaitForChangeResult,
    WindowGeometry, WindowInfo, current_egid, current_euid, default_journal_path,
    default_panic_stop_path, default_socket_path,
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

    fn tail_filtered(
        &self,
        limit: usize,
        method_filter: Option<&str>,
        ok: Option<bool>,
    ) -> Result<Vec<JournalEntry>> {
        tail_journal_entries(&self.path, limit, method_filter, ok)
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
struct PanicStopState {
    path: PathBuf,
}

impl PanicStopState {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn status(&self) -> PanicStopStatus {
        PanicStopStatus {
            enabled: self.path.exists(),
            path: self.path.clone(),
        }
    }

    fn set_enabled(&self, enabled: bool) -> Result<PanicStopStatus> {
        if enabled {
            let parent = self.path.parent().ok_or_else(|| {
                anyhow::anyhow!("panic-stop path has no parent: {}", self.path.display())
            })?;
            fs::create_dir_all(parent)
                .with_context(|| format!("create panic-stop dir {}", parent.display()))?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .with_context(|| format!("set panic-stop dir permissions {}", parent.display()))?;
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&self.path)
                .with_context(|| format!("create panic-stop file {}", self.path.display()))?;
            writeln!(file, "enabled_at_unix_ms={}", unix_time_ms()?)
                .context("write panic-stop file")?;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("set panic-stop permissions {}", self.path.display()))?;
        } else {
            match fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("remove panic-stop file {}", self.path.display())
                    });
                }
            }
        }
        Ok(self.status())
    }
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

#[derive(Debug, Default, Deserialize)]
struct DaemonConfigFile {
    daemon: Option<DaemonFileConfig>,
    policy: Option<PolicyFileConfig>,
    apps: Option<AppsFileConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct DaemonFileConfig {
    socket: Option<String>,
    journal: Option<String>,
    panic_stop_file: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PolicyFileConfig {
    default_observe: Option<ToolApprovalLevel>,
    default_control: Option<ToolApprovalLevel>,
    default_clipboard_read: Option<ToolApprovalLevel>,
    default_clipboard_write: Option<ToolApprovalLevel>,
    #[serde(alias = "full_resolution_screenshot")]
    default_full_resolution_screenshot: Option<ToolApprovalLevel>,
}

#[derive(Debug, Default, Deserialize)]
struct AppsFileConfig {
    allow: Option<Vec<String>>,
    deny: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
struct AppPolicy {
    allow: Vec<String>,
    deny: Vec<String>,
}

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot local desktop-control daemon")]
struct Args {
    #[arg(long, env = "PLASMA_PILOT_CONFIG")]
    config: Option<PathBuf>,

    #[arg(long, env = "PLASMA_PILOT_SOCKET")]
    socket: Option<PathBuf>,

    #[arg(long, env = "PLASMA_PILOT_JOURNAL")]
    journal: Option<PathBuf>,

    #[arg(long, env = "PLASMA_PILOT_PANIC_STOP_FILE")]
    panic_stop_file: Option<PathBuf>,

    #[arg(long, env = "PLASMA_PILOT_ALLOW_CONTROL")]
    allow_control: bool,

    #[arg(long, env = "PLASMA_PILOT_ALLOW_CLIPBOARD_READ")]
    allow_clipboard_read: bool,

    #[arg(long, env = "PLASMA_PILOT_ALLOW_FULL_RESOLUTION_SCREENSHOT")]
    allow_full_resolution_screenshot: bool,

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

    let file_config = load_daemon_config(args.config.as_deref())?;
    let daemon_file_config = file_config.daemon.as_ref();

    let socket = configured_path(
        args.socket,
        daemon_file_config.and_then(|config| config.socket.as_deref()),
        default_socket_path,
    )
    .context("resolve daemon socket path")?;
    let journal = configured_path(
        args.journal,
        daemon_file_config.and_then(|config| config.journal.as_deref()),
        default_journal_path,
    )
    .context("resolve daemon journal path")?;
    let panic_stop_file = configured_path(
        args.panic_stop_file,
        daemon_file_config.and_then(|config| config.panic_stop_file.as_deref()),
        default_panic_stop_path,
    )
    .context("resolve daemon panic-stop path")?;

    let policy_config = policy_config(
        file_config.policy.as_ref(),
        args.allow_control,
        args.allow_clipboard_read,
        args.allow_full_resolution_screenshot,
    );
    let app_policy = app_policy(file_config.apps.as_ref());

    run(socket, journal, panic_stop_file, policy_config, app_policy).await
}

async fn run(
    socket: PathBuf,
    journal_path: PathBuf,
    panic_stop_path: PathBuf,
    policy_config: PolicyConfig,
    app_policy: AppPolicy,
) -> Result<()> {
    let journal = ActionJournal::new(journal_path);
    let panic_stop = PanicStopState::new(panic_stop_path);
    let policy = PolicyEngine::new(policy_config);
    let active_window_state = ActiveWindowState::default();
    let _kwin_bridge_connection = match start_kwin_bridge(active_window_state.clone()).await {
        Ok(connection) => Some(connection),
        Err(err) => {
            warn!(error = %err, "KWin bridge DBus service is unavailable");
            None
        }
    };
    let kwin_bridge_registered = _kwin_bridge_connection.is_some();

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
        let panic_stop = panic_stop.clone();
        let policy = policy.clone();
        let app_policy = app_policy.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_client(
                stream,
                active_window_state,
                kwin_bridge_registered,
                journal,
                panic_stop,
                policy,
                app_policy,
            )
            .await
            {
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
    kwin_bridge_registered: bool,
    journal: ActionJournal,
    panic_stop: PanicStopState,
    policy: PolicyEngine,
    app_policy: AppPolicy,
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
    let response = handle_request(
        request,
        &active_window_state,
        kwin_bridge_registered,
        &journal,
        &panic_stop,
        &policy,
        &app_policy,
    );
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
    kwin_bridge_registered: bool,
    journal: &ActionJournal,
    panic_stop: &PanicStopState,
    policy: &PolicyEngine,
    app_policy: &AppPolicy,
) -> DaemonResponse {
    if let Err(err) = enforce_policy(policy, &request) {
        return DaemonResponse::Error {
            message: format_error_chain(&err),
        };
    }
    if let Err(err) = enforce_panic_stop(panic_stop, &request) {
        return DaemonResponse::Error {
            message: format_error_chain(&err),
        };
    }
    if let Err(err) = enforce_active_window_guard(active_window_state, &request) {
        return DaemonResponse::Error {
            message: format_error_chain(&err),
        };
    }
    if let Err(err) = enforce_app_policy(active_window_state, app_policy, &request) {
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
        DaemonRequest::PanicStopStatus => DaemonResponse::PanicStop(panic_stop.status()),
        DaemonRequest::SetPanicStop(request) => match set_panic_stop(panic_stop, request) {
            Ok(status) => DaemonResponse::PanicStop(status),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::KwinBridgeStatus => {
            match kwin_bridge_status(active_window_state, kwin_bridge_registered) {
                Ok(status) => DaemonResponse::KwinBridgeStatus(status),
                Err(err) => DaemonResponse::Error {
                    message: format_error_chain(&err),
                },
            }
        }
        DaemonRequest::UinputStatus => match uinput_status() {
            Ok(status) => DaemonResponse::UinputStatus(status),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::InputBackendStatus => match input_backend_status() {
            Ok(status) => DaemonResponse::InputBackendStatus(status),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::PointerCalibration => match pointer_calibration_status() {
            Ok(status) => DaemonResponse::PointerCalibration(status),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
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
        DaemonRequest::WaitForChange(request) => match wait_for_change(request) {
            Ok(result) => DaemonResponse::WaitForChange(Box::new(result)),
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
        DaemonRequest::TypeText(request) => match type_text(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::KeyCombo(request) => match key_combo(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::MovePointer(request) => match move_pointer(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ClickPointer(request) => match click_pointer(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => DaemonResponse::Error {
                message: format_error_chain(&err),
            },
        },
        DaemonRequest::ScrollPointer(request) => match scroll_pointer(request) {
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
        DaemonRequest::JournalTail(request) => {
            match journal.tail_filtered(request.limit, request.method_filter.as_deref(), request.ok)
            {
                Ok(entries) => DaemonResponse::Journal(entries),
                Err(err) => DaemonResponse::Error {
                    message: format_error_chain(&err),
                },
            }
        }
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
        default_full_resolution_screenshot: config.default_full_resolution_screenshot.clone(),
        default_clipboard_read: config.default_clipboard_read.clone(),
        default_clipboard_write: config.default_clipboard_write.clone(),
    }
}

fn load_daemon_config(explicit_path: Option<&Path>) -> Result<DaemonConfigFile> {
    let path = explicit_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_config_path);

    if !path.exists() {
        if explicit_path.is_some() {
            bail!("config file does not exist: {}", path.display());
        }
        return Ok(DaemonConfigFile::default());
    }

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("read config file {}", path.display()))?;
    toml::from_str(&contents).with_context(|| format!("parse config file {}", path.display()))
}

fn default_config_path() -> PathBuf {
    xdg_config_home().join("plasma-pilot/config.toml")
}

fn configured_path(
    cli_path: Option<PathBuf>,
    config_path: Option<&str>,
    default_path: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<PathBuf> {
    if let Some(path) = cli_path {
        return Ok(path);
    }
    if let Some(path) = config_path {
        return expand_config_path(path);
    }
    default_path().map_err(Into::into)
}

fn expand_config_path(value: &str) -> Result<PathBuf> {
    let mut expanded = value.to_string();
    for name in [
        "XDG_RUNTIME_DIR",
        "XDG_STATE_HOME",
        "XDG_CONFIG_HOME",
        "HOME",
    ] {
        let marker = format!("${name}");
        if expanded.contains(&marker) {
            let replacement = env::var(name)
                .with_context(|| format!("{name} is required to expand config path {value}"))?;
            expanded = expanded.replace(&marker, &replacement);
        }
    }
    Ok(PathBuf::from(expanded))
}

fn policy_config(
    file_policy: Option<&PolicyFileConfig>,
    allow_control: bool,
    allow_clipboard_read: bool,
    allow_full_resolution_screenshot: bool,
) -> PolicyConfig {
    let mut config = PolicyConfig::default();
    if let Some(file_policy) = file_policy {
        if let Some(level) = &file_policy.default_observe {
            config.default_observe = level.clone();
        }
        if let Some(level) = &file_policy.default_control {
            config.default_control = level.clone();
        }
        if let Some(level) = &file_policy.default_clipboard_read {
            config.default_clipboard_read = level.clone();
        }
        if let Some(level) = &file_policy.default_clipboard_write {
            config.default_clipboard_write = level.clone();
        }
        if let Some(level) = &file_policy.default_full_resolution_screenshot {
            config.default_full_resolution_screenshot = level.clone();
        }
    }
    if allow_control {
        config.default_control = ToolApprovalLevel::Allow;
    }
    if allow_clipboard_read {
        config.default_clipboard_read = ToolApprovalLevel::Allow;
    }
    if allow_full_resolution_screenshot {
        config.default_full_resolution_screenshot = ToolApprovalLevel::Allow;
    }
    config
}

fn app_policy(file_apps: Option<&AppsFileConfig>) -> AppPolicy {
    let Some(file_apps) = file_apps else {
        return AppPolicy::default();
    };
    AppPolicy {
        allow: normalize_app_policy_list(file_apps.allow.as_deref().unwrap_or(&[])),
        deny: normalize_app_policy_list(file_apps.deny.as_deref().unwrap_or(&[])),
    }
}

fn normalize_app_policy_list(values: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !normalized.iter().any(|seen| app_id_matches(seen, value)) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn kwin_bridge_status(
    active_window_state: &ActiveWindowState,
    dbus_service_registered: bool,
) -> Result<KwinBridgeStatus> {
    let active_window_snapshot = active_window_state.snapshot()?;
    let active_window_update_seen = active_window_snapshot.is_some();
    let active_window = active_window_snapshot.flatten();
    let package_dir = xdg_data_home().join("kwin/scripts/plasma-pilot-bridge");
    let config_path = xdg_config_home().join("kwinrc");
    let script_enabled = read_kwin_bridge_enabled(&config_path)?;

    Ok(KwinBridgeStatus {
        dbus_service_registered,
        active_window_update_seen,
        active_window,
        package_installed: package_dir.join("metadata.json").is_file(),
        package_dir,
        config_path,
        script_enabled,
    })
}

fn xdg_data_home() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

fn xdg_config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

fn read_kwin_bridge_enabled(config_path: &Path) -> Result<Option<bool>> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", config_path.display())),
    };
    Ok(parse_kwin_bridge_enabled(&content))
}

fn parse_kwin_bridge_enabled(content: &str) -> Option<bool> {
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
        if key.trim() != "plasma-pilot-bridgeEnabled" {
            continue;
        }
        return parse_bool_config_value(value.trim());
    }
    None
}

fn uinput_status() -> Result<UinputStatus> {
    let path = plasma_pilot_uinput::uinput_path().to_path_buf();
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => Some(metadata),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    };
    let available = plasma_pilot_uinput::available();
    let exists = metadata.is_some();
    let is_char_device = metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_char_device());
    let mode = metadata
        .as_ref()
        .map(|metadata| metadata.permissions().mode() & 0o7777);
    let owner_uid = metadata.as_ref().map(MetadataExt::uid);
    let owner_gid = metadata.as_ref().map(MetadataExt::gid);
    let process_uid = current_euid().context("read daemon effective uid")?;
    let process_gid = current_egid().context("read daemon effective gid")?;

    Ok(UinputStatus {
        path,
        available,
        exists,
        is_char_device,
        mode,
        owner_uid,
        owner_gid,
        process_uid,
        process_gid,
        setup_hint: uinput_setup_hint(available, exists, is_char_device),
    })
}

fn uinput_setup_hint(available: bool, exists: bool, is_char_device: bool) -> String {
    if available {
        return "uinput available to daemon process".to_string();
    }
    if !exists {
        return "load the uinput kernel module and install the udev rule before starting plasma-pilotd"
            .to_string();
    }
    if !is_char_device {
        return "refusing /dev/uinput because it is not a character device".to_string();
    }
    "grant the daemon read/write access to /dev/uinput with the packaged udev rule, reload udev, add the user to the configured group, then restart the user session or service".to_string()
}

fn input_backend_status() -> Result<InputBackendStatus> {
    let uinput = uinput_status()?;
    let remote_desktop_portal = remote_desktop_portal_status();
    let libei = libei_status();
    let preferred_available_backend =
        preferred_input_backend(&remote_desktop_portal, &libei, uinput.available);
    let setup_hint = input_backend_setup_hint(
        preferred_available_backend.as_deref(),
        &remote_desktop_portal,
        &libei,
        uinput.available,
    );

    Ok(InputBackendStatus {
        uinput_available: uinput.available,
        remote_desktop_portal,
        libei,
        preferred_available_backend,
        setup_hint,
    })
}

fn remote_desktop_portal_status() -> RemoteDesktopPortalStatus {
    let busctl_available = command_exists("busctl");
    if !busctl_available {
        return RemoteDesktopPortalStatus {
            busctl_available,
            portal_service_available: false,
            remote_desktop_interface_available: false,
            kde_portal_service_available: false,
            setup_hint: remote_desktop_portal_setup_hint(false, false, false, false),
        };
    }

    let service_list =
        command_stdout("busctl", &["--user", "--no-pager", "--list"]).unwrap_or_default();
    let portal_service_available = service_list.contains("org.freedesktop.portal.Desktop");
    let kde_portal_service_available =
        service_list.contains("org.freedesktop.impl.portal.desktop.kde");
    let remote_desktop_interface_available = portal_service_available
        && command_success(
            "busctl",
            &[
                "--user",
                "--no-pager",
                "introspect",
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.RemoteDesktop",
            ],
        );

    RemoteDesktopPortalStatus {
        busctl_available,
        portal_service_available,
        remote_desktop_interface_available,
        kde_portal_service_available,
        setup_hint: remote_desktop_portal_setup_hint(
            busctl_available,
            portal_service_available,
            remote_desktop_interface_available,
            kde_portal_service_available,
        ),
    }
}

fn libei_status() -> LibeiStatus {
    let pkg_config_available = command_exists("pkg-config");
    let client_library_available =
        pkg_config_available && command_success("pkg-config", &["--exists", "libei-1.0"]);
    let socket_env_present = env::var_os("LIBEI_SOCKET").is_some();

    LibeiStatus {
        pkg_config_available,
        client_library_available,
        socket_env_present,
        setup_hint: libei_setup_hint(
            pkg_config_available,
            client_library_available,
            socket_env_present,
        ),
    }
}

fn preferred_input_backend(
    remote_desktop_portal: &RemoteDesktopPortalStatus,
    libei: &LibeiStatus,
    uinput_available: bool,
) -> Option<String> {
    if remote_desktop_portal.remote_desktop_interface_available {
        return Some("portal_remote_desktop".to_string());
    }
    if libei.socket_env_present || libei.client_library_available {
        return Some("libei".to_string());
    }
    if uinput_available {
        return Some("uinput".to_string());
    }
    None
}

fn input_backend_setup_hint(
    preferred: Option<&str>,
    remote_desktop_portal: &RemoteDesktopPortalStatus,
    libei: &LibeiStatus,
    uinput_available: bool,
) -> String {
    match preferred {
        Some("portal_remote_desktop") => {
            "prefer xdg-desktop-portal RemoteDesktop for consented input sessions; uinput remains fallback".to_string()
        }
        Some("libei") => {
            "libei client support is visible; verify the compositor or portal grants an EIS connection before using it for control".to_string()
        }
        Some("uinput") => {
            "only uinput is currently available; keep it behind policy, panic-stop, active-window guards, and journal checks".to_string()
        }
        _ if !remote_desktop_portal.busctl_available => {
            "install busctl/systemd tools or run in a user session with DBus before probing portal RemoteDesktop; configure libei or uinput fallback as needed".to_string()
        }
        _ if !remote_desktop_portal.remote_desktop_interface_available
            && !libei.client_library_available
            && !libei.socket_env_present
            && !uinput_available =>
        {
            "no input backend is currently available; configure portal RemoteDesktop/libei or install the uinput rule".to_string()
        }
        _ => "input backend state is partial; inspect individual portal, libei, and uinput fields".to_string(),
    }
}

fn remote_desktop_portal_setup_hint(
    busctl_available: bool,
    portal_service_available: bool,
    remote_desktop_interface_available: bool,
    kde_portal_service_available: bool,
) -> String {
    if !busctl_available {
        return "busctl is unavailable; cannot probe xdg-desktop-portal RemoteDesktop".to_string();
    }
    if !portal_service_available {
        return "org.freedesktop.portal.Desktop is not visible on the user bus".to_string();
    }
    if !remote_desktop_interface_available {
        return "portal service is visible, but org.freedesktop.portal.RemoteDesktop did not introspect successfully".to_string();
    }
    if !kde_portal_service_available {
        return "RemoteDesktop portal is visible; KDE portal backend service was not listed"
            .to_string();
    }
    "portal RemoteDesktop interface and KDE portal backend are visible".to_string()
}

fn libei_setup_hint(
    pkg_config_available: bool,
    client_library_available: bool,
    socket_env_present: bool,
) -> String {
    if socket_env_present {
        return "LIBEI_SOCKET is set; verify the socket belongs to the intended compositor or broker".to_string();
    }
    if client_library_available {
        return "libei client library is available; an EIS connection still needs compositor or portal mediation".to_string();
    }
    if !pkg_config_available {
        return "pkg-config is unavailable; cannot probe libei client library metadata".to_string();
    }
    "libei client library metadata was not found by pkg-config".to_string()
}

fn parse_bool_config_value(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
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

fn enforce_panic_stop(panic_stop: &PanicStopState, request: &DaemonRequest) -> Result<()> {
    let status = panic_stop.status();
    let safety_class = safety_class_for_request(request);
    if status.enabled
        && matches!(
            safety_class,
            SafetyClass::ControlPointer
                | SafetyClass::ControlKeyboard
                | SafetyClass::ControlSemantic
        )
    {
        bail!(
            "panic-stop is active at {}; refusing {:?}",
            status.path.display(),
            safety_class
        );
    }
    Ok(())
}

fn enforce_app_policy(
    active_window_state: &ActiveWindowState,
    app_policy: &AppPolicy,
    request: &DaemonRequest,
) -> Result<()> {
    if app_policy.allow.is_empty() && app_policy.deny.is_empty() {
        return Ok(());
    }
    if !matches!(
        safety_class_for_request(request),
        SafetyClass::ControlPointer | SafetyClass::ControlKeyboard | SafetyClass::ControlSemantic
    ) {
        return Ok(());
    }

    if let DaemonRequest::FocusWindow(request) = request {
        let windows = list_windows().context("app policy could not list focus targets")?;
        let target = windows
            .iter()
            .find(|window| window.id == request.window_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "app policy could not find focus target window {}",
                    request.window_id
                )
            })?;
        return enforce_app_policy_for_app(app_policy, target.app_id.as_deref(), "focus target");
    }

    let window = active_window(active_window_state)
        .context("app policy could not read active window")?
        .ok_or_else(|| anyhow::anyhow!("app policy requires an active window for control"))?;
    enforce_app_policy_for_app(app_policy, window.app_id.as_deref(), "active window")
}

fn enforce_app_policy_for_app(
    app_policy: &AppPolicy,
    app_id: Option<&str>,
    context: &str,
) -> Result<()> {
    let app_id = app_id
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("app policy could not determine {context} app id"))?;

    if app_policy
        .deny
        .iter()
        .any(|denied| app_id_matches(denied, app_id))
    {
        bail!("app policy denied {context} app {}", app_id);
    }
    if !app_policy.allow.is_empty()
        && !app_policy
            .allow
            .iter()
            .any(|allowed| app_id_matches(allowed, app_id))
    {
        bail!("app policy did not allow {context} app {}", app_id);
    }
    Ok(())
}

fn app_id_matches(policy_value: &str, app_id: &str) -> bool {
    policy_value.eq_ignore_ascii_case(app_id)
}

fn enforce_active_window_guard(
    active_window_state: &ActiveWindowState,
    request: &DaemonRequest,
) -> Result<()> {
    let Some(guard) = active_window_guard_for_request(request) else {
        return Ok(());
    };
    let window = active_window(active_window_state)
        .context("active-window guard could not read active window")?
        .ok_or_else(|| anyhow::anyhow!("active-window guard failed: no active window"))?;

    if let Some(expected) = &guard.expected_window_id
        && window.id != *expected
    {
        bail!(
            "active-window guard failed: expected window id {}, got {}",
            expected,
            window.id
        );
    }
    if let Some(expected) = &guard.expected_app_id
        && window.app_id.as_deref() != Some(expected.as_str())
    {
        bail!(
            "active-window guard failed: expected app id {}, got {}",
            expected,
            window.app_id.as_deref().unwrap_or("")
        );
    }
    if let Some(expected) = &guard.title_contains {
        let title = window.title.to_ascii_lowercase();
        let expected = expected.to_ascii_lowercase();
        if !title.contains(&expected) {
            bail!(
                "active-window guard failed: expected title containing {}, got {}",
                guard.title_contains.as_deref().unwrap_or(""),
                window.title
            );
        }
    }
    Ok(())
}

fn active_window_guard_for_request(request: &DaemonRequest) -> Option<&ActiveWindowGuard> {
    match request {
        DaemonRequest::FocusWindow(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilityInvoke(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilitySetText(request) => request.guard.as_ref(),
        DaemonRequest::TypeText(request) => request.guard.as_ref(),
        DaemonRequest::KeyCombo(request) => request.guard.as_ref(),
        DaemonRequest::MovePointer(request) => request.guard.as_ref(),
        DaemonRequest::ClickPointer(request) => request.guard.as_ref(),
        DaemonRequest::ScrollPointer(request) => request.guard.as_ref(),
        DaemonRequest::ClickButton(request) => request.guard.as_ref(),
        DaemonRequest::SetTextField(request) => request.guard.as_ref(),
        DaemonRequest::ActivateTab(request) => request.guard.as_ref(),
        DaemonRequest::SelectMenu(request) => request.guard.as_ref(),
        _ => None,
    }
}

fn safety_class_for_request(request: &DaemonRequest) -> SafetyClass {
    match request {
        DaemonRequest::Health
        | DaemonRequest::Capabilities
        | DaemonRequest::PolicyStatus
        | DaemonRequest::PanicStopStatus
        | DaemonRequest::SetPanicStop(_)
        | DaemonRequest::UinputStatus
        | DaemonRequest::InputBackendStatus
        | DaemonRequest::PointerCalibration
        | DaemonRequest::JournalTail(_) => SafetyClass::Policy,
        DaemonRequest::ListMonitors
        | DaemonRequest::ListWindows
        | DaemonRequest::KwinBridgeStatus
        | DaemonRequest::ActiveWindow
        | DaemonRequest::ScreenshotTile(_)
        | DaemonRequest::WaitForChange(_)
        | DaemonRequest::FocusedAccessibilityTree(_)
        | DaemonRequest::AccessibilityFind(_) => SafetyClass::Observe,
        DaemonRequest::Observe(request) => {
            if request
                .screenshot
                .as_ref()
                .is_some_and(|screenshot| screenshot.full_resolution)
            {
                SafetyClass::FullResolutionScreenshot
            } else {
                SafetyClass::Observe
            }
        }
        DaemonRequest::Screenshot(request) => {
            if request.full_resolution {
                SafetyClass::FullResolutionScreenshot
            } else {
                SafetyClass::Observe
            }
        }
        DaemonRequest::ClipboardGet(_) => SafetyClass::ClipboardRead,
        DaemonRequest::ClipboardSet(_) => SafetyClass::ClipboardWrite,
        DaemonRequest::MovePointer(_)
        | DaemonRequest::ClickPointer(_)
        | DaemonRequest::ScrollPointer(_) => SafetyClass::ControlPointer,
        DaemonRequest::TypeText(_) | DaemonRequest::KeyCombo(_) => SafetyClass::ControlKeyboard,
        DaemonRequest::FocusWindow(_)
        | DaemonRequest::AccessibilityInvoke(_)
        | DaemonRequest::AccessibilitySetText(_)
        | DaemonRequest::ClickButton(_)
        | DaemonRequest::SetTextField(_)
        | DaemonRequest::ActivateTab(_)
        | DaemonRequest::SelectMenu(_) => SafetyClass::ControlSemantic,
    }
}

fn set_panic_stop(
    panic_stop: &PanicStopState,
    request: SetPanicStopRequest,
) -> Result<PanicStopStatus> {
    panic_stop.set_enabled(request.enabled)
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
    if clipboard_read_backend().is_some() && clipboard_write_backend().is_some() {
        capabilities.push(BackendCapability::ClipboardText);
    }
    if plasma_pilot_uinput::available() {
        capabilities.push(BackendCapability::KeyboardInput);
        capabilities.push(BackendCapability::PointerInput);
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

fn command_success(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn command_stdout(command: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("run {command}"))?;
    if !output.status.success() {
        bail!("{command} exited with status {}", output.status);
    }
    String::from_utf8(output.stdout).with_context(|| format!("{command} stdout is not UTF-8"))
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

fn wait_for_change(request: WaitForChangeRequest) -> Result<WaitForChangeResult> {
    validate_wait_for_change_request(&request)?;
    let timeout = Duration::from_millis(request.timeout_ms);
    let interval = Duration::from_millis(request.interval_ms);
    let started = Instant::now();
    let screenshot_request = || ScreenshotRequest {
        output: request.output.clone(),
        max_edge: request.max_edge.or(Some(1600)),
        full_resolution: false,
    };

    let baseline_info = capture_screenshot(screenshot_request())?;
    let baseline = read_image_sample(&baseline_info.path)?;
    let mut final_info = baseline_info;
    let mut captures = 1;
    let mut score = 0.0;
    let mut changed = false;

    while started.elapsed() < timeout {
        let remaining = timeout.saturating_sub(started.elapsed());
        thread::sleep(interval.min(remaining));
        final_info = capture_screenshot(screenshot_request())?;
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

fn list_monitors() -> Result<Vec<libplasma_pilot::MonitorInfo>> {
    plasma_pilot_kwin::list_monitors().map_err(|err| anyhow::anyhow!(err))
}

fn list_windows() -> Result<Vec<libplasma_pilot::WindowInfo>> {
    let monitors = list_monitors().unwrap_or_default();
    list_windows_with_monitors(&monitors)
}

fn list_windows_with_monitors(
    monitors: &[libplasma_pilot::MonitorInfo],
) -> Result<Vec<libplasma_pilot::WindowInfo>> {
    let mut windows = plasma_pilot_kwin::list_windows().map_err(|err| anyhow::anyhow!(err))?;
    assign_monitor_ids(&mut windows, monitors);
    Ok(windows)
}

fn active_window(active_window_state: &ActiveWindowState) -> Result<Option<WindowInfo>> {
    let monitors = list_monitors().unwrap_or_default();
    active_window_with_monitors(active_window_state, &monitors)
}

fn active_window_with_monitors(
    active_window_state: &ActiveWindowState,
    monitors: &[libplasma_pilot::MonitorInfo],
) -> Result<Option<WindowInfo>> {
    if let Some(window) = active_window_state.snapshot()? {
        return Ok(window.map(|mut window| {
            assign_monitor_id(&mut window, monitors);
            window
        }));
    }
    let mut window = plasma_pilot_kwin::active_window().map_err(|err| anyhow::anyhow!(err))?;
    if let Some(window) = window.as_mut() {
        assign_monitor_id(window, monitors);
    }
    Ok(window)
}

fn assign_monitor_ids(windows: &mut [WindowInfo], monitors: &[libplasma_pilot::MonitorInfo]) {
    for window in windows {
        assign_monitor_id(window, monitors);
    }
}

fn assign_monitor_id(window: &mut WindowInfo, monitors: &[libplasma_pilot::MonitorInfo]) {
    if window.monitor_id.is_none() {
        window.monitor_id = window_monitor_id(window, monitors);
    }
}

fn window_monitor_id(
    window: &WindowInfo,
    monitors: &[libplasma_pilot::MonitorInfo],
) -> Option<String> {
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

fn logical_overlap_area(geometry: &WindowGeometry, monitor: &libplasma_pilot::MonitorInfo) -> i64 {
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

fn observe_desktop(
    request: ObserveRequest,
    active_window_state: &ActiveWindowState,
) -> Result<DesktopObservation> {
    let monitors = list_monitors().unwrap_or_default();
    let windows = list_windows_with_monitors(&monitors).unwrap_or_default();
    let active_window =
        active_window_with_monitors(active_window_state, &monitors).unwrap_or_default();
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
    let backend = clipboard_read_backend()
        .ok_or_else(|| anyhow::anyhow!("no clipboard text read backend is available"))?;
    let text = match backend {
        ClipboardBackend::WlClipboard => clipboard_get_text_wl()?,
        ClipboardBackend::KdeKlipper => clipboard_get_text_klipper()?,
    };
    Ok(bound_clipboard_text(
        text,
        request.max_bytes,
        backend.name().to_string(),
    ))
}

fn clipboard_get_text_wl() -> Result<String> {
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

    String::from_utf8(output.stdout).context("clipboard text is not valid UTF-8")
}

fn clipboard_get_text_klipper() -> Result<String> {
    let output = Command::new("qdbus6")
        .args([
            "org.kde.klipper",
            "/klipper",
            "org.kde.klipper.klipper.getClipboardContents",
        ])
        .output()
        .context("run KDE Klipper clipboard read backend")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "KDE Klipper clipboard read backend exited with status {}: {stderr}",
            output.status
        );
    }
    let mut text =
        String::from_utf8(output.stdout).context("KDE Klipper clipboard text is not UTF-8")?;
    if text.ends_with('\n') {
        text.pop();
    }
    Ok(text)
}

fn bound_clipboard_text(
    mut text: String,
    max_bytes: Option<usize>,
    backend: String,
) -> ClipboardText {
    let original_bytes = text.len();
    let Some(max_bytes) = max_bytes else {
        return ClipboardText {
            text,
            truncated: false,
            original_bytes,
            backend,
        };
    };
    if original_bytes <= max_bytes {
        return ClipboardText {
            text,
            truncated: false,
            original_bytes,
            backend,
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
        backend,
    }
}

fn clipboard_set_text(text: &str) -> Result<ActionResult> {
    let backend = clipboard_write_backend()
        .ok_or_else(|| anyhow::anyhow!("no clipboard text write backend is available"))?;
    match backend {
        ClipboardBackend::WlClipboard => clipboard_set_text_wl(text)?,
        ClipboardBackend::KdeKlipper => clipboard_set_text_klipper(text)?,
    }

    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "set clipboard text length={} backend={}",
            text.len(),
            backend.name()
        )),
    })
}

fn clipboard_set_text_wl(text: &str) -> Result<()> {
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
    Ok(())
}

fn clipboard_set_text_klipper(text: &str) -> Result<()> {
    let status = Command::new("qdbus6")
        .args([
            "org.kde.klipper",
            "/klipper",
            "org.kde.klipper.klipper.setClipboardContents",
            text,
        ])
        .status()
        .context("run KDE Klipper clipboard write backend")?;
    if !status.success() {
        bail!("KDE Klipper clipboard write backend exited with status {status}");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardBackend {
    WlClipboard,
    KdeKlipper,
}

impl ClipboardBackend {
    fn name(self) -> &'static str {
        match self {
            Self::WlClipboard => "wl-clipboard",
            Self::KdeKlipper => "kde-klipper",
        }
    }
}

fn clipboard_read_backend() -> Option<ClipboardBackend> {
    clipboard_read_backend_from_availability(command_exists("wl-paste"), kde_klipper_available())
}

fn clipboard_read_backend_from_availability(
    wl_paste_available: bool,
    kde_klipper_available: bool,
) -> Option<ClipboardBackend> {
    if wl_paste_available {
        return Some(ClipboardBackend::WlClipboard);
    }
    if kde_klipper_available {
        return Some(ClipboardBackend::KdeKlipper);
    }
    None
}

fn clipboard_write_backend() -> Option<ClipboardBackend> {
    clipboard_write_backend_from_availability(command_exists("wl-copy"), kde_klipper_available())
}

fn clipboard_write_backend_from_availability(
    wl_copy_available: bool,
    kde_klipper_available: bool,
) -> Option<ClipboardBackend> {
    if wl_copy_available {
        return Some(ClipboardBackend::WlClipboard);
    }
    if kde_klipper_available {
        return Some(ClipboardBackend::KdeKlipper);
    }
    None
}

fn kde_klipper_available() -> bool {
    Command::new("qdbus6")
        .args(["org.kde.klipper", "/klipper"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
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

fn type_text(request: TypeTextRequest) -> Result<ActionResult> {
    if request.text.is_empty() {
        bail!("text must be non-empty");
    }
    if request.text.chars().count() > 8192 {
        bail!("text must be at most 8192 characters");
    }
    plasma_pilot_uinput::type_text(&request.text).map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "typed text length={}",
            request.text.chars().count()
        )),
    })
}

fn key_combo(request: KeyComboRequest) -> Result<ActionResult> {
    if request.combo.trim().is_empty() {
        bail!("combo must be non-empty");
    }
    let key_count =
        plasma_pilot_uinput::key_combo(&request.combo).map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!("sent key combo keys={key_count}")),
    })
}

fn pointer_calibration_status() -> Result<PointerCalibrationStatus> {
    let monitors = list_monitors()?;
    pointer_calibration_status_from_monitors(&monitors)
}

fn pointer_calibration_status_from_monitors(
    monitors: &[libplasma_pilot::MonitorInfo],
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
        setup_hint: "physical_pixel pointer coordinates are derived from KWin monitor logical origins, scale factors, and physical sizes; verify with a guarded disposable test window before production click use".to_string(),
    })
}

fn pointer_monitor_calibrations(
    monitors: &[libplasma_pilot::MonitorInfo],
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

fn move_pointer(request: MovePointerRequest) -> Result<ActionResult> {
    let bounds = physical_pointer_bounds()?;
    validate_pointer_point(request.point, bounds)?;
    plasma_pilot_uinput::move_pointer(request.point.x, request.point.y, bounds)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "moved pointer x={:.0} y={:.0} space={:?}",
            request.point.x, request.point.y, request.point.space
        )),
    })
}

fn click_pointer(request: ClickPointerRequest) -> Result<ActionResult> {
    if request.clicks == 0 || request.clicks > 2 {
        bail!("clicks must be 1 or 2");
    }
    let bounds = physical_pointer_bounds()?;
    validate_pointer_point(request.point, bounds)?;
    plasma_pilot_uinput::click_pointer(
        request.point.x,
        request.point.y,
        bounds,
        pointer_button_to_uinput(request.button),
        request.clicks,
    )
    .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "clicked pointer button={:?} clicks={} x={:.0} y={:.0} space={:?}",
            request.button, request.clicks, request.point.x, request.point.y, request.point.space
        )),
    })
}

fn scroll_pointer(request: ScrollPointerRequest) -> Result<ActionResult> {
    if request.vertical == 0 && request.horizontal == 0 {
        bail!("scroll request must include a non-zero delta");
    }
    let bounds = physical_pointer_bounds()?;
    plasma_pilot_uinput::scroll_pointer(request.vertical, request.horizontal, bounds)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        message: Some(format!(
            "scrolled pointer vertical={} horizontal={}",
            request.vertical, request.horizontal
        )),
    })
}

fn physical_pointer_bounds() -> Result<plasma_pilot_uinput::PointerBounds> {
    physical_pointer_bounds_from_monitors(&list_monitors()?)
}

fn physical_pointer_bounds_from_monitors(
    monitors: &[libplasma_pilot::MonitorInfo],
) -> Result<plasma_pilot_uinput::PointerBounds> {
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
    Ok(plasma_pilot_uinput::PointerBounds {
        min_x,
        min_y,
        width,
        height,
    })
}

fn scaled_physical_origin(origin: i32, scale_factor: f64) -> Result<i32> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        bail!("monitor scale factor must be finite and positive");
    }
    let scaled = f64::from(origin) * scale_factor;
    if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        bail!("scaled monitor origin overflows i32");
    }
    Ok(scaled.round() as i32)
}

fn validate_pointer_point(point: Point, bounds: plasma_pilot_uinput::PointerBounds) -> Result<()> {
    if point.space != CoordinateSpace::PhysicalPixel {
        bail!(
            "pointer actions currently require physical_pixel coordinate space, got {:?}",
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

fn pointer_button_to_uinput(button: PointerButton) -> plasma_pilot_uinput::PointerButton {
    match button {
        PointerButton::Left => plasma_pilot_uinput::PointerButton::Left,
        PointerButton::Middle => plasma_pilot_uinput::PointerButton::Middle,
        PointerButton::Right => plasma_pilot_uinput::PointerButton::Right,
    }
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

fn tail_journal_entries(
    path: &Path,
    limit: usize,
    method_filter: Option<&str>,
    ok: Option<bool>,
) -> Result<Vec<JournalEntry>> {
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
        if let Some(method_filter) = method_filter
            && entry.method != method_filter
        {
            continue;
        }
        if let Some(ok) = ok
            && entry.ok != ok
        {
            continue;
        }
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
        DaemonResponse::PanicStop(status) => format!(
            "panic-stop enabled={} path={}",
            status.enabled,
            status.path.display()
        ),
        DaemonResponse::KwinBridgeStatus(status) => format!(
            "kwin bridge dbus={} update_seen={} installed={} enabled={}",
            status.dbus_service_registered,
            status.active_window_update_seen,
            status.package_installed,
            status
                .script_enabled
                .map(|enabled| enabled.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
        DaemonResponse::UinputStatus(status) => format!(
            "uinput available={} exists={} char_device={} mode={}",
            status.available,
            status.exists,
            status.is_char_device,
            status
                .mode
                .map(|mode| format!("{mode:o}"))
                .unwrap_or_else(|| "unknown".to_string())
        ),
        DaemonResponse::InputBackendStatus(status) => format!(
            "input backends preferred={} portal_remote_desktop={} libei={} uinput={}",
            status
                .preferred_available_backend
                .as_deref()
                .unwrap_or("none"),
            status
                .remote_desktop_portal
                .remote_desktop_interface_available,
            status.libei.client_library_available || status.libei.socket_env_present,
            status.uinput_available
        ),
        DaemonResponse::PointerCalibration(status) => format!(
            "pointer calibration bounds={},{} {}x{} monitors={} coordinate_space={:?}",
            status.bounds.min_x,
            status.bounds.min_y,
            status.bounds.width,
            status.bounds.height,
            status.monitors.len(),
            status.coordinate_space
        ),
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
        DaemonResponse::WaitForChange(result) => format!(
            "wait_for_change changed={} captures={} score={:.6} threshold={:.6} path={}",
            result.changed,
            result.captures,
            result.score,
            result.threshold,
            result.screenshot.path.display()
        ),
        DaemonResponse::ClipboardText(text) => format!(
            "clipboard text length={} truncated={} original_bytes={} backend={}",
            text.text.len(),
            text.truncated,
            text.original_bytes,
            text.backend
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
    fn assigns_window_monitor_by_largest_logical_overlap() {
        let monitors = vec![
            monitor("left", -1920, 0, 1920, 1080, 1920, 1080, 1.0),
            monitor("main", 0, 0, 7680, 4320, 5120, 2880, 1.5),
        ];
        let mut window = WindowInfo {
            id: "window-1".to_string(),
            app_id: Some("org.kde.test".to_string()),
            title: "Test".to_string(),
            pid: None,
            monitor_id: None,
            geometry: Some(WindowGeometry {
                x: -100,
                y: 200,
                width: 500,
                height: 300,
                space: CoordinateSpace::LogicalPixel,
            }),
        };

        assign_monitor_id(&mut window, &monitors);

        assert_eq!(window.monitor_id.as_deref(), Some("main"));
    }

    #[test]
    fn active_window_with_monitors_enriches_bridge_window() {
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
        let monitors = vec![monitor("main", 0, 0, 7680, 4320, 5120, 2880, 1.5)];

        let window = active_window_with_monitors(&state, &monitors)
            .expect("active window resolves")
            .expect("active window exists");

        assert_eq!(window.pid, Some(1234));
        assert_eq!(window.monitor_id.as_deref(), Some("main"));
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

        let entries = journal
            .tail_filtered(1, None, None)
            .expect("journal tail succeeds");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sequence, 2);
        assert_eq!(entries[0].method, "capabilities");
        assert!(entries[0].ok);

        let entries = journal
            .tail_filtered(10, Some("health"), Some(true))
            .expect("filtered journal tail succeeds");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "health");
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
    fn bounded_screenshot_requests_pass_default_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(
            &policy,
            &DaemonRequest::Screenshot(ScreenshotRequest {
                output: temp_test_path("bounded-screenshot.png"),
                max_edge: Some(1600),
                full_resolution: false,
            }),
        )
        .expect("bounded screenshot requests are allowed by default");
    }

    #[test]
    fn full_resolution_screenshot_fails_closed_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::Screenshot(ScreenshotRequest {
                output: temp_test_path("full-resolution-screenshot.png"),
                max_edge: None,
                full_resolution: true,
            }),
        )
        .expect_err("full-resolution screenshots require approval by default");
        assert!(err.to_string().contains("FullResolutionScreenshot"));
    }

    #[test]
    fn full_resolution_observe_screenshot_fails_closed_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::Observe(ObserveRequest {
                screenshot: Some(ScreenshotRequest {
                    output: temp_test_path("full-resolution-observe.png"),
                    max_edge: None,
                    full_resolution: true,
                }),
            }),
        )
        .expect_err("full-resolution observe screenshots require approval by default");
        assert!(err.to_string().contains("FullResolutionScreenshot"));
    }

    #[test]
    fn allow_full_resolution_screenshot_config_allows_full_resolution_policy() {
        let policy = PolicyEngine::new(policy_config(None, false, false, true));
        enforce_policy(
            &policy,
            &DaemonRequest::Screenshot(ScreenshotRequest {
                output: temp_test_path("allowed-full-resolution-screenshot.png"),
                max_edge: None,
                full_resolution: true,
            }),
        )
        .expect("explicit full-resolution screenshot override allows capture");
        assert_eq!(
            policy_status_from_config(policy.config()).default_full_resolution_screenshot,
            ToolApprovalLevel::Allow
        );
    }

    #[test]
    fn config_file_policy_values_override_defaults() {
        let file_policy = PolicyFileConfig {
            default_observe: Some(ToolApprovalLevel::Deny),
            default_control: Some(ToolApprovalLevel::Deny),
            default_clipboard_read: Some(ToolApprovalLevel::Allow),
            default_clipboard_write: Some(ToolApprovalLevel::Prompt),
            default_full_resolution_screenshot: Some(ToolApprovalLevel::Deny),
        };

        let config = policy_config(Some(&file_policy), false, false, false);

        assert_eq!(config.default_observe, ToolApprovalLevel::Deny);
        assert_eq!(config.default_control, ToolApprovalLevel::Deny);
        assert_eq!(config.default_clipboard_read, ToolApprovalLevel::Allow);
        assert_eq!(config.default_clipboard_write, ToolApprovalLevel::Prompt);
        assert_eq!(
            config.default_full_resolution_screenshot,
            ToolApprovalLevel::Deny
        );
    }

    #[test]
    fn approval_flags_override_config_file_policy_values() {
        let file_policy = PolicyFileConfig {
            default_observe: None,
            default_control: Some(ToolApprovalLevel::Deny),
            default_clipboard_read: Some(ToolApprovalLevel::Deny),
            default_clipboard_write: None,
            default_full_resolution_screenshot: Some(ToolApprovalLevel::Deny),
        };

        let config = policy_config(Some(&file_policy), true, true, true);

        assert_eq!(config.default_control, ToolApprovalLevel::Allow);
        assert_eq!(config.default_clipboard_read, ToolApprovalLevel::Allow);
        assert_eq!(
            config.default_full_resolution_screenshot,
            ToolApprovalLevel::Allow
        );
    }

    #[test]
    fn app_policy_from_config_normalizes_lists() {
        let file_apps = AppsFileConfig {
            allow: Some(vec![
                " org.kde.kate ".to_string(),
                "ORG.KDE.KATE".to_string(),
                "".to_string(),
            ]),
            deny: Some(vec![" org.keepassxc.KeePassXC ".to_string()]),
        };

        let policy = app_policy(Some(&file_apps));

        assert_eq!(policy.allow, vec!["org.kde.kate"]);
        assert_eq!(policy.deny, vec!["org.keepassxc.KeePassXC"]);
    }

    #[test]
    fn app_policy_denies_matching_app() {
        let policy = AppPolicy {
            allow: Vec::new(),
            deny: vec!["org.keepassxc.KeePassXC".to_string()],
        };

        let err =
            enforce_app_policy_for_app(&policy, Some("org.keepassxc.KeePassXC"), "active window")
                .expect_err("deny list blocks matching app");

        assert!(err.to_string().contains("app policy denied active window"));
    }

    #[test]
    fn app_policy_deny_takes_precedence_over_allow() {
        let policy = AppPolicy {
            allow: vec!["org.keepassxc.KeePassXC".to_string()],
            deny: vec!["org.keepassxc.KeePassXC".to_string()],
        };

        let err =
            enforce_app_policy_for_app(&policy, Some("org.keepassxc.KeePassXC"), "active window")
                .expect_err("deny list wins over allow list");

        assert!(err.to_string().contains("app policy denied active window"));
    }

    #[test]
    fn app_policy_allowlist_blocks_unlisted_app() {
        let policy = AppPolicy {
            allow: vec!["org.kde.kate".to_string()],
            deny: Vec::new(),
        };

        let err = enforce_app_policy_for_app(&policy, Some("org.kde.konsole"), "active window")
            .expect_err("allow list blocks unlisted app");

        assert!(
            err.to_string()
                .contains("app policy did not allow active window")
        );
    }

    #[test]
    fn app_policy_fails_closed_without_app_id() {
        let policy = AppPolicy {
            allow: vec!["org.kde.kate".to_string()],
            deny: Vec::new(),
        };

        let err = enforce_app_policy_for_app(&policy, None, "active window")
            .expect_err("configured app policy requires app id");

        assert!(
            err.to_string()
                .contains("could not determine active window app id")
        );
    }

    #[test]
    fn parses_daemon_config_file() {
        let path = temp_test_path("daemon-config.toml");
        fs::write(
            &path,
            r#"
[daemon]
socket = "$XDG_RUNTIME_DIR/plasma-pilot/configured.sock"
journal = "$XDG_STATE_HOME/plasma-pilot/configured.jsonl"
panic_stop_file = "$XDG_RUNTIME_DIR/plasma-pilot/configured-panic-stop"

[policy]
default_observe = "allow"
default_control = "deny"
default_clipboard_read = "allow"
default_clipboard_write = "prompt"
full_resolution_screenshot = "deny"

[apps]
allow = ["org.kde.kate"]
deny = ["org.keepassxc.KeePassXC"]
"#,
        )
        .expect("config fixture is written");

        let config = load_daemon_config(Some(&path)).expect("config file parses");
        let daemon = config.daemon.expect("daemon section is present");
        assert_eq!(
            daemon.socket.as_deref(),
            Some("$XDG_RUNTIME_DIR/plasma-pilot/configured.sock")
        );

        let policy = config.policy.expect("policy section is present");
        assert_eq!(policy.default_control, Some(ToolApprovalLevel::Deny));
        assert_eq!(
            policy.default_full_resolution_screenshot,
            Some(ToolApprovalLevel::Deny)
        );
        let apps = config.apps.expect("apps section is present");
        assert_eq!(
            apps.allow.as_deref(),
            Some(&["org.kde.kate".to_string()][..])
        );
        assert_eq!(
            apps.deny.as_deref(),
            Some(&["org.keepassxc.KeePassXC".to_string()][..])
        );
        fs::remove_file(&path).ok();
    }

    #[test]
    fn configured_path_prefers_cli_over_config() {
        let path = configured_path(
            Some(PathBuf::from("/tmp/plasma-pilot-cli.sock")),
            Some("/tmp/plasma-pilot-config.sock"),
            || Ok(PathBuf::from("/tmp/plasma-pilot-default.sock")),
        )
        .expect("configured path resolves");

        assert_eq!(path, PathBuf::from("/tmp/plasma-pilot-cli.sock"));
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
    fn panic_stop_state_round_trips_file_flag() {
        let path = temp_test_path("panic-stop-state");
        let state = PanicStopState::new(path.clone());

        assert!(!state.status().enabled);
        let status = state.set_enabled(true).expect("panic-stop can be enabled");
        assert!(status.enabled);
        assert_eq!(status.path, path);
        assert!(state.status().enabled);

        let status = state
            .set_enabled(false)
            .expect("panic-stop can be disabled");
        assert!(!status.enabled);
        assert!(!path.exists());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn panic_stop_blocks_control_after_policy_allows_it() {
        let path = temp_test_path("panic-stop-blocks-control");
        fs::write(&path, "enabled").expect("panic-stop fixture file is written");
        let panic_stop = PanicStopState::new(path.clone());
        let policy = PolicyEngine::new(policy_config(None, true, false, false));

        let err = enforce_panic_stop(
            &panic_stop,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                guard: None,
            }),
        )
        .expect_err("panic-stop blocks allowed control");
        assert!(err.to_string().contains("panic-stop is active"));

        enforce_policy(
            &policy,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                guard: None,
            }),
        )
        .expect("control policy is explicitly allowed");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn panic_stop_does_not_block_observe_requests() {
        let path = temp_test_path("panic-stop-observe");
        fs::write(&path, "enabled").expect("panic-stop fixture file is written");
        let panic_stop = PanicStopState::new(path.clone());

        enforce_panic_stop(&panic_stop, &DaemonRequest::ListWindows)
            .expect("panic-stop does not block observe");
        enforce_panic_stop(&panic_stop, &DaemonRequest::PanicStopStatus)
            .expect("panic-stop does not block status");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn active_window_guard_allows_matching_active_window() {
        let state = ActiveWindowState::default();
        state
            .update_from_payload(
                r#"{
                    "active": true,
                    "id": "current-window",
                    "title": "main.rs - Kate",
                    "app_id": "org.kde.kate",
                    "pid": 1234,
                    "geometry": {"x": 10, "y": 20, "width": 800, "height": 600}
                }"#,
            )
            .expect("payload updates active-window state");

        enforce_active_window_guard(
            &state,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: Some(ActiveWindowGuard {
                    expected_window_id: Some("current-window".to_string()),
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: Some("main.rs".to_string()),
                }),
            }),
        )
        .expect("matching active-window guard passes");
    }

    #[test]
    fn active_window_guard_rejects_changed_active_window() {
        let state = ActiveWindowState::default();
        state
            .update_from_payload(
                r#"{
                    "active": true,
                    "id": "other-window",
                    "title": "Terminal",
                    "app_id": "org.kde.konsole"
                }"#,
            )
            .expect("payload updates active-window state");

        let err = enforce_active_window_guard(
            &state,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: Some(ActiveWindowGuard {
                    expected_window_id: Some("current-window".to_string()),
                    expected_app_id: None,
                    title_contains: None,
                }),
            }),
        )
        .expect_err("stale active-window guard fails");
        assert!(err.to_string().contains("active-window guard failed"));
    }

    #[test]
    fn app_policy_blocks_control_for_denied_active_app() {
        let state = ActiveWindowState::default();
        state
            .update_from_payload(
                r#"{
                    "active": true,
                    "id": "secrets-window",
                    "title": "Vault",
                    "app_id": "org.keepassxc.KeePassXC"
                }"#,
            )
            .expect("payload updates active-window state");
        let policy = AppPolicy {
            allow: Vec::new(),
            deny: vec!["org.keepassxc.KeePassXC".to_string()],
        };

        let err = enforce_app_policy(
            &state,
            &policy,
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "should-not-type".to_string(),
                guard: None,
            }),
        )
        .expect_err("denied active app blocks keyboard control");

        assert!(err.to_string().contains("app policy denied active window"));
    }

    #[test]
    fn validates_wait_for_change_request() {
        validate_wait_for_change_request(&WaitForChangeRequest {
            output: temp_test_path("wait-valid.png"),
            max_edge: Some(1600),
            timeout_ms: 1000,
            interval_ms: 100,
            threshold: libplasma_pilot::DEFAULT_WAIT_FOR_CHANGE_THRESHOLD,
        })
        .expect("valid wait request passes");

        let err = validate_wait_for_change_request(&WaitForChangeRequest {
            output: temp_test_path("wait-invalid.png"),
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
    fn wait_for_change_is_observe_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(
            &policy,
            &DaemonRequest::WaitForChange(WaitForChangeRequest {
                output: temp_test_path("wait-policy.png"),
                max_edge: Some(1600),
                timeout_ms: 1000,
                interval_ms: 100,
                threshold: libplasma_pilot::DEFAULT_WAIT_FOR_CHANGE_THRESHOLD,
            }),
        )
        .expect("wait_for_change is observe policy");
    }

    #[test]
    fn parses_kwin_bridge_enabled_from_plugins_group() {
        let config = r#"
            [Other]
            plasma-pilot-bridgeEnabled=false

            [Plugins]
            unrelated=true
            plasma-pilot-bridgeEnabled=true
        "#;
        assert_eq!(parse_kwin_bridge_enabled(config), Some(true));
        assert_eq!(
            parse_kwin_bridge_enabled("[Plugins]\nplasma-pilot-bridgeEnabled=off\n"),
            Some(false)
        );
        assert_eq!(
            parse_kwin_bridge_enabled("[Plugins]\nunrelated=true\n"),
            None
        );
    }

    #[test]
    fn kwin_bridge_status_is_observe_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(&policy, &DaemonRequest::KwinBridgeStatus)
            .expect("kwin bridge status is observe policy");
    }

    #[test]
    fn uinput_status_is_policy_class() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(&policy, &DaemonRequest::UinputStatus)
            .expect("uinput status is allowed as policy diagnostics");
        enforce_policy(&policy, &DaemonRequest::InputBackendStatus)
            .expect("input backend status is allowed as policy diagnostics");
        enforce_policy(&policy, &DaemonRequest::PointerCalibration)
            .expect("pointer calibration is allowed as policy diagnostics");
    }

    #[test]
    fn uinput_setup_hint_reports_access_state() {
        assert_eq!(
            uinput_setup_hint(true, true, true),
            "uinput available to daemon process"
        );
        assert!(uinput_setup_hint(false, false, false).contains("load the uinput kernel module"));
        assert!(uinput_setup_hint(false, true, false).contains("not a character device"));
        assert!(
            uinput_setup_hint(false, true, true).contains("grant the daemon read/write access")
        );
    }

    #[test]
    fn input_backend_preference_uses_portal_libei_then_uinput() {
        let portal = remote_desktop_status(true);
        let libei = libei_status_fixture(true, false);
        assert_eq!(
            preferred_input_backend(&portal, &libei, true).as_deref(),
            Some("portal_remote_desktop")
        );

        let portal = remote_desktop_status(false);
        assert_eq!(
            preferred_input_backend(&portal, &libei, true).as_deref(),
            Some("libei")
        );

        let libei = libei_status_fixture(false, false);
        assert_eq!(
            preferred_input_backend(&portal, &libei, true).as_deref(),
            Some("uinput")
        );
        assert_eq!(preferred_input_backend(&portal, &libei, false), None);
    }

    #[test]
    fn input_backend_setup_hints_report_missing_probe_paths() {
        let portal = RemoteDesktopPortalStatus {
            busctl_available: false,
            portal_service_available: false,
            remote_desktop_interface_available: false,
            kde_portal_service_available: false,
            setup_hint: String::new(),
        };
        let libei = libei_status_fixture(false, false);
        let hint = input_backend_setup_hint(None, &portal, &libei, false);
        assert!(hint.contains("busctl"));

        assert!(remote_desktop_portal_setup_hint(false, false, false, false).contains("busctl"));
        assert!(
            remote_desktop_portal_setup_hint(true, true, false, true)
                .contains("did not introspect")
        );
        assert!(libei_setup_hint(false, false, false).contains("pkg-config"));
        assert!(libei_setup_hint(true, false, true).contains("LIBEI_SOCKET"));
    }

    #[test]
    fn focus_window_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                guard: None,
            }),
        )
        .expect_err("focus requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn keyboard_input_is_control_keyboard_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
            }),
        )
        .expect_err("type_text requires keyboard control approval by default");
        assert!(err.to_string().contains("ControlKeyboard"));

        let err = enforce_policy(
            &policy,
            &DaemonRequest::KeyCombo(KeyComboRequest {
                combo: "Ctrl+L".to_string(),
                guard: None,
            }),
        )
        .expect_err("key_combo requires keyboard control approval by default");
        assert!(err.to_string().contains("ControlKeyboard"));
    }

    #[test]
    fn pointer_input_is_control_pointer_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::MovePointer(MovePointerRequest {
                point: physical_point(3840.0, 2160.0),
                guard: None,
            }),
        )
        .expect_err("move pointer requires pointer control approval by default");
        assert!(err.to_string().contains("ControlPointer"));

        let err = enforce_policy(
            &policy,
            &DaemonRequest::ClickPointer(ClickPointerRequest {
                point: physical_point(100.0, 200.0),
                button: PointerButton::Left,
                clicks: 1,
                guard: None,
            }),
        )
        .expect_err("click pointer requires pointer control approval by default");
        assert!(err.to_string().contains("ControlPointer"));

        let err = enforce_policy(
            &policy,
            &DaemonRequest::ScrollPointer(ScrollPointerRequest {
                vertical: -1,
                horizontal: 0,
                guard: None,
            }),
        )
        .expect_err("scroll pointer requires pointer control approval by default");
        assert!(err.to_string().contains("ControlPointer"));
    }

    #[test]
    fn panic_stop_blocks_keyboard_input_after_policy_allows_it() {
        let path = temp_test_path("panic-stop-blocks-keyboard");
        fs::write(&path, "enabled").expect("panic-stop fixture file is written");
        let panic_stop = PanicStopState::new(path.clone());

        let err = enforce_panic_stop(
            &panic_stop,
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
            }),
        )
        .expect_err("panic-stop blocks keyboard control");
        assert!(err.to_string().contains("panic-stop is active"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn panic_stop_blocks_pointer_input_after_policy_allows_it() {
        let path = temp_test_path("panic-stop-blocks-pointer");
        fs::write(&path, "enabled").expect("panic-stop fixture file is written");
        let panic_stop = PanicStopState::new(path.clone());

        let err = enforce_panic_stop(
            &panic_stop,
            &DaemonRequest::ClickPointer(ClickPointerRequest {
                point: physical_point(100.0, 200.0),
                button: PointerButton::Left,
                clicks: 1,
                guard: None,
            }),
        )
        .expect_err("panic-stop blocks pointer control");
        assert!(err.to_string().contains("panic-stop is active"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn physical_pointer_bounds_cover_8k_and_scaled_negative_origins() {
        let bounds = physical_pointer_bounds_from_monitors(&[
            monitor("left", -1920, 0, 1920, 1080, 1920, 1080, 1.0),
            monitor("main-8k", 0, 0, 7680, 4320, 3840, 2160, 2.0),
        ])
        .expect("monitor union maps to physical pointer bounds");

        assert_eq!(bounds.min_x, -1920);
        assert_eq!(bounds.min_y, 0);
        assert_eq!(bounds.width, 9600);
        assert_eq!(bounds.height, 4320);
    }

    #[test]
    fn validates_physical_pointer_points() {
        let bounds = plasma_pilot_uinput::PointerBounds {
            min_x: -1920,
            min_y: 0,
            width: 9600,
            height: 4320,
        };

        validate_pointer_point(physical_point(-1920.0, 0.0), bounds)
            .expect("minimum physical point is valid");
        validate_pointer_point(physical_point(7679.0, 4319.0), bounds)
            .expect("maximum physical point is valid");

        let err = validate_pointer_point(
            Point {
                x: 10.0,
                y: 10.0,
                space: CoordinateSpace::LogicalPixel,
            },
            bounds,
        )
        .expect_err("logical coordinate space is rejected for now");
        assert!(err.to_string().contains("physical_pixel"));

        let err = validate_pointer_point(physical_point(7680.0, 4319.0), bounds)
            .expect_err("out-of-bounds coordinate is rejected");
        assert!(err.to_string().contains("outside physical desktop bounds"));
    }

    #[test]
    fn pointer_calibration_reports_monitor_physical_mapping() {
        let status = pointer_calibration_status_from_monitors(&[
            monitor("left", -1920, 0, 1920, 1080, 1920, 1080, 1.0),
            monitor("main-8k", 0, 0, 7680, 4320, 5120, 2880, 1.5),
        ])
        .expect("calibration maps monitors");

        assert_eq!(status.coordinate_space, CoordinateSpace::PhysicalPixel);
        assert_eq!(
            status.bounds,
            PointerPhysicalBounds {
                min_x: -1920,
                min_y: 0,
                max_x: 7679,
                max_y: 4319,
                width: 9600,
                height: 4320,
            }
        );
        assert_eq!(status.monitors.len(), 2);
        assert_eq!(status.monitors[0].physical_origin_x, -1920);
        assert_eq!(status.monitors[1].physical_origin_x, 0);
        assert_eq!(
            status.sample_points,
            vec![
                PointerCalibrationPoint {
                    label: "top_left".to_string(),
                    x: -1920,
                    y: 0,
                },
                PointerCalibrationPoint {
                    label: "center".to_string(),
                    x: 2880,
                    y: 2160,
                },
                PointerCalibrationPoint {
                    label: "bottom_right".to_string(),
                    x: 7679,
                    y: 4319,
                },
            ]
        );
    }

    #[test]
    fn accessibility_invoke_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityInvoke(AccessibilityInvokeRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                action: libplasma_pilot::AccessibilityAction::Press,
                guard: None,
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
                guard: None,
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
                guard: None,
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
                guard: None,
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
                guard: None,
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
                guard: None,
            }),
        )
        .expect_err("select menu requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn panic_stop_requests_are_policy_class() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(&policy, &DaemonRequest::PanicStopStatus)
            .expect("panic-stop status is allowed by policy");
        enforce_policy(
            &policy,
            &DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: true }),
        )
        .expect("panic-stop mutation is policy class and journaled by daemon");
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
        let policy = PolicyEngine::new(policy_config(None, true, false, false));
        enforce_policy(
            &policy,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                guard: None,
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
        let policy = PolicyEngine::new(policy_config(None, false, true, false));
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
        let bounded = bound_clipboard_text("abécd".to_string(), Some(4), "test".to_string());
        assert_eq!(bounded.text, "abé");
        assert!(bounded.truncated);
        assert_eq!(bounded.original_bytes, 6);
        assert_eq!(bounded.backend, "test");
    }

    #[test]
    fn clipboard_text_can_be_unbounded() {
        let bounded = bound_clipboard_text("hello".to_string(), None, "test".to_string());
        assert_eq!(bounded.text, "hello");
        assert!(!bounded.truncated);
        assert_eq!(bounded.original_bytes, 5);
        assert_eq!(bounded.backend, "test");
    }

    #[test]
    fn clipboard_backend_prefers_wl_then_klipper() {
        assert_eq!(
            clipboard_read_backend_from_availability(true, true),
            Some(ClipboardBackend::WlClipboard)
        );
        assert_eq!(
            clipboard_read_backend_from_availability(false, true),
            Some(ClipboardBackend::KdeKlipper)
        );
        assert_eq!(clipboard_read_backend_from_availability(false, false), None);
        assert_eq!(
            clipboard_write_backend_from_availability(true, true),
            Some(ClipboardBackend::WlClipboard)
        );
        assert_eq!(
            clipboard_write_backend_from_availability(false, true),
            Some(ClipboardBackend::KdeKlipper)
        );
        assert_eq!(
            clipboard_write_backend_from_availability(false, false),
            None
        );
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

    fn physical_point(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            space: CoordinateSpace::PhysicalPixel,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn monitor(
        id: &str,
        logical_origin_x: i32,
        logical_origin_y: i32,
        physical_width: u32,
        physical_height: u32,
        logical_width: u32,
        logical_height: u32,
        scale_factor: f64,
    ) -> libplasma_pilot::MonitorInfo {
        libplasma_pilot::MonitorInfo {
            id: id.to_string(),
            name: Some(id.to_string()),
            physical_width,
            physical_height,
            logical_width,
            logical_height,
            scale_factor,
            logical_origin_x,
            logical_origin_y,
            transform: None,
        }
    }

    fn remote_desktop_status(available: bool) -> RemoteDesktopPortalStatus {
        RemoteDesktopPortalStatus {
            busctl_available: true,
            portal_service_available: available,
            remote_desktop_interface_available: available,
            kde_portal_service_available: available,
            setup_hint: String::new(),
        }
    }

    fn libei_status_fixture(
        client_library_available: bool,
        socket_env_present: bool,
    ) -> LibeiStatus {
        LibeiStatus {
            pkg_config_available: client_library_available,
            client_library_available,
            socket_env_present,
            setup_hint: String::new(),
        }
    }

    fn temp_test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "plasma-pilot-{name}-{}-{}",
            std::process::id(),
            unix_time_ms().expect("time is available")
        ))
    }
}
