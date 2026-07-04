use std::{
    fs,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use libplasma_pilot::{
    BackendCapability, CapabilitySet, DaemonRequest, DaemonResponse, HealthStatus, PolicyStatus,
    ToolApprovalLevel, current_euid, default_socket_path,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};
use tracing::{error, info, warn};

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
    prepare_socket_path(&socket)?;
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("bind daemon socket at {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("set socket permissions on {}", socket.display()))?;
    validate_socket_permissions(&socket)?;

    info!(socket = %socket.display(), "plasma-pilotd listening");

    loop {
        let (stream, _addr) = listener.accept().await.context("accept client")?;
        tokio::spawn(async move {
            if let Err(err) = handle_client(stream).await {
                warn!(error = %err, "client request failed");
            }
        });
    }
}

async fn handle_client(stream: UnixStream) -> Result<()> {
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
    let response = handle_request(request);
    let mut stream = reader.into_inner();
    let response_line = serde_json::to_string(&response).context("serialize daemon response")?;
    stream
        .write_all(response_line.as_bytes())
        .await
        .context("write response")?;
    stream.write_all(b"\n").await.context("write newline")?;
    Ok(())
}

fn handle_request(request: DaemonRequest) -> DaemonResponse {
    match request {
        DaemonRequest::Health => DaemonResponse::Health(health()),
        DaemonRequest::Capabilities => DaemonResponse::Capabilities(capabilities()),
        DaemonRequest::PolicyStatus => DaemonResponse::PolicyStatus(policy_status()),
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
        capabilities: vec![
            BackendCapability::DaemonHealth,
            BackendCapability::DaemonPolicyStatus,
        ],
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
