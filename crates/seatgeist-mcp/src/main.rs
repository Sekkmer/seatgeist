use std::{
    fs::OpenOptions,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::fs::OpenOptionsExt,
    os::unix::net::UnixStream,
    path::PathBuf,
};

mod capture;
mod readiness;

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use clap::{Parser, ValueEnum};
use libseatgeist::{
    AccessibilityAction, AccessibilityCopyTextRequest, AccessibilityCutTextRequest,
    AccessibilityDeleteTextRequest, AccessibilityFindRequest, AccessibilityInsertTextRequest,
    AccessibilityInvokeRequest, AccessibilityPasteTextRequest, AccessibilitySetCaretRequest,
    AccessibilitySetSelectionRequest, AccessibilitySetTextRequest,
    AccessibilityTextAttributesRequest, ActionSettleCondition, ActivateLinkRequest,
    ActivateTabRequest, ActiveWindowGuard, ClickButtonRequest, ClickPointerRequest,
    ClipboardGetRequest, ClipboardSetRequest, CoordinateSpace, DEFAULT_CLIPBOARD_MAX_BYTES,
    DEFAULT_POST_ACTION_SETTLE_INTERVAL_MS, DEFAULT_POST_ACTION_SETTLE_TIMEOUT_MS,
    DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS, DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS,
    DEFAULT_WAIT_FOR_CHANGE_THRESHOLD, DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS, DaemonClientIdentity,
    DaemonRequest, DaemonRequestEnvelope, DaemonResponse, DaemonResponseOptions,
    DragPointerRequest, FocusTextFieldRequest, FocusWindowRequest, FocusedAccessibilityTreeRequest,
    JournalTailRequest, KeyComboRequest, LaunchWindowRequest, MovePointerRequest,
    MoveWindowRequest, ObserveRequest, PageZoomOperation, PageZoomRequest, Point, PointerButton,
    PortalScreenshotTarget, PostActionImageOptions, PostActionOptions, RemoteDesktopPersistMode,
    RemoteDesktopSessionProbeRequest, ResizeWindowRequest, ScreenshotInfo, ScreenshotRequest,
    ScreenshotTileRequest, ScrollPointerRequest, SelectItemRequest, SelectMenuRequest,
    SetPanicStopRequest, SetTextFieldRequest, SetValueRequest, TargetWindowGuard,
    ToggleCheckRequest, TypeTextRequest, WaitForChangeRequest, WindowActivationMode,
    WindowPlacementAnchor, default_screenshot_output_path, default_socket_path,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "seatgeist";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const CLIENT_TOOL_NAME: &str = "seatgeist-mcp";
const MAX_MCP_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MCP_IMAGE_EDGE: u32 = 2_048;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const SERVER_INSTRUCTIONS: &str = "Seatgeist exposes local KDE Plasma observation and carefully policy-gated control tools. Prefer observe/list/screenshot tools first, keep outputs compact, and expect control tools such as focus_window to fail unless the daemon is started with an explicit approval/control policy.";

#[derive(Debug, Parser)]
#[command(version, about = "Seatgeist MCP stdio server")]
struct Args {
    #[arg(long)]
    stdio: bool,

    #[arg(long, env = "SEATGEIST_SOCKET")]
    socket: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = ToolProfile::All, env = "SEATGEIST_TOOL_PROFILE")]
    tool_profile: ToolProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ToolProfile {
    Core,
    Expert,
    All,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Clone)]
struct McpServer {
    socket: PathBuf,
    tool_profile: ToolProfile,
    readiness_cache: readiness::ReadinessCache,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if !args.stdio {
        bail!("seatgeist-mcp currently supports only --stdio");
    }
    let socket = match args.socket {
        Some(path) => path,
        None => default_socket_path().context("resolve default daemon socket path")?,
    };
    McpServer {
        socket,
        tool_profile: args.tool_profile,
        readiness_cache: readiness::ReadinessCache::default(),
    }
    .run_stdio()
}

impl McpServer {
    fn run_stdio(&self) -> Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        for line in stdin.lock().lines() {
            let line = line.context("read MCP request line")?;
            if line.trim().is_empty() {
                continue;
            }
            match self.handle_line(&line) {
                Ok(Some(response)) => {
                    serde_json::to_writer(&mut stdout, &response).context("write MCP response")?;
                    stdout.write_all(b"\n").context("write response newline")?;
                    stdout.flush().context("flush MCP response")?;
                }
                Ok(None) => {}
                Err(err) => {
                    let response = JsonRpcResponse::error(None, -32603, err.to_string());
                    serde_json::to_writer(&mut stdout, &response)
                        .context("write MCP error response")?;
                    stdout.write_all(b"\n").context("write error newline")?;
                    stdout.flush().context("flush MCP error response")?;
                }
            }
        }
        Ok(())
    }

    fn handle_line(&self, line: &str) -> Result<Option<JsonRpcResponse>> {
        let request = serde_json::from_str::<JsonRpcRequest>(line).context("parse JSON-RPC")?;
        if request.jsonrpc.as_deref() != Some("2.0") {
            return Ok(Some(JsonRpcResponse::error(
                request.id,
                -32600,
                "jsonrpc must be \"2.0\"",
            )));
        }
        if request.id.is_none() {
            return self.handle_notification(request);
        }
        Ok(Some(self.handle_request(request)))
    }

    fn handle_notification(&self, request: JsonRpcRequest) -> Result<Option<JsonRpcResponse>> {
        match request.method.as_str() {
            "notifications/initialized" | "notifications/cancelled" => Ok(None),
            _ => Ok(None),
        }
    }

    fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone();
        let result = match request.method.as_str() {
            "initialize" => Ok(initialize_result(self.tool_profile)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_definitions(self.tool_profile) })),
            "resources/list" => Ok(json!({ "resources": [] })),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "tools/call" => self.handle_tool_call(&request.params),
            _ => {
                return JsonRpcResponse::error(
                    id,
                    -32601,
                    format!("method not found: {}", request.method),
                );
            }
        };

        match result {
            Ok(result) => JsonRpcResponse::result(id, result),
            Err(err) => JsonRpcResponse::error(id, -32602, err.to_string()),
        }
    }

    fn handle_tool_call(&self, params: &Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("tools/call params.name is required"))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !tool_available(self.tool_profile, name) {
            bail!(
                "tool {name} is not available in the {:?} tool profile",
                self.tool_profile
            );
        }
        if name == "seatgeist.computer_status"
            && let Some(result) = self.readiness_cache.get(&self.socket)?
        {
            return Ok(result);
        }
        if name != "seatgeist.computer_status" {
            self.readiness_cache.invalidate()?;
        }
        let request = daemon_request_for_tool(name, &arguments)?;
        let attach_image = image_attachment_requested(&request, &arguments)?;
        let response_options = response_options_for_request(&request, &arguments)?;
        let response = self.send_daemon_request(request, response_options)?;
        let result = tool_result_from_daemon(name, &response, attach_image);
        if name == "seatgeist.computer_status"
            && matches!(response, DaemonResponse::ComputerUseReadiness(_))
        {
            self.readiness_cache.store(&self.socket, result.clone())?;
        }
        Ok(result)
    }

    fn send_daemon_request(
        &self,
        request: DaemonRequest,
        response_options: Option<DaemonResponseOptions>,
    ) -> Result<DaemonResponse> {
        let mut stream = UnixStream::connect(&self.socket)
            .with_context(|| format!("connect to daemon socket {}", self.socket.display()))?;
        let request_line = serde_json::to_string(&request_envelope(request, response_options))
            .context("serialize daemon request")?;
        stream
            .write_all(request_line.as_bytes())
            .context("write daemon request")?;
        stream
            .write_all(b"\n")
            .context("write daemon request newline")?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .context("read daemon response")?;
        serde_json::from_str(&response_line).context("parse daemon response")
    }
}

fn request_envelope(
    request: DaemonRequest,
    response_options: Option<DaemonResponseOptions>,
) -> DaemonRequestEnvelope {
    DaemonRequestEnvelope {
        request,
        client: Some(DaemonClientIdentity {
            tool: Some(CLIENT_TOOL_NAME.to_string()),
        }),
        response_options,
    }
}

impl JsonRpcResponse {
    fn result(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

fn initialize_result(profile: ToolProfile) -> Value {
    let instructions = match profile {
        ToolProfile::Core => {
            "Seatgeist core mode exposes six bounded tools. Call computer_status once when needed, open one window_session for retained capture and sticky raw control, snapshot for one native image, act for exactly one policy-gated action plus its settled observation, wait for bounded visual change, and panic_stop for emergency state."
        }
        ToolProfile::Expert | ToolProfile::All => SERVER_INSTRUCTIONS,
    };
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION
        },
        "instructions": instructions
    })
}

