use std::{
    collections::BTreeSet,
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

mod capture;
mod target;

use anyhow::{Context, Result, bail};
use capture::CaptureCommand;
use clap::{Parser, Subcommand, ValueEnum};
use libseatgeist::{
    AccessibilityAction, AccessibilityCopyTextRequest, AccessibilityCutTextRequest,
    AccessibilityDeleteTextRequest, AccessibilityFindRequest, AccessibilityInsertTextRequest,
    AccessibilityInvokeRequest, AccessibilityPasteTextRequest, AccessibilitySetCaretRequest,
    AccessibilitySetSelectionRequest, AccessibilitySetTextRequest,
    AccessibilityTextAttributesRequest, ActivateLinkRequest, ActivateTabRequest, ActiveWindowGuard,
    ClickButtonRequest, ClickPointerRequest, ClipboardGetRequest, ClipboardSetRequest,
    CloseWindowRequest, CoordinateSpace, DEFAULT_CLIPBOARD_MAX_BYTES,
    DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS, DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS,
    DEFAULT_WAIT_FOR_CHANGE_THRESHOLD, DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS, DaemonClientIdentity,
    DaemonRequest, DaemonRequestEnvelope, DaemonResponse, DragPointerRequest,
    FocusTextFieldRequest, FocusWindowRequest, FocusedAccessibilityTreeRequest, JournalTailRequest,
    KeyComboRequest, LaunchWindowRequest, MovePointerRequest, MoveWindowRequest, ObserveRequest,
    PageZoomOperation, PageZoomRequest, Point, PointerButton, PortalScreenshotTarget,
    RemoteDesktopPersistMode, RemoteDesktopSessionProbeRequest, ReplayTrace, ResizeWindowRequest,
    SafetyClass, ScreenshotRequest, ScreenshotTileRequest, ScrollPointerRequest, SelectItemRequest,
    SelectMenuRequest, SetPanicStopRequest, SetTextFieldRequest, SetValueRequest,
    ToggleCheckRequest, TypeTextRequest, WaitForChangeRequest, WindowActivationMode,
    WindowPlacementAnchor, default_approval_file_path, default_screenshot_output_path,
    default_socket_path,
};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use target::TargetGuardArgs;

const CLIENT_TOOL_NAME: &str = "seatgeist-cli";

#[derive(Debug, Parser)]
#[command(version, about = "Seatgeist diagnostics and manual control CLI")]
struct Cli {
    #[arg(long, env = "SEATGEIST_SOCKET")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor,
    Capabilities,
    PolicyStatus,
    SafetyStatus,
    DesktopSessionStatus,
    Readiness,
    KwinBridgeStatus,
    CaptureBackends,
    Capture {
        #[command(subcommand)]
        command: CaptureCommand,
    },
    Monitors,
    Screenshot {
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        max_edge: Option<u32>,
        #[arg(long)]
        full_resolution: bool,
        #[arg(long)]
        portal_interactive: bool,
        #[arg(long, value_enum)]
        portal_target: Option<CliPortalScreenshotTarget>,
        #[arg(long, value_name = "KWIN_WINDOW_ID")]
        visible_window_crop: Option<String>,
    },
    ScreenshotTile {
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long)]
        max_edge: Option<u32>,
        #[arg(long)]
        portal_interactive: bool,
    },
    Observe {
        #[arg(long)]
        screenshot_output: Option<String>,
        #[arg(long)]
        max_edge: Option<u32>,
        #[arg(long)]
        full_resolution: bool,
        #[arg(long)]
        portal_interactive: bool,
        #[arg(long, value_enum)]
        portal_target: Option<CliPortalScreenshotTarget>,
        #[arg(long, value_name = "KWIN_WINDOW_ID")]
        visible_window_crop: Option<String>,
    },
    WaitForChange {
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        max_edge: Option<u32>,
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
    Close {
        #[arg(long)]
        window: String,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    Resize {
        #[arg(long)]
        window: String,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    Move {
        #[arg(long)]
        window: String,
        #[arg(long)]
        x: i32,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    Launch {
        #[arg(long)]
        desktop_entry: String,
        #[arg(long, value_enum, default_value = "top-left")]
        anchor: CliWindowAnchor,
        #[arg(long)]
        monitor: Option<String>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long, default_value_t = 0)]
        margin: u32,
        #[arg(long, value_enum, default_value = "preserve-focus")]
        activation: CliWindowActivation,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    PageZoom {
        #[arg(long, value_enum)]
        operation: CliPageZoomOperation,
        #[arg(long, default_value_t = 1)]
        steps: u8,
        #[arg(long)]
        expected_active_window: String,
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
    Approve {
        #[arg(long)]
        approval_file: Option<PathBuf>,
        #[arg(long)]
        safety_class: SafetyClass,
        #[arg(long)]
        method: String,
        #[arg(long, default_value_t = 60_000)]
        ttl_ms: u64,
        #[arg(long)]
        reason: Option<String>,
    },
    Trace {
        #[command(subcommand)]
        command: TraceCommand,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliPortalScreenshotTarget {
    Screen,
    Window,
    Area,
    ActiveWindow,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliPageZoomOperation {
    In,
    Out,
    Reset,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliWindowAnchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

impl From<CliWindowAnchor> for WindowPlacementAnchor {
    fn from(value: CliWindowAnchor) -> Self {
        match value {
            CliWindowAnchor::TopLeft => Self::TopLeft,
            CliWindowAnchor::TopRight => Self::TopRight,
            CliWindowAnchor::BottomLeft => Self::BottomLeft,
            CliWindowAnchor::BottomRight => Self::BottomRight,
            CliWindowAnchor::Center => Self::Center,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliWindowActivation {
    PreserveFocus,
    Activate,
}

impl From<CliWindowActivation> for WindowActivationMode {
    fn from(value: CliWindowActivation) -> Self {
        match value {
            CliWindowActivation::PreserveFocus => Self::PreserveFocus,
            CliWindowActivation::Activate => Self::Activate,
        }
    }
}

impl From<CliPageZoomOperation> for PageZoomOperation {
    fn from(value: CliPageZoomOperation) -> Self {
        match value {
            CliPageZoomOperation::In => Self::In,
            CliPageZoomOperation::Out => Self::Out,
            CliPageZoomOperation::Reset => Self::Reset,
        }
    }
}

impl From<CliPortalScreenshotTarget> for PortalScreenshotTarget {
    fn from(value: CliPortalScreenshotTarget) -> Self {
        match value {
            CliPortalScreenshotTarget::Screen => Self::Screen,
            CliPortalScreenshotTarget::Window => Self::Window,
            CliPortalScreenshotTarget::Area => Self::Area,
            CliPortalScreenshotTarget::ActiveWindow => Self::ActiveWindow,
        }
    }
}

#[derive(Debug, Subcommand)]
enum ClipboardCommand {
    Status,
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
    UinputStatus,
    PointerCalibration,
    RemoteDesktopProbe {
        #[arg(long)]
        keyboard: bool,
        #[arg(long)]
        pointer: bool,
        #[arg(long)]
        touchscreen: bool,
        #[arg(long)]
        restore_token: Option<String>,
        #[arg(long)]
        persist_mode: Option<String>,
        #[arg(long)]
        parent_window: Option<String>,
        #[arg(long, default_value_t = DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS)]
        timeout_ms: u64,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    RemoteDesktopEisProbe {
        #[arg(long)]
        keyboard: bool,
        #[arg(long)]
        pointer: bool,
        #[arg(long)]
        touchscreen: bool,
        #[arg(long)]
        restore_token: Option<String>,
        #[arg(long)]
        persist_mode: Option<String>,
        #[arg(long)]
        parent_window: Option<String>,
        #[arg(long, default_value_t = DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS)]
        timeout_ms: u64,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    RemoteDesktopEisStart {
        #[arg(long)]
        keyboard: bool,
        #[arg(long)]
        pointer: bool,
        #[arg(long)]
        touchscreen: bool,
        #[arg(long)]
        restore_token: Option<String>,
        #[arg(long)]
        persist_mode: Option<String>,
        #[arg(long)]
        parent_window: Option<String>,
        #[arg(long, default_value_t = DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS)]
        timeout_ms: u64,
        #[arg(long)]
        expected_active_window: Option<String>,
        #[arg(long)]
        expected_active_app: Option<String>,
        #[arg(long)]
        active_title_contains: Option<String>,
    },
    RemoteDesktopEisSessionStatus,
    RemoteDesktopEisStop,
    MovePointer {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        capture_revision: Option<String>,
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
        session_id: Option<String>,
        #[arg(long)]
        capture_revision: Option<String>,
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
        session_id: Option<String>,
        #[arg(long)]
        capture_revision: Option<String>,
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
        #[arg(long)]
        session_id: Option<String>,
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
        #[arg(long)]
        session_id: Option<String>,
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
        #[arg(long)]
        session_id: Option<String>,
        #[arg(value_name = "COMBO")]
        combo: String,
        #[arg(long)]
        destructive: bool,
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
    QualityStatus,
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
    TextAttributes {
        #[arg(long)]
        node: String,
        #[arg(long)]
        offset: i32,
        #[arg(long)]
        include_defaults: bool,
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
    SetCaret {
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
    SetSelection {
        #[arg(long)]
        node: String,
        #[arg(long, default_value_t = 0)]
        selection_num: i32,
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
        #[command(flatten)]
        target: TargetGuardArgs,
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
        #[command(flatten)]
        target: TargetGuardArgs,
    },
    FocusTextField {
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
        #[command(flatten)]
        target: TargetGuardArgs,
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
        #[command(flatten)]
        target: TargetGuardArgs,
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
        #[command(flatten)]
        target: TargetGuardArgs,
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
        #[command(flatten)]
        target: TargetGuardArgs,
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
        #[command(flatten)]
        target: TargetGuardArgs,
    },
    SelectItem {
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
        #[command(flatten)]
        target: TargetGuardArgs,
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
        #[command(flatten)]
        target: TargetGuardArgs,
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
    Validate {
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file")]
        dir: Option<PathBuf>,
    },
    Replay {
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file")]
        dir: Option<PathBuf>,
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
        Command::SafetyStatus => print_daemon_response(&socket, DaemonRequest::SafetyStatus)?,
        Command::DesktopSessionStatus => {
            print_daemon_response(&socket, DaemonRequest::DesktopSessionStatus)?;
        }
        Command::Readiness => {
            print_daemon_response(&socket, DaemonRequest::ComputerUseReadiness)?;
        }
        Command::KwinBridgeStatus => {
            print_daemon_response(&socket, DaemonRequest::KwinBridgeStatus)?;
        }
        Command::CaptureBackends => {
            print_daemon_response(&socket, DaemonRequest::CaptureBackendStatus)?;
        }
        Command::Capture { command } => {
            print_daemon_response(&socket, command.into_request()?)?;
        }
        Command::Monitors => print_daemon_response(&socket, DaemonRequest::ListMonitors)?,
        Command::Screenshot {
            output,
            max_edge,
            full_resolution,
            portal_interactive,
            portal_target,
            visible_window_crop,
        } => {
            let output = screenshot_output_or_default(output, "screenshot")?;
            print_daemon_response(
                &socket,
                DaemonRequest::Screenshot(ScreenshotRequest {
                    output,
                    max_edge: if full_resolution { None } else { max_edge },
                    full_resolution,
                    portal_interactive,
                    portal_target: portal_target.map(Into::into),
                    visible_window_crop_id: visible_window_crop,
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
            portal_interactive,
        } => {
            let output = screenshot_output_or_default(output, "tile")?;
            print_daemon_response(
                &socket,
                DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
                    output,
                    x,
                    y,
                    width,
                    height,
                    max_edge,
                    portal_interactive,
                }),
            )?;
        }
        Command::Observe {
            screenshot_output,
            max_edge,
            full_resolution,
            portal_interactive,
            portal_target,
            visible_window_crop,
        } => print_daemon_response(
            &socket,
            DaemonRequest::Observe(ObserveRequest {
                screenshot: screenshot_output.map(|output| ScreenshotRequest {
                    output: output.into(),
                    max_edge: if full_resolution { None } else { max_edge },
                    full_resolution,
                    portal_interactive,
                    portal_target: portal_target.map(Into::into),
                    visible_window_crop_id: visible_window_crop,
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
                output: screenshot_output_or_default(output, "wait-for-change")?,
                max_edge,
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
        Command::Close {
            window,
            session_id,
            expected_active_window,
            expected_active_app,
            active_title_contains,
        } => print_daemon_response(
            &socket,
            DaemonRequest::CloseWindow(CloseWindowRequest {
                window_id: window,
                session_id,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Resize {
            window,
            width,
            height,
            expected_active_window,
            expected_active_app,
            active_title_contains,
        } => print_daemon_response(
            &socket,
            DaemonRequest::ResizeWindow(ResizeWindowRequest {
                window_id: window,
                width,
                height,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Move {
            window,
            x,
            y,
            expected_active_window,
            expected_active_app,
            active_title_contains,
        } => print_daemon_response(
            &socket,
            DaemonRequest::MoveWindow(MoveWindowRequest {
                window_id: window,
                x,
                y,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Launch {
            desktop_entry,
            anchor,
            monitor,
            width,
            height,
            margin,
            activation,
            timeout_ms,
            expected_active_window,
            expected_active_app,
            active_title_contains,
        } => print_daemon_response(
            &socket,
            DaemonRequest::LaunchWindow(LaunchWindowRequest {
                desktop_entry,
                anchor: anchor.into(),
                monitor_id: monitor,
                width,
                height,
                margin,
                activation: activation.into(),
                timeout_ms,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::PageZoom {
            operation,
            steps,
            expected_active_window,
            expected_active_app,
            active_title_contains,
        } => print_daemon_response(
            &socket,
            DaemonRequest::PageZoom(PageZoomRequest {
                operation: operation.into(),
                steps,
                guard: active_window_guard(
                    Some(expected_active_window),
                    expected_active_app,
                    active_title_contains,
                )
                .expect("required active-window id creates a guard"),
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
            command: ClipboardCommand::Status,
        } => print_daemon_response(&socket, DaemonRequest::ClipboardBackendStatus)?,
        Command::Clipboard {
            command: ClipboardCommand::Set { text },
        } => print_daemon_response(
            &socket,
            DaemonRequest::ClipboardSet(ClipboardSetRequest { text }),
        )?,
        Command::Input {
            command: InputCommand::Status | InputCommand::Backends,
        } => print_daemon_response(&socket, DaemonRequest::InputBackendStatus)?,
        Command::Input {
            command: InputCommand::UinputStatus,
        } => print_daemon_response(&socket, DaemonRequest::UinputStatus)?,
        Command::Input {
            command: InputCommand::PointerCalibration,
        } => print_daemon_response(&socket, DaemonRequest::PointerCalibration)?,
        Command::Input {
            command:
                InputCommand::RemoteDesktopProbe {
                    keyboard,
                    pointer,
                    touchscreen,
                    restore_token,
                    persist_mode,
                    parent_window,
                    timeout_ms,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => {
            let any_device_flag = keyboard || pointer || touchscreen;
            print_daemon_response(
                &socket,
                DaemonRequest::RemoteDesktopSessionProbe(RemoteDesktopSessionProbeRequest {
                    keyboard: if any_device_flag { keyboard } else { true },
                    pointer: if any_device_flag { pointer } else { true },
                    touchscreen,
                    restore_token,
                    persist_mode: persist_mode
                        .as_deref()
                        .map(parse_remote_desktop_persist_mode)
                        .transpose()?,
                    parent_window,
                    timeout_ms,
                    guard: active_window_guard(
                        expected_active_window,
                        expected_active_app,
                        active_title_contains,
                    ),
                }),
            )?;
        }
        Command::Input {
            command:
                InputCommand::RemoteDesktopEisProbe {
                    keyboard,
                    pointer,
                    touchscreen,
                    restore_token,
                    persist_mode,
                    parent_window,
                    timeout_ms,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => {
            let any_device_flag = keyboard || pointer || touchscreen;
            print_daemon_response(
                &socket,
                DaemonRequest::RemoteDesktopEisProbe(RemoteDesktopSessionProbeRequest {
                    keyboard: if any_device_flag { keyboard } else { true },
                    pointer: if any_device_flag { pointer } else { true },
                    touchscreen,
                    restore_token,
                    persist_mode: persist_mode
                        .as_deref()
                        .map(parse_remote_desktop_persist_mode)
                        .transpose()?,
                    parent_window,
                    timeout_ms,
                    guard: active_window_guard(
                        expected_active_window,
                        expected_active_app,
                        active_title_contains,
                    ),
                }),
            )?;
        }
        Command::Input {
            command:
                InputCommand::RemoteDesktopEisStart {
                    keyboard,
                    pointer,
                    touchscreen,
                    restore_token,
                    persist_mode,
                    parent_window,
                    timeout_ms,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => {
            let any_device_flag = keyboard || pointer || touchscreen;
            print_daemon_response(
                &socket,
                DaemonRequest::RemoteDesktopEisStart(RemoteDesktopSessionProbeRequest {
                    keyboard: if any_device_flag { keyboard } else { true },
                    pointer: if any_device_flag { pointer } else { true },
                    touchscreen,
                    restore_token,
                    persist_mode: persist_mode
                        .as_deref()
                        .map(parse_remote_desktop_persist_mode)
                        .transpose()?,
                    parent_window,
                    timeout_ms,
                    guard: active_window_guard(
                        expected_active_window,
                        expected_active_app,
                        active_title_contains,
                    ),
                }),
            )?;
        }
        Command::Input {
            command: InputCommand::RemoteDesktopEisSessionStatus,
        } => print_daemon_response(&socket, DaemonRequest::RemoteDesktopEisSessionStatus)?,
        Command::Input {
            command: InputCommand::RemoteDesktopEisStop,
        } => print_daemon_response(&socket, DaemonRequest::RemoteDesktopEisStop)?,
        Command::Input {
            command:
                InputCommand::MovePointer {
                    session_id,
                    capture_revision,
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
                capture_revision,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
                session_id,
            }),
        )?,
        Command::Input {
            command:
                InputCommand::ClickPointer {
                    session_id,
                    capture_revision,
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
                capture_revision,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
                session_id,
            }),
        )?,
        Command::Input {
            command:
                InputCommand::DragPointer {
                    session_id,
                    capture_revision,
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
                capture_revision,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
                session_id,
            }),
        )?,
        Command::Input {
            command:
                InputCommand::ScrollPointer {
                    session_id,
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
                session_id,
            }),
        )?,
        Command::Input {
            command:
                InputCommand::TypeText {
                    session_id,
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
                session_id,
            }),
        )?,
        Command::Input {
            command:
                InputCommand::KeyCombo {
                    session_id,
                    combo,
                    destructive,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::KeyCombo(KeyComboRequest {
                combo,
                destructive,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
                session_id,
            }),
        )?,
        Command::Atspi {
            command: AtspiCommand::QualityStatus,
        } => print_daemon_response(&socket, DaemonRequest::AccessibilityQualityStatus)?,
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
                AtspiCommand::TextAttributes {
                    node,
                    offset,
                    include_defaults,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilityTextAttributes(AccessibilityTextAttributesRequest {
                node_id: node,
                offset,
                include_defaults,
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
        Command::Atspi {
            command:
                AtspiCommand::SetCaret {
                    node,
                    offset,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilitySetCaret(AccessibilitySetCaretRequest {
                node_id: node,
                offset,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
            }),
        )?,
        Command::Atspi {
            command:
                AtspiCommand::SetSelection {
                    node,
                    selection_num,
                    start_offset,
                    end_offset,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::AccessibilitySetSelection(AccessibilitySetSelectionRequest {
                node_id: node,
                selection_num,
                start_offset,
                end_offset,
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
                    target,
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
                target_guard: target.into_guard()?,
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
                    target,
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
                target_guard: target.into_guard()?,
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::FocusTextField {
                    name,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                    target,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::FocusTextField(FocusTextFieldRequest {
                name,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
                target_guard: target.into_guard()?,
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
                    target,
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
                target_guard: target.into_guard()?,
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
                    target,
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
                target_guard: target.into_guard()?,
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
                    target,
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
                target_guard: target.into_guard()?,
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
                    target,
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
                target_guard: target.into_guard()?,
            }),
        )?,
        Command::Semantic {
            command:
                SemanticCommand::SelectItem {
                    name,
                    app,
                    window_name_contains,
                    max_nodes,
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                    target,
                },
        } => print_daemon_response(
            &socket,
            DaemonRequest::SelectItem(SelectItemRequest {
                name,
                app,
                window_name_contains,
                max_nodes,
                guard: active_window_guard(
                    expected_active_window,
                    expected_active_app,
                    active_title_contains,
                ),
                target_guard: target.into_guard()?,
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
                    target,
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
                target_guard: target.into_guard()?,
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
        Command::Approve {
            approval_file,
            safety_class,
            method,
            ttl_ms,
            reason,
        } => {
            let approval_file = match approval_file {
                Some(path) => path,
                None => default_approval_file_path().context("resolve default approval file")?,
            };
            write_approval_grant(&approval_file, safety_class, &method, ttl_ms, reason)?;
        }
        Command::Trace { command } => match command {
            TraceCommand::Validate { file, dir } => validate_trace_command(file, dir)?,
            TraceCommand::Replay { file, dir } => replay_trace_command(&socket, file, dir)?,
        },
    }

    Ok(())
}

fn write_approval_grant(
    path: &Path,
    safety_class: SafetyClass,
    method: &str,
    ttl_ms: u64,
    reason: Option<String>,
) -> Result<()> {
    if method.trim().is_empty() {
        bail!("approval method must not be empty");
    }
    if ttl_ms == 0 {
        bail!("approval ttl-ms must be greater than zero");
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("approval file has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create approval dir {}", parent.display()))?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("set approval dir permissions {}", parent.display()))?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("approval file must be a regular file: {}", path.display());
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    }

    let expires_unix_ms = unix_time_ms()?.saturating_add(ttl_ms);
    let grant = serde_json::json!({
        "safety_class": safety_class.clone(),
        "method": method.trim(),
        "expires_unix_ms": expires_unix_ms,
        "reason": reason,
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open approval file {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("set approval file permissions {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&grant)?)
        .with_context(|| format!("write approval file {}", path.display()))?;

    write_stdout_line(&serde_json::to_string_pretty(&serde_json::json!({
        "approval_file": path.display().to_string(),
        "safety_class": safety_class,
        "method": method.trim(),
        "expires_unix_ms": expires_unix_ms,
    }))?)?;
    Ok(())
}

fn unix_time_ms() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before unix epoch")?;
    u64::try_from(duration.as_millis()).context("unix time milliseconds overflowed u64")
}

fn parse_menu_path_argument(path: &str) -> Vec<String> {
    path.split(['/', '>'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_remote_desktop_persist_mode(value: &str) -> Result<RemoteDesktopPersistMode> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "do_not_persist" | "none" | "0" => Ok(RemoteDesktopPersistMode::DoNotPersist),
        "application_lifetime" | "app_lifetime" | "1" => {
            Ok(RemoteDesktopPersistMode::ApplicationLifetime)
        }
        "explicitly_revoked" | "revoked" | "2" => Ok(RemoteDesktopPersistMode::ExplicitlyRevoked),
        other => bail!("unsupported RemoteDesktop persist mode: {other}"),
    }
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
        desktop_revision: None,
        expected_window_id,
        expected_app_id,
        title_contains,
    })
}

fn print_daemon_response(socket: &PathBuf, request: DaemonRequest) -> Result<()> {
    let response = send_request(socket, request)?;
    match response {
        DaemonResponse::Error {
            kind,
            reason_code,
            message,
        } => {
            bail!(
                "daemon returned {kind:?} error reason={}: {message}",
                reason_code.as_deref().unwrap_or("unspecified")
            )
        }
        response => write_stdout_line(&serde_json::to_string_pretty(&response)?)?,
    }
    Ok(())
}

fn write_stdout_line(text: &str) -> Result<()> {
    let stdout = io::stdout();
    write_line_ignoring_broken_pipe(&mut stdout.lock(), text)
}

fn write_line_ignoring_broken_pipe(writer: &mut impl Write, text: &str) -> Result<()> {
    match writeln!(writer, "{text}") {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(err).context("write command output"),
    }
}

fn load_trace(file: &Path) -> Result<ReplayTrace> {
    let contents =
        fs::read_to_string(file).with_context(|| format!("read trace {}", file.display()))?;
    serde_json::from_str(&contents).with_context(|| format!("parse trace {}", file.display()))
}

fn validate_trace(trace: &ReplayTrace) -> Result<()> {
    if trace.version != 1 {
        bail!("unsupported trace version {}", trace.version);
    }
    if trace.steps.is_empty() {
        bail!("trace must contain at least one step");
    }
    let mut labels = BTreeSet::new();
    for (index, step) in trace.steps.iter().enumerate() {
        if let Some(label) = step.label.as_deref() {
            if label.trim().is_empty() {
                bail!(
                    "trace {} label must not be empty",
                    trace_step_context(index, step)
                );
            }
            if !labels.insert(label) {
                bail!(
                    "trace {} duplicates label {label:?}",
                    trace_step_context(index, step)
                );
            }
        }
        if let Some(expected) = &step.expect_response_type
            && !known_response_types().contains(&expected.as_str())
        {
            bail!(
                "trace {} expects unknown response type {expected}",
                trace_step_context(index, step)
            );
        }
        if let Some(expected_error) = &step.expect_error_contains {
            if expected_error.trim().is_empty() {
                bail!(
                    "trace {} expect_error_contains must not be empty",
                    trace_step_context(index, step)
                );
            }
            if let Some(expected_response_type) = &step.expect_response_type
                && expected_response_type != "error"
            {
                bail!(
                    "trace {} expects error text but expect_response_type is {expected_response_type}",
                    trace_step_context(index, step)
                );
            }
            if step.expect_ok == Some(true) {
                bail!(
                    "trace {} expects error text but expect_ok is true",
                    trace_step_context(index, step)
                );
            }
        }
        for expectation in &step.expect_json {
            if expectation.pointer.is_empty() || !expectation.pointer.starts_with('/') {
                bail!(
                    "trace {} JSON expectation pointer must start with '/'",
                    trace_step_context(index, step)
                );
            }
            if expectation.equals.is_none()
                && expectation.value_type.is_none()
                && expectation.value_types.is_empty()
                && expectation.exists.is_none()
            {
                bail!(
                    "trace {} JSON expectation must set equals, value_type, value_types, or exists",
                    trace_step_context(index, step)
                );
            }
            if expectation.exists == Some(false)
                && (expectation.equals.is_some()
                    || expectation.value_type.is_some()
                    || !expectation.value_types.is_empty())
            {
                bail!(
                    "trace {} JSON expectation cannot combine exists=false with equals, value_type, or value_types",
                    trace_step_context(index, step)
                );
            }
            if expectation.value_type.is_some() && !expectation.value_types.is_empty() {
                bail!(
                    "trace {} JSON expectation cannot combine value_type and value_types",
                    trace_step_context(index, step)
                );
            }
            if let Some(expected_type) = &expectation.value_type
                && !known_json_value_types().contains(&expected_type.as_str())
            {
                bail!(
                    "trace {} JSON expectation has unknown value_type {expected_type}",
                    trace_step_context(index, step)
                );
            }
            for expected_type in &expectation.value_types {
                if !known_json_value_types().contains(&expected_type.as_str()) {
                    bail!(
                        "trace {} JSON expectation has unknown value_type {expected_type}",
                        trace_step_context(index, step)
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_trace_command(file: Option<PathBuf>, dir: Option<PathBuf>) -> Result<()> {
    match (file, dir) {
        (Some(file), None) => validate_trace_file(file),
        (None, Some(dir)) => validate_trace_dir(dir),
        (None, None) => bail!("trace validate requires --file <path> or --dir <path>"),
        (Some(_), Some(_)) => bail!("trace validate accepts either --file <path> or --dir <path>"),
    }
}

fn validate_trace_file(file: PathBuf) -> Result<()> {
    let trace = load_trace(&file)?;
    validate_trace(&trace)?;
    let (step_count, steps) = trace_validation_steps(&trace);

    write_stdout_line(&serde_json::to_string_pretty(&serde_json::json!({
        "type": "trace_validation",
        "trace_version": trace.version,
        "description": trace.description,
        "step_count": step_count,
        "steps": steps,
    }))?)?;
    Ok(())
}

fn validate_trace_dir(dir: PathBuf) -> Result<()> {
    let files = trace_files_in_dir(&dir)?;

    let mut total_steps = 0usize;
    let mut traces = Vec::with_capacity(files.len());
    for file in files {
        let trace = load_trace(&file)?;
        validate_trace(&trace).with_context(|| format!("validate trace {}", file.display()))?;
        let (step_count, steps) = trace_validation_steps(&trace);
        total_steps += step_count;
        traces.push(serde_json::json!({
            "file": file.display().to_string(),
            "trace_version": trace.version,
            "description": trace.description,
            "step_count": step_count,
            "steps": steps,
        }));
    }

    write_stdout_line(&serde_json::to_string_pretty(&serde_json::json!({
        "type": "trace_validation_set",
        "dir": dir.display().to_string(),
        "trace_count": traces.len(),
        "step_count": total_steps,
        "traces": traces,
    }))?)?;
    Ok(())
}

fn trace_files_in_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(dir)
        .with_context(|| format!("read trace dir {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("read trace paths in {}", dir.display()))?;
    files.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    files.sort();
    if files.is_empty() {
        bail!("trace dir {} contains no .json traces", dir.display());
    }
    Ok(files)
}

fn trace_validation_steps(trace: &ReplayTrace) -> (usize, Vec<serde_json::Value>) {
    let steps = trace
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            serde_json::json!({
                "index": index,
                "label": step.label,
                "method": step.request.method_name(),
                "expect_response_type": step.expect_response_type,
                "expect_ok": step.expect_ok,
                "expect_error_contains": step.expect_error_contains,
                "expect_json_count": step.expect_json.len(),
            })
        })
        .collect::<Vec<_>>();
    (steps.len(), steps)
}

fn replay_trace_command(
    socket: &PathBuf,
    file: Option<PathBuf>,
    dir: Option<PathBuf>,
) -> Result<()> {
    match (file, dir) {
        (Some(file), None) => replay_trace_file(socket, file),
        (None, Some(dir)) => replay_trace_dir(socket, dir),
        (None, None) => bail!("trace replay requires --file <path> or --dir <path>"),
        (Some(_), Some(_)) => bail!("trace replay accepts either --file <path> or --dir <path>"),
    }
}

fn replay_trace_file(socket: &PathBuf, file: PathBuf) -> Result<()> {
    let trace = load_trace(&file)?;
    validate_trace(&trace)?;
    let results = replay_trace_steps(socket, &trace)?;

    write_stdout_line(&serde_json::to_string_pretty(&serde_json::json!({
        "type": "trace_replay",
        "trace_version": trace.version,
        "description": trace.description,
        "steps": results,
    }))?)?;
    Ok(())
}

fn replay_trace_dir(socket: &PathBuf, dir: PathBuf) -> Result<()> {
    let files = trace_files_in_dir(&dir)?;
    let mut total_steps = 0usize;
    let mut traces = Vec::with_capacity(files.len());
    for file in files {
        let trace = load_trace(&file)?;
        validate_trace(&trace).with_context(|| format!("validate trace {}", file.display()))?;
        let results = replay_trace_steps(socket, &trace)
            .with_context(|| format!("replay trace {}", file.display()))?;
        total_steps += results.len();
        traces.push(serde_json::json!({
            "file": file.display().to_string(),
            "trace_version": trace.version,
            "description": trace.description,
            "step_count": results.len(),
            "steps": results,
        }));
    }

    write_stdout_line(&serde_json::to_string_pretty(&serde_json::json!({
        "type": "trace_replay_set",
        "dir": dir.display().to_string(),
        "trace_count": traces.len(),
        "step_count": total_steps,
        "traces": traces,
    }))?)?;
    Ok(())
}

fn replay_trace_steps(socket: &PathBuf, trace: &ReplayTrace) -> Result<Vec<serde_json::Value>> {
    let mut results = Vec::with_capacity(trace.steps.len());
    for (index, step) in trace.steps.iter().enumerate() {
        let response = send_request(socket, step.request.clone())
            .with_context(|| format!("replay {}", trace_step_context(index, step)))?;
        let response_type = response.response_type();
        let ok = response.ok();

        if let Some(expected) = &step.expect_response_type
            && expected != response_type
        {
            bail!(
                "trace {} expected response type {expected}, got {response_type}",
                trace_step_context(index, step)
            );
        }
        if let Some(expected_ok) = step.expect_ok
            && expected_ok != ok
        {
            bail!(
                "trace {} expected ok={expected_ok}, got ok={ok}",
                trace_step_context(index, step)
            );
        }
        if let Some(expected_error) = &step.expect_error_contains {
            match &response {
                DaemonResponse::Error { message, .. } if message.contains(expected_error) => {}
                DaemonResponse::Error { message, .. } => {
                    bail!(
                        "trace {} expected error containing {expected_error:?}, got {message:?}",
                        trace_step_context(index, step)
                    );
                }
                _ => {
                    bail!(
                        "trace {} expected error containing {expected_error:?}, got response type {response_type}",
                        trace_step_context(index, step)
                    );
                }
            }
        }
        if !step.expect_json.is_empty() {
            let response_value =
                serde_json::to_value(&response).context("serialize daemon response for trace")?;
            for expectation in &step.expect_json {
                let actual = response_value.pointer(&expectation.pointer);
                if expectation.exists == Some(false) {
                    if actual.is_some() {
                        bail!(
                            "trace {} expected JSON pointer {} to be absent",
                            trace_step_context(index, step),
                            expectation.pointer
                        );
                    }
                    continue;
                }

                let Some(actual) = actual else {
                    bail!(
                        "trace {} missing JSON pointer {}",
                        trace_step_context(index, step),
                        expectation.pointer
                    );
                };

                if let Some(expected_type) = &expectation.value_type {
                    let actual_type = json_value_type(actual);
                    if actual_type != expected_type {
                        bail!(
                            "trace {} expected JSON pointer {} to have type {expected_type}, got {actual_type}",
                            trace_step_context(index, step),
                            expectation.pointer
                        );
                    }
                }

                if !expectation.value_types.is_empty() {
                    let actual_type = json_value_type(actual);
                    if !expectation
                        .value_types
                        .iter()
                        .any(|expected_type| expected_type == actual_type)
                    {
                        bail!(
                            "trace {} expected JSON pointer {} to have one of types {}, got {actual_type}",
                            trace_step_context(index, step),
                            expectation.pointer,
                            expectation.value_types.join("/")
                        );
                    }
                }

                if let Some(expected) = &expectation.equals
                    && actual != expected
                {
                    bail!(
                        "trace {} expected JSON pointer {} to match expected value",
                        trace_step_context(index, step),
                        expectation.pointer
                    );
                }
            }
        }

        let error_kind = match &response {
            DaemonResponse::Error { kind, .. } => {
                Some(serde_json::to_value(kind).context("serialize daemon error kind for trace")?)
            }
            _ => None,
        };
        results.push(serde_json::json!({
            "index": index,
            "label": step.label,
            "method": step.request.method_name(),
            "response_type": response_type,
            "ok": ok,
            "error_kind": error_kind,
        }));
    }
    Ok(results)
}

fn known_json_value_types() -> &'static [&'static str] {
    &["null", "boolean", "number", "string", "array", "object"]
}

fn json_value_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn known_response_types() -> &'static [&'static str] {
    &[
        "health",
        "capabilities",
        "policy_status",
        "safety_status",
        "desktop_session_status",
        "computer_use_readiness",
        "panic_stop",
        "kwin_bridge_status",
        "uinput_status",
        "input_backend_status",
        "remote_desktop_session_probe",
        "remote_desktop_eis_probe",
        "remote_desktop_eis_session_status",
        "capture_backend_status",
        "capture_session_status",
        "capture_frame",
        "capture_wait",
        "pointer_calibration",
        "monitors",
        "windows",
        "active_window",
        "observation",
        "screenshot",
        "wait_for_change",
        "clipboard_backend_status",
        "clipboard_text",
        "accessibility_quality_status",
        "accessibility_tree",
        "accessibility_matches",
        "accessibility_text_attributes",
        "journal",
        "action",
        "error",
    ]
}

fn trace_step_context(index: usize, step: &libseatgeist::TraceStep) -> String {
    match step.label.as_deref() {
        Some(label) => format!(
            "step {index} label={label:?} method={}",
            step.request.method_name()
        ),
        None => format!("step {index} method={}", step.request.method_name()),
    }
}

fn send_request(socket: &PathBuf, request: DaemonRequest) -> Result<DaemonResponse> {
    let mut stream =
        UnixStream::connect(socket).with_context(|| format!("connect to {}", socket.display()))?;
    let request_line =
        serde_json::to_string(&request_envelope(request)).context("serialize daemon request")?;
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

fn request_envelope(request: DaemonRequest) -> DaemonRequestEnvelope {
    DaemonRequestEnvelope {
        request,
        client: Some(DaemonClientIdentity {
            tool: Some(CLIENT_TOOL_NAME.to_string()),
        }),
        response_options: None,
    }
}

fn screenshot_output_or_default(output: Option<PathBuf>, kind: &str) -> Result<PathBuf> {
    if let Some(output) = output {
        return Ok(output);
    }

    default_screenshot_output_path(kind).context("resolve default screenshot output path")
}

#[cfg(test)]
mod output_tests {
    use std::io::{self, Write};

    use super::write_line_ignoring_broken_pipe;

    struct BrokenPipeWriter;

    impl Write for BrokenPipeWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn command_output_appends_one_newline() {
        let mut output = Vec::new();
        write_line_ignoring_broken_pipe(&mut output, "{\"ok\":true}").expect("output is written");
        assert_eq!(output, b"{\"ok\":true}\n");
    }

    #[test]
    fn command_output_treats_closed_pipe_as_success() {
        write_line_ignoring_broken_pipe(&mut BrokenPipeWriter, "ignored")
            .expect("broken pipe is a normal pipeline termination");
    }
}
