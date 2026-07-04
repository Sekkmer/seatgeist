use std::{
    fs,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{Context, Error, Result, bail};
use clap::Parser;
use image::{GenericImageView, imageops::FilterType};
use libplasma_pilot::{
    BackendCapability, CapabilitySet, CoordinateSpace, DaemonRequest, DaemonResponse, HealthStatus,
    PolicyStatus, ScreenshotInfo, ScreenshotRequest, ScreenshotTileRequest, ScreenshotTransform,
    ToolApprovalLevel, WindowGeometry, WindowInfo, current_euid, default_socket_path,
};
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};
use tracing::{error, info, warn};

static SCREENSHOT_CAPTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

    run(socket).await
}

async fn run(socket: PathBuf) -> Result<()> {
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
        tokio::spawn(async move {
            if let Err(err) = handle_client(stream, active_window_state).await {
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

async fn handle_client(stream: UnixStream, active_window_state: ActiveWindowState) -> Result<()> {
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
    let response = handle_request(request, &active_window_state);
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
) -> DaemonResponse {
    match request {
        DaemonRequest::Health => DaemonResponse::Health(health()),
        DaemonRequest::Capabilities => DaemonResponse::Capabilities(capabilities()),
        DaemonRequest::PolicyStatus => DaemonResponse::PolicyStatus(policy_status()),
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

fn policy_status() -> PolicyStatus {
    PolicyStatus {
        default_observe: ToolApprovalLevel::Allow,
        default_control: ToolApprovalLevel::Prompt,
        default_clipboard_read: ToolApprovalLevel::Prompt,
        default_clipboard_write: ToolApprovalLevel::Allow,
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
}