fn daemon_request_for_tool(name: &str, arguments: &Value) -> Result<DaemonRequest> {
    match name {
        "seatgeist.computer_status" => Ok(DaemonRequest::ComputerUseReadiness),
        "seatgeist.window_session" => capture::window_session_request(arguments),
        "seatgeist.snapshot" => capture::core_snapshot_request(arguments),
        "seatgeist.wait" => capture::core_wait_request(arguments),
        "seatgeist.panic_stop" => match optional_bool(arguments, "enabled")? {
            Some(enabled) => Ok(DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled })),
            None => Ok(DaemonRequest::PanicStopStatus),
        },
        "seatgeist.act" => {
            let action = required_string(arguments, "action")?;
            let expert_tool = facade_action_tool(&action)?;
            let sticky_session = optional_string(arguments, "session_id")?;
            let active_guard = active_window_guard(arguments)?;
            if target_window_guard(arguments)?.is_some()
                && !facade_action_supports_target_guard(expert_tool)
            {
                bail!("target-window guards are supported only for semantic seatgeist.act actions");
            }
            if sticky_session.is_some() && !facade_action_supports_sticky_session(expert_tool) {
                bail!(
                    "session_id is supported only for raw keyboard or pointer seatgeist.act actions"
                );
            }
            if sticky_session.is_some() && active_guard.is_some() {
                bail!(
                    "session_id and active-window guards are mutually exclusive for sticky raw seatgeist.act actions"
                );
            }
            daemon_request_for_tool(expert_tool, arguments)
        }
        "seatgeist.health" => Ok(DaemonRequest::Health),
        "seatgeist.capabilities" => Ok(DaemonRequest::Capabilities),
        "seatgeist.policy_status" => Ok(DaemonRequest::PolicyStatus),
        "seatgeist.safety_status" => Ok(DaemonRequest::SafetyStatus),
        "seatgeist.desktop_session_status" => Ok(DaemonRequest::DesktopSessionStatus),
        "seatgeist.computer_use_readiness" => Ok(DaemonRequest::ComputerUseReadiness),
        "seatgeist.panic_stop_status" => Ok(DaemonRequest::PanicStopStatus),
        "seatgeist.panic_stop_enable" => Ok(DaemonRequest::SetPanicStop(SetPanicStopRequest {
            enabled: true,
        })),
        "seatgeist.panic_stop_disable" => Ok(DaemonRequest::SetPanicStop(SetPanicStopRequest {
            enabled: false,
        })),
        "seatgeist.kwin_bridge_status" => Ok(DaemonRequest::KwinBridgeStatus),
        "seatgeist.uinput_status" => Ok(DaemonRequest::UinputStatus),
        "seatgeist.input_backend_status" => Ok(DaemonRequest::InputBackendStatus),
        "seatgeist.remote_desktop_session_probe" => Ok(DaemonRequest::RemoteDesktopSessionProbe(
            remote_desktop_session_request(arguments)?,
        )),
        "seatgeist.remote_desktop_eis_probe" => Ok(DaemonRequest::RemoteDesktopEisProbe(
            remote_desktop_session_request(arguments)?,
        )),
        "seatgeist.remote_desktop_eis_start" => Ok(DaemonRequest::RemoteDesktopEisStart(
            remote_desktop_session_request(arguments)?,
        )),
        "seatgeist.remote_desktop_eis_session_status" => {
            Ok(DaemonRequest::RemoteDesktopEisSessionStatus)
        }
        "seatgeist.remote_desktop_eis_stop" => Ok(DaemonRequest::RemoteDesktopEisStop),
        "seatgeist.capture_backend_status" => Ok(DaemonRequest::CaptureBackendStatus),
        "seatgeist.capture_open" => capture::generic_open_request(arguments),
        "seatgeist.window_capture_open" => capture::open_request(arguments),
        "seatgeist.capture_session_status" => Ok(DaemonRequest::CaptureSessionStatus),
        "seatgeist.capture_session_renew" => capture::renew_request(arguments),
        "seatgeist.capture_snapshot" => capture::expert_snapshot_request(arguments),
        "seatgeist.capture_wait" => capture::expert_wait_request(arguments),
        "seatgeist.capture_session_close" => capture::close_request(arguments),
        "seatgeist.pointer_calibration" => Ok(DaemonRequest::PointerCalibration),
        "seatgeist.list_monitors" => Ok(DaemonRequest::ListMonitors),
        "seatgeist.list_windows" => Ok(DaemonRequest::ListWindows),
        "seatgeist.active_window" => Ok(DaemonRequest::ActiveWindow),
        "seatgeist.observe" => {
            let screenshot = match optional_string(arguments, "screenshot_output")? {
                Some(output) => Some(ScreenshotRequest {
                    output: output.into(),
                    max_edge: optional_u64(arguments, "max_edge")?
                        .map(u64_to_u32)
                        .transpose()?,
                    full_resolution: optional_bool(arguments, "full_resolution")?.unwrap_or(false),
                    portal_interactive: optional_bool(arguments, "portal_interactive")?
                        .unwrap_or(false),
                    portal_target: optional_portal_screenshot_target(arguments, "portal_target")?,
                    visible_window_crop_id: optional_string(arguments, "visible_window_crop_id")?,
                }),
                None => None,
            };
            Ok(DaemonRequest::Observe(ObserveRequest { screenshot }))
        }
        "seatgeist.journal_tail" => Ok(DaemonRequest::JournalTail(JournalTailRequest {
            limit: optional_u64(arguments, "limit")?.unwrap_or(20) as usize,
            method_filter: optional_string(arguments, "method")?,
            ok: optional_bool(arguments, "ok")?,
        })),
        "seatgeist.screenshot" => Ok(DaemonRequest::Screenshot(ScreenshotRequest {
            output: optional_output_path(arguments, "output", "screenshot")?,
            max_edge: optional_u64(arguments, "max_edge")?
                .map(u64_to_u32)
                .transpose()?,
            full_resolution: optional_bool(arguments, "full_resolution")?.unwrap_or(false),
            portal_interactive: optional_bool(arguments, "portal_interactive")?.unwrap_or(false),
            portal_target: optional_portal_screenshot_target(arguments, "portal_target")?,
            visible_window_crop_id: optional_string(arguments, "visible_window_crop_id")?,
        })),
        "seatgeist.screenshot_tile" => Ok(DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
            output: optional_output_path(arguments, "output", "tile")?,
            x: required_u32(arguments, "x")?,
            y: required_u32(arguments, "y")?,
            width: required_u32(arguments, "width")?,
            height: required_u32(arguments, "height")?,
            max_edge: optional_u64(arguments, "max_edge")?
                .map(u64_to_u32)
                .transpose()?,
            portal_interactive: optional_bool(arguments, "portal_interactive")?.unwrap_or(false),
        })),
        "seatgeist.wait_for_change" => Ok(DaemonRequest::WaitForChange(WaitForChangeRequest {
            output: optional_output_path(arguments, "output", "wait-for-change")?,
            max_edge: optional_u64(arguments, "max_edge")?
                .map(u64_to_u32)
                .transpose()?,
            timeout_ms: optional_u64(arguments, "timeout_ms")?
                .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS),
            interval_ms: optional_u64(arguments, "interval_ms")?
                .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS),
            threshold: optional_f64(arguments, "threshold")?
                .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_THRESHOLD),
        })),
        "seatgeist.clipboard_status" => Ok(DaemonRequest::ClipboardBackendStatus),
        "seatgeist.clipboard_get_text" => {
            let full = optional_bool(arguments, "full")?.unwrap_or(false);
            Ok(DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: if full {
                    None
                } else {
                    Some(
                        optional_u64(arguments, "max_bytes")?
                            .map(u64_to_usize)
                            .transpose()?
                            .unwrap_or(DEFAULT_CLIPBOARD_MAX_BYTES),
                    )
                },
            }))
        }
        "seatgeist.clipboard_set_text" => Ok(DaemonRequest::ClipboardSet(ClipboardSetRequest {
            text: required_string(arguments, "text")?,
        })),
        "seatgeist.a11y_quality_status" => Ok(DaemonRequest::AccessibilityQualityStatus),
        "seatgeist.a11y_focused_tree" => Ok(DaemonRequest::FocusedAccessibilityTree(
            FocusedAccessibilityTreeRequest {
                depth: optional_u64(arguments, "depth")?
                    .map(u64_to_usize)
                    .transpose()?
                    .unwrap_or(2),
                max_nodes: optional_u64(arguments, "max_nodes")?
                    .map(u64_to_usize)
                    .transpose()?
                    .unwrap_or(256),
            },
        )),
        "seatgeist.a11y_find" => Ok(DaemonRequest::AccessibilityFind(AccessibilityFindRequest {
            role: optional_string(arguments, "role")?,
            name_contains: optional_string(arguments, "name_contains")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            depth: optional_u64(arguments, "depth")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(0),
            max_results: optional_u64(arguments, "max_results")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(10),
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(512),
        })),
        "seatgeist.a11y_text_attributes" => Ok(DaemonRequest::AccessibilityTextAttributes(
            AccessibilityTextAttributesRequest {
                node_id: required_string(arguments, "node_id")?,
                offset: required_i32(arguments, "offset")?,
                include_defaults: optional_bool(arguments, "include_defaults")?.unwrap_or(false),
            },
        )),
        "seatgeist.a11y_invoke" => Ok(DaemonRequest::AccessibilityInvoke(
            AccessibilityInvokeRequest {
                node_id: required_string(arguments, "node_id")?,
                action: required_accessibility_action(arguments, "action")?,
                destructive: optional_bool(arguments, "destructive")?.unwrap_or(false),
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_set_text" => Ok(DaemonRequest::AccessibilitySetText(
            AccessibilitySetTextRequest {
                node_id: required_string(arguments, "node_id")?,
                text: required_string(arguments, "text")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_insert_text" => Ok(DaemonRequest::AccessibilityInsertText(
            AccessibilityInsertTextRequest {
                node_id: required_string(arguments, "node_id")?,
                offset: required_i32(arguments, "offset")?,
                text: required_string(arguments, "text")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_delete_text" => Ok(DaemonRequest::AccessibilityDeleteText(
            AccessibilityDeleteTextRequest {
                node_id: required_string(arguments, "node_id")?,
                start_offset: required_i32(arguments, "start_offset")?,
                end_offset: required_i32(arguments, "end_offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_copy_text" => Ok(DaemonRequest::AccessibilityCopyText(
            AccessibilityCopyTextRequest {
                node_id: required_string(arguments, "node_id")?,
                start_offset: required_i32(arguments, "start_offset")?,
                end_offset: required_i32(arguments, "end_offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_cut_text" => Ok(DaemonRequest::AccessibilityCutText(
            AccessibilityCutTextRequest {
                node_id: required_string(arguments, "node_id")?,
                start_offset: required_i32(arguments, "start_offset")?,
                end_offset: required_i32(arguments, "end_offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_paste_text" => Ok(DaemonRequest::AccessibilityPasteText(
            AccessibilityPasteTextRequest {
                node_id: required_string(arguments, "node_id")?,
                offset: required_i32(arguments, "offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_set_caret" => Ok(DaemonRequest::AccessibilitySetCaret(
            AccessibilitySetCaretRequest {
                node_id: required_string(arguments, "node_id")?,
                offset: required_i32(arguments, "offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.a11y_set_selection" => Ok(DaemonRequest::AccessibilitySetSelection(
            AccessibilitySetSelectionRequest {
                node_id: required_string(arguments, "node_id")?,
                selection_num: optional_i32(arguments, "selection_num")?.unwrap_or(0),
                start_offset: required_i32(arguments, "start_offset")?,
                end_offset: required_i32(arguments, "end_offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "seatgeist.type_text" => Ok(DaemonRequest::TypeText(TypeTextRequest {
            text: required_string(arguments, "text")?,
            guard: active_window_guard(arguments)?,
            session_id: optional_string(arguments, "session_id")?,
        })),
        "seatgeist.key_combo" => Ok(DaemonRequest::KeyCombo(KeyComboRequest {
            combo: required_string(arguments, "combo")?,
            guard: active_window_guard(arguments)?,
            session_id: optional_string(arguments, "session_id")?,
        })),
        "seatgeist.move_pointer" => Ok(DaemonRequest::MovePointer(MovePointerRequest {
            point: Point {
                x: required_f64(arguments, "x")?,
                y: required_f64(arguments, "y")?,
                space: required_coordinate_space(arguments, "coordinate_space")?,
            },
            guard: active_window_guard(arguments)?,
            session_id: optional_string(arguments, "session_id")?,
        })),
        "seatgeist.click_pointer" => Ok(DaemonRequest::ClickPointer(ClickPointerRequest {
            point: Point {
                x: required_f64(arguments, "x")?,
                y: required_f64(arguments, "y")?,
                space: required_coordinate_space(arguments, "coordinate_space")?,
            },
            button: required_pointer_button(arguments, "button")?,
            clicks: optional_u64(arguments, "clicks")?
                .map(u64_to_u8)
                .transpose()?
                .unwrap_or(1),
            guard: active_window_guard(arguments)?,
            session_id: optional_string(arguments, "session_id")?,
        })),
        "seatgeist.drag_pointer" => {
            let coordinate_space = required_coordinate_space(arguments, "coordinate_space")?;
            Ok(DaemonRequest::DragPointer(DragPointerRequest {
                from: Point {
                    x: required_f64(arguments, "from_x")?,
                    y: required_f64(arguments, "from_y")?,
                    space: coordinate_space,
                },
                to: Point {
                    x: required_f64(arguments, "to_x")?,
                    y: required_f64(arguments, "to_y")?,
                    space: coordinate_space,
                },
                button: optional_pointer_button(arguments, "button")?
                    .unwrap_or(PointerButton::Left),
                duration_ms: optional_u64(arguments, "duration_ms")?.unwrap_or(250),
                guard: active_window_guard(arguments)?,
                session_id: optional_string(arguments, "session_id")?,
            }))
        }
        "seatgeist.scroll_pointer" => Ok(DaemonRequest::ScrollPointer(ScrollPointerRequest {
            vertical: optional_i32(arguments, "vertical")?.unwrap_or(0),
            horizontal: optional_i32(arguments, "horizontal")?.unwrap_or(0),
            guard: active_window_guard(arguments)?,
            session_id: optional_string(arguments, "session_id")?,
        })),
        "seatgeist.click_button" => Ok(DaemonRequest::ClickButton(ClickButtonRequest {
            name: required_string(arguments, "name")?,
            destructive: optional_bool(arguments, "destructive")?.unwrap_or(false),
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.set_text_field" => Ok(DaemonRequest::SetTextField(SetTextFieldRequest {
            name: required_string(arguments, "name")?,
            text: required_string(arguments, "text")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.focus_text_field" => Ok(DaemonRequest::FocusTextField(FocusTextFieldRequest {
            name: required_string(arguments, "name")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.activate_tab" => Ok(DaemonRequest::ActivateTab(ActivateTabRequest {
            name: required_string(arguments, "name")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.activate_link" => Ok(DaemonRequest::ActivateLink(ActivateLinkRequest {
            name: required_string(arguments, "name")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.toggle_check" => Ok(DaemonRequest::ToggleCheck(ToggleCheckRequest {
            name: required_string(arguments, "name")?,
            checked: optional_bool(arguments, "checked")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.set_value" => Ok(DaemonRequest::SetValue(SetValueRequest {
            name: required_string(arguments, "name")?,
            value: required_f64(arguments, "value")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.select_item" => Ok(DaemonRequest::SelectItem(SelectItemRequest {
            name: required_string(arguments, "name")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.select_menu" => Ok(DaemonRequest::SelectMenu(SelectMenuRequest {
            path: required_string_array(arguments, "path")?,
            destructive: optional_bool(arguments, "destructive")?.unwrap_or(false),
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
            target_guard: target_window_guard(arguments)?,
        })),
        "seatgeist.focus_window" => Ok(DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: required_string(arguments, "window_id")?,
            guard: active_window_guard(arguments)?,
        })),
        "seatgeist.resize_window" => Ok(DaemonRequest::ResizeWindow(ResizeWindowRequest {
            window_id: required_string(arguments, "window_id")?,
            width: required_u32(arguments, "width")?,
            height: required_u32(arguments, "height")?,
            guard: active_window_guard(arguments)?,
        })),
        "seatgeist.move_window" => Ok(DaemonRequest::MoveWindow(MoveWindowRequest {
            window_id: required_string(arguments, "window_id")?,
            x: required_i32(arguments, "x")?,
            y: required_i32(arguments, "y")?,
            guard: active_window_guard(arguments)?,
        })),
        "seatgeist.launch_window" => Ok(DaemonRequest::LaunchWindow(LaunchWindowRequest {
            desktop_entry: required_string(arguments, "desktop_entry")?,
            anchor: match required_string(arguments, "anchor")?.as_str() {
                "top_left" => WindowPlacementAnchor::TopLeft,
                "top_right" => WindowPlacementAnchor::TopRight,
                "bottom_left" => WindowPlacementAnchor::BottomLeft,
                "bottom_right" => WindowPlacementAnchor::BottomRight,
                "center" => WindowPlacementAnchor::Center,
                other => bail!("unsupported window placement anchor: {other}"),
            },
            monitor_id: optional_string(arguments, "monitor_id")?,
            width: optional_u32(arguments, "width")?,
            height: optional_u32(arguments, "height")?,
            margin: optional_u32(arguments, "margin")?.unwrap_or(0),
            activation: match optional_string(arguments, "activation")?.as_deref() {
                None | Some("preserve_focus") => WindowActivationMode::PreserveFocus,
                Some("activate") => WindowActivationMode::Activate,
                Some(other) => bail!("unsupported window activation mode: {other}"),
            },
            timeout_ms: optional_u64(arguments, "timeout_ms")?.unwrap_or(10_000),
            guard: active_window_guard(arguments)?,
        })),
        "seatgeist.page_zoom" => Ok(DaemonRequest::PageZoom(PageZoomRequest {
            operation: match required_string(arguments, "operation")?.as_str() {
                "in" => PageZoomOperation::In,
                "out" => PageZoomOperation::Out,
                "reset" => PageZoomOperation::Reset,
                other => bail!("unsupported page zoom operation: {other}"),
            },
            steps: optional_u64(arguments, "steps")?
                .map(u64_to_u8)
                .transpose()?
                .unwrap_or(1),
            guard: active_window_guard(arguments)?
                .ok_or_else(|| anyhow!("page zoom requires an active-window guard"))?,
        })),
        _ => bail!("unknown tool: {name}"),
    }
}

fn facade_action_tool(action: &str) -> Result<&'static str> {
    match action
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "focus_window" => Ok("seatgeist.focus_window"),
        "move_window" => Ok("seatgeist.move_window"),
        "launch_window" => Ok("seatgeist.launch_window"),
        "resize_window" => Ok("seatgeist.resize_window"),
        "page_zoom" => Ok("seatgeist.page_zoom"),
        "type_text" => Ok("seatgeist.type_text"),
        "key_combo" => Ok("seatgeist.key_combo"),
        "click_pointer" => Ok("seatgeist.click_pointer"),
        "scroll_pointer" => Ok("seatgeist.scroll_pointer"),
        "click_button" => Ok("seatgeist.click_button"),
        "set_text_field" => Ok("seatgeist.set_text_field"),
        "focus_text_field" => Ok("seatgeist.focus_text_field"),
        "select_menu" => Ok("seatgeist.select_menu"),
        "activate_tab" => Ok("seatgeist.activate_tab"),
        "activate_link" => Ok("seatgeist.activate_link"),
        "toggle_check" => Ok("seatgeist.toggle_check"),
        "set_value" => Ok("seatgeist.set_value"),
        "select_item" => Ok("seatgeist.select_item"),
        other => bail!("unsupported seatgeist.act action: {other}"),
    }
}

fn facade_action_supports_target_guard(tool: &str) -> bool {
    matches!(
        tool,
        "seatgeist.click_button"
            | "seatgeist.set_text_field"
            | "seatgeist.focus_text_field"
            | "seatgeist.select_menu"
            | "seatgeist.activate_tab"
            | "seatgeist.activate_link"
            | "seatgeist.toggle_check"
            | "seatgeist.set_value"
            | "seatgeist.select_item"
    )
}

fn facade_action_supports_sticky_session(tool: &str) -> bool {
    matches!(
        tool,
        "seatgeist.type_text"
            | "seatgeist.key_combo"
            | "seatgeist.click_pointer"
            | "seatgeist.scroll_pointer"
    )
}

fn tool_result_from_daemon(
    tool_name: &str,
    response: &DaemonResponse,
    attach_image: bool,
) -> Value {
    let structured = serde_json::to_value(response).unwrap_or_else(|err| {
        json!({
            "type": "error",
            "data": {
                "message": format!("serialize daemon response: {err}")
            }
        })
    });
    let is_error = matches!(response, DaemonResponse::Error { .. });
    let mut content = vec![json!({
        "type": "text",
        "text": compact_tool_text(tool_name, response)
    })];
    if attach_image && !is_error {
        match screenshot_info_from_response(response).and_then(mcp_png_image_content) {
            Ok(image) => content.push(image),
            Err(err) => content.push(json!({
                "type": "text",
                "text": format!("image attachment omitted: {err}")
            })),
        }
    }
    json!({
        "content": content,
        "structuredContent": structured,
        "isError": is_error
    })
}

fn image_attachment_requested(request: &DaemonRequest, arguments: &Value) -> Result<bool> {
    let requested = optional_bool(arguments, "include_image")?;
    match request {
        DaemonRequest::Observe(request) => match request.screenshot.as_ref() {
            Some(screenshot) => bounded_image_attachment(screenshot.full_resolution, requested),
            None if requested == Some(true) => {
                bail!("include_image=true requires screenshot_output for seatgeist.observe")
            }
            None => Ok(false),
        },
        DaemonRequest::Screenshot(request) => {
            bounded_image_attachment(request.full_resolution, requested)
        }
        DaemonRequest::ScreenshotTile(_) | DaemonRequest::WaitForChange(_) => {
            Ok(requested.unwrap_or(true))
        }
        DaemonRequest::CaptureSnapshot(_) | DaemonRequest::CaptureWait(_) => {
            Ok(requested.unwrap_or(true))
        }
        request if request.returns_action() => Ok(requested.unwrap_or(false)),
        _ => Ok(false),
    }
}

fn response_options_for_request(
    request: &DaemonRequest,
    arguments: &Value,
) -> Result<Option<DaemonResponseOptions>> {
    if !request_returns_action(request) {
        return Ok(None);
    }
    let observe_after = optional_bool(arguments, "observe_after")?.unwrap_or(true);
    let requested_condition = optional_string(arguments, "settle_condition")?
        .map(|value| {
            value
                .parse::<ActionSettleCondition>()
                .map_err(anyhow::Error::msg)
        })
        .transpose()?;
    if !observe_after {
        if requested_condition.is_some_and(|condition| condition != ActionSettleCondition::None) {
            bail!("observe_after=false requires settle_condition=none or omission");
        }
        if optional_bool(arguments, "include_image")? == Some(true)
            || optional_string(arguments, "capture_session_id")?.is_some()
            || optional_string(arguments, "image_output")?.is_some()
            || optional_u64(arguments, "image_max_edge")?.is_some()
            || optional_u64(arguments, "image_timeout_ms")?.is_some()
        {
            bail!("post-action images require observe_after=true");
        }
        return Ok(None);
    }
    let settle_timeout_ms = optional_u64(arguments, "settle_timeout_ms")?
        .unwrap_or(DEFAULT_POST_ACTION_SETTLE_TIMEOUT_MS);
    let settle_interval_ms = optional_u64(arguments, "settle_interval_ms")?
        .unwrap_or(DEFAULT_POST_ACTION_SETTLE_INTERVAL_MS);
    let include_image = optional_bool(arguments, "include_image")?.unwrap_or(false);
    let capture_session_id = optional_string(arguments, "capture_session_id")?;
    let image_output = optional_string(arguments, "image_output")?;
    let image_max_edge = optional_u64(arguments, "image_max_edge")?
        .map(u64_to_u32)
        .transpose()?;
    let image_timeout_ms = optional_u64(arguments, "image_timeout_ms")?;
    let image_arguments_present = capture_session_id.is_some()
        || image_output.is_some()
        || image_max_edge.is_some()
        || image_timeout_ms.is_some();
    if !include_image && image_arguments_present {
        bail!("post-action image options require include_image=true");
    }
    let image = if include_image {
        let session_id = capture_session_id
            .or(optional_string(arguments, "session_id")?)
            .ok_or_else(|| {
                anyhow!("include_image=true requires capture_session_id or a sticky session_id")
            })?;
        Some(PostActionImageOptions {
            session_id,
            output: match image_output {
                Some(output) => output.into(),
                None => default_screenshot_output_path("post-action")
                    .context("resolve default post-action screenshot output path")?,
            },
            max_edge: image_max_edge,
            timeout_ms: image_timeout_ms.unwrap_or(1_500),
        })
    } else {
        None
    };
    Ok(Some(DaemonResponseOptions {
        post_action: Some(PostActionOptions {
            observe_after,
            settle_condition: requested_condition.unwrap_or(ActionSettleCondition::Auto),
            settle_timeout_ms,
            settle_interval_ms,
            image,
        }),
    }))
}

fn request_returns_action(request: &DaemonRequest) -> bool {
    request.returns_action()
}

fn bounded_image_attachment(full_resolution: bool, requested: Option<bool>) -> Result<bool> {
    if full_resolution {
        if requested == Some(true) {
            bail!(
                "full-resolution screenshots cannot be embedded in MCP responses; inspect the policy-gated output path instead"
            );
        }
        return Ok(false);
    }
    Ok(requested.unwrap_or(true))
}

fn screenshot_info_from_response(response: &DaemonResponse) -> Result<&ScreenshotInfo> {
    let info = match response {
        DaemonResponse::Screenshot(info) => info,
        DaemonResponse::WaitForChange(result) => &result.screenshot,
        DaemonResponse::CaptureFrame(result) => &result.screenshot,
        DaemonResponse::CaptureWait(result) => &result.frame.screenshot,
        DaemonResponse::Observation(observation) => observation
            .screenshot
            .as_ref()
            .ok_or_else(|| anyhow!("daemon response did not include screenshot metadata"))?,
        DaemonResponse::Action(action) => action
            .screenshot
            .as_ref()
            .ok_or_else(|| anyhow!("daemon action response did not include screenshot metadata"))?,
        DaemonResponse::Error { .. } => bail!("daemon returned an error instead of an image"),
        _ => bail!("daemon response does not carry an image"),
    };
    Ok(info)
}

fn mcp_png_image_content(info: &ScreenshotInfo) -> Result<Value> {
    if info.output_width > MAX_MCP_IMAGE_EDGE || info.output_height > MAX_MCP_IMAGE_EDGE {
        bail!(
            "screenshot output exceeds the {} pixel MCP attachment edge limit",
            MAX_MCP_IMAGE_EDGE
        );
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&info.path)
        .context("open screenshot output without following symlinks")?;
    let metadata = file.metadata().context("stat opened screenshot output")?;
    if !metadata.is_file() {
        bail!("screenshot output is not a regular file");
    }
    if metadata.len() > MAX_MCP_IMAGE_BYTES {
        bail!(
            "screenshot output exceeds the {} byte MCP attachment limit",
            MAX_MCP_IMAGE_BYTES
        );
    }

    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    (&mut file)
        .take(MAX_MCP_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("read screenshot output")?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_MCP_IMAGE_BYTES {
        bail!(
            "screenshot output exceeds the {} byte MCP attachment limit",
            MAX_MCP_IMAGE_BYTES
        );
    }
    if !bytes.starts_with(PNG_SIGNATURE) {
        bail!("screenshot output is not a PNG image");
    }

    Ok(json!({
        "type": "image",
        "data": BASE64_STANDARD.encode(bytes),
        "mimeType": "image/png"
    }))
}

fn compact_tool_text(tool_name: &str, response: &DaemonResponse) -> String {
    match response {
        DaemonResponse::Health(status) => {
            format!("{} {} ({})", status.service, status.status, status.version)
        }
        DaemonResponse::Capabilities(capabilities) => {
            format!("{} capabilities", capabilities.capabilities.len())
        }
        DaemonResponse::PolicyStatus(status) => format!(
            "observe={:?} control={:?} destructive_actions={:?} secret_fields={:?} full_resolution_screenshot={:?} clipboard_read={:?} clipboard_write={:?}",
            status.default_observe,
            status.default_control,
            status.default_destructive_actions,
            status.default_secret_fields,
            status.default_full_resolution_screenshot,
            status.default_clipboard_read,
            status.default_clipboard_write
        ),
        DaemonResponse::SafetyStatus(status) => format!(
            "focus_guard={} human_pause={} human_signal_fresh={} human_quiet_ms={} activity_backend={} activity_trusted={} last_activity={} last_provenance={} control_rate_limit_per_minute={} preview_max_edge={} tile_max_edge={} redactions={} journal_artifacts={}",
            status.require_focus_guard,
            status.pause_on_human_input,
            status.human_input_signal_fresh,
            status.human_input_quiet_ms,
            status
                .human_input_activity_backend
                .as_deref()
                .unwrap_or("none"),
            status.human_input_activity_trusted,
            status.human_input_last_class.as_deref().unwrap_or("none"),
            status
                .human_input_last_provenance
                .as_deref()
                .unwrap_or("none"),
            status
                .control_rate_limit_per_minute
                .map(|limit| limit.to_string())
                .unwrap_or_else(|| "disabled".to_string()),
            status.preview_max_edge,
            status.tile_max_edge,
            status.screenshot_redaction_count,
            status.journal_artifact_metadata_enabled
        ),
        DaemonResponse::DesktopSessionStatus(status) => format!(
            "desktop session type={} desktop={} kde={} wayland={} display={} dbus={} runtime={}",
            status.xdg_session_type.as_deref().unwrap_or("unknown"),
            status.xdg_current_desktop.as_deref().unwrap_or("unknown"),
            status.kde_session_version.as_deref().unwrap_or("unknown"),
            status.wayland_display.as_deref().unwrap_or("none"),
            status.display.as_deref().unwrap_or("none"),
            status.dbus_session_bus_address_present,
            status.xdg_runtime_dir_present
        ),
        DaemonResponse::ComputerUseReadiness(status) => format!(
            "readiness observe={} screenshot={} window_control={} keyboard={} pointer={} semantic={} clipboard_read={} clipboard_write={} desktop_revision={} focus_guard={} panic_stop={} issues={} capture_backend={} input_backend={} a11y={}",
            status.observe_state.as_str(),
            status.screenshot_state.as_str(),
            status.window_control_state.as_str(),
            status.keyboard_input_state.as_str(),
            status.pointer_input_state.as_str(),
            status.semantic_action_state.as_str(),
            status.clipboard_read_state.as_str(),
            status.clipboard_write_state.as_str(),
            status.desktop_revision.as_deref().unwrap_or("none"),
            status.focus_guard_required,
            status.panic_stop_enabled,
            status.issues.len(),
            status.capture_backend.as_deref().unwrap_or("none"),
            status.input_backend.as_deref().unwrap_or("none"),
            status.accessibility_backend
        ),
        DaemonResponse::PanicStop(status) => format!(
            "panic-stop enabled={} path={}",
            status.enabled,
            status.path.display()
        ),
        DaemonResponse::KwinBridgeStatus(status) => format!(
            "kwin bridge dbus={} window_resize={} window_move={} window_launch={} active_update_seen={} window_list_update_seen={} window_count={} installed={} enabled={}",
            status.dbus_service_registered,
            status.window_resize_supported,
            status.window_move_supported,
            status.window_launch_supported,
            status.active_window_update_seen,
            status.window_list_update_seen,
            status.window_count,
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
            "input backends configured={} preferred={} implemented={} portal_remote_desktop={} libei={} uinput={} eis_keymap_source={} eis_keymap_layout={}",
            status.configured_backend,
            status
                .preferred_available_backend
                .as_deref()
                .unwrap_or("none"),
            status
                .implemented_available_backend
                .as_deref()
                .unwrap_or("none"),
            status
                .remote_desktop_portal
                .remote_desktop_interface_available,
            status.libei.client_library_available || status.libei.socket_env_present,
            status.uinput_available,
            status.eis_keymap.source,
            status.eis_keymap.layout.as_deref().unwrap_or("default")
        ),
        DaemonResponse::RemoteDesktopSessionProbe(status) => format!(
            "remote desktop probe started={} requested={} selected={} clipboard={} transient_closed={}",
            status.started,
            status.requested_devices.join("+"),
            if status.selected_devices.is_empty() {
                "none".to_string()
            } else {
                status.selected_devices.join("+")
            },
            status.clipboard_enabled,
            status.transient_session_closed
        ),
        DaemonResponse::RemoteDesktopEisProbe(status) => format!(
            "remote desktop EIS probe started={} eis_connected={} runtime_connected={} events={} bound={} resumed_devices={} requested={} selected={} clipboard={} fd_closed={} transient_closed={}",
            status.started,
            status.eis_connected,
            status.eis_runtime_connected,
            status.eis_event_count,
            if status.eis_bound_capabilities.is_empty() {
                "none".to_string()
            } else {
                status.eis_bound_capabilities.join("+")
            },
            status.eis_resumed_device_count,
            status.requested_devices.join("+"),
            if status.selected_devices.is_empty() {
                "none".to_string()
            } else {
                status.selected_devices.join("+")
            },
            status.clipboard_enabled,
            status.eis_fd_closed,
            status.transient_session_closed
        ),
        DaemonResponse::RemoteDesktopEisSessionStatus(status) => format!(
            "remote desktop EIS session active={} runtime_connected={} bound={} resumed_devices={} selected={} clipboard={}",
            status.active,
            status.runtime_connected,
            if status.bound_capabilities.is_empty() {
                "none".to_string()
            } else {
                status.bound_capabilities.join("+")
            },
            status.resumed_device_count,
            if status.selected_devices.is_empty() {
                "none".to_string()
            } else {
                status.selected_devices.join("+")
            },
            status.clipboard_enabled,
        ),
        DaemonResponse::CaptureBackendStatus(status) => format!(
            "capture backends preferred={} implemented={} portal_screenshot={} portal_version={} portal_targets={} portal_screencast={} kwin_metadata={} spectacle={}",
            status
                .preferred_available_backend
                .as_deref()
                .unwrap_or("none"),
            status
                .implemented_available_backend
                .as_deref()
                .unwrap_or("none"),
            status.screenshot_portal.screenshot_interface_available,
            status
                .screenshot_portal
                .screenshot_interface_version
                .map(|version| version.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            if status
                .screenshot_portal
                .screenshot_available_targets
                .is_empty()
            {
                "unknown".to_string()
            } else {
                status
                    .screenshot_portal
                    .screenshot_available_targets
                    .join("+")
            },
            status.screenshot_portal.screencast_interface_available,
            status.kwin_metadata.support_information_available,
            status.spectacle.command_available
        ),
        DaemonResponse::CaptureSessionStatus(status) => compact_capture_session_status(status),
        DaemonResponse::CaptureFrame(frame) => format!(
            "capture frame session={} revision={} sequence={} output={}x{} backend={} occlusion_possible={}",
            frame.session_id,
            frame.revision,
            frame.sequence,
            frame.screenshot.output_width,
            frame.screenshot.output_height,
            frame.screenshot.backend,
            frame.screenshot.occlusion_possible
        ),
        DaemonResponse::CaptureWait(result) => format!(
            "capture wait session={} changed={} timed_out={} elapsed_ms={} revision={} sequence={} backend={} occlusion_possible={}",
            result.frame.session_id,
            result.changed,
            result.timed_out,
            result.elapsed_ms,
            result.frame.revision,
            result.frame.sequence,
            result.frame.screenshot.backend,
            result.frame.screenshot.occlusion_possible
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
        DaemonResponse::ActiveWindow(Some(window)) => format!(
            "active window id={} app={} title={}",
            window.id,
            window.app_id.as_deref().unwrap_or(""),
            window.title
        ),
        DaemonResponse::ActiveWindow(None) => "no active window".to_string(),
        DaemonResponse::WindowInventory(inventory) => format!(
            "window_inventory revision={} windows={} active={}",
            inventory.revision,
            inventory.windows.len(),
            inventory
                .active_window
                .as_ref()
                .map(|window| window.id.as_str())
                .unwrap_or("none")
        ),
        DaemonResponse::WindowInventoryWait(result) => format!(
            "window_inventory_wait changed={} timed_out={} elapsed_ms={} revision={} windows={} active={}",
            result.changed,
            result.timed_out,
            result.elapsed_ms,
            result.inventory.revision,
            result.inventory.windows.len(),
            result
                .inventory
                .active_window
                .as_ref()
                .map(|window| window.id.as_str())
                .unwrap_or("none")
        ),
        DaemonResponse::Screenshot(info) => format!(
            "{} wrote {}x{} image from {}x{} source via {} occlusion_possible={} to {}",
            tool_name,
            info.output_width,
            info.output_height,
            info.source_width,
            info.source_height,
            info.backend,
            info.occlusion_possible,
            info.path.display()
        ),
        DaemonResponse::WaitForChange(result) => format!(
            "wait_for_change changed={} timed_out={} captures={} elapsed_ms={} timeout_ms={} interval_ms={} score={:.6} threshold={:.6} backend={} occlusion_possible={} path={}",
            result.changed,
            result.timed_out,
            result.captures,
            result.elapsed_ms,
            result.timeout_ms,
            result.interval_ms,
            result.score,
            result.threshold,
            result.screenshot.backend,
            result.screenshot.occlusion_possible,
            result.screenshot.path.display()
        ),
        DaemonResponse::ClipboardBackendStatus(status) => format!(
            "clipboard backends read={} write={} wl_paste={} wl_copy={} kde_klipper={}",
            status.read_backend.as_deref().unwrap_or("none"),
            status.write_backend.as_deref().unwrap_or("none"),
            status.wl_paste_available,
            status.wl_copy_available,
            status.kde_klipper_available
        ),
        DaemonResponse::ClipboardText(text) => format!(
            "clipboard text length={} truncated={} original_bytes={} backend={}",
            text.text.len(),
            text.truncated,
            text.original_bytes,
            text.backend
        ),
        DaemonResponse::AccessibilityQualityStatus(status) => format!(
            "accessibility quality atspi={} target_events={} event_backend={} focused={} reliable={} nodes={} named={} actionable={} text={} generic={} flat={} fallback={}",
            status.atspi_available,
            status.target_event_settle_available,
            status.event_backend,
            status.focused_node_present,
            status.semantic_targeting_reliable,
            status.sampled_node_count,
            status.named_node_count,
            status.actionable_node_count,
            status.text_node_count,
            status.generic_role_count,
            status.tree_flat,
            status.recommended_fallback
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
        DaemonResponse::AccessibilityTextAttributes(attributes) => format!(
            "text attributes range={}..{} count={} node={}",
            attributes.start_offset,
            attributes.end_offset,
            attributes.attributes.len(),
            attributes.node_id
        ),
        DaemonResponse::Journal(entries) => format!("{} journal entries", entries.len()),
        DaemonResponse::Action(result) => {
            let message = result
                .message
                .clone()
                .unwrap_or_else(|| format!("action {} ok={}", result.id, result.ok));
            match result
                .observation
                .as_ref()
                .and_then(|observation| observation.settle.as_ref())
            {
                Some(settle) => format!(
                    "{message} dispatch=accepted confirmation={} settle={:?} backend={:?} target_scoped={} event={} settled={} timed_out={} samples={} elapsed_ms={}",
                    settle.confirmation.as_str(),
                    settle.condition,
                    settle.backend,
                    settle.target_scoped,
                    settle.event.as_deref().unwrap_or("none"),
                    settle.settled,
                    settle.timed_out,
                    settle.samples,
                    settle.elapsed_ms
                ),
                None => format!("{message} dispatch=accepted confirmation=not_requested"),
            }
        }
        DaemonResponse::Error { kind, message } => format!("error kind={kind:?}: {message}"),
    }
}

fn compact_capture_session_status(status: &libseatgeist::CaptureSessionStatus) -> String {
    let execution = status.execution.as_ref();
    let focus = execution.and_then(|execution| execution.focus_lease.as_ref());
    let settle = execution.and_then(|execution| execution.settle.as_ref());
    format!(
        "capture_session active={} opening={} id={} backend={} source={} occlusion_possible={} requested_source={} requested_id={} owner_tool={} owner_scope={} owner_pid={} restore_ref={} revision={} sticky_target={} target_window={} expires_ms={} end_reason={} capture_exec={} semantic={} raw={} last_method={} last_backend={} last_policy={} focus_policy={} activity_backend={} activity_trusted={} focus_reacquired={} focus_restored={} settle_backend={} settled={}",
        status.active,
        status.opening,
        status.session_id.as_deref().unwrap_or("none"),
        status.backend.as_deref().unwrap_or("none"),
        status.source_type.as_deref().unwrap_or("none"),
        status.occlusion_possible,
        status.requested_source_type.as_deref().unwrap_or("none"),
        status.requested_source_id.as_deref().unwrap_or("none"),
        status.owner_tool.as_deref().unwrap_or("none"),
        status.owner_scope.as_deref().unwrap_or("none"),
        status
            .owner_pid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        status.restore_token_reference.as_deref().unwrap_or("none"),
        status.latest_revision.as_deref().unwrap_or("none"),
        status.sticky_target_bound,
        status.target_window_id.as_deref().unwrap_or("none"),
        status
            .target_expires_in_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        status.last_end_reason.as_deref().unwrap_or("none"),
        execution
            .map(|execution| execution.capture_backend.as_str())
            .unwrap_or("none"),
        execution
            .and_then(|execution| execution.semantic_backend.as_deref())
            .unwrap_or("none"),
        execution
            .and_then(|execution| execution.raw_input_backend.as_deref())
            .unwrap_or("none"),
        execution
            .and_then(|execution| execution.last_action_method.as_deref())
            .unwrap_or("none"),
        execution
            .and_then(|execution| execution.last_action_backend.as_deref())
            .unwrap_or("none"),
        execution
            .and_then(|execution| execution.last_policy_result.as_deref())
            .unwrap_or("none"),
        execution
            .and_then(|execution| execution.cooperative_focus_policy.as_deref())
            .unwrap_or("none"),
        execution
            .and_then(|execution| execution.activity_backend.as_deref())
            .unwrap_or("none"),
        execution.is_some_and(|execution| execution.activity_trusted),
        focus
            .map(|focus| focus.focus_reacquired.to_string())
            .unwrap_or_else(|| "none".to_string()),
        focus
            .map(|focus| focus.focus_restored.to_string())
            .unwrap_or_else(|| "none".to_string()),
        settle
            .map(|settle| format!("{:?}", settle.backend).to_ascii_lowercase())
            .unwrap_or_else(|| "none".to_string()),
        settle
            .map(|settle| settle.settled.to_string())
            .unwrap_or_else(|| "none".to_string()),
    )
}

fn tool_definitions(profile: ToolProfile) -> Vec<Value> {
    match profile {
        ToolProfile::Core => core_tool_definitions(),
        ToolProfile::Expert => expert_tool_definitions(),
        ToolProfile::All => {
            let mut tools = core_tool_definitions();
            tools.extend(expert_tool_definitions());
            tools
        }
    }
}

fn tool_available(profile: ToolProfile, name: &str) -> bool {
    tool_definitions(profile)
        .iter()
        .any(|tool| tool["name"] == name)
}

fn core_tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "seatgeist.computer_status",
            "Computer Status",
            "Return the compact safe computer-use readiness summary before a session begins or after an actionable failure.",
            object_schema(vec![], vec![]),
        ),
        capture::window_session_tool(),
        tool(
            "seatgeist.snapshot",
            "Snapshot",
            "Capture one bounded image from the required retained window session and attach it as native MCP image content without another chooser. Whole-desktop compatibility capture is expert-only.",
            capture::core_snapshot_schema(),
        ),
        tool(
            "seatgeist.act",
            "Act",
            "Perform exactly one policy-gated logical UI action and return its compact settled post-action observation. This is not an action batch.",
            facade_act_schema(),
        ),
        tool(
            "seatgeist.wait",
            "Wait",
            "Wait a bounded time for a frame revision from the required retained window session. Returns the latest bounded image plus explicit timeout metadata.",
            capture::core_wait_schema(),
        ),
        tool(
            "seatgeist.panic_stop",
            "Panic Stop",
            "Read panic-stop state when enabled is omitted, or explicitly enable/disable the journaled daemon panic-stop flag.",
            object_schema(
                vec![(
                    "enabled",
                    json!({"type": "boolean", "description": "Desired panic-stop state; omit to read current state."}),
                )],
                vec![],
            ),
        ),
    ]
}

fn facade_act_schema() -> Value {
    object_schema(
        with_semantic_guard_properties(vec![
            (
                "action",
                json!({"type": "string", "enum": ["focus_window", "move_window", "launch_window", "resize_window", "page_zoom", "type_text", "key_combo", "click_pointer", "scroll_pointer", "click_button", "set_text_field", "focus_text_field", "select_menu", "activate_tab", "activate_link", "toggle_check", "set_value", "select_item"], "description": "One logical action to execute."}),
            ),
            ("window_id", json!({"type": "string"})),
            ("desktop_entry", json!({"type": "string"})),
            (
                "anchor",
                json!({"type": "string", "enum": ["top_left", "top_right", "bottom_left", "bottom_right", "center"]}),
            ),
            ("monitor_id", json!({"type": "string"})),
            (
                "margin",
                json!({"type": "integer", "minimum": 0, "maximum": 32768}),
            ),
            (
                "activation",
                json!({"type": "string", "enum": ["preserve_focus", "activate"]}),
            ),
            (
                "timeout_ms",
                json!({"type": "integer", "minimum": 1000, "maximum": 30000}),
            ),
            (
                "width",
                json!({"type": "integer", "minimum": 64, "maximum": 32768}),
            ),
            (
                "height",
                json!({"type": "integer", "minimum": 64, "maximum": 32768}),
            ),
            (
                "operation",
                json!({"type": "string", "enum": ["in", "out", "reset"]}),
            ),
            (
                "steps",
                json!({"type": "integer", "minimum": 1, "maximum": 20}),
            ),
            ("text", json!({"type": "string"})),
            ("combo", json!({"type": "string"})),
            ("name", json!({"type": "string"})),
            (
                "path",
                json!({"type": "array", "items": {"type": "string"}, "minItems": 1}),
            ),
            ("value", json!({"type": "number"})),
            ("checked", json!({"type": "boolean"})),
            ("destructive", json!({"type": "boolean"})),
            ("app", json!({"type": "string"})),
            ("window_name_contains", json!({"type": "string"})),
            (
                "max_nodes",
                json!({"type": "integer", "minimum": 1, "maximum": 4096}),
            ),
            ("x", json!({"type": "number"})),
            ("y", json!({"type": "number"})),
            (
                "coordinate_space",
                json!({"type": "string", "enum": ["physical_pixel", "logical_pixel", "window_local"]}),
            ),
            (
                "button",
                json!({"type": "string", "enum": ["left", "middle", "right"]}),
            ),
            (
                "clicks",
                json!({"type": "integer", "minimum": 1, "maximum": 2}),
            ),
            ("vertical", json!({"type": "integer"})),
            ("horizontal", json!({"type": "integer"})),
            (
                "session_id",
                json!({"type": "string", "description": "Pinned interaction-session id for daemon-owned focus reacquisition before raw input."}),
            ),
        ]),
        vec!["action"],
    )
}

fn expert_tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "seatgeist.health",
            "Daemon Health",
            "Check the Seatgeist daemon health.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.capabilities",
            "Capabilities",
            "List daemon backend capabilities.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.policy_status",
            "Policy Status",
            "Read current daemon policy defaults.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.safety_status",
            "Safety Status",
            "Read active daemon safety gates, screenshot bounds/redactions, and journal artifact metadata state.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.desktop_session_status",
            "Desktop Session Status",
            "Report sanitized KDE/Wayland/session environment diagnostics for portal, KWin, DBus, and runtime troubleshooting.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.computer_use_readiness",
            "Computer Use Readiness",
            "Summarize safe preflight readiness for observe, screenshots, window control, input, semantic actions, clipboard, and active safety blockers.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.panic_stop_status",
            "Panic Stop Status",
            "Read whether the daemon panic-stop flag is active.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.panic_stop_enable",
            "Enable Panic Stop",
            "Enable the daemon panic-stop flag. This is journaled and blocks control-class actions.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.panic_stop_disable",
            "Disable Panic Stop",
            "Disable the daemon panic-stop flag after explicit local operator intent.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.kwin_bridge_status",
            "KWin Bridge Status",
            "Report daemon DBus receiver state, latest active-window and window-list bridge update state, and user-local KWin script install/config status.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.uinput_status",
            "Uinput Status",
            "Report whether the daemon can open /dev/uinput for virtual keyboard and pointer fallback, with file metadata and setup hints.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.input_backend_status",
            "Input Backend Status",
            "Probe read-only input backend availability in priority order: xdg-desktop-portal RemoteDesktop, libei, then uinput fallback.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.remote_desktop_session_probe",
            "RemoteDesktop Session Probe",
            "Explicitly request a transient xdg-desktop-portal RemoteDesktop session to validate consent and selected devices. This is policy-gated control, may open a portal dialog, closes the session after probing, and sends no input.",
            object_schema(
                vec![
                    (
                        "keyboard",
                        json!({"type": "boolean", "description": "Request keyboard control permission. Defaults to true when no device flags are supplied."}),
                    ),
                    (
                        "pointer",
                        json!({"type": "boolean", "description": "Request pointer control permission. Defaults to true when no device flags are supplied."}),
                    ),
                    (
                        "touchscreen",
                        json!({"type": "boolean", "description": "Request touchscreen control permission. Defaults to false."}),
                    ),
                    (
                        "restore_token",
                        json!({"type": "string", "description": "Optional single-use portal restore token from a previous started session."}),
                    ),
                    (
                        "persist_mode",
                        json!({"type": "string", "enum": ["do_not_persist", "application_lifetime", "explicitly_revoked"], "description": "Requested portal permission persistence mode."}),
                    ),
                    (
                        "parent_window",
                        json!({"type": "string", "description": "Optional portal parent window identifier."}),
                    ),
                    (
                        "timeout_ms",
                        json!({"type": "integer", "minimum": 1, "maximum": 300000, "description": "Maximum time to wait for each portal interaction. Defaults to 120000."}),
                    ),
                    (
                        "expected_active_window",
                        json!({"type": "string", "description": "Optional active-window id guard checked before opening the portal interaction."}),
                    ),
                    (
                        "expected_active_app",
                        json!({"type": "string", "description": "Optional active app id guard checked before opening the portal interaction."}),
                    ),
                    (
                        "active_title_contains",
                        json!({"type": "string", "description": "Optional active-window title substring guard checked before opening the portal interaction."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.remote_desktop_eis_probe",
            "RemoteDesktop EIS Probe",
            "Explicitly request a transient xdg-desktop-portal RemoteDesktop session, call ConnectToEIS, report compact libei runtime state, close the returned FD, and send no input. This is policy-gated control and may open a portal dialog.",
            object_schema(
                vec![
                    (
                        "keyboard",
                        json!({"type": "boolean", "description": "Request keyboard control permission. Defaults to true when no device flags are supplied."}),
                    ),
                    (
                        "pointer",
                        json!({"type": "boolean", "description": "Request pointer control permission. Defaults to true when no device flags are supplied."}),
                    ),
                    (
                        "touchscreen",
                        json!({"type": "boolean", "description": "Request touchscreen control permission. Defaults to false."}),
                    ),
                    (
                        "restore_token",
                        json!({"type": "string", "description": "Optional single-use portal restore token from a previous started session."}),
                    ),
                    (
                        "persist_mode",
                        json!({"type": "string", "enum": ["do_not_persist", "application_lifetime", "explicitly_revoked"], "description": "Requested portal permission persistence mode."}),
                    ),
                    (
                        "parent_window",
                        json!({"type": "string", "description": "Optional portal parent window identifier."}),
                    ),
                    (
                        "timeout_ms",
                        json!({"type": "integer", "minimum": 1, "maximum": 300000, "description": "Maximum time to wait for each portal interaction. Defaults to 120000."}),
                    ),
                    (
                        "expected_active_window",
                        json!({"type": "string", "description": "Optional active-window id guard checked before opening the portal interaction."}),
                    ),
                    (
                        "expected_active_app",
                        json!({"type": "string", "description": "Optional active app id guard checked before opening the portal interaction."}),
                    ),
                    (
                        "active_title_contains",
                        json!({"type": "string", "description": "Optional active-window title substring guard checked before opening the portal interaction."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.remote_desktop_eis_start",
            "RemoteDesktop EIS Session Start",
            "Explicitly request and retain a daemon-owned xdg-desktop-portal RemoteDesktop EIS session. This is policy-gated control, may open a portal dialog, and still sends no input by itself.",
            object_schema(
                vec![
                    (
                        "keyboard",
                        json!({"type": "boolean", "description": "Request keyboard control permission. Defaults to true when no device flags are supplied."}),
                    ),
                    (
                        "pointer",
                        json!({"type": "boolean", "description": "Request pointer control permission. Defaults to true when no device flags are supplied."}),
                    ),
                    (
                        "touchscreen",
                        json!({"type": "boolean", "description": "Request touchscreen control permission. Defaults to false."}),
                    ),
                    (
                        "restore_token",
                        json!({"type": "string", "description": "Optional single-use portal restore token from a previous started session."}),
                    ),
                    (
                        "persist_mode",
                        json!({"type": "string", "enum": ["do_not_persist", "application_lifetime", "explicitly_revoked"], "description": "Requested portal permission persistence mode."}),
                    ),
                    (
                        "parent_window",
                        json!({"type": "string", "description": "Optional portal parent window identifier."}),
                    ),
                    (
                        "timeout_ms",
                        json!({"type": "integer", "minimum": 1, "maximum": 300000, "description": "Maximum time to wait for each portal interaction. Defaults to 120000."}),
                    ),
                    (
                        "expected_active_window",
                        json!({"type": "string", "description": "Optional active-window id guard checked before opening the portal interaction."}),
                    ),
                    (
                        "expected_active_app",
                        json!({"type": "string", "description": "Optional active app id guard checked before opening the portal interaction."}),
                    ),
                    (
                        "active_title_contains",
                        json!({"type": "string", "description": "Optional active-window title substring guard checked before opening the portal interaction."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.remote_desktop_eis_session_status",
            "RemoteDesktop EIS Session Status",
            "Report whether the daemon currently holds a RemoteDesktop EIS session, compact runtime readiness metadata, and selected devices. This does not open a portal dialog or send input.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.remote_desktop_eis_stop",
            "RemoteDesktop EIS Session Stop",
            "Drop the daemon-owned RemoteDesktop EIS session if one exists. This is journaled and sends no input.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.capture_backend_status",
            "Capture Backend Status",
            "Probe read-only capture backend availability: xdg-desktop-portal Screenshot/ScreenCast, KWin metadata, and Spectacle fallback.",
            object_schema(vec![], vec![]),
        ),
        capture::open_tool(),
        capture::generic_open_tool(),
        capture::status_tool(),
        capture::renew_tool(),
        capture::snapshot_tool(),
        capture::wait_tool(),
        capture::close_tool(),
        tool(
            "seatgeist.pointer_calibration",
            "Pointer Calibration",
            "Report monitor-derived physical pointer bounds, per-monitor physical origins, and representative test points without moving the pointer.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.list_monitors",
            "List Monitors",
            "List monitor geometry and scale metadata.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.list_windows",
            "List Windows",
            "List compact KWin window metadata, using the KWin script bridge for pid/app/geometry enrichment when available.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.active_window",
            "Active Window",
            "Read the latest active-window bridge update.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.observe",
            "Observe Desktop",
            "Return compact desktop state: monitors, windows, active window, and optional screenshot metadata. Full-resolution screenshots require explicit policy approval.",
            object_schema(
                vec![
                    (
                        "screenshot_output",
                        json!({"type": "string", "description": "Optional PNG output path. When omitted, observe returns metadata only."}),
                    ),
                    (
                        "max_edge",
                        json!({"type": "integer", "minimum": 1, "description": "Screenshot preview max edge in pixels. Defaults to the daemon safety config."}),
                    ),
                    (
                        "full_resolution",
                        json!({"type": "boolean", "description": "Capture the source image without downscaling. This is policy-gated separately and prompts by default."}),
                    ),
                    (
                        "include_image",
                        json!({"type": "boolean", "description": "Attach a bounded PNG directly to the MCP result. Defaults to true when screenshot_output is provided. Full-resolution screenshots cannot be embedded."}),
                    ),
                    (
                        "portal_interactive",
                        json!({"type": "boolean", "description": "When the portal Screenshot backend is selected, ask the portal to offer interactive capture UI. Defaults to false."}),
                    ),
                    (
                        "portal_target",
                        json!({"type": "string", "enum": ["screen", "window", "area", "active_window"], "description": "Optional xdg-desktop-portal Screenshot v3 target hint. Requires capture_backend_status.screenshot_portal.screenshot_target_option_supported=true; otherwise the daemon fails closed."}),
                    ),
                    (
                        "visible_window_crop_id",
                        json!({"type": "string", "description": "Explicit compatibility mode: crop the currently visible pixels of this exact KWin window from a composed desktop capture. Reports backend=visible_window_crop and may contain occlusion; never use as hidden-window capture."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.screenshot",
            "Screenshot",
            "Capture a screenshot to a PNG path. Defaults to a bounded preview; full_resolution is policy-gated separately and prompts by default.",
            object_schema(
                vec![
                    (
                        "output",
                        json!({"type": "string", "description": "Optional PNG output path on the local filesystem. When omitted, Seatgeist writes a timestamped PNG under the runtime screenshot directory."}),
                    ),
                    (
                        "max_edge",
                        json!({"type": "integer", "minimum": 1, "description": "Preview max edge in pixels. Defaults to the daemon safety config."}),
                    ),
                    (
                        "full_resolution",
                        json!({"type": "boolean", "description": "Capture the source image without downscaling. This is policy-gated separately and prompts by default."}),
                    ),
                    (
                        "include_image",
                        json!({"type": "boolean", "description": "Attach the bounded PNG directly to the MCP result. Defaults to true. Full-resolution screenshots cannot be embedded."}),
                    ),
                    (
                        "portal_interactive",
                        json!({"type": "boolean", "description": "When the portal Screenshot backend is selected, ask the portal to offer interactive capture UI. Defaults to false."}),
                    ),
                    (
                        "portal_target",
                        json!({"type": "string", "enum": ["screen", "window", "area", "active_window"], "description": "Optional xdg-desktop-portal Screenshot v3 target hint. Requires capture_backend_status.screenshot_portal.screenshot_target_option_supported=true; otherwise the daemon fails closed."}),
                    ),
                    (
                        "visible_window_crop_id",
                        json!({"type": "string", "description": "Explicit compatibility mode: crop the currently visible pixels of this exact KWin window from a composed desktop capture. Reports backend=visible_window_crop and may contain occlusion; never use as hidden-window capture."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.screenshot_tile",
            "Screenshot Tile",
            "Capture and optionally downscale a physical-pixel screenshot tile.",
            object_schema(
                vec![
                    (
                        "output",
                        json!({"type": "string", "description": "Optional PNG output path on the local filesystem. When omitted, Seatgeist writes a timestamped PNG under the runtime screenshot directory."}),
                    ),
                    ("x", json!({"type": "integer", "minimum": 0})),
                    ("y", json!({"type": "integer", "minimum": 0})),
                    ("width", json!({"type": "integer", "minimum": 1})),
                    ("height", json!({"type": "integer", "minimum": 1})),
                    (
                        "max_edge",
                        json!({"type": "integer", "minimum": 1, "description": "Output max edge in pixels. Defaults to the daemon safety config."}),
                    ),
                    (
                        "include_image",
                        json!({"type": "boolean", "description": "Attach the bounded PNG directly to the MCP result. Defaults to true."}),
                    ),
                    (
                        "portal_interactive",
                        json!({"type": "boolean", "description": "When the portal Screenshot backend is selected for the tile source, ask the portal to offer interactive capture UI. Defaults to false."}),
                    ),
                ],
                vec!["x", "y", "width", "height"],
            ),
        ),
        tool(
            "seatgeist.wait_for_change",
            "Wait For Change",
            "Poll bounded screenshots until the normalized pixel delta reaches a threshold or the timeout expires.",
            object_schema(
                vec![
                    (
                        "output",
                        json!({"type": "string", "description": "Optional PNG output path for the latest bounded screenshot. When omitted, Seatgeist writes a timestamped PNG under the runtime screenshot directory."}),
                    ),
                    (
                        "max_edge",
                        json!({"type": "integer", "minimum": 1, "description": "Screenshot preview max edge in pixels. Defaults to the daemon safety config."}),
                    ),
                    (
                        "timeout_ms",
                        json!({"type": "integer", "minimum": 1, "description": "Maximum time to wait in milliseconds. Defaults to 5000."}),
                    ),
                    (
                        "interval_ms",
                        json!({"type": "integer", "minimum": 1, "description": "Polling interval in milliseconds. Defaults to 250."}),
                    ),
                    (
                        "threshold",
                        json!({"type": "number", "exclusiveMinimum": 0.0, "maximum": 1.0, "description": "Normalized RGB delta threshold. Defaults to 0.01."}),
                    ),
                    (
                        "include_image",
                        json!({"type": "boolean", "description": "Attach the latest bounded PNG directly to the MCP result. Defaults to true."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.focus_window",
            "Focus Window",
            "Focus a listed KWin window by id. This is policy-gated control and usually requires explicit daemon approval mode.",
            object_schema(
                with_guard_properties(vec![(
                    "window_id",
                    json!({"type": "string", "description": "KWin window id from seatgeist.list_windows."}),
                )]),
                vec!["window_id"],
            ),
        ),
        tool(
            "seatgeist.launch_window",
            "Launch Window",
            "Launch a desktop entry only after KWin arms a one-shot work-area placement intent. Can preserve the current focus while anchoring and optionally sizing the new window.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "desktop_entry",
                        json!({"type": "string", "description": "Desktop entry id such as org.kde.kcalc; arbitrary commands and paths are rejected."}),
                    ),
                    (
                        "anchor",
                        json!({"type": "string", "enum": ["top_left", "top_right", "bottom_left", "bottom_right", "center"]}),
                    ),
                    (
                        "monitor_id",
                        json!({"type": "string", "description": "Optional KWin output name."}),
                    ),
                    (
                        "width",
                        json!({"type": "integer", "minimum": 64, "maximum": 32768}),
                    ),
                    (
                        "height",
                        json!({"type": "integer", "minimum": 64, "maximum": 32768}),
                    ),
                    (
                        "margin",
                        json!({"type": "integer", "minimum": 0, "maximum": 32768, "default": 0}),
                    ),
                    (
                        "activation",
                        json!({"type": "string", "enum": ["preserve_focus", "activate"], "default": "preserve_focus"}),
                    ),
                    (
                        "timeout_ms",
                        json!({"type": "integer", "minimum": 1000, "maximum": 30000, "default": 10000}),
                    ),
                ]),
                vec!["desktop_entry", "anchor"],
            ),
        ),
        tool(
            "seatgeist.move_window",
            "Move Window",
            "Move a listed KWin window to an explicit logical-pixel position while preserving its size. This is policy-gated control and requires the installed KWin script bridge.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "window_id",
                        json!({"type": "string", "description": "KWin window id from seatgeist.list_windows."}),
                    ),
                    (
                        "x",
                        json!({"type": "integer", "minimum": -32768, "maximum": 32768, "description": "Requested frame x coordinate in KWin logical pixels."}),
                    ),
                    (
                        "y",
                        json!({"type": "integer", "minimum": -32768, "maximum": 32768, "description": "Requested frame y coordinate in KWin logical pixels."}),
                    ),
                ]),
                vec!["window_id", "x", "y"],
            ),
        ),
        tool(
            "seatgeist.resize_window",
            "Resize Window",
            "Resize a listed KWin window to an explicit logical-pixel width and height while preserving its position. This is policy-gated control and requires the installed KWin script bridge.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "window_id",
                        json!({"type": "string", "description": "KWin window id from seatgeist.list_windows."}),
                    ),
                    (
                        "width",
                        json!({"type": "integer", "minimum": 64, "maximum": 32768, "description": "Requested frame width in KWin logical pixels."}),
                    ),
                    (
                        "height",
                        json!({"type": "integer", "minimum": 64, "maximum": 32768, "description": "Requested frame height in KWin logical pixels."}),
                    ),
                ]),
                vec!["window_id", "width", "height"],
            ),
        ),
        tool(
            "seatgeist.page_zoom",
            "Page Zoom",
            "Adjust or reset page zoom in an active Firefox or Chromium-family window using standard browser shortcuts through the configured policy-gated keyboard backend. The browser's configured zoom ladder determines the resulting percentage.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "operation",
                        json!({"type": "string", "enum": ["in", "out", "reset"]}),
                    ),
                    (
                        "steps",
                        json!({"type": "integer", "minimum": 1, "maximum": 20, "description": "Number of zoom steps; reset always performs one reset shortcut."}),
                    ),
                ]),
                vec!["operation", "expected_active_window"],
            ),
        ),
        tool(
            "seatgeist.type_text",
            "Type Text",
            "Type text through the configured input executor. uinput uses the local US-keyboard map; explicit portal/libei backends use the stored EIS text capability when ready. This is policy-gated keyboard control and summaries report text length only.",
            object_schema(
                with_raw_guard_properties(vec![(
                    "text",
                    json!({"type": "string", "description": "Text to type. uinput supports US keyboard ASCII plus newline and tab; EIS execution uses the text capability."}),
                )]),
                vec!["text"],
            ),
        ),
        tool(
            "seatgeist.key_combo",
            "Key Combo",
            "Send a named evdev key combination through the configured input executor, such as Ctrl+L or Alt+F4. This is policy-gated keyboard control.",
            object_schema(
                with_raw_guard_properties(vec![(
                    "combo",
                    json!({"type": "string", "description": "Key combination, such as Ctrl+L, Shift+F4, or Super+Space."}),
                )]),
                vec!["combo"],
            ),
        ),
        tool(
            "seatgeist.move_pointer",
            "Move Pointer",
            "Move the pointer to an explicit coordinate. This is policy-gated pointer control; the daemon accepts physical_pixel, global logical_pixel, or guarded active-window window_local coordinates.",
            object_schema(
                with_raw_guard_properties(vec![
                    (
                        "x",
                        json!({"type": "number", "description": "Target x coordinate."}),
                    ),
                    (
                        "y",
                        json!({"type": "number", "description": "Target y coordinate."}),
                    ),
                    (
                        "coordinate_space",
                        json!({"type": "string", "enum": ["physical_pixel", "logical_pixel", "window_local", "accessibility_node"], "description": "Coordinate space for x and y. Supported daemon spaces are physical_pixel, global logical_pixel, and window_local. window_local is relative to the active window and requires an active-window guard."}),
                    ),
                ]),
                vec!["x", "y", "coordinate_space"],
            ),
        ),
        tool(
            "seatgeist.click_pointer",
            "Click Pointer",
            "Move the pointer to an explicit coordinate and click once or twice. This is policy-gated pointer control; the daemon accepts physical_pixel, global logical_pixel, or guarded active-window window_local coordinates.",
            object_schema(
                with_raw_guard_properties(vec![
                    (
                        "x",
                        json!({"type": "number", "description": "Target x coordinate."}),
                    ),
                    (
                        "y",
                        json!({"type": "number", "description": "Target y coordinate."}),
                    ),
                    (
                        "coordinate_space",
                        json!({"type": "string", "enum": ["physical_pixel", "logical_pixel", "window_local", "accessibility_node"], "description": "Coordinate space for x and y. Supported daemon spaces are physical_pixel, global logical_pixel, and window_local. window_local is relative to the active window and requires an active-window guard."}),
                    ),
                    (
                        "button",
                        json!({"type": "string", "enum": ["left", "middle", "right"], "description": "Pointer button to click."}),
                    ),
                    (
                        "clicks",
                        json!({"type": "integer", "minimum": 1, "maximum": 2, "description": "Click count. Defaults to 1; use 2 for double-click."}),
                    ),
                ]),
                vec!["x", "y", "coordinate_space", "button"],
            ),
        ),
        tool(
            "seatgeist.drag_pointer",
            "Drag Pointer",
            "Drag from one explicit coordinate to another by pressing, moving, and releasing a pointer button. This is policy-gated pointer control; the daemon accepts physical_pixel, global logical_pixel, or guarded active-window window_local coordinates.",
            object_schema(
                with_raw_guard_properties(vec![
                    (
                        "from_x",
                        json!({"type": "number", "description": "Starting x coordinate."}),
                    ),
                    (
                        "from_y",
                        json!({"type": "number", "description": "Starting y coordinate."}),
                    ),
                    (
                        "to_x",
                        json!({"type": "number", "description": "Ending x coordinate."}),
                    ),
                    (
                        "to_y",
                        json!({"type": "number", "description": "Ending y coordinate."}),
                    ),
                    (
                        "coordinate_space",
                        json!({"type": "string", "enum": ["physical_pixel", "logical_pixel", "window_local", "accessibility_node"], "description": "Coordinate space for all coordinates. Supported daemon spaces are physical_pixel, global logical_pixel, and window_local. window_local is relative to the active window and requires an active-window guard."}),
                    ),
                    (
                        "button",
                        json!({"type": "string", "enum": ["left", "middle", "right"], "description": "Pointer button to hold during the drag. Defaults to left."}),
                    ),
                    (
                        "duration_ms",
                        json!({"type": "integer", "minimum": 0, "maximum": 10000, "description": "Approximate drag duration in milliseconds. Defaults to 250."}),
                    ),
                ]),
                vec!["from_x", "from_y", "to_x", "to_y", "coordinate_space"],
            ),
        ),
        tool(
            "seatgeist.scroll_pointer",
            "Scroll Pointer",
            "Emit vertical and/or horizontal wheel deltas at the current pointer position. This is policy-gated pointer control.",
            object_schema(
                with_raw_guard_properties(vec![
                    (
                        "vertical",
                        json!({"type": "integer", "description": "Vertical wheel delta. Positive values scroll up in evdev wheel units."}),
                    ),
                    (
                        "horizontal",
                        json!({"type": "integer", "description": "Horizontal wheel delta. Positive values scroll left in evdev wheel units."}),
                    ),
                ]),
                vec![],
            ),
        ),
        tool(
            "seatgeist.click_button",
            "Click Button",
            "Find a named non-sensitive AT-SPI button and invoke its press action only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible button name to match."}),
                    ),
                    (
                        "destructive",
                        json!({"type": "boolean", "description": "Set true when pressing this button may delete, discard, close, quit, overwrite, or otherwise lose state; routes through destructive-action policy."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name"],
            ),
        ),
        tool(
            "seatgeist.set_text_field",
            "Set Text Field",
            "Find a named non-sensitive AT-SPI text field and replace its contents only when exactly one viable match is found. This is policy-gated semantic control and summaries report text length only.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible text field name to match."}),
                    ),
                    (
                        "text",
                        json!({"type": "string", "description": "Replacement text for the matched field."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name", "text"],
            ),
        ),
        tool(
            "seatgeist.focus_text_field",
            "Focus Text Field",
            "Find a named non-sensitive focusable AT-SPI text field and move focus to it only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible text field name to match."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name"],
            ),
        ),
        tool(
            "seatgeist.activate_tab",
            "Activate Tab",
            "Find a named non-sensitive AT-SPI tab and activate it only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible tab name to match."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name"],
            ),
        ),
        tool(
            "seatgeist.activate_link",
            "Activate Link",
            "Find a named non-sensitive AT-SPI link and activate it only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible link name to match."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name"],
            ),
        ),
        tool(
            "seatgeist.toggle_check",
            "Toggle Check",
            "Find a named non-sensitive AT-SPI checkbox, radio button, or checkable menu item and press/select it only when exactly one viable match is found. Pass checked=true or checked=false to request a desired state and avoid an unnecessary toggle when AT-SPI state already matches. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible checkbox, radio button, or checkable menu item name to match."}),
                    ),
                    (
                        "checked",
                        json!({"type": "boolean", "description": "Optional desired checked state. When omitted, the matched control is toggled."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name"],
            ),
        ),
        tool(
            "seatgeist.set_value",
            "Set Value",
            "Find a named non-sensitive AT-SPI slider, spin button, scrollbar, or dial and set its numeric CurrentValue only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible value-control name to match."}),
                    ),
                    (
                        "value",
                        json!({"type": "number", "description": "Finite numeric value to set through org.a11y.atspi.Value.CurrentValue."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name", "value"],
            ),
        ),
        tool(
            "seatgeist.select_item",
            "Select Item",
            "Find a named non-sensitive AT-SPI list, tree, table-row, combo-box, or option item and select/press it only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "name",
                        json!({"type": "string", "description": "Accessible item or option name to match."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["name"],
            ),
        ),
        tool(
            "seatgeist.select_menu",
            "Select Menu",
            "Select a visible AT-SPI menu path only when exactly one non-sensitive activatable item matches. This is policy-gated semantic control.",
            object_schema(
                with_semantic_guard_properties(vec![
                    (
                        "path",
                        json!({"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Visible menu path segments, such as [\"File\", \"Open\"]."}),
                    ),
                    (
                        "destructive",
                        json!({"type": "boolean", "description": "Set true when selecting this menu item may delete, discard, close, quit, overwrite, or otherwise lose state; routes through destructive-action policy."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Optional application accessible-name guard."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Optional containing frame/dialog/window accessible-name guard."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum accessibility nodes to scan. Defaults to 1024."}),
                    ),
                ]),
                vec!["path"],
            ),
        ),
        tool(
            "seatgeist.clipboard_status",
            "Clipboard Status",
            "Report available clipboard text read/write backends and setup hints without reading clipboard contents.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.clipboard_get_text",
            "Clipboard Get Text",
            "Read UTF-8 text from the Wayland clipboard. This is policy-gated and bounded by default.",
            object_schema(
                vec![
                    (
                        "max_bytes",
                        json!({"type": "integer", "minimum": 1, "description": "Maximum UTF-8 bytes to return before truncating. Defaults to 65536."}),
                    ),
                    (
                        "full",
                        json!({"type": "boolean", "description": "Return the full clipboard text without a byte cap."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.clipboard_set_text",
            "Clipboard Set Text",
            "Set UTF-8 text on the Wayland clipboard. The daemon journals the action without echoing the text in summaries.",
            object_schema(
                with_post_action_properties(vec![(
                    "text",
                    json!({"type": "string", "description": "UTF-8 text to place on the clipboard."}),
                )]),
                vec!["text"],
            ),
        ),
        tool(
            "seatgeist.a11y_quality_status",
            "Accessibility Quality Status",
            "Report bounded AT-SPI availability and semantic-tree quality before choosing semantic actions or screenshot/pointer fallback.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "seatgeist.a11y_focused_tree",
            "Focused Accessibility Tree",
            "Return a compact AT-SPI subtree rooted at the currently focused accessibility node.",
            object_schema(
                vec![
                    (
                        "depth",
                        json!({"type": "integer", "minimum": 0, "description": "Child depth to return from the focused node. Defaults to 2."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 2000, "description": "Maximum nodes to scan and return. Defaults to 256."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.a11y_find",
            "Find Accessibility Nodes",
            "Find compact AT-SPI nodes by role, name substring, application name, or containing window name.",
            object_schema(
                vec![
                    (
                        "role",
                        json!({"type": "string", "description": "Exact AT-SPI role name, such as button, frame, menu item, or text."}),
                    ),
                    (
                        "name_contains",
                        json!({"type": "string", "description": "Case-insensitive accessible-name substring."}),
                    ),
                    (
                        "app",
                        json!({"type": "string", "description": "Case-insensitive application accessible-name substring."}),
                    ),
                    (
                        "window_name_contains",
                        json!({"type": "string", "description": "Case-insensitive containing frame/dialog/window name substring."}),
                    ),
                    (
                        "depth",
                        json!({"type": "integer", "minimum": 0, "description": "Child depth to include for each match. Defaults to 0."}),
                    ),
                    (
                        "max_results",
                        json!({"type": "integer", "minimum": 1, "maximum": 100, "description": "Maximum matched nodes to return. Defaults to 10."}),
                    ),
                    (
                        "max_nodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000, "description": "Maximum nodes to scan. Defaults to 512."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "seatgeist.a11y_text_attributes",
            "Text Attributes",
            "Read the AT-SPI text attribute run at a character offset on a non-sensitive text node.",
            object_schema(
                vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from seatgeist.a11y_find or seatgeist.a11y_focused_tree."}),
                    ),
                    (
                        "offset",
                        json!({"type": "integer", "minimum": 0, "description": "Character offset whose attribute run should be inspected."}),
                    ),
                    (
                        "include_defaults",
                        json!({"type": "boolean", "description": "Include default text attributes in the returned attribute set. Defaults to false."}),
                    ),
                ],
                vec!["node_id", "offset"],
            ),
        ),
        tool(
            "seatgeist.a11y_invoke",
            "Invoke Accessibility Action",
            "Invoke a normalized AT-SPI action on a node returned by an accessibility tree/find call. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "action",
                        json!({"type": "string", "enum": ["press", "focus", "select"], "description": "Normalized action to invoke."}),
                    ),
                    (
                        "destructive",
                        json!({"type": "boolean", "description": "Set true when invoking this node may delete, discard, close, quit, overwrite, or otherwise lose state; routes through destructive-action policy."}),
                    ),
                ]),
                vec!["node_id", "action"],
            ),
        ),
        tool(
            "seatgeist.a11y_set_text",
            "Set Accessibility Text",
            "Replace text on a non-sensitive AT-SPI EditableText node. This is policy-gated semantic control and summaries report text length only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "text",
                        json!({"type": "string", "description": "Replacement text for the editable node."}),
                    ),
                ]),
                vec!["node_id", "text"],
            ),
        ),
        tool(
            "seatgeist.a11y_insert_text",
            "Insert Accessibility Text",
            "Insert UTF-8 text at a character offset on a non-sensitive AT-SPI EditableText node. This is policy-gated semantic control and summaries report text length only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "offset",
                        json!({"type": "integer", "minimum": 0, "description": "Character offset at which to insert the text."}),
                    ),
                    (
                        "text",
                        json!({"type": "string", "description": "UTF-8 text to insert."}),
                    ),
                ]),
                vec!["node_id", "offset", "text"],
            ),
        ),
        tool(
            "seatgeist.a11y_delete_text",
            "Delete Accessibility Text",
            "Delete a character-offset range from a non-sensitive AT-SPI EditableText node without copying it to clipboard. This is policy-gated semantic control and summaries report offsets only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "start_offset",
                        json!({"type": "integer", "minimum": 0, "description": "Starting character offset to delete."}),
                    ),
                    (
                        "end_offset",
                        json!({"type": "integer", "minimum": 1, "description": "First character offset past the deleted range."}),
                    ),
                ]),
                vec!["node_id", "start_offset", "end_offset"],
            ),
        ),
        tool(
            "seatgeist.a11y_copy_text",
            "Copy Accessibility Text",
            "Copy a character-offset range from a non-sensitive AT-SPI EditableText node into the system clipboard. This is policy-gated semantic control and summaries report offsets only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "start_offset",
                        json!({"type": "integer", "minimum": 0, "description": "Starting character offset to copy."}),
                    ),
                    (
                        "end_offset",
                        json!({"type": "integer", "minimum": 1, "description": "First character offset past the copied range."}),
                    ),
                ]),
                vec!["node_id", "start_offset", "end_offset"],
            ),
        ),
        tool(
            "seatgeist.a11y_cut_text",
            "Cut Accessibility Text",
            "Cut a character-offset range from a non-sensitive AT-SPI EditableText node into the system clipboard. This is policy-gated semantic control and summaries report offsets only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "start_offset",
                        json!({"type": "integer", "minimum": 0, "description": "Starting character offset to cut."}),
                    ),
                    (
                        "end_offset",
                        json!({"type": "integer", "minimum": 1, "description": "First character offset past the cut range."}),
                    ),
                ]),
                vec!["node_id", "start_offset", "end_offset"],
            ),
        ),
        tool(
            "seatgeist.a11y_paste_text",
            "Paste Accessibility Text",
            "Paste current system clipboard text at a character offset on a non-sensitive AT-SPI EditableText node. This is policy-gated semantic control and summaries report offset only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "offset",
                        json!({"type": "integer", "minimum": 0, "description": "Character offset at which to paste clipboard text."}),
                    ),
                ]),
                vec!["node_id", "offset"],
            ),
        ),
        tool(
            "seatgeist.a11y_set_caret",
            "Set Accessibility Caret",
            "Move the caret to a character offset on a non-sensitive AT-SPI Text node. This is policy-gated semantic control and summaries report offset only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "offset",
                        json!({"type": "integer", "minimum": 0, "description": "Character offset for the caret."}),
                    ),
                ]),
                vec!["node_id", "offset"],
            ),
        ),
        tool(
            "seatgeist.a11y_set_selection",
            "Set Accessibility Selection",
            "Set an existing text selection range on a non-sensitive AT-SPI Text node. This is policy-gated semantic control and summaries report the selection index and offsets only.",
            object_schema(
                with_guard_properties(vec![
                    (
                        "node_id",
                        json!({"type": "string", "description": "AT-SPI node id from a previous accessibility result."}),
                    ),
                    (
                        "selection_num",
                        json!({"type": "integer", "minimum": 0, "description": "Zero-based text selection index. Defaults to 0."}),
                    ),
                    (
                        "start_offset",
                        json!({"type": "integer", "minimum": 0, "description": "Starting character offset for the selection."}),
                    ),
                    (
                        "end_offset",
                        json!({"type": "integer", "minimum": 1, "description": "First character offset past the selected range."}),
                    ),
                ]),
                vec!["node_id", "start_offset", "end_offset"],
            ),
        ),
        tool(
            "seatgeist.journal_tail",
            "Journal Tail",
            "Read recent compact daemon journal entries.",
            object_schema(
                vec![
                    (
                        "limit",
                        json!({"type": "integer", "minimum": 1, "maximum": 200}),
                    ),
                    (
                        "method",
                        json!({"type": "string", "description": "Optional daemon method name filter, such as focus_window."}),
                    ),
                    (
                        "ok",
                        json!({"type": "boolean", "description": "Optional success filter."}),
                    ),
                ],
                vec![],
            ),
        ),
    ]
}

fn tool(name: &str, title: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn object_schema(properties: Vec<(&str, Value)>, required: Vec<&str>) -> Value {
    let mut property_map = serde_json::Map::new();
    for (name, schema) in properties {
        property_map.insert(name.to_string(), schema);
    }
    json!({
        "type": "object",
        "properties": property_map,
        "required": required,
        "additionalProperties": false
    })
}

fn guard_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "desktop_revision",
            json!({"type": "string", "description": "Opaque active-desktop revision returned by observe or readiness. Prefer this single guard over copying window metadata."}),
        ),
        (
            "expected_active_window",
            json!({"type": "string", "description": "Optional current active-window id guard."}),
        ),
        (
            "expected_active_app",
            json!({"type": "string", "description": "Optional current active-window app id guard."}),
        ),
        (
            "active_title_contains",
            json!({"type": "string", "description": "Optional current active-window title substring guard."}),
        ),
    ]
}

fn target_guard_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "semantic_handle",
            json!({"type": "string", "description": "Preferred owner-bound, one-shot semantic window handle from window_session operation=inventory. Expires after 10 seconds."}),
        ),
        (
            "target_window_id",
            json!({"type": "string", "description": "Pinned KWin destination id. Enables target-scoped background semantic authorization without asserting that this window is active."}),
        ),
        (
            "target_app_id",
            json!({"type": "string", "description": "Optional exact KWin app id expected on the pinned target."}),
        ),
        (
            "target_pid",
            json!({"type": "integer", "minimum": 1, "maximum": 4_294_967_295_u64, "description": "Optional PID generation guard for close/reopen detection."}),
        ),
        (
            "target_title_contains",
            json!({"type": "string", "description": "Optional KWin title substring expected on the pinned target."}),
        ),
    ]
}

