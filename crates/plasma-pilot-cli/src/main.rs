use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use libplasma_pilot::{
    AccessibilityAction, AccessibilityCopyTextRequest, AccessibilityCutTextRequest,
    AccessibilityDeleteTextRequest, AccessibilityFindRequest, AccessibilityInsertTextRequest,
    AccessibilityInvokeRequest, AccessibilityPasteTextRequest, AccessibilitySetTextRequest,
    ActivateLinkRequest, ActivateTabRequest, ActiveWindowGuard, ClickButtonRequest,
    ClickPointerRequest, ClipboardGetRequest, ClipboardSetRequest, CoordinateSpace,
    DEFAULT_CLIPBOARD_MAX_BYTES, DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS,
    DEFAULT_WAIT_FOR_CHANGE_THRESHOLD, DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS, DaemonRequest,
    DaemonResponse, DragPointerRequest, FocusWindowRequest, FocusedAccessibilityTreeRequest,
    JournalTailRequest, KeyComboRequest, MovePointerRequest, ObserveRequest, Point, PointerButton,
    ReplayTrace, ScreenshotRequest, ScreenshotTileRequest, ScrollPointerRequest, SelectMenuRequest,
    SetPanicStopRequest, SetTextFieldRequest, SetValueRequest, ToggleCheckRequest, TypeTextRequest,
    WaitForChangeRequest, default_socket_path,
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
    KwinBridgeStatus,
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
    WaitForChange {
        #[arg(long)]
        output: String,
        #[arg(long, default_value_t = 1600)]
        max_edge: u32,
        #[arg(long, default_value_t = DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS)]
        timeout_ms: u64,
        #[arg(long, default_value_t = DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS)]
        interval_ms: u64,
        #[arg(long, default_value_t = DEFAULT_WAIT_FOR_CHANGE_THRESHOLD)]
        threshold: f64,
    },
    Windows,
    ActiveWindow,
    Focus {
        #[arg(long)]
        window: String,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    Clipboard {
        #[command(subcommand)]
        command: ClipboardCommand,
    },
    Input {
        #[command(subcommand)]
        command: InputCommand,
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
enum InputCommand {
    Status,
    Backends,
    PointerCalibration,
    MovePointer {
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long)]
        coordinate_space: CoordinateSpace,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    ClickPointer {
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long)]
        coordinate_space: CoordinateSpace,
        #[arg(long)]
        button: PointerButton,
        #[arg(long, default_value_t = 1)]
        clicks: u8,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    DragPointer {
        #[arg(long)]
        from_x: f64,
        #[arg(long)]
        from_y: f64,
        #[arg(long)]
        to_x: f64,
        #[arg(long)]
        to_y: f64,
        #[arg(long)]
        coordinate_space: CoordinateSpace,
        #[arg(long, default_value = "left")]
        button: PointerButton,
        #[arg(long, default_value_t = 250)]
        duration_ms: u64,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    ScrollPointer {
        #[arg(long, default_value_t = 0)]
        vertical: i32,
        #[arg(long, default_value_t = 0)]
        horizontal: i32,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    TypeText {
        #[arg(value_name = "TEXT")]
        text: String,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    KeyCombo {
        #[arg(value_name = "COMBO")]
        combo: String,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
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
        #[arg(long)]
        destructive: bool,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    SetText {
        #[arg(long)]
        node: String,
        #[arg(value_name = "TEXT")]
        text: String,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    InsertText {
        #[arg(long)]
        node: String,
        #[arg(long)]
        offset: i32,
        #[arg(value_name = "TEXT")]
        text: String,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    DeleteText {
        #[arg(long)]
        node: String,
        #[arg(long)]
        start_offset: i32,
        #[arg(long)]
        end_offset: i32,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    CopyText {
        #[arg(long)]
        node: String,
        #[arg(long)]
        start_offset: i32,
        #[arg(long)]
        end_offset: i32,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    CutText {
        #[arg(long)]
        node: String,
        #[arg(long)]
        start_offset: i32,
        #[arg(long)]
        end_offset: i32,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    PasteText {
        #[arg(long)]
        node: String,
        #[arg(long)]
        offset: i32,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum SemanticCommand {
    ClickButton {
        #[arg(long)]
        name: String,
        #[arg(long)]
        destructive: bool,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
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
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
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
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    ActivateLink {
        #[arg(long)]
        name: String,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    ToggleCheck {
        #[arg(long)]
        name: String,
        #[arg(long)]
        checked: Option<bool>,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    SetValue {
        #[arg(long)]
        name: String,
        #[arg(long)]
        value: f64,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    SelectMenu {
        #[arg(long)]
        path: String,
        #[arg(long)]
        destructive: bool,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        window_name_contains: Option<String>,
        #[arg(long, default_value_t = 1024)]
        max_nodes: usize,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
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
        Command::KwinBridgeStatus => {
            print_daemon_response(&socket, DaemonRequest::KwinBridgeStatus)?;
        }
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
        Command::WaitForChange {
            output,
            max_edge,
            timeout_ms,
            interval_ms,
            threshold,
        } => print_daemon_response(
            &socket,
            DaemonRequest::WaitForChange(WaitForChangeRequest {
                output: output.into(),
                max_edge: Some(max_edge),
                timeout_ms,
                interval_ms,
                threshold,
            }),
        )?,
        Command::Windows => print_daemon_response(&socket, DaemonRequest::ListWindows)?,
        Command::ActiveWindow => print_daemon_response(&socket, DaemonRequest::ActiveWindow)?,
        Command::Focus {
            window,
            expected_active_window,
            expected_active_app,
            active_title_contains,
        } => print_daemon_response(
            &socket,
            DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: window,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
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
        Command::Input {
            command: InputCommand::Status,
        } => print_daemon_response(&socket, DaemonRequest::UinputStatus)?,
        Command::Input {
            command: InputCommand::Backends,
        } => print_daemon_response(&socket, DaemonRequest::InputBackendStatus)?,
        Command::Input {
            command: InputCommand::PointerCalibration,
        } => print_daemon_response(&socket, DaemonRequest::PointerCalibration)?,
        Command::Input {
            command:
                InputCommand::MovePointer {
                    x,
                    y,
                    coordinate_space,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::MovePointer(MovePointerRequest {
                point: Point {
                    x,
                    y,
                    space: coordinate_space,
                },
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Input {
            command:
                InputCommand::ClickPointer {
                    x,
                    y,
                    coordinate_space,
                    button,
                    clicks,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ClickPointer(ClickPointerRequest {
                point: Point {
                    x,
                    y,
                    space: coordinate_space,
                },
                button,
                clicks,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Input {
            command:
                InputCommand::DragPointer {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    coordinate_space,
                    button,
                    duration_ms,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::DragPointer(DragPointerRequest {
                from: Point {
                    x: from_x,
                    y: from_y,
                    space: coordinate_space,
                },
                to: Point {
                    x: to_x,
                    y: to_y,
                    space: coordinate_space,
                },
                button,
                duration_ms,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Input {
            command:
                InputCommand::ScrollPointer {
                    vertical,
                    horizontal,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ScrollPointer(ScrollPointerRequest {
                vertical,
                horizontal,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Input {
            command:
                InputCommand::TypeText {
                    text,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::TypeText(TypeTextRequest {
                text,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Input {
            command:
                InputCommand::KeyCombo {
                    combo,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::KeyCombo(KeyComboRequest {
                combo,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
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
            command:
                AtspiCommand::Invoke {
                    node,
                    action,
                    destructive,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityInvoke(AccessibilityInvokeRequest {
                node_id: node,
                action,
                destructive,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::SetText {
                    node,
                    text,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilitySetText(AccessibilitySetTextRequest {
                node_id: node,
                text,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::InsertText {
                    node,
                    offset,
                    text,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityInsertText(AccessibilityInsertTextRequest {
                node_id: node,
                offset,
                text,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::DeleteText {
                    node,
                    start_offset,
                    end_offset,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityDeleteText(AccessibilityDeleteTextRequest {
                node_id: node,
                start_offset,
                end_offset,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::CopyText {
                    node,
                    start_offset,
                    end_offset,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityCopyText(AccessibilityCopyTextRequest {
                node_id: node,
                start_offset,
                end_offset,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::CutText {
                    node,
                    start_offset,
                    end_offset,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityCutText(AccessibilityCutTextRequest {
                node_id: node,
                start_offset,
                end_offset,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::PasteText {
                    node,
                    offset,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityPasteText(AccessibilityPasteTextRequest {
                node_id: node,
                offset,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::ClickButton {
                    name,
                    destructive,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ClickButton(ClickButtonRequest {
                name,
                destructive,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
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
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::SetTextField(SetTextFieldRequest {
                name,
                text,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::ActivateTab {
                    name,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ActivateTab(ActivateTabRequest {
                name,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::ActivateLink {
                    name,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ActivateLink(ActivateLinkRequest {
                name,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::ToggleCheck {
                    name,
                    checked,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ToggleCheck(ToggleCheckRequest {
                name,
                checked,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::SetValue {
                    name,
                    value,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::SetValue(SetValueRequest {
                name,
                value,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::SelectMenu {
                    path,
                    destructive,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::SelectMenu(SelectMenuRequest {
                path: parse_menu_path_argument(&path),
                destructive,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
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

fn active_window_guard(
    expected_window_id: Option<String>,
    expected_app_id: Option<String>,
    title_contains: Option<String>,
) -> Option<ActiveWindowGuard> {
    if expected_window_id.is_none() && expected_app_id.is_none() && title_contains.is_none() {
        return None;
    }
    Some(ActiveWindowGuard {
        expected_window_id,
        expected_app_id,
        title_contains,
    })
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
