use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use libplasma_pilot::{
    AccessibilityAction, AccessibilityFindRequest, AccessibilityInvokeRequest,
    AccessibilitySetTextRequest, ActivateTabRequest, ClickButtonRequest, ClipboardGetRequest,
    ClipboardSetRequest, DEFAULT_CLIPBOARD_MAX_BYTES, DaemonRequest, DaemonResponse,
    FocusWindowRequest, FocusedAccessibilityTreeRequest, JournalTailRequest, ObserveRequest,
    ReplayTrace, ScreenshotRequest, ScreenshotTileRequest, SelectMenuRequest, SetPanicStopRequest,
    SetTextFieldRequest, default_socket_path,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot diagnostics and manual control CLI")]
struct Cli {
    #[arg(long, env = "PLASMA_PILOT_SOCKET")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor,
    Capabilities,
    PolicyStatus,
    Monitors,
    Screenshot {
        #[arg(long)]
        output: String,
        #[arg(long, default_value_t = 1600)]
        max_edge: u32,
        #[arg(long)]
        full_resolution: bool,
    },
    ScreenshotTile {
        #[arg(long)]
        output: String,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long, default_value_t = 1600)]
        max_edge: u32,
    },
    Observe {
        #[arg(long)]
        screenshot_output: Option<String>,
        #[arg(long, default_value_t = 1600)]
        max_edge: u32,
        #[arg(long)]
        full_resolution: bool,
    },
    Windows,
    ActiveWindow,
    Focus {
        #[arg(long)]
        window: String,
    },
    Clipboard {
        #[command(subcommand)]
        command: ClipboardCommand,
    },
    Atspi {
        #[command(subcommand)]
        command: AtspiCommand,
    },
    Semantic {
        #[command(subcommand)]
        command: SemanticCommand,
    },
    Journal {
        #[command(subcommand)]
        command: JournalCommand,
    },
    PanicStop {
        #[command(subcommand)]
        command: PanicStopCommand,
    },
    Trace {
        #[command(subcommand)]
        command: TraceCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ClipboardCommand {
    Get {
        #[arg(long, default_value_t = DEFAULT_CLIPBOARD_MAX_BYTES)]
        max_bytes: usize,
        #[arg(long)]
        full: bool,
    },
    Set {
        #[arg(value_name = "TEXT")]
        text: String,
    },
}

#[derive(Debug, Subcommand)]
enum AtspiCommand {
    Tree {
        #[arg(long)]
        focused: bool,
        #[arg(long, default_value_t = 2)]
        depth: usize,
        #[arg(long, default_value_t = 256)]
        max_nodes: usize,
    },
    Find {
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        name_contains: Option<String>,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 0)]
        depth: usize,
        #[arg(long, default_value_t = 10)]
        max_results: usize,
        #[arg(long, default_value_t = 512)]
        max_nodes: usize,
    },
    Invoke {
        #[arg(long)]
        node: String,
        #[arg(long)]
        action: AccessibilityAction,
    },
    SetText {
        #[arg(long)]
        node: String,
        #[arg(value_name = "TEXT")]
        text: String,
    },
}

#[derive(Debug, Subcommand)]
enum SemanticCommand {
    ClickButton {
        #[arg(long)]
        name: String,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
    },
    SetTextField {
        #[arg(long)]
        name: String,
        #[arg(value_name = "TEXT")]
        text: String,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
    },
    ActivateTab {
        #[arg(long)]
        name: String,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
    },
    SelectMenu {
        #[arg(long)]
        path: String,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
    },
}

#[derive(Debug, Subcommand)]
enum JournalCommand {
    Tail {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        method: Option<String>,
        #[arg(long)]
        ok: Option<bool>,
    },
}

#[derive(Debug, Subcommand)]
enum PanicStopCommand {
    Status,
    Enable,
    Disable,
}