fn post_action_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "observe_after",
            json!({"type": "boolean", "description": "Return a compact post-action observation in the same result. Defaults to true."}),
        ),
        (
            "settle_condition",
            json!({"type": "string", "enum": ["none", "auto", "stable", "active_window_change", "accessibility_change", "any_change"], "description": "Bounded condition used before returning the post-action observation. Auto uses active-window change for focus_window, target-scoped AT-SPI events for guarded semantic actions, and stable polling otherwise."}),
        ),
        (
            "settle_timeout_ms",
            json!({"type": "integer", "minimum": 1, "maximum": 10000, "description": "Maximum post-action settle time. Defaults to 1500ms."}),
        ),
        (
            "settle_interval_ms",
            json!({"type": "integer", "minimum": 10, "maximum": 1000, "description": "Post-action observation polling interval. Defaults to 100ms."}),
        ),
        (
            "include_image",
            json!({"type": "boolean", "description": "After settling, attach one bounded frame from a matching retained window session. Defaults to false and never opens portal consent."}),
        ),
        (
            "capture_session_id",
            json!({"type": "string", "description": "Retained session used only for the optional post-action image. Its pinned KWin identity must match the action target."}),
        ),
        (
            "image_output",
            json!({"type": "string", "description": "Optional private PNG path for the post-action retained frame."}),
        ),
        (
            "image_max_edge",
            json!({"type": "integer", "minimum": 1, "maximum": 2048, "description": "Maximum edge for the post-action retained frame."}),
        ),
        (
            "image_timeout_ms",
            json!({"type": "integer", "minimum": 1, "maximum": 30000, "description": "Frame acquisition timeout. Defaults to 1500ms."}),
        ),
    ]
}

fn with_post_action_properties(
    mut properties: Vec<(&'static str, Value)>,
) -> Vec<(&'static str, Value)> {
    properties.extend(post_action_properties());
    properties
}

fn with_guard_properties(mut properties: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
    properties.extend(guard_properties());
    with_post_action_properties(properties)
}

fn with_raw_guard_properties(
    mut properties: Vec<(&'static str, Value)>,
) -> Vec<(&'static str, Value)> {
    properties.push((
        "session_id",
        json!({"type": "string", "description": "Pinned window-session id. The daemon revalidates and focuses that target immediately before this raw action."}),
    ));
    with_guard_properties(properties)
}

fn with_semantic_guard_properties(
    mut properties: Vec<(&'static str, Value)>,
) -> Vec<(&'static str, Value)> {
    properties.extend(guard_properties());
    properties.extend(target_guard_properties());
    with_post_action_properties(properties)
}

fn active_window_guard(arguments: &Value) -> Result<Option<ActiveWindowGuard>> {
    let desktop_revision = optional_string(arguments, "desktop_revision")?;
    let expected_window_id = optional_string(arguments, "expected_active_window")?;
    let expected_app_id = optional_string(arguments, "expected_active_app")?;
    let title_contains = optional_string(arguments, "active_title_contains")?;
    if desktop_revision.is_none()
        && expected_window_id.is_none()
        && expected_app_id.is_none()
        && title_contains.is_none()
    {
        return Ok(None);
    }
    Ok(Some(ActiveWindowGuard {
        desktop_revision,
        expected_window_id,
        expected_app_id,
        title_contains,
    }))
}

fn target_window_guard(arguments: &Value) -> Result<Option<TargetWindowGuard>> {
    let semantic_handle = optional_string(arguments, "semantic_handle")?;
    let expected_window_id = optional_string(arguments, "target_window_id")?;
    let expected_app_id = optional_string(arguments, "target_app_id")?;
    let expected_pid = optional_u64(arguments, "target_pid")?
        .map(u64_to_u32)
        .transpose()?;
    let title_contains = optional_string(arguments, "target_title_contains")?;
    if semantic_handle.is_none()
        && expected_window_id.is_none()
        && expected_app_id.is_none()
        && expected_pid.is_none()
        && title_contains.is_none()
    {
        return Ok(None);
    }
    if semantic_handle.is_some() && expected_window_id.is_some() {
        bail!("semantic_handle and target_window_id are mutually exclusive");
    }
    if semantic_handle.is_some()
        && (expected_app_id.is_some() || expected_pid.is_some() || title_contains.is_some())
    {
        bail!("semantic_handle cannot be combined with copied target metadata");
    }
    let expected_window_id = semantic_handle.or(expected_window_id).ok_or_else(|| {
        anyhow!("semantic_handle or target_window_id is required when a target guard is supplied")
    })?;
    Ok(Some(TargetWindowGuard {
        expected_window_id,
        expected_app_id,
        expected_pid,
        title_contains,
    }))
}

fn required_string(arguments: &Value, key: &str) -> Result<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("argument '{key}' is required and must be a non-empty string"))
}

fn optional_output_path(arguments: &Value, key: &str, kind: &str) -> Result<PathBuf> {
    match optional_string(arguments, key)? {
        Some(output) => Ok(output.into()),
        None => default_screenshot_output_path(kind)
            .with_context(|| format!("resolve default screenshot output path for {kind}")),
    }
}

fn required_u32(arguments: &Value, key: &str) -> Result<u32> {
    let value = arguments
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("argument '{key}' is required and must be an unsigned integer"))?;
    u64_to_u32(value)
}