#[derive(Debug, Subcommand)]
enum TraceCommand {
    Replay {
        #[arg(long)]
        file: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = match cli.socket {
        Some(path) => path,
        None => default_socket_path().context("resolve default socket path")?,
    };

    match cli.command {
        Command::Doctor => print_daemon_response(&socket, DaemonRequest::Health)?,
        Command::Capabilities => print_daemon_response(&socket, DaemonRequest::Capabilities)?,
        Command::PolicyStatus => print_daemon_response(&socket, DaemonRequest::PolicyStatus)?,
        Command::Monitors => print_daemon_response(&socket, DaemonRequest::ListMonitors)?,
        Command::Screenshot {
            output,
            max_edge,
            full_resolution,
        } => {
            print_daemon_response(
                &socket,
                DaemonRequest::Screenshot(ScreenshotRequest {
                    output: output.into(),
                    max_edge: if full_resolution {
                        None
                    } else {
                        Some(max_edge)
                    },
                    full_resolution,
                }),
            )?;
        }
        Command::ScreenshotTile {
            output,
            x,
            y,
            width,
            height,
            max_edge,
        } => {
            print_daemon_response(
                &socket,
                DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
                    output: output.into(),
                    x,
                    y,
                    width,
                    height,
                    max_edge: Some(max_edge),
                }),
            )?;
        }
        Command::Observe {
            screenshot_output,
            max_edge,
            full_resolution,
        } => print_daemon_response(
            &socket,
            DaemonRequest::Observe(ObserveRequest {
                screenshot: screenshot_output.map(|output| ScreenshotRequest {
                    output: output.into(),
                    max_edge: if full_resolution {
                        None
                    } else {
                        Some(max_edge)
                    },
                    full_resolution,
                }),
            }),
        )?,
        Command::Windows => print_daemon_response(&socket, DaemonRequest::ListWindows)?,
        Command::ActiveWindow => print_daemon_response(&socket, DaemonRequest::ActiveWindow)?,
        Command::Focus { window } => print_daemon_response(
            &socket,
            DaemonRequest::FocusWindow(FocusWindowRequest { window_id: window }),
        )?,
        Command::Clipboard {
            command: ClipboardCommand::Get { max_bytes, full },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: if full { None } else { Some(max_bytes) },
            }),
        )?,
        Command::Clipboard {
            command: ClipboardCommand::Set { text },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ClipboardSet(ClipboardSetRequest { text }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::Tree {
                    focused,
                    depth,
                    max_nodes,
                },
        } => {
            if !focused {
                bail!("atspi tree currently requires --focused");
            }
            print_daemon_response(
                &socket,
                DaemonRequest::FocusedAccessibilityTree(FocusedAccessibilityTreeRequest {
                    depth,
                    max_nodes,
                }),
            )?;
        }
        Command::Atspi {
            command:
                AtspiCommand::Find {
                    role,
                    name_contains,
                    app,
                    window_name_contains,
                    depth,
                    max_results,
                    max_nodes,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityFind(AccessibilityFindRequest {
                role,
                name_contains,
                app,
                window_name_contains,
                depth,
                max_results,
                max_nodes,
            }),
        )?,
        Command::Atspi {
            command: AtspiCommand::Invoke { node, action },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityInvoke(AccessibilityInvokeRequest {
                node_id: node,
                action,
            }),
        )?,
        Command::Atspi {
            command: AtspiCommand::SetText { node, text },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilitySetText(AccessibilitySetTextRequest {
                node_id: node,
                text,
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::ClickButton {
                    name,
                    app,
                    window_name_contains,
                    max_nodes,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ClickButton(ClickButtonRequest {
                name,
                app,
                window_name_contains,
                max_nodes,
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::SetTextField {
                    name,
                    text,
                    app,
                    window_name_contains,
                    max_nodes,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::SetTextField(SetTextFieldRequest {
                name,
                text,
                app,
                window_name_contains,
                max_nodes,
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::ActivateTab {
                    name,
                    app,
                    window_name_contains,
                    max_nodes,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ActivateTab(ActivateTabRequest {
                name,
                app,
                window_name_contains,
                max_nodes,
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::SelectMenu {
                    path,
                    app,
                    window_name_contains,
                    max_nodes,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::SelectMenu(SelectMenuRequest {
                path: parse_menu_path_argument(&path),
                app,
                window_name_contains,
                max_nodes,
            }),
        )?,
        Command::Journal {
            command: JournalCommand::Tail { limit, method, ok },
        } => print_daemon_response(
            &socket,
            DaemonRequest::JournalTail(JournalTailRequest {
                limit,
                method_filter: method,
                ok,
            }),
        )?,
        Command::PanicStop {
            command: PanicStopCommand::Status,
        } => print_daemon_response(&socket, DaemonRequest::PanicStopStatus)?,
        Command::PanicStop {
            command: PanicStopCommand::Enable,
        } => print_daemon_response(
            &socket,
            DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: true }),
        )?,
        Command::PanicStop {
            command: PanicStopCommand::Disable,
        } => print_daemon_response(
            &socket,
            DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: false }),
        )?,
        Command::Trace {
            command: TraceCommand::Replay { file },
        } => replay_trace(&socket, file)?,
    }

    Ok(())
}

fn parse_menu_path_argument(path: &str) -> Vec<String> {
    path.split(['/', '>'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn print_daemon_response(socket: &PathBuf, request: DaemonRequest) -> Result<()> {
    let response = send_request(socket, request)?;
    match response {
        DaemonResponse::Error { message } => bail!("daemon returned error: {message}"),
        response => println!("{}", serde_json::to_string_pretty(&response)?),
    }
    Ok(())
}

fn replay_trace(socket: &PathBuf, file: PathBuf) -> Result<()> {
    let contents =
        fs::read_to_string(&file).with_context(|| format!("read trace {}", file.display()))?;
    let trace: ReplayTrace = serde_json::from_str(&contents)
        .with_context(|| format!("parse trace {}", file.display()))?;
    if trace.version != 1 {
        bail!("unsupported trace version {}", trace.version);
    }

    let mut results = Vec::with_capacity(trace.steps.len());
    for (index, step) in trace.steps.iter().enumerate() {
        let response = send_request(socket, step.request.clone())
            .with_context(|| format!("replay trace step {index}"))?;
        let response_type = response.response_type();
        let ok = response.ok();

        if let Some(expected) = &step.expect_response_type
            && expected != response_type
        {
            bail!("trace step {index} expected response type {expected}, got {response_type}");
        }
        if let Some(expected_ok) = step.expect_ok
            && expected_ok != ok
        {
            bail!("trace step {index} expected ok={expected_ok}, got ok={ok}");
        }

        results.push(serde_json::json!({
            "index": index,
            "label": step.label,
            "method": step.request.method_name(),
            "response_type": response_type,
            "ok": ok,
        }));
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "type": "trace_replay",
            "trace_version": trace.version,
            "description": trace.description,
            "steps": results,
        }))?
    );
    Ok(())
}

fn send_request(socket: &PathBuf, request: DaemonRequest) -> Result<DaemonResponse> {
    let mut stream =
        UnixStream::connect(socket).with_context(|| format!("connect to {}", socket.display()))?;
    let request_line = serde_json::to_string(&request).context("serialize daemon request")?;
    stream
        .write_all(request_line.as_bytes())
        .context("write request")?;
    stream.write_all(b"\n").context("write request newline")?;

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .context("read daemon response")?;
    serde_json::from_str(&response_line).context("parse daemon response")
}