fn required_i32(arguments: &Value, key: &str) -> Result<i32> {
    let value = arguments
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("argument '{key}' is required and must be a signed integer"))?;
    i64_to_i32(value)
}

fn required_f64(arguments: &Value, key: &str) -> Result<f64> {
    let value = arguments
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("argument '{key}' is required and must be a number"))?;
    if !value.is_finite() {
        bail!("argument '{key}' must be finite");
    }
    Ok(value)
}

fn optional_u64(arguments: &Value, key: &str) -> Result<Option<u64>> {
    match arguments.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| anyhow!("argument '{key}' must be an unsigned integer")),
    }
}

fn optional_u32(arguments: &Value, key: &str) -> Result<Option<u32>> {
    optional_u64(arguments, key)?.map(u64_to_u32).transpose()
}

fn optional_i32(arguments: &Value, key: &str) -> Result<Option<i32>> {
    match arguments.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value
            .as_i64()
            .map(i64_to_i32)
            .transpose()?
            .map(Some)
            .ok_or_else(|| anyhow!("argument '{key}' must be a signed integer")),
    }
}

fn optional_f64(arguments: &Value, key: &str) -> Result<Option<f64>> {
    match arguments.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| anyhow!("argument '{key}' must be a number")),
    }
}

fn optional_string(arguments: &Value, key: &str) -> Result<Option<String>> {
    match arguments.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| anyhow!("argument '{key}' must be a non-empty string")),
    }
}

fn optional_bool(arguments: &Value, key: &str) -> Result<Option<bool>> {
    match arguments.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| anyhow!("argument '{key}' must be a boolean")),
    }
}

fn required_accessibility_action(arguments: &Value, key: &str) -> Result<AccessibilityAction> {
    required_string(arguments, key)?
        .parse()
        .map_err(|err: String| anyhow!(err))
}

fn required_coordinate_space(arguments: &Value, key: &str) -> Result<CoordinateSpace> {
    required_string(arguments, key)?
        .parse()
        .map_err(|err: String| anyhow!(err))
}

fn required_pointer_button(arguments: &Value, key: &str) -> Result<PointerButton> {
    required_string(arguments, key)?
        .parse()
        .map_err(|err: String| anyhow!(err))
}

fn optional_pointer_button(arguments: &Value, key: &str) -> Result<Option<PointerButton>> {
    optional_string(arguments, key)?
        .map(|value| value.parse().map_err(|err: String| anyhow!(err)))
        .transpose()
}

fn optional_portal_screenshot_target(
    arguments: &Value,
    key: &str,
) -> Result<Option<PortalScreenshotTarget>> {
    optional_string(arguments, key)?
        .map(|value| value.parse().map_err(|err: String| anyhow!(err)))
        .transpose()
}

fn optional_remote_desktop_persist_mode(
    arguments: &Value,
    key: &str,
) -> Result<Option<RemoteDesktopPersistMode>> {
    optional_string(arguments, key)?
        .map(
            |value| match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
                "do_not_persist" | "none" | "0" => Ok(RemoteDesktopPersistMode::DoNotPersist),
                "application_lifetime" | "app_lifetime" | "1" => {
                    Ok(RemoteDesktopPersistMode::ApplicationLifetime)
                }
                "explicitly_revoked" | "revoked" | "2" => {
                    Ok(RemoteDesktopPersistMode::ExplicitlyRevoked)
                }
                other => bail!("unsupported RemoteDesktop persist mode: {other}"),
            },
        )
        .transpose()
}

fn remote_desktop_session_request(arguments: &Value) -> Result<RemoteDesktopSessionProbeRequest> {
    let keyboard = optional_bool(arguments, "keyboard")?;
    let pointer = optional_bool(arguments, "pointer")?;
    let touchscreen = optional_bool(arguments, "touchscreen")?.unwrap_or(false);
    let any_device = keyboard.is_some() || pointer.is_some() || touchscreen;
    Ok(RemoteDesktopSessionProbeRequest {
        keyboard: keyboard.unwrap_or(!any_device),
        pointer: pointer.unwrap_or(!any_device),
        touchscreen,
        restore_token: optional_string(arguments, "restore_token")?,
        persist_mode: optional_remote_desktop_persist_mode(arguments, "persist_mode")?,
        parent_window: optional_string(arguments, "parent_window")?,
        timeout_ms: optional_u64(arguments, "timeout_ms")?
            .unwrap_or(DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS),
        guard: active_window_guard(arguments)?,
    })
}

fn required_string_array(arguments: &Value, key: &str) -> Result<Vec<String>> {
    let array = arguments
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("argument '{key}' is required and must be an array of strings"))?;
    let values = array
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().to_string())
                .ok_or_else(|| anyhow!("argument '{key}' must contain non-empty strings"))
        })
        .collect::<Result<Vec<_>>>()?;
    if values.is_empty() {
        bail!("argument '{key}' must contain at least one string");
    }
    Ok(values)
}

fn u64_to_u32(value: u64) -> Result<u32> {
    u32::try_from(value).map_err(|_| anyhow!("integer argument {value} exceeds u32"))
}

fn u64_to_usize(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| anyhow!("integer argument {value} exceeds usize"))
}

fn u64_to_u8(value: u64) -> Result<u8> {
    u8::try_from(value).map_err(|_| anyhow!("integer argument {value} exceeds u8"))
}

fn i64_to_i32(value: i64) -> Result<i32> {
    i32::try_from(value).map_err(|_| anyhow!("integer argument {value} exceeds i32"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use libseatgeist::{
        AccessibilityQualityStatus, CaptureBackendStatus, CaptureSessionStatus,
        ClipboardBackendStatus, ComputerUseReadinessStatus, InputBackendStatus, KwinMetadataStatus,
        LibeiStatus, RemoteDesktopPortalStatus, SafetyStatus, ScreenshotPortalStatus,
        SpectacleStatus, XkbKeymapStatus,
    };
    use libseatgeist::{ScreenshotTransform, WaitForChangeResult};
    use std::{
        fs,
        os::unix::fs::symlink,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn initialize_advertises_tools_capability() {
        let result = initialize_result(ToolProfile::All);
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        assert!(
            result["instructions"]
                .as_str()
                .unwrap_or_default()
                .contains("policy-gated")
        );
    }

    #[test]
    fn empty_resource_discovery_is_protocol_compatible() {
        let server = McpServer {
            socket: PathBuf::from("/tmp/seatgeist-test.sock"),
            tool_profile: ToolProfile::All,
            readiness_cache: readiness::ReadinessCache::default(),
        };
        let resources = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"resources/list","params":{}}"#)
            .expect("resource request parses")
            .expect("resource request responds");
        assert_eq!(resources.result, Some(json!({"resources": []})));
        assert!(resources.error.is_none());

        let templates = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":2,"method":"resources/templates/list","params":{}}"#,
            )
            .expect("template request parses")
            .expect("template request responds");
        assert_eq!(templates.result, Some(json!({"resourceTemplates": []})));
        assert!(templates.error.is_none());
    }

    #[test]
    fn screenshot_compact_text_includes_backend_provenance() {
        let screenshot = sample_screenshot_info("spectacle");
        let text = compact_tool_text(
            "seatgeist.screenshot",
            &DaemonResponse::Screenshot(screenshot.clone()),
        );
        assert!(text.contains("via spectacle"));

        let mut crop = screenshot.clone();
        crop.backend = "visible_window_crop".to_string();
        crop.occlusion_possible = true;
        let text = compact_tool_text("seatgeist.screenshot", &DaemonResponse::Screenshot(crop));
        assert!(text.contains("via visible_window_crop"));
        assert!(text.contains("occlusion_possible=true"));

        let text = compact_tool_text(
            "seatgeist.wait_for_change",
            &DaemonResponse::WaitForChange(Box::new(WaitForChangeResult {
                changed: true,
                timed_out: false,
                timeout_ms: 5_000,
                interval_ms: 250,
                captures: 2,
                elapsed_ms: 250,
                score: 0.25,
                threshold: 0.01,
                screenshot: screenshot.clone(),
            })),
        );
        assert!(text.contains("backend=spectacle"));
        assert!(text.contains("timed_out=false"));
        assert!(text.contains("elapsed_ms=250"));

        let text = compact_tool_text(
            "seatgeist.wait_for_change",
            &DaemonResponse::WaitForChange(Box::new(WaitForChangeResult {
                changed: false,
                timed_out: true,
                timeout_ms: 5_000,
                interval_ms: 250,
                captures: 20,
                elapsed_ms: 5_000,
                score: 0.0,
                threshold: 0.01,
                screenshot,
            })),
        );
        assert!(text.contains("changed=false"));
        assert!(text.contains("timed_out=true"));
        assert!(text.contains("timeout_ms=5000"));
    }

    #[test]
    fn capture_status_reports_only_the_opaque_restore_reference() {
        let response = DaemonResponse::CaptureSessionStatus(CaptureSessionStatus {
            active: true,
            opening: false,
            session_id: Some("capture-1".to_string()),
            backend: Some("portal_screencast_pipewire".to_string()),
            source_type: Some("window".to_string()),
            source_id: Some("opaque-source".to_string()),
            restore_token_reference: Some("screencast-a1b2c3d4".to_string()),
            requested_window_id: Some("window-1".to_string()),
            requested_source_type: Some("window".to_string()),
            requested_source_id: Some("window-1".to_string()),
            owner_tool: Some("seatgeist-mcp".to_string()),
            owner_pid: Some(4242),
            owner_scope: Some("process".to_string()),
            latest_revision: None,
            consent_required: true,
            occlusion_possible: false,
            sticky_target_bound: false,
            target_window_id: None,
            target_app_id: None,
            target_pid: None,
            target_expires_in_ms: None,
            last_end_reason: None,
            execution: Some(Box::new(libseatgeist::SessionExecutionStatus {
                capture_backend: "portal_screencast_pipewire".to_string(),
                semantic_backend: Some("atspi".to_string()),
                raw_input_backend: Some("uinput".to_string()),
                last_action_backend: Some("atspi".to_string()),
                last_action_method: Some("set_text_field".to_string()),
                last_action_safety_class: Some(libseatgeist::SafetyClass::ControlKeyboard),
                last_action_id: None,
                last_action_unix_ms: Some(1_725_000_000_000),
                target_policy_result: Some("allow".to_string()),
                last_policy_result: Some("allow".to_string()),
                cooperative_focus_policy: Some(
                    "reacquire_verify_inject_restore_if_safe".to_string(),
                ),
                activity_backend: Some("kwin_input_spy_v1".to_string()),
                activity_trusted: true,
                last_activity_class: Some("keyboard".to_string()),
                last_activity_provenance: Some("seatgeist_injected".to_string()),
                focus_lease: Some(libseatgeist::SessionFocusLeaseStatus {
                    lease_id: "00000000-0000-0000-0000-000000000000"
                        .parse()
                        .expect("nil lease id parses"),
                    focus_reacquired: true,
                    focus_restored: true,
                    restoration: "restored".to_string(),
                }),
                settle: Some(libseatgeist::ActionSettleResult {
                    confirmation: libseatgeist::ActionConfirmation::Confirmed,
                    condition: libseatgeist::ActionSettleCondition::AccessibilityChange,
                    backend: libseatgeist::ActionSettleBackend::AtspiEvent,
                    target_scoped: true,
                    event: Some("object:text-changed".to_string()),
                    settled: true,
                    timed_out: false,
                    timeout_ms: 1_000,
                    interval_ms: 100,
                    samples: 1,
                    elapsed_ms: 12,
                    before_revision: Some("before".to_string()),
                    after_revision: "after".to_string(),
                }),
            })),
        });
        let text = compact_tool_text("seatgeist.window_session", &response);
        assert!(text.contains("restore_ref=screencast-a1b2c3d4"));
        assert!(text.contains("requested_source=window"));
        assert!(text.contains("requested_id=window-1"));
        assert!(text.contains("owner_tool=seatgeist-mcp"));
        assert!(text.contains("owner_scope=process"));
        assert!(text.contains("owner_pid=4242"));
        assert!(text.contains("end_reason=none"));
        assert!(text.contains("capture_exec=portal_screencast_pipewire"));
        assert!(text.contains("semantic=atspi"));
        assert!(text.contains("raw=uinput"));
        assert!(text.contains("last_method=set_text_field"));
        assert!(text.contains("last_policy=allow"));
        assert!(text.contains("activity_backend=kwin_input_spy_v1"));
        assert!(text.contains("activity_trusted=true"));
        assert!(text.contains("focus_reacquired=true"));
        assert!(text.contains("focus_restored=true"));
        assert!(text.contains("settle_backend=atspievent"));
        assert!(text.contains("settled=true"));
        assert!(!text.contains("private-restore-token"));
    }

    #[test]
    fn action_compact_text_reports_target_event_settle_provenance() {
        let response = DaemonResponse::Action(Box::new(libseatgeist::ActionResult {
            id: "00000000-0000-0000-0000-000000000000"
                .parse()
                .expect("nil action id parses"),
            ok: true,
            observation: Some(libseatgeist::Observation {
                active_window: None,
                target_window: None,
                windows: Vec::new(),
                monitors: Vec::new(),
                focused_accessibility: None,
                target_accessibility: None,
                screenshot_path: None,
                revision: Some("revision".to_string()),
                issues: Vec::new(),
                settle: Some(libseatgeist::ActionSettleResult {
                    confirmation: libseatgeist::ActionConfirmation::Confirmed,
                    condition: ActionSettleCondition::AccessibilityChange,
                    backend: libseatgeist::ActionSettleBackend::AtspiEvent,
                    target_scoped: true,
                    event: Some("object.statechanged".to_string()),
                    settled: true,
                    timed_out: false,
                    timeout_ms: 1_500,
                    interval_ms: 100,
                    samples: 1,
                    elapsed_ms: 4,
                    before_revision: Some("before".to_string()),
                    after_revision: "after".to_string(),
                }),
            }),
            screenshot: None,
            message: Some("clicked button".to_string()),
        }));

        let text = compact_tool_text("seatgeist.click_button", &response);
        assert!(text.contains("backend=AtspiEvent"));
        assert!(text.contains("target_scoped=true"));
        assert!(text.contains("event=object.statechanged"));
        assert!(!text.contains("before"));
        assert!(!text.contains("after"));
    }

    #[test]
    fn bounded_screenshot_result_attaches_native_mcp_image_content() {
        const ONE_PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let path = temporary_test_path("native-image.png");
        fs::write(
            &path,
            BASE64_STANDARD
                .decode(ONE_PIXEL_PNG)
                .expect("fixture PNG decodes"),
        )
        .expect("fixture PNG writes");
        let mut screenshot = sample_screenshot_info("mock");
        screenshot.path = path.clone();

        let result = tool_result_from_daemon(
            "seatgeist.screenshot",
            &DaemonResponse::Screenshot(screenshot),
            true,
        );

        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][1]["type"], "image");
        assert_eq!(result["content"][1]["mimeType"], "image/png");
        assert_eq!(result["content"][1]["data"], ONE_PIXEL_PNG);
        fs::remove_file(path).expect("fixture PNG removes");
    }

    #[test]
    fn action_result_can_attach_one_retained_post_action_frame() {
        const ONE_PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let path = temporary_test_path("native-action-image.png");
        fs::write(
            &path,
            BASE64_STANDARD
                .decode(ONE_PIXEL_PNG)
                .expect("fixture PNG decodes"),
        )
        .expect("fixture PNG writes");
        let mut screenshot = sample_screenshot_info("portal_screencast_pipewire");
        screenshot.path = path.clone();
        let response = DaemonResponse::Action(Box::new(libseatgeist::ActionResult {
            id: "00000000-0000-0000-0000-000000000000"
                .parse()
                .expect("nil action id parses"),
            ok: true,
            observation: None,
            screenshot: Some(screenshot),
            message: Some("action complete".to_string()),
        }));

        assert!(
            image_attachment_requested(
                &DaemonRequest::TypeText(TypeTextRequest {
                    text: "x".to_string(),
                    guard: None,
                    session_id: Some("capture-1".to_string()),
                }),
                &json!({"include_image": true})
            )
            .expect("action image opt-in maps")
        );
        let result = tool_result_from_daemon("seatgeist.act", &response, true);
        assert_eq!(result["content"][1]["type"], "image");
        assert_eq!(result["content"][1]["data"], ONE_PIXEL_PNG);
        fs::remove_file(path).expect("fixture PNG removes");
    }

    #[test]
    fn mcp_image_attachment_refuses_symlink_outputs() {
        const ONE_PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let target = temporary_test_path("image-target.png");
        let link = temporary_test_path("image-link.png");
        fs::write(
            &target,
            BASE64_STANDARD
                .decode(ONE_PIXEL_PNG)
                .expect("fixture PNG decodes"),
        )
        .expect("fixture PNG writes");
        symlink(&target, &link).expect("fixture symlink creates");

        let mut screenshot = sample_screenshot_info("mock");
        screenshot.path = link.clone();
        let err = mcp_png_image_content(&screenshot).expect_err("symlink image must be refused");
        assert!(err.to_string().contains("without following symlinks"));
        fs::remove_file(link).expect("fixture symlink removes");
        fs::remove_file(target).expect("fixture target removes");
    }

    #[test]
    fn mcp_image_attachment_refuses_oversized_dimensions_before_reading() {
        let mut screenshot = sample_screenshot_info("mock");
        screenshot.output_width = MAX_MCP_IMAGE_EDGE + 1;
        let err = mcp_png_image_content(&screenshot).expect_err("oversized image must be refused");
        assert!(err.to_string().contains("pixel MCP attachment edge limit"));
    }

    #[test]
    fn image_attachment_defaults_to_bounded_screenshot_tools_only() {
        let bounded = DaemonRequest::Screenshot(ScreenshotRequest {
            output: "/tmp/bounded.png".into(),
            max_edge: Some(800),
            full_resolution: false,
            portal_interactive: false,
            portal_target: None,
            visible_window_crop_id: None,
        });
        assert!(image_attachment_requested(&bounded, &json!({})).expect("bounded default"));
        assert!(
            !image_attachment_requested(&bounded, &json!({"include_image": false}))
                .expect("bounded opt out")
        );

        let full = DaemonRequest::Screenshot(ScreenshotRequest {
            output: "/tmp/full.png".into(),
            max_edge: None,
            full_resolution: true,
            portal_interactive: false,
            portal_target: None,
            visible_window_crop_id: None,
        });
        assert!(!image_attachment_requested(&full, &json!({})).expect("full default omitted"));
        let err = image_attachment_requested(&full, &json!({"include_image": true}))
            .expect_err("full image embedding is refused");
        assert!(err.to_string().contains("cannot be embedded"));

        let observe = DaemonRequest::Observe(ObserveRequest { screenshot: None });
        let err = image_attachment_requested(&observe, &json!({"include_image": true}))
            .expect_err("observe requires screenshot output");
        assert!(err.to_string().contains("requires screenshot_output"));
    }

    #[test]
    fn action_tools_default_to_one_post_action_observation() {
        let request = DaemonRequest::TypeText(TypeTextRequest {
            text: "hello".to_string(),
            guard: None,
            session_id: None,
        });
        let options = response_options_for_request(&request, &json!({}))
            .expect("default response options map")
            .and_then(|options| options.post_action)
            .expect("post-action options are present");
        assert!(options.observe_after);
        assert_eq!(options.settle_condition, ActionSettleCondition::Auto);
        assert_eq!(
            options.settle_timeout_ms,
            DEFAULT_POST_ACTION_SETTLE_TIMEOUT_MS
        );
        assert_eq!(
            options.settle_interval_ms,
            DEFAULT_POST_ACTION_SETTLE_INTERVAL_MS
        );
        assert!(options.image.is_none());

        let image = response_options_for_request(
            &request,
            &json!({
                "include_image": true,
                "capture_session_id": "capture-1",
                "image_output": "/tmp/post-action.png",
                "image_max_edge": 800,
                "image_timeout_ms": 1200
            }),
        )
        .expect("post-action image options map")
        .and_then(|options| options.post_action)
        .and_then(|options| options.image)
        .expect("post-action image is present");
        assert_eq!(image.session_id, "capture-1");
        assert_eq!(image.output, PathBuf::from("/tmp/post-action.png"));
        assert_eq!(image.max_edge, Some(800));
        assert_eq!(image.timeout_ms, 1_200);

        let err = response_options_for_request(&request, &json!({"include_image": true}))
            .expect_err("post-action image requires a retained session");
        assert!(err.to_string().contains("requires capture_session_id"));

        assert_eq!(
            response_options_for_request(&request, &json!({"observe_after": false}))
                .expect("post-action observation opts out"),
            None
        );
        let err = response_options_for_request(
            &request,
            &json!({"observe_after": false, "settle_condition": "stable"}),
        )
        .expect_err("settling without observation is invalid");
        assert!(err.to_string().contains("observe_after=false"));
    }

    #[test]
    fn action_tool_schemas_expose_post_action_controls() {
        let tools = tool_definitions(ToolProfile::All);
        for name in [
            "seatgeist.focus_window",
            "seatgeist.type_text",
            "seatgeist.click_pointer",
            "seatgeist.click_button",
            "seatgeist.clipboard_set_text",
            "seatgeist.a11y_set_text",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("action tool is listed");
            let properties = &tool["inputSchema"]["properties"];
            assert!(properties["observe_after"].is_object(), "{name}");
            assert!(properties["settle_condition"].is_object(), "{name}");
            assert!(properties["settle_timeout_ms"].is_object(), "{name}");
            assert!(properties["settle_interval_ms"].is_object(), "{name}");
            assert!(properties["include_image"].is_object(), "{name}");
            assert!(properties["capture_session_id"].is_object(), "{name}");
            assert!(properties["image_max_edge"].is_object(), "{name}");
        }

        for name in [
            "seatgeist.type_text",
            "seatgeist.key_combo",
            "seatgeist.move_pointer",
            "seatgeist.click_pointer",
            "seatgeist.drag_pointer",
            "seatgeist.scroll_pointer",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("raw action tool is listed");
            assert!(
                tool["inputSchema"]["properties"]["session_id"].is_object(),
                "{name}"
            );
        }
    }

    #[test]
    fn text_attributes_compact_text_reports_range_and_count_only() {
        let text = compact_tool_text(
            "seatgeist.a11y_text_attributes",
            &DaemonResponse::AccessibilityTextAttributes(
                libseatgeist::AccessibilityTextAttributes {
                    node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                    start_offset: 2,
                    end_offset: 8,
                    attributes: vec![libseatgeist::TextAttribute {
                        name: "weight".to_string(),
                        value: "bold".to_string(),
                    }],
                },
            ),
        );
        assert_eq!(
            text,
            "text attributes range=2..8 count=1 node=atspi://:1.42/org/a11y/atspi/accessible/7"
        );
        assert!(!text.contains("bold"));
    }

    #[test]
    fn accessibility_quality_compact_text_reports_fallback() {
        let text = compact_tool_text(
            "seatgeist.a11y_quality_status",
            &DaemonResponse::AccessibilityQualityStatus(AccessibilityQualityStatus {
                atspi_available: true,
                target_event_settle_available: true,
                event_backend: "atspi_registry".to_string(),
                target_event_classes: vec![
                    "object".to_string(),
                    "window".to_string(),
                    "focus".to_string(),
                ],
                focused_node_present: true,
                sample_depth: 4,
                sample_max_nodes: 512,
                sampled_node_count: 12,
                named_node_count: 5,
                actionable_node_count: 3,
                text_node_count: 2,
                sensitive_node_count: 1,
                generic_role_count: 2,
                max_depth_seen: 3,
                tree_flat: false,
                semantic_targeting_reliable: true,
                recommended_fallback: "atspi_semantic".to_string(),
                setup_hint: "prefer semantic actions".to_string(),
            }),
        );
        assert!(text.contains("atspi=true"));
        assert!(text.contains("target_events=true"));
        assert!(text.contains("event_backend=atspi_registry"));
        assert!(text.contains("reliable=true"));
        assert!(text.contains("nodes=12"));
        assert!(text.contains("fallback=atspi_semantic"));
    }

    #[test]
    fn readiness_compact_text_reports_preflight_summary() {
        let text = compact_tool_text(
            "seatgeist.computer_use_readiness",
            &DaemonResponse::ComputerUseReadiness(ComputerUseReadinessStatus {
                ready_for_observe: true,
                ready_for_screenshot: true,
                ready_for_window_control: false,
                ready_for_keyboard_input: false,
                ready_for_pointer_input: false,
                ready_for_semantic_actions: true,
                ready_for_clipboard_read: false,
                ready_for_clipboard_write: true,
                observe_state: libseatgeist::ActionReadiness::Ready,
                screenshot_state: libseatgeist::ActionReadiness::Ready,
                window_control_state: libseatgeist::ActionReadiness::NeedsApproval,
                keyboard_input_state: libseatgeist::ActionReadiness::Unavailable,
                pointer_input_state: libseatgeist::ActionReadiness::Unavailable,
                semantic_action_state: libseatgeist::ActionReadiness::NeedsApproval,
                clipboard_read_state: libseatgeist::ActionReadiness::NeedsApproval,
                clipboard_write_state: libseatgeist::ActionReadiness::Ready,
                desktop_revision: Some("aw1:test".to_string()),
                focus_guard_required: true,
                panic_stop_enabled: false,
                human_input_pause_enabled: true,
                human_input_signal_fresh: false,
                desktop_session_ready: true,
                dbus_session_bus_present: true,
                runtime_dir_present: true,
                capture_backend: Some("portal_screenshot".to_string()),
                input_backend: None,
                clipboard_read_backend: None,
                clipboard_write_backend: Some("wl-clipboard".to_string()),
                accessibility_backend: "atspi_semantic".to_string(),
                issues: vec!["no executable input backend is available".to_string()],
                next_steps: vec!["check seatgeist.input_backend_status".to_string()],
            }),
        );
        assert!(text.contains("observe=ready"));
        assert!(text.contains("keyboard=unavailable"));
        assert!(text.contains("window_control=needs_approval"));
        assert!(text.contains("desktop_revision=aw1:test"));
        assert!(text.contains("issues=1"));
        assert!(text.contains("capture_backend=portal_screenshot"));
        assert!(text.contains("input_backend=none"));
    }

    #[test]
    fn safety_status_compact_text_reports_active_gates() {
        let text = compact_tool_text(
            "seatgeist.safety_status",
            &DaemonResponse::SafetyStatus(SafetyStatus {
                require_focus_guard: true,
                pause_on_human_input: true,
                human_input_activity_file: Some(PathBuf::from(
                    "/run/user/1000/seatgeist/human-input-active",
                )),
                human_input_quiet_ms: 2500,
                human_input_signal_fresh: true,
                human_input_signal_age_ms: Some(100),
                human_input_activity_backend: Some("kwin_input_spy_v1".to_string()),
                human_input_activity_trusted: true,
                human_input_last_class: Some("pointer".to_string()),
                human_input_last_provenance: Some("trusted_physical".to_string()),
                control_rate_limit_per_minute: Some(120),
                preview_max_edge: 1600,
                tile_max_edge: 1600,
                screenshot_redaction_count: 2,
                journal_artifact_metadata_enabled: true,
            }),
        );
        assert!(text.contains("focus_guard=true"));
        assert!(text.contains("human_signal_fresh=true"));
        assert!(text.contains("activity_backend=kwin_input_spy_v1"));
        assert!(text.contains("activity_trusted=true"));
        assert!(text.contains("last_provenance=trusted_physical"));
        assert!(text.contains("control_rate_limit_per_minute=120"));
        assert!(text.contains("preview_max_edge=1600"));
        assert!(text.contains("tile_max_edge=1600"));
        assert!(text.contains("redactions=2"));
        assert!(text.contains("journal_artifacts=true"));
    }

    #[test]
    fn backend_status_compact_text_reports_implemented_backend() {
        let input_text = compact_tool_text(
            "seatgeist.input_backend_status",
            &DaemonResponse::InputBackendStatus(InputBackendStatus {
                uinput_available: true,
                remote_desktop_portal: RemoteDesktopPortalStatus {
                    busctl_available: true,
                    portal_service_available: true,
                    remote_desktop_interface_available: true,
                    kde_portal_service_available: true,
                    setup_hint: "portal visible".to_string(),
                },
                libei: LibeiStatus {
                    pkg_config_available: true,
                    client_library_available: true,
                    socket_env_present: false,
                    setup_hint: "libei visible".to_string(),
                },
                eis_keymap: XkbKeymapStatus {
                    source: "kde_current_layout".to_string(),
                    rules: None,
                    model: Some("pc105".to_string()),
                    layout: Some("de".to_string()),
                    variant: Some("nodeadkeys".to_string()),
                    options: Some("".to_string()),
                    kde_current_layout: Some("de(nodeadkeys)".to_string()),
                    kde_config_layouts: Some("de,us".to_string()),
                    setup_hint: "using KDE current keyboard layout".to_string(),
                },
                configured_backend: "portal_remote_desktop".to_string(),
                preferred_available_backend: Some("portal_remote_desktop".to_string()),
                implemented_available_backend: Some("uinput".to_string()),
                setup_hint: "portal visible, uinput implemented".to_string(),
            }),
        );
        assert!(input_text.contains("configured=portal_remote_desktop"));
        assert!(input_text.contains("preferred=portal_remote_desktop"));
        assert!(input_text.contains("implemented=uinput"));
        assert!(input_text.contains("eis_keymap_source=kde_current_layout"));
        assert!(input_text.contains("eis_keymap_layout=de"));

        let remote_desktop_text = compact_tool_text(
            "seatgeist.remote_desktop_session_probe",
            &DaemonResponse::RemoteDesktopSessionProbe(libseatgeist::RemoteDesktopSessionProbe {
                started: true,
                requested_devices: vec!["keyboard".to_string(), "pointer".to_string()],
                selected_devices: vec!["pointer".to_string()],
                clipboard_enabled: false,
                restore_token: None,
                session_handle: None,
                create_request_path: None,
                select_request_path: None,
                start_request_path: None,
                transient_session_closed: true,
                setup_hint: "transient probe".to_string(),
            }),
        );
        assert!(remote_desktop_text.contains("started=true"));
        assert!(remote_desktop_text.contains("requested=keyboard+pointer"));
        assert!(remote_desktop_text.contains("selected=pointer"));
        assert!(remote_desktop_text.contains("transient_closed=true"));

        let remote_desktop_eis_text = compact_tool_text(
            "seatgeist.remote_desktop_eis_probe",
            &DaemonResponse::RemoteDesktopEisProbe(libseatgeist::RemoteDesktopEisProbe {
                started: true,
                eis_connected: true,
                eis_runtime_connected: true,
                eis_event_count: 2,
                eis_bound_capabilities: vec!["text".to_string()],
                eis_resumed_device_count: 1,
                requested_devices: vec!["keyboard".to_string(), "pointer".to_string()],
                selected_devices: vec!["keyboard".to_string(), "pointer".to_string()],
                clipboard_enabled: false,
                restore_token: None,
                session_handle: None,
                create_request_path: None,
                select_request_path: None,
                start_request_path: None,
                eis_fd_closed: true,
                transient_session_closed: true,
                setup_hint: "EIS probe".to_string(),
            }),
        );
        assert!(remote_desktop_eis_text.contains("eis_connected=true"));
        assert!(remote_desktop_eis_text.contains("runtime_connected=true"));
        assert!(remote_desktop_eis_text.contains("events=2"));
        assert!(remote_desktop_eis_text.contains("bound=text"));
        assert!(remote_desktop_eis_text.contains("resumed_devices=1"));
        assert!(remote_desktop_eis_text.contains("fd_closed=true"));
        assert!(remote_desktop_eis_text.contains("transient_closed=true"));

        let remote_desktop_eis_status_text = compact_tool_text(
            "seatgeist.remote_desktop_eis_session_status",
            &DaemonResponse::RemoteDesktopEisSessionStatus(
                libseatgeist::RemoteDesktopEisSessionStatus {
                    active: true,
                    runtime_connected: true,
                    bound_capabilities: vec!["text".to_string(), "pointer".to_string()],
                    resumed_device_count: 2,
                    selected_devices: vec!["keyboard".to_string()],
                    clipboard_enabled: false,
                    restore_token: None,
                    session_handle: None,
                    create_request_path: None,
                    select_request_path: None,
                    start_request_path: None,
                    setup_hint: "stored session".to_string(),
                },
            ),
        );
        assert!(remote_desktop_eis_status_text.contains("active=true"));
        assert!(remote_desktop_eis_status_text.contains("runtime_connected=true"));
        assert!(remote_desktop_eis_status_text.contains("bound=text+pointer"));
        assert!(remote_desktop_eis_status_text.contains("resumed_devices=2"));
        assert!(remote_desktop_eis_status_text.contains("selected=keyboard"));

        let capture_text = compact_tool_text(
            "seatgeist.capture_backend_status",
            &DaemonResponse::CaptureBackendStatus(CaptureBackendStatus {
                screenshot_portal: ScreenshotPortalStatus {
                    busctl_available: true,
                    portal_service_available: true,
                    screenshot_interface_available: true,
                    screenshot_interface_version: Some(3),
                    screenshot_available_targets_mask: Some(15),
                    screenshot_available_targets: vec![
                        "screen".to_string(),
                        "window".to_string(),
                        "area".to_string(),
                        "active_window".to_string(),
                    ],
                    screenshot_target_option_supported: true,
                    screencast_interface_available: true,
                    kde_portal_service_available: true,
                    setup_hint: "portal visible".to_string(),
                },
                kwin_metadata: KwinMetadataStatus {
                    busctl_available: true,
                    kwin_service_available: true,
                    support_information_available: true,
                    setup_hint: "kwin visible".to_string(),
                },
                spectacle: SpectacleStatus {
                    command_available: true,
                    setup_hint: "spectacle visible".to_string(),
                },
                preferred_available_backend: Some("portal_screenshot".to_string()),
                implemented_available_backend: Some("spectacle".to_string()),
                setup_hint: "portal visible, spectacle implemented".to_string(),
            }),
        );
        assert!(capture_text.contains("preferred=portal_screenshot"));
        assert!(capture_text.contains("implemented=spectacle"));

        let clipboard_text = compact_tool_text(
            "seatgeist.clipboard_status",
            &DaemonResponse::ClipboardBackendStatus(ClipboardBackendStatus {
                wl_paste_available: true,
                wl_copy_available: false,
                kde_klipper_available: true,
                read_backend: Some("wl-clipboard".to_string()),
                write_backend: Some("kde-klipper".to_string()),
                setup_hint: "clipboard text read backend=wl-clipboard write backend=kde-klipper"
                    .to_string(),
            }),
        );
        assert!(clipboard_text.contains("read=wl-clipboard"));
        assert!(clipboard_text.contains("write=kde-klipper"));
        assert!(clipboard_text.contains("wl_paste=true"));
    }

    #[test]
    fn lists_compact_tool_definitions() {
        let tools = tool_definitions(ToolProfile::All);
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.list_windows")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.focus_window")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.panic_stop_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.safety_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.desktop_session_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.computer_use_readiness")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.panic_stop_enable")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.panic_stop_disable")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.kwin_bridge_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.uinput_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.input_backend_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.remote_desktop_session_probe")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.remote_desktop_eis_probe")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.remote_desktop_eis_start")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.remote_desktop_eis_session_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.remote_desktop_eis_stop")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.capture_backend_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.pointer_calibration")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.wait_for_change")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.type_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.key_combo")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.move_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.click_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.drag_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.scroll_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.click_button")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.set_text_field")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.focus_text_field")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.activate_tab")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.activate_link")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.toggle_check")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.set_value")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.select_item")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.select_menu")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.clipboard_get_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.clipboard_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_quality_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_focused_tree")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_find")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_text_attributes")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_invoke")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_set_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_copy_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_cut_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_paste_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_set_caret")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "seatgeist.a11y_set_selection")
        );
    }

    #[test]
    fn core_profile_is_bounded_and_excludes_expert_tools() {
        let tools = tool_definitions(ToolProfile::Core);
        assert!(tools.len() <= 8);
        for name in [
            "seatgeist.computer_status",
            "seatgeist.window_session",
            "seatgeist.snapshot",
            "seatgeist.act",
            "seatgeist.wait",
            "seatgeist.panic_stop",
        ] {
            assert!(tools.iter().any(|tool| tool["name"] == name), "{name}");
        }
        assert!(!tool_available(ToolProfile::Core, "seatgeist.type_text"));
        assert!(!tool_available(ToolProfile::Expert, "seatgeist.act"));
        assert!(tool_available(ToolProfile::All, "seatgeist.act"));
        assert!(tool_available(ToolProfile::All, "seatgeist.type_text"));
    }

    #[test]
    fn retained_capture_facade_routes_session_lifecycle_and_frames() {
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.capture_open",
                &json!({"source": "monitor", "requested_source_id": "DP-1"})
            )
            .expect("expert monitor capture maps"),
            DaemonRequest::CaptureOpen(libseatgeist::CaptureOpenRequest {
                source: libseatgeist::CaptureSourceKind::Monitor,
                ..
            })
        ));
        assert_eq!(
            daemon_request_for_tool("seatgeist.window_session", &json!({}))
                .expect("window session defaults to status"),
            DaemonRequest::CaptureSessionStatus
        );
        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.window_session",
                &json!({"operation": "inventory"})
            )
            .expect("window inventory maps"),
            DaemonRequest::WindowInventory
        );
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.window_session",
                &json!({"operation": "wait_inventory", "after_revision": "wi1:old"})
            )
            .expect("window inventory wait maps"),
            DaemonRequest::WindowInventoryWait(_)
        ));
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.window_session",
                &json!({"operation": "renew", "session_id": "capture-1"})
            )
            .expect("window session renew maps"),
            DaemonRequest::CaptureSessionRenew(_)
        ));
        let renew_error =
            daemon_request_for_tool("seatgeist.window_session", &json!({"operation": "renew"}))
                .expect_err("window session renew requires a session id");
        assert!(
            renew_error
                .to_string()
                .contains("argument 'session_id' is required")
        );
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.capture_session_renew",
                &json!({"session_id": "capture-1"})
            )
            .expect("expert session renew maps"),
            DaemonRequest::CaptureSessionRenew(_)
        ));
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.window_session",
                &json!({
                    "operation": "open",
                    "requested_window_id": "kwin-window-7",
                    "timeout_ms": 30000
                })
            )
            .expect("window session open maps"),
            DaemonRequest::WindowCaptureOpen(_)
        ));
        let open_error =
            daemon_request_for_tool("seatgeist.window_session", &json!({"operation": "open"}))
                .expect_err("core window session open requires an exact target");
        assert!(
            open_error
                .to_string()
                .contains("argument 'requested_window_id' is required")
        );
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.snapshot",
                &json!({"session_id": "capture-1", "max_edge": 800})
            )
            .expect("facade snapshot maps retained session"),
            DaemonRequest::CaptureSnapshot(_)
        ));
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.wait",
                &json!({"session_id": "capture-1", "after_revision": "revision-1"})
            )
            .expect("facade wait maps retained session"),
            DaemonRequest::CaptureWait(_)
        ));
        for name in ["seatgeist.snapshot", "seatgeist.wait"] {
            let error = daemon_request_for_tool(name, &json!({}))
                .expect_err("core retained capture requires a session id");
            assert!(
                error
                    .to_string()
                    .contains("argument 'session_id' is required")
            );
        }
        let tools = core_tool_definitions();
        assert!(!tool_available(ToolProfile::Core, "seatgeist.capture_open"));
        assert!(tool_available(
            ToolProfile::Expert,
            "seatgeist.capture_open"
        ));
        for name in ["seatgeist.snapshot", "seatgeist.wait"] {
            let schema = &tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("core capture tool exists")["inputSchema"];
            assert_eq!(schema["required"], json!(["session_id"]));
        }
        let snapshot_schema = &tools
            .iter()
            .find(|tool| tool["name"] == "seatgeist.snapshot")
            .expect("core snapshot exists")["inputSchema"];
        assert!(snapshot_schema["properties"].get("portal_target").is_none());
        assert!(
            snapshot_schema["properties"]
                .get("portal_interactive")
                .is_none()
        );
        let wait_schema = &tools
            .iter()
            .find(|tool| tool["name"] == "seatgeist.wait")
            .expect("core wait exists")["inputSchema"];
        assert!(wait_schema["properties"].get("interval_ms").is_none());
        assert!(wait_schema["properties"].get("threshold").is_none());
        assert!(matches!(
            daemon_request_for_tool(
                "seatgeist.window_session",
                &json!({"operation": "close", "session_id": "capture-1"})
            )
            .expect("window session close maps"),
            DaemonRequest::CaptureSessionClose(_)
        ));
    }

    #[test]
    fn facade_routes_one_logical_action_to_existing_policy_gated_request() {
        let request = daemon_request_for_tool(
            "seatgeist.act",
            &json!({
                "action": "set_text_field",
                "name": "Search",
                "text": "private query",
                "expected_active_app": "org.mozilla.firefox"
            }),
        )
        .expect("facade action maps");
        assert!(matches!(request, DaemonRequest::SetTextField(_)));
        if let DaemonRequest::SetTextField(request) = request {
            assert_eq!(request.name, "Search");
            assert_eq!(request.text, "private query");
            assert_eq!(
                request.guard.and_then(|guard| guard.expected_app_id),
                Some("org.mozilla.firefox".to_string())
            );
        }

        let err =
            daemon_request_for_tool("seatgeist.act", &json!({"action": "run_arbitrary_batch"}))
                .expect_err("facade rejects arbitrary action types");
        assert!(err.to_string().contains("unsupported seatgeist.act action"));

        let err = daemon_request_for_tool(
            "seatgeist.act",
            &json!({
                "action": "type_text",
                "text": "hello",
                "target_window_id": "kwin-window-1"
            }),
        )
        .expect_err("raw facade action must not ignore a target-window guard");
        assert!(err.to_string().contains("only for semantic"));

        let sticky = daemon_request_for_tool(
            "seatgeist.act",
            &json!({
                "action": "type_text",
                "text": "hello",
                "session_id": "capture-1"
            }),
        )
        .expect("raw facade action accepts a sticky session");
        assert!(matches!(
            sticky,
            DaemonRequest::TypeText(TypeTextRequest {
                session_id: Some(ref session_id),
                ..
            }) if session_id == "capture-1"
        ));

        let err = daemon_request_for_tool(
            "seatgeist.act",
            &json!({
                "action": "type_text",
                "text": "hello",
                "session_id": "capture-1",
                "expected_active_window": "firefox-1"
            }),
        )
        .expect_err("sticky facade rejects contradictory active focus guard locally");
        assert!(err.to_string().contains("mutually exclusive"));

        let err = daemon_request_for_tool(
            "seatgeist.act",
            &json!({
                "action": "click_button",
                "name": "OK",
                "session_id": "capture-1"
            }),
        )
        .expect_err("semantic facade action must not reinterpret sticky raw focus");
        assert!(err.to_string().contains("only for raw keyboard or pointer"));

        let err = daemon_request_for_tool(
            "seatgeist.click_button",
            &json!({"name": "OK", "target_app_id": "org.kde.kate"}),
        )
        .expect_err("partial target guard is rejected");
        assert!(err.to_string().contains("target_window_id is required"));
    }

    #[test]
    fn screenshot_output_is_optional_in_tool_schemas() {
        let tools = tool_definitions(ToolProfile::All);
        for name in [
            "seatgeist.screenshot",
            "seatgeist.screenshot_tile",
            "seatgeist.wait_for_change",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("tool is listed");
            let required = tool["inputSchema"]["required"]
                .as_array()
                .expect("required list is present");
            assert!(
                !required.iter().any(|field| field == "output"),
                "{name} should not require output"
            );
        }
        for name in ["seatgeist.screenshot", "seatgeist.observe"] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("compatibility screenshot tool is listed");
            assert!(
                tool["inputSchema"]["properties"]
                    .get("visible_window_crop_id")
                    .is_some(),
                "{name} should expose explicit visible-window crop compatibility"
            );
        }
    }

    #[test]
    fn maps_screenshot_tile_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.screenshot_tile",
            &json!({
                "output": "/tmp/tile.png",
                "x": 10,
                "y": 20,
                "width": 640,
                "height": 480,
                "max_edge": 320
            }),
        )
        .expect("tile args map");
        assert_eq!(
            request,
            DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
                output: "/tmp/tile.png".into(),
                x: 10,
                y: 20,
                width: 640,
                height: 480,
                max_edge: Some(320),
                portal_interactive: false,
            })
        );
    }

    #[test]
    fn omitted_screenshot_outputs_use_runtime_defaults() {
        let request = daemon_request_for_tool("seatgeist.screenshot", &json!({}))
            .expect("screenshot args map");
        assert!(matches!(request, DaemonRequest::Screenshot(_)));
        if let DaemonRequest::Screenshot(request) = request {
            assert_default_screenshot_path(&request.output, "screenshot");
            assert_eq!(request.max_edge, None);
            assert!(!request.full_resolution);
            assert!(!request.portal_interactive);
        }

        let request = daemon_request_for_tool(
            "seatgeist.screenshot_tile",
            &json!({
                "x": 10,
                "y": 20,
                "width": 640,
                "height": 480
            }),
        )
        .expect("tile args map");
        assert!(matches!(request, DaemonRequest::ScreenshotTile(_)));
        if let DaemonRequest::ScreenshotTile(request) = request {
            assert_default_screenshot_path(&request.output, "tile");
            assert_eq!(request.max_edge, None);
            assert!(!request.portal_interactive);
        }

        let request = daemon_request_for_tool("seatgeist.wait_for_change", &json!({}))
            .expect("wait args map");
        assert!(matches!(request, DaemonRequest::WaitForChange(_)));
        if let DaemonRequest::WaitForChange(request) = request {
            assert_default_screenshot_path(&request.output, "wait-for-change");
            assert_eq!(request.max_edge, None);
            assert_eq!(request.timeout_ms, DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS);
            assert_eq!(request.interval_ms, DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS);
            assert_eq!(request.threshold, DEFAULT_WAIT_FOR_CHANGE_THRESHOLD);
        }
    }

    #[test]
    fn omitted_screenshot_max_edges_use_daemon_defaults() {
        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.screenshot",
                &json!({
                    "output": "/tmp/screen.png"
                }),
            )
            .expect("screenshot args map"),
            DaemonRequest::Screenshot(ScreenshotRequest {
                output: "/tmp/screen.png".into(),
                max_edge: None,
                full_resolution: false,
                portal_interactive: false,
                portal_target: None,
                visible_window_crop_id: None,
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.screenshot_tile",
                &json!({
                    "output": "/tmp/tile.png",
                    "x": 10,
                    "y": 20,
                    "width": 640,
                    "height": 480
                }),
            )
            .expect("tile args map"),
            DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
                output: "/tmp/tile.png".into(),
                x: 10,
                y: 20,
                width: 640,
                height: 480,
                max_edge: None,
                portal_interactive: false,
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.observe",
                &json!({
                    "screenshot_output": "/tmp/observe.png"
                }),
            )
            .expect("observe screenshot args map"),
            DaemonRequest::Observe(ObserveRequest {
                screenshot: Some(ScreenshotRequest {
                    output: "/tmp/observe.png".into(),
                    max_edge: None,
                    full_resolution: false,
                    portal_interactive: false,
                    portal_target: None,
                    visible_window_crop_id: None,
                }),
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.wait_for_change",
                &json!({
                    "output": "/tmp/wait.png"
                }),
            )
            .expect("wait args map"),
            DaemonRequest::WaitForChange(WaitForChangeRequest {
                output: "/tmp/wait.png".into(),
                max_edge: None,
                timeout_ms: DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS,
                interval_ms: DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS,
                threshold: DEFAULT_WAIT_FOR_CHANGE_THRESHOLD,
            })
        );
    }

    #[test]
    fn maps_portal_interactive_screenshot_arguments() {
        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.screenshot",
                &json!({
                    "output": "/tmp/screen.png",
                    "portal_interactive": true,
                    "portal_target": "active_window"
                }),
            )
            .expect("screenshot args map"),
            DaemonRequest::Screenshot(ScreenshotRequest {
                output: "/tmp/screen.png".into(),
                max_edge: None,
                full_resolution: false,
                portal_interactive: true,
                portal_target: Some(PortalScreenshotTarget::ActiveWindow),
                visible_window_crop_id: None,
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.screenshot",
                &json!({
                    "output": "/tmp/visible-window.png",
                    "visible_window_crop_id": "kwin-window-7"
                }),
            )
            .expect("visible-window crop args map"),
            DaemonRequest::Screenshot(ScreenshotRequest {
                output: "/tmp/visible-window.png".into(),
                max_edge: None,
                full_resolution: false,
                portal_interactive: false,
                portal_target: None,
                visible_window_crop_id: Some("kwin-window-7".to_string()),
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.screenshot_tile",
                &json!({
                    "output": "/tmp/tile.png",
                    "x": 10,
                    "y": 20,
                    "width": 640,
                    "height": 480,
                    "portal_interactive": true
                }),
            )
            .expect("tile args map"),
            DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
                output: "/tmp/tile.png".into(),
                x: 10,
                y: 20,
                width: 640,
                height: 480,
                max_edge: None,
                portal_interactive: true,
            })
        );
    }

    #[test]
    fn maps_wait_for_change_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.wait_for_change",
            &json!({
                "output": "/tmp/wait.png",
                "max_edge": 800,
                "timeout_ms": 2000,
                "interval_ms": 100,
                "threshold": 0.02
            }),
        )
        .expect("wait args map");
        assert_eq!(
            request,
            DaemonRequest::WaitForChange(WaitForChangeRequest {
                output: "/tmp/wait.png".into(),
                max_edge: Some(800),
                timeout_ms: 2000,
                interval_ms: 100,
                threshold: 0.02,
            })
        );
    }

    #[test]
    fn maps_focus_window_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.focus_window",
            &json!({"window_id": "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}"}),
        )
        .expect("focus args map");
        assert_eq!(
            request,
            DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                guard: None,
            })
        );
    }

    #[test]
    fn maps_window_resize_and_page_zoom_arguments() {
        let launch = daemon_request_for_tool(
            "seatgeist.launch_window",
            &json!({
                "desktop_entry": "org.kde.kcalc",
                "anchor": "top_right",
                "monitor_id": "DP-1",
                "width": 400,
                "height": 300,
                "margin": 20
            }),
        )
        .expect("launch args map");
        assert_eq!(
            launch,
            DaemonRequest::LaunchWindow(LaunchWindowRequest {
                desktop_entry: "org.kde.kcalc".to_string(),
                anchor: WindowPlacementAnchor::TopRight,
                monitor_id: Some("DP-1".to_string()),
                width: Some(400),
                height: Some(300),
                margin: 20,
                activation: WindowActivationMode::PreserveFocus,
                timeout_ms: 10_000,
                guard: None,
            })
        );

        let moved = daemon_request_for_tool(
            "seatgeist.move_window",
            &json!({"window_id": "window-1", "x": -20, "y": 40}),
        )
        .expect("move args map");
        assert_eq!(
            moved,
            DaemonRequest::MoveWindow(MoveWindowRequest {
                window_id: "window-1".to_string(),
                x: -20,
                y: 40,
                guard: None,
            })
        );

        let resize = daemon_request_for_tool(
            "seatgeist.resize_window",
            &json!({"window_id": "window-1", "width": 1280, "height": 720}),
        )
        .expect("resize args map");
        assert_eq!(
            resize,
            DaemonRequest::ResizeWindow(ResizeWindowRequest {
                window_id: "window-1".to_string(),
                width: 1280,
                height: 720,
                guard: None,
            })
        );

        let zoom = daemon_request_for_tool(
            "seatgeist.page_zoom",
            &json!({
                "operation": "out",
                "steps": 2,
                "expected_active_window": "window-1",
                "expected_active_app": "org.mozilla.firefox"
            }),
        )
        .expect("page zoom args map");
        assert_eq!(
            zoom,
            DaemonRequest::PageZoom(PageZoomRequest {
                operation: PageZoomOperation::Out,
                steps: 2,
                guard: ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: Some("window-1".to_string()),
                    expected_app_id: Some("org.mozilla.firefox".to_string()),
                    title_contains: None,
                },
            })
        );
        assert!(
            daemon_request_for_tool("seatgeist.page_zoom", &json!({"operation": "reset"}))
                .expect_err("unguarded page zoom fails")
                .to_string()
                .contains("requires an active-window guard")
        );
    }

    #[test]
    fn maps_active_window_guard_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.focus_window",
            &json!({
                "window_id": "target-window",
                "expected_active_window": "current-window",
                "expected_active_app": "org.kde.kate",
                "active_title_contains": "main.rs"
            }),
        )
        .expect("guarded focus args map");
        assert_eq!(
            request,
            DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: Some("current-window".to_string()),
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: Some("main.rs".to_string()),
                }),
            })
        );
    }

    #[test]
    fn maps_keyboard_input_arguments() {
        let type_text = daemon_request_for_tool(
            "seatgeist.type_text",
            &json!({"text": "hello", "session_id": "capture-1"}),
        )
        .expect("sticky type text maps");
        assert_eq!(
            type_text,
            DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
                session_id: Some("capture-1".to_string()),
            })
        );

        let key_combo = daemon_request_for_tool("seatgeist.key_combo", &json!({"combo": "Ctrl+L"}))
            .expect("key combo maps");
        assert_eq!(
            key_combo,
            DaemonRequest::KeyCombo(KeyComboRequest {
                combo: "Ctrl+L".to_string(),
                guard: None,
                session_id: None,
            })
        );
    }

    #[test]
    fn maps_pointer_input_arguments() {
        let move_pointer = daemon_request_for_tool(
            "seatgeist.move_pointer",
            &json!({
                "x": 3840.0,
                "y": 2160.0,
                "coordinate_space": "physical_pixel",
                "expected_active_app": "org.kde.kate"
            }),
        )
        .expect("move pointer maps");
        assert_eq!(
            move_pointer,
            DaemonRequest::MovePointer(MovePointerRequest {
                point: Point {
                    x: 3840.0,
                    y: 2160.0,
                    space: CoordinateSpace::PhysicalPixel,
                },
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: None,
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: None,
                }),
                session_id: None,
            })
        );

        let click_pointer = daemon_request_for_tool(
            "seatgeist.click_pointer",
            &json!({
                "x": 100.0,
                "y": 200.0,
                "coordinate_space": "physical_pixel",
                "button": "left",
                "clicks": 2
            }),
        )
        .expect("click pointer maps");
        assert_eq!(
            click_pointer,
            DaemonRequest::ClickPointer(ClickPointerRequest {
                point: Point {
                    x: 100.0,
                    y: 200.0,
                    space: CoordinateSpace::PhysicalPixel,
                },
                button: PointerButton::Left,
                clicks: 2,
                guard: None,
                session_id: None,
            })
        );

        let drag_pointer = daemon_request_for_tool(
            "seatgeist.drag_pointer",
            &json!({
                "from_x": 100.0,
                "from_y": 200.0,
                "to_x": 300.0,
                "to_y": 400.0,
                "coordinate_space": "physical_pixel",
                "button": "right",
                "duration_ms": 500,
                "active_title_contains": "Canvas"
            }),
        )
        .expect("drag pointer maps");
        assert_eq!(
            drag_pointer,
            DaemonRequest::DragPointer(DragPointerRequest {
                from: Point {
                    x: 100.0,
                    y: 200.0,
                    space: CoordinateSpace::PhysicalPixel,
                },
                to: Point {
                    x: 300.0,
                    y: 400.0,
                    space: CoordinateSpace::PhysicalPixel,
                },
                button: PointerButton::Right,
                duration_ms: 500,
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: None,
                    expected_app_id: None,
                    title_contains: Some("Canvas".to_string()),
                }),
                session_id: None,
            })
        );

        let scroll_pointer = daemon_request_for_tool(
            "seatgeist.scroll_pointer",
            &json!({
                "vertical": -3,
                "horizontal": 1
            }),
        )
        .expect("scroll pointer maps");
        assert_eq!(
            scroll_pointer,
            DaemonRequest::ScrollPointer(ScrollPointerRequest {
                vertical: -3,
                horizontal: 1,
                guard: None,
                session_id: None,
            })
        );
    }

    #[test]
    fn maps_panic_stop_tools() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.safety_status", &json!({}))
                .expect("safety status maps"),
            DaemonRequest::SafetyStatus
        );
        assert_eq!(
            daemon_request_for_tool("seatgeist.desktop_session_status", &json!({}))
                .expect("desktop session status maps"),
            DaemonRequest::DesktopSessionStatus
        );
        assert_eq!(
            daemon_request_for_tool("seatgeist.computer_use_readiness", &json!({}))
                .expect("computer use readiness maps"),
            DaemonRequest::ComputerUseReadiness
        );
        assert_eq!(
            daemon_request_for_tool("seatgeist.panic_stop_status", &json!({}))
                .expect("panic-stop status maps"),
            DaemonRequest::PanicStopStatus
        );
        assert_eq!(
            daemon_request_for_tool("seatgeist.panic_stop_enable", &json!({}))
                .expect("panic-stop enable maps"),
            DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: true })
        );
        assert_eq!(
            daemon_request_for_tool("seatgeist.panic_stop_disable", &json!({}))
                .expect("panic-stop disable maps"),
            DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: false })
        );
    }

    #[test]
    fn maps_kwin_bridge_status_tool() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.kwin_bridge_status", &json!({}))
                .expect("bridge status maps"),
            DaemonRequest::KwinBridgeStatus
        );
    }

    #[test]
    fn maps_uinput_status_tool() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.uinput_status", &json!({}))
                .expect("uinput status maps"),
            DaemonRequest::UinputStatus
        );
    }

    #[test]
    fn maps_input_backend_status_tool() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.input_backend_status", &json!({}))
                .expect("input backend status maps"),
            DaemonRequest::InputBackendStatus
        );
    }

    #[test]
    fn maps_remote_desktop_session_probe_tool() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.remote_desktop_session_probe", &json!({}))
                .expect("default remote desktop probe maps"),
            DaemonRequest::RemoteDesktopSessionProbe(RemoteDesktopSessionProbeRequest {
                keyboard: true,
                pointer: true,
                touchscreen: false,
                restore_token: None,
                persist_mode: None,
                parent_window: None,
                timeout_ms: DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS,
                guard: None,
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.remote_desktop_session_probe",
                &json!({
                    "keyboard": false,
                    "pointer": true,
                    "touchscreen": false,
                    "persist_mode": "application-lifetime",
                    "restore_token": "restore_once",
                    "timeout_ms": 30000,
                    "expected_active_app": "org.kde.kwrite"
                }),
            )
            .expect("remote desktop probe maps"),
            DaemonRequest::RemoteDesktopSessionProbe(RemoteDesktopSessionProbeRequest {
                keyboard: false,
                pointer: true,
                touchscreen: false,
                restore_token: Some("restore_once".to_string()),
                persist_mode: Some(RemoteDesktopPersistMode::ApplicationLifetime),
                parent_window: None,
                timeout_ms: 30_000,
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: None,
                    expected_app_id: Some("org.kde.kwrite".to_string()),
                    title_contains: None,
                }),
            })
        );
    }

    #[test]
    fn maps_remote_desktop_eis_probe_tool() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.remote_desktop_eis_probe", &json!({}))
                .expect("default remote desktop EIS probe maps"),
            DaemonRequest::RemoteDesktopEisProbe(RemoteDesktopSessionProbeRequest {
                keyboard: true,
                pointer: true,
                touchscreen: false,
                restore_token: None,
                persist_mode: None,
                parent_window: None,
                timeout_ms: DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS,
                guard: None,
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.remote_desktop_eis_probe",
                &json!({
                    "keyboard": true,
                    "pointer": false,
                    "touchscreen": false,
                    "persist_mode": "explicitly_revoked",
                    "parent_window": "wayland:app-window",
                    "timeout_ms": 30000,
                    "active_title_contains": "scratch"
                }),
            )
            .expect("remote desktop EIS probe maps"),
            DaemonRequest::RemoteDesktopEisProbe(RemoteDesktopSessionProbeRequest {
                keyboard: true,
                pointer: false,
                touchscreen: false,
                restore_token: None,
                persist_mode: Some(RemoteDesktopPersistMode::ExplicitlyRevoked),
                parent_window: Some("wayland:app-window".to_string()),
                timeout_ms: 30_000,
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: None,
                    expected_app_id: None,
                    title_contains: Some("scratch".to_string()),
                }),
            })
        );
    }

    #[test]
    fn maps_remote_desktop_eis_session_lifecycle_tools() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.remote_desktop_eis_start", &json!({}))
                .expect("default remote desktop EIS session start maps"),
            DaemonRequest::RemoteDesktopEisStart(RemoteDesktopSessionProbeRequest {
                keyboard: true,
                pointer: true,
                touchscreen: false,
                restore_token: None,
                persist_mode: None,
                parent_window: None,
                timeout_ms: DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS,
                guard: None,
            })
        );

        assert_eq!(
            daemon_request_for_tool(
                "seatgeist.remote_desktop_eis_start",
                &json!({
                    "keyboard": false,
                    "pointer": true,
                    "touchscreen": false,
                    "persist_mode": "application_lifetime",
                    "restore_token": "restore_once",
                    "parent_window": "wayland:app-window",
                    "timeout_ms": 45000,
                    "expected_active_window": "window-1"
                }),
            )
            .expect("remote desktop EIS session start maps"),
            DaemonRequest::RemoteDesktopEisStart(RemoteDesktopSessionProbeRequest {
                keyboard: false,
                pointer: true,
                touchscreen: false,
                restore_token: Some("restore_once".to_string()),
                persist_mode: Some(RemoteDesktopPersistMode::ApplicationLifetime),
                parent_window: Some("wayland:app-window".to_string()),
                timeout_ms: 45_000,
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: Some("window-1".to_string()),
                    expected_app_id: None,
                    title_contains: None,
                }),
            })
        );

        assert_eq!(
            daemon_request_for_tool("seatgeist.remote_desktop_eis_session_status", &json!({}))
                .expect("remote desktop EIS session status maps"),
            DaemonRequest::RemoteDesktopEisSessionStatus
        );
        assert_eq!(
            daemon_request_for_tool("seatgeist.remote_desktop_eis_stop", &json!({}))
                .expect("remote desktop EIS session stop maps"),
            DaemonRequest::RemoteDesktopEisStop
        );
    }

    #[test]
    fn maps_capture_backend_status_tool() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.capture_backend_status", &json!({}))
                .expect("capture backend status maps"),
            DaemonRequest::CaptureBackendStatus
        );
    }

    #[test]
    fn maps_pointer_calibration_tool() {
        assert_eq!(
            daemon_request_for_tool("seatgeist.pointer_calibration", &json!({}))
                .expect("pointer calibration maps"),
            DaemonRequest::PointerCalibration
        );
    }

    #[test]
    fn maps_click_button_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.click_button",
            &json!({
                "name": "OK",
                "app": "kate",
                "window_name_contains": "settings",
                "max_nodes": 256,
                "target_window_id": "kwin-kate-1",
                "target_app_id": "org.kde.kate",
                "target_pid": 4242,
                "target_title_contains": "settings"
            }),
        )
        .expect("click button args map");
        assert_eq!(
            request,
            DaemonRequest::ClickButton(ClickButtonRequest {
                name: "OK".to_string(),
                destructive: false,
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: Some(TargetWindowGuard {
                    expected_window_id: "kwin-kate-1".to_string(),
                    expected_app_id: Some("org.kde.kate".to_string()),
                    expected_pid: Some(4242),
                    title_contains: Some("settings".to_string()),
                }),
            })
        );
    }

    #[test]
    fn maps_set_text_field_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.set_text_field",
            &json!({
                "name": "Search",
                "text": "query",
                "app": "kate",
                "window_name_contains": "settings",
                "max_nodes": 256
            }),
        )
        .expect("set text field args map");
        assert_eq!(
            request,
            DaemonRequest::SetTextField(SetTextFieldRequest {
                name: "Search".to_string(),
                text: "query".to_string(),
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_focus_text_field_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.focus_text_field",
            &json!({
                "name": "Search",
                "app": "kate",
                "window_name_contains": "settings",
                "max_nodes": 256
            }),
        )
        .expect("focus text field args map");
        assert_eq!(
            request,
            DaemonRequest::FocusTextField(FocusTextFieldRequest {
                name: "Search".to_string(),
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_activate_tab_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.activate_tab",
            &json!({
                "name": "General",
                "app": "settings",
                "window_name_contains": "preferences",
                "max_nodes": 256
            }),
        )
        .expect("activate tab args map");
        assert_eq!(
            request,
            DaemonRequest::ActivateTab(ActivateTabRequest {
                name: "General".to_string(),
                app: Some("settings".to_string()),
                window_name_contains: Some("preferences".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_activate_link_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.activate_link",
            &json!({
                "name": "Release notes",
                "app": "firefox",
                "window_name_contains": "docs",
                "max_nodes": 256
            }),
        )
        .expect("activate link args map");
        assert_eq!(
            request,
            DaemonRequest::ActivateLink(ActivateLinkRequest {
                name: "Release notes".to_string(),
                app: Some("firefox".to_string()),
                window_name_contains: Some("docs".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_toggle_check_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.toggle_check",
            &json!({
                "name": "Enable feature",
                "checked": true,
                "app": "settings",
                "window_name_contains": "preferences",
                "max_nodes": 256
            }),
        )
        .expect("toggle check args map");
        assert_eq!(
            request,
            DaemonRequest::ToggleCheck(ToggleCheckRequest {
                name: "Enable feature".to_string(),
                checked: Some(true),
                app: Some("settings".to_string()),
                window_name_contains: Some("preferences".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_set_value_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.set_value",
            &json!({
                "name": "Volume",
                "value": 0.75,
                "app": "settings",
                "window_name_contains": "sound",
                "max_nodes": 256
            }),
        )
        .expect("set value args map");
        assert_eq!(
            request,
            DaemonRequest::SetValue(SetValueRequest {
                name: "Volume".to_string(),
                value: 0.75,
                app: Some("settings".to_string()),
                window_name_contains: Some("sound".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_select_item_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.select_item",
            &json!({
                "name": "Printer",
                "app": "systemsettings",
                "window_name_contains": "devices",
                "max_nodes": 256
            }),
        )
        .expect("select item args map");
        assert_eq!(
            request,
            DaemonRequest::SelectItem(SelectItemRequest {
                name: "Printer".to_string(),
                app: Some("systemsettings".to_string()),
                window_name_contains: Some("devices".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_select_menu_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.select_menu",
            &json!({
                "path": ["File", "Open"],
                "app": "kate",
                "window_name_contains": "editor",
                "max_nodes": 256
            }),
        )
        .expect("select menu args map");
        assert_eq!(
            request,
            DaemonRequest::SelectMenu(SelectMenuRequest {
                path: vec!["File".to_string(), "Open".to_string()],
                destructive: false,
                app: Some("kate".to_string()),
                window_name_contains: Some("editor".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            })
        );
    }

    #[test]
    fn maps_clipboard_set_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.clipboard_set_text",
            &json!({"text": "copy this"}),
        )
        .expect("clipboard set args map");
        assert_eq!(
            request,
            DaemonRequest::ClipboardSet(ClipboardSetRequest {
                text: "copy this".to_string(),
            })
        );
    }

    #[test]
    fn maps_clipboard_status_tool() {
        let request = daemon_request_for_tool("seatgeist.clipboard_status", &json!({}))
            .expect("clipboard status maps");
        assert_eq!(request, DaemonRequest::ClipboardBackendStatus);
    }

    #[test]
    fn maps_accessibility_quality_status_tool() {
        let request = daemon_request_for_tool("seatgeist.a11y_quality_status", &json!({}))
            .expect("accessibility quality status maps");
        assert_eq!(request, DaemonRequest::AccessibilityQualityStatus);
    }

    #[test]
    fn maps_clipboard_get_arguments() {
        let request =
            daemon_request_for_tool("seatgeist.clipboard_get_text", &json!({})).expect("get maps");
        assert_eq!(
            request,
            DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: Some(DEFAULT_CLIPBOARD_MAX_BYTES),
            })
        );
    }

    #[test]
    fn maps_unbounded_clipboard_get_arguments() {
        let request =
            daemon_request_for_tool("seatgeist.clipboard_get_text", &json!({"full": true}))
                .expect("full get maps");
        assert_eq!(
            request,
            DaemonRequest::ClipboardGet(ClipboardGetRequest { max_bytes: None })
        );
    }

    #[test]
    fn maps_filtered_journal_tail_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.journal_tail",
            &json!({
                "limit": 7,
                "method": "focus_window",
                "ok": false
            }),
        )
        .expect("journal tail args map");
        assert_eq!(
            request,
            DaemonRequest::JournalTail(JournalTailRequest {
                limit: 7,
                method_filter: Some("focus_window".to_string()),
                ok: Some(false),
            })
        );
    }

    #[test]
    fn maps_focused_accessibility_tree_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_focused_tree",
            &json!({"depth": 3, "max_nodes": 128}),
        )
        .expect("a11y focused tree args map");
        assert_eq!(
            request,
            DaemonRequest::FocusedAccessibilityTree(FocusedAccessibilityTreeRequest {
                depth: 3,
                max_nodes: 128,
            })
        );
    }

    #[test]
    fn maps_accessibility_find_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_find",
            &json!({
                "role": "button",
                "name_contains": "ok",
                "app": "kate",
                "window_name_contains": "settings",
                "depth": 1,
                "max_results": 3,
                "max_nodes": 200
            }),
        )
        .expect("a11y find args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityFind(AccessibilityFindRequest {
                role: Some("button".to_string()),
                name_contains: Some("ok".to_string()),
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                depth: 1,
                max_results: 3,
                max_nodes: 200,
            })
        );
    }

    #[test]
    fn maps_accessibility_text_attributes_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_text_attributes",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "offset": 4,
                "include_defaults": true
            }),
        )
        .expect("a11y text attributes args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityTextAttributes(AccessibilityTextAttributesRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 4,
                include_defaults: true,
            })
        );
    }

    #[test]
    fn maps_accessibility_invoke_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_invoke",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "action": "press"
            }),
        )
        .expect("a11y invoke args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityInvoke(AccessibilityInvokeRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                action: AccessibilityAction::Press,
                destructive: false,
                guard: None,
            })
        );
    }

    #[test]
    fn maps_accessibility_set_text_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_set_text",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "text": "hello"
            }),
        )
        .expect("a11y set-text args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilitySetText(AccessibilitySetTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                text: "hello".to_string(),
                guard: None,
            })
        );
    }

    #[test]
    fn maps_accessibility_insert_text_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_insert_text",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "offset": 5,
                "text": "hello"
            }),
        )
        .expect("a11y insert-text args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityInsertText(AccessibilityInsertTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 5,
                text: "hello".to_string(),
                guard: None,
            })
        );
    }

    #[test]
    fn maps_accessibility_delete_text_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_delete_text",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "start_offset": 2,
                "end_offset": 5
            }),
        )
        .expect("a11y delete-text args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityDeleteText(AccessibilityDeleteTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                start_offset: 2,
                end_offset: 5,
                guard: None,
            })
        );
    }

    #[test]
    fn maps_accessibility_copy_text_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_copy_text",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "start_offset": 2,
                "end_offset": 5
            }),
        )
        .expect("a11y copy-text args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityCopyText(AccessibilityCopyTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                start_offset: 2,
                end_offset: 5,
                guard: None,
            })
        );
    }

    #[test]
    fn maps_accessibility_cut_text_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_cut_text",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "start_offset": 2,
                "end_offset": 5
            }),
        )
        .expect("a11y cut-text args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityCutText(AccessibilityCutTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                start_offset: 2,
                end_offset: 5,
                guard: None,
            })
        );
    }

    #[test]
    fn maps_accessibility_paste_text_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_paste_text",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "offset": 5
            }),
        )
        .expect("a11y paste-text args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilityPasteText(AccessibilityPasteTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 5,
                guard: None,
            })
        );
    }

    #[test]
    fn maps_accessibility_set_caret_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_set_caret",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "offset": 5,
                "expected_active_app": "org.kde.kate"
            }),
        )
        .expect("a11y set-caret args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilitySetCaret(AccessibilitySetCaretRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 5,
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: None,
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: None,
                }),
            })
        );
    }

    #[test]
    fn maps_accessibility_set_selection_arguments() {
        let request = daemon_request_for_tool(
            "seatgeist.a11y_set_selection",
            &json!({
                "node_id": "atspi://:1.42/org/a11y/atspi/accessible/7",
                "start_offset": 2,
                "end_offset": 8
            }),
        )
        .expect("a11y set-selection args map");
        assert_eq!(
            request,
            DaemonRequest::AccessibilitySetSelection(AccessibilitySetSelectionRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                selection_num: 0,
                start_offset: 2,
                end_offset: 8,
                guard: None,
            })
        );
    }

    #[test]
    fn maps_observe_arguments_without_screenshot() {
        let request =
            daemon_request_for_tool("seatgeist.observe", &json!({})).expect("observe args map");
        assert_eq!(
            request,
            DaemonRequest::Observe(ObserveRequest { screenshot: None })
        );
    }

    #[test]
    fn maps_observe_arguments_with_screenshot() {
        let request = daemon_request_for_tool(
            "seatgeist.observe",
            &json!({
                "screenshot_output": "/tmp/observe.png",
                "max_edge": 1200,
                "full_resolution": false
            }),
        )
        .expect("observe screenshot args map");
        assert_eq!(
            request,
            DaemonRequest::Observe(ObserveRequest {
                screenshot: Some(ScreenshotRequest {
                    output: "/tmp/observe.png".into(),
                    max_edge: Some(1200),
                    full_resolution: false,
                    portal_interactive: false,
                    portal_target: None,
                    visible_window_crop_id: None,
                }),
            })
        );
    }

    #[test]
    fn daemon_errors_become_tool_errors() {
        let response = DaemonResponse::Error {
            kind: libseatgeist::ErrorKind::PolicyDenied,
            message: "policy denied".to_string(),
        };
        let result = tool_result_from_daemon("seatgeist.focus_window", &response, false);
        assert_eq!(result["isError"], true);
        assert_eq!(
            result["content"][0]["text"],
            "error kind=PolicyDenied: policy denied"
        );
        assert_eq!(result["structuredContent"]["data"]["kind"], "policy_denied");
    }

    fn assert_default_screenshot_path(path: &Path, kind: &str) {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("path has UTF-8 file name");
        assert!(
            file_name.ends_with(&format!("-{kind}.png")),
            "unexpected default screenshot file name: {file_name}"
        );
        assert_eq!(
            path.parent().and_then(|value| value.file_name()),
            Some(std::ffi::OsStr::new("screenshots"))
        );
        assert_eq!(
            path.parent()
                .and_then(|value| value.parent())
                .and_then(|value| value.file_name()),
            Some(std::ffi::OsStr::new("seatgeist"))
        );
    }

    fn temporary_test_path(suffix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "seatgeist-mcp-{}-{nanos}-{suffix}",
            std::process::id()
        ))
    }

    fn sample_screenshot_info(backend: &str) -> ScreenshotInfo {
        ScreenshotInfo {
            path: PathBuf::from("/tmp/seatgeist-summary.png"),
            backend: backend.to_string(),
            occlusion_possible: false,
            source_width: 7680,
            source_height: 4320,
            output_width: 1600,
            output_height: 900,
            transform: ScreenshotTransform {
                source_coordinate_space: CoordinateSpace::PhysicalPixel,
                output_coordinate_space: CoordinateSpace::PhysicalPixel,
                source_origin_x: 0,
                source_origin_y: 0,
                scale_x: 1600.0 / 7680.0,
                scale_y: 900.0 / 4320.0,
            },
            coordinate_space: CoordinateSpace::PhysicalPixel,
            monitors: Vec::new(),
        }
    }
}
