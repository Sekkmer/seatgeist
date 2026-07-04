use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
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
    ScreenshotRequest, ScreenshotTileRequest, ScrollPointerRequest, SelectItemRequest,
    SelectMenuRequest, SetPanicStopRequest, SetTextFieldRequest, SetValueRequest,
    ToggleCheckRequest, TypeTextRequest, WaitForChangeRequest, default_socket_path,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "plasmapilot";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const SERVER_INSTRUCTIONS: &str = "PlasmaPilot exposes local KDE Plasma observation and carefully policy-gated control tools. Prefer observe/list/screenshot tools first, keep outputs compact, and expect control tools such as focus_window to fail unless the daemon is started with an explicit approval/control policy.";

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot MCP stdio server")]
struct Args {
    #[arg(long)]
    stdio: bool,

    #[arg(long, env = "PLASMA_PILOT_SOCKET")]
    socket: Option<PathBuf>,
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
}

fn main() -> Result<()> {
    let args = Args::parse();
    if !args.stdio {
        bail!("plasma-pilot-mcp currently supports only --stdio");
    }
    let socket = match args.socket {
        Some(path) => path,
        None => default_socket_path().context("resolve default daemon socket path")?,
    };
    McpServer { socket }.run_stdio()
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
            "initialize" => Ok(initialize_result()),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
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
        let request = daemon_request_for_tool(name, &arguments)?;
        let response = self.send_daemon_request(request)?;
        Ok(tool_result_from_daemon(name, &response))
    }

    fn send_daemon_request(&self, request: DaemonRequest) -> Result<DaemonResponse> {
        let mut stream = UnixStream::connect(&self.socket)
            .with_context(|| format!("connect to daemon socket {}", self.socket.display()))?;
        let request_line = serde_json::to_string(&request).context("serialize daemon request")?;
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

fn initialize_result() -> Value {
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
        "instructions": SERVER_INSTRUCTIONS
    })
}

fn daemon_request_for_tool(name: &str, arguments: &Value) -> Result<DaemonRequest> {
    match name {
        "plasma.health" => Ok(DaemonRequest::Health),
        "plasma.capabilities" => Ok(DaemonRequest::Capabilities),
        "plasma.policy_status" => Ok(DaemonRequest::PolicyStatus),
        "plasma.panic_stop_status" => Ok(DaemonRequest::PanicStopStatus),
        "plasma.panic_stop_enable" => Ok(DaemonRequest::SetPanicStop(SetPanicStopRequest {
            enabled: true,
        })),
        "plasma.panic_stop_disable" => Ok(DaemonRequest::SetPanicStop(SetPanicStopRequest {
            enabled: false,
        })),
        "plasma.kwin_bridge_status" => Ok(DaemonRequest::KwinBridgeStatus),
        "plasma.uinput_status" => Ok(DaemonRequest::UinputStatus),
        "plasma.input_backend_status" => Ok(DaemonRequest::InputBackendStatus),
        "plasma.capture_backend_status" => Ok(DaemonRequest::CaptureBackendStatus),
        "plasma.pointer_calibration" => Ok(DaemonRequest::PointerCalibration),
        "plasma.list_monitors" => Ok(DaemonRequest::ListMonitors),
        "plasma.list_windows" => Ok(DaemonRequest::ListWindows),
        "plasma.active_window" => Ok(DaemonRequest::ActiveWindow),
        "plasma.observe" => {
            let screenshot = match optional_string(arguments, "screenshot_output")? {
                Some(output) => Some(ScreenshotRequest {
                    output: output.into(),
                    max_edge: optional_u64(arguments, "max_edge")?
                        .map(u64_to_u32)
                        .transpose()?,
                    full_resolution: optional_bool(arguments, "full_resolution")?.unwrap_or(false),
                }),
                None => None,
            };
            Ok(DaemonRequest::Observe(ObserveRequest { screenshot }))
        }
        "plasma.journal_tail" => Ok(DaemonRequest::JournalTail(JournalTailRequest {
            limit: optional_u64(arguments, "limit")?.unwrap_or(20) as usize,
            method_filter: optional_string(arguments, "method")?,
            ok: optional_bool(arguments, "ok")?,
        })),
        "plasma.screenshot" => Ok(DaemonRequest::Screenshot(ScreenshotRequest {
            output: required_string(arguments, "output")?.into(),
            max_edge: optional_u64(arguments, "max_edge")?
                .map(u64_to_u32)
                .transpose()?,
            full_resolution: optional_bool(arguments, "full_resolution")?.unwrap_or(false),
        })),
        "plasma.screenshot_tile" => Ok(DaemonRequest::ScreenshotTile(ScreenshotTileRequest {
            output: required_string(arguments, "output")?.into(),
            x: required_u32(arguments, "x")?,
            y: required_u32(arguments, "y")?,
            width: required_u32(arguments, "width")?,
            height: required_u32(arguments, "height")?,
            max_edge: optional_u64(arguments, "max_edge")?
                .map(u64_to_u32)
                .transpose()?
                .or(Some(1600)),
        })),
        "plasma.wait_for_change" => Ok(DaemonRequest::WaitForChange(WaitForChangeRequest {
            output: required_string(arguments, "output")?.into(),
            max_edge: optional_u64(arguments, "max_edge")?
                .map(u64_to_u32)
                .transpose()?
                .or(Some(1600)),
            timeout_ms: optional_u64(arguments, "timeout_ms")?
                .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS),
            interval_ms: optional_u64(arguments, "interval_ms")?
                .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_INTERVAL_MS),
            threshold: optional_f64(arguments, "threshold")?
                .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_THRESHOLD),
        })),
        "plasma.clipboard_get_text" => {
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
        "plasma.clipboard_set_text" => Ok(DaemonRequest::ClipboardSet(ClipboardSetRequest {
            text: required_string(arguments, "text")?,
        })),
        "plasma.a11y_focused_tree" => Ok(DaemonRequest::FocusedAccessibilityTree(
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
        "plasma.a11y_find" => Ok(DaemonRequest::AccessibilityFind(AccessibilityFindRequest {
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
        "plasma.a11y_invoke" => Ok(DaemonRequest::AccessibilityInvoke(
            AccessibilityInvokeRequest {
                node_id: required_string(arguments, "node_id")?,
                action: required_accessibility_action(arguments, "action")?,
                destructive: optional_bool(arguments, "destructive")?.unwrap_or(false),
                guard: active_window_guard(arguments)?,
            },
        )),
        "plasma.a11y_set_text" => Ok(DaemonRequest::AccessibilitySetText(
            AccessibilitySetTextRequest {
                node_id: required_string(arguments, "node_id")?,
                text: required_string(arguments, "text")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "plasma.a11y_insert_text" => Ok(DaemonRequest::AccessibilityInsertText(
            AccessibilityInsertTextRequest {
                node_id: required_string(arguments, "node_id")?,
                offset: required_i32(arguments, "offset")?,
                text: required_string(arguments, "text")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "plasma.a11y_delete_text" => Ok(DaemonRequest::AccessibilityDeleteText(
            AccessibilityDeleteTextRequest {
                node_id: required_string(arguments, "node_id")?,
                start_offset: required_i32(arguments, "start_offset")?,
                end_offset: required_i32(arguments, "end_offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "plasma.a11y_copy_text" => Ok(DaemonRequest::AccessibilityCopyText(
            AccessibilityCopyTextRequest {
                node_id: required_string(arguments, "node_id")?,
                start_offset: required_i32(arguments, "start_offset")?,
                end_offset: required_i32(arguments, "end_offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "plasma.a11y_cut_text" => Ok(DaemonRequest::AccessibilityCutText(
            AccessibilityCutTextRequest {
                node_id: required_string(arguments, "node_id")?,
                start_offset: required_i32(arguments, "start_offset")?,
                end_offset: required_i32(arguments, "end_offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "plasma.a11y_paste_text" => Ok(DaemonRequest::AccessibilityPasteText(
            AccessibilityPasteTextRequest {
                node_id: required_string(arguments, "node_id")?,
                offset: required_i32(arguments, "offset")?,
                guard: active_window_guard(arguments)?,
            },
        )),
        "plasma.type_text" => Ok(DaemonRequest::TypeText(TypeTextRequest {
            text: required_string(arguments, "text")?,
            guard: active_window_guard(arguments)?,
        })),
        "plasma.key_combo" => Ok(DaemonRequest::KeyCombo(KeyComboRequest {
            combo: required_string(arguments, "combo")?,
            guard: active_window_guard(arguments)?,
        })),
        "plasma.move_pointer" => Ok(DaemonRequest::MovePointer(MovePointerRequest {
            point: Point {
                x: required_f64(arguments, "x")?,
                y: required_f64(arguments, "y")?,
                space: required_coordinate_space(arguments, "coordinate_space")?,
            },
            guard: active_window_guard(arguments)?,
        })),
        "plasma.click_pointer" => Ok(DaemonRequest::ClickPointer(ClickPointerRequest {
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
        })),
        "plasma.drag_pointer" => {
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
            }))
        }
        "plasma.scroll_pointer" => Ok(DaemonRequest::ScrollPointer(ScrollPointerRequest {
            vertical: optional_i32(arguments, "vertical")?.unwrap_or(0),
            horizontal: optional_i32(arguments, "horizontal")?.unwrap_or(0),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.click_button" => Ok(DaemonRequest::ClickButton(ClickButtonRequest {
            name: required_string(arguments, "name")?,
            destructive: optional_bool(arguments, "destructive")?.unwrap_or(false),
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.set_text_field" => Ok(DaemonRequest::SetTextField(SetTextFieldRequest {
            name: required_string(arguments, "name")?,
            text: required_string(arguments, "text")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.activate_tab" => Ok(DaemonRequest::ActivateTab(ActivateTabRequest {
            name: required_string(arguments, "name")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.activate_link" => Ok(DaemonRequest::ActivateLink(ActivateLinkRequest {
            name: required_string(arguments, "name")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.toggle_check" => Ok(DaemonRequest::ToggleCheck(ToggleCheckRequest {
            name: required_string(arguments, "name")?,
            checked: optional_bool(arguments, "checked")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.set_value" => Ok(DaemonRequest::SetValue(SetValueRequest {
            name: required_string(arguments, "name")?,
            value: required_f64(arguments, "value")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.select_item" => Ok(DaemonRequest::SelectItem(SelectItemRequest {
            name: required_string(arguments, "name")?,
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.select_menu" => Ok(DaemonRequest::SelectMenu(SelectMenuRequest {
            path: required_string_array(arguments, "path")?,
            destructive: optional_bool(arguments, "destructive")?.unwrap_or(false),
            app: optional_string(arguments, "app")?,
            window_name_contains: optional_string(arguments, "window_name_contains")?,
            max_nodes: optional_u64(arguments, "max_nodes")?
                .map(u64_to_usize)
                .transpose()?
                .unwrap_or(1024),
            guard: active_window_guard(arguments)?,
        })),
        "plasma.focus_window" => Ok(DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: required_string(arguments, "window_id")?,
            guard: active_window_guard(arguments)?,
        })),
        _ => bail!("unknown tool: {name}"),
    }
}

fn tool_result_from_daemon(tool_name: &str, response: &DaemonResponse) -> Value {
    let structured = serde_json::to_value(response).unwrap_or_else(|err| {
        json!({
            "type": "error",
            "data": {
                "message": format!("serialize daemon response: {err}")
            }
        })
    });
    let is_error = matches!(response, DaemonResponse::Error { .. });
    json!({
        "content": [
            {
                "type": "text",
                "text": compact_tool_text(tool_name, response)
            }
        ],
        "structuredContent": structured,
        "isError": is_error
    })
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
        DaemonResponse::CaptureBackendStatus(status) => format!(
            "capture backends preferred={} portal_screenshot={} portal_screencast={} kwin_metadata={} spectacle={}",
            status
                .preferred_available_backend
                .as_deref()
                .unwrap_or("none"),
            status.screenshot_portal.screenshot_interface_available,
            status.screenshot_portal.screencast_interface_available,
            status.kwin_metadata.support_information_available,
            status.spectacle.command_available
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
        DaemonResponse::Screenshot(info) => format!(
            "{} wrote {}x{} image from {}x{} source to {}",
            tool_name,
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
            .unwrap_or_else(|| format!("action {} ok={}", result.id, result.ok)),
        DaemonResponse::Error { message } => message.clone(),
    }
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "plasma.health",
            "Daemon Health",
            "Check the PlasmaPilot daemon health.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.capabilities",
            "Capabilities",
            "List daemon backend capabilities.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.policy_status",
            "Policy Status",
            "Read current daemon policy defaults.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.panic_stop_status",
            "Panic Stop Status",
            "Read whether the daemon panic-stop flag is active.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.panic_stop_enable",
            "Enable Panic Stop",
            "Enable the daemon panic-stop flag. This is journaled and blocks control-class actions.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.panic_stop_disable",
            "Disable Panic Stop",
            "Disable the daemon panic-stop flag after explicit local operator intent.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.kwin_bridge_status",
            "KWin Bridge Status",
            "Report daemon DBus receiver state, latest active-window bridge update state, and user-local KWin script install/config status.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.uinput_status",
            "Uinput Status",
            "Report whether the daemon can open /dev/uinput for virtual keyboard and pointer fallback, with file metadata and setup hints.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.input_backend_status",
            "Input Backend Status",
            "Probe read-only input backend availability in priority order: xdg-desktop-portal RemoteDesktop, libei, then uinput fallback.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.capture_backend_status",
            "Capture Backend Status",
            "Probe read-only capture backend availability: xdg-desktop-portal Screenshot/ScreenCast, KWin metadata, and Spectacle fallback.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.pointer_calibration",
            "Pointer Calibration",
            "Report monitor-derived physical pointer bounds, per-monitor physical origins, and representative test points without moving the pointer.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.list_monitors",
            "List Monitors",
            "List monitor geometry and scale metadata.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.list_windows",
            "List Windows",
            "List compact KWin window metadata.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.active_window",
            "Active Window",
            "Read the latest active-window bridge update.",
            object_schema(vec![], vec![]),
        ),
        tool(
            "plasma.observe",
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
                        json!({"type": "integer", "minimum": 1, "description": "Screenshot preview max edge in pixels."}),
                    ),
                    (
                        "full_resolution",
                        json!({"type": "boolean", "description": "Capture the source image without downscaling. This is policy-gated separately and prompts by default."}),
                    ),
                ],
                vec![],
            ),
        ),
        tool(
            "plasma.screenshot",
            "Screenshot",
            "Capture a screenshot to a PNG path. Defaults to a bounded preview; full_resolution is policy-gated separately and prompts by default.",
            object_schema(
                vec![
                    (
                        "output",
                        json!({"type": "string", "description": "PNG output path on the local filesystem."}),
                    ),
                    (
                        "max_edge",
                        json!({"type": "integer", "minimum": 1, "description": "Preview max edge in pixels."}),
                    ),
                    (
                        "full_resolution",
                        json!({"type": "boolean", "description": "Capture the source image without downscaling. This is policy-gated separately and prompts by default."}),
                    ),
                ],
                vec!["output"],
            ),
        ),
        tool(
            "plasma.screenshot_tile",
            "Screenshot Tile",
            "Capture and optionally downscale a physical-pixel screenshot tile.",
            object_schema(
                vec![
                    (
                        "output",
                        json!({"type": "string", "description": "PNG output path on the local filesystem."}),
                    ),
                    ("x", json!({"type": "integer", "minimum": 0})),
                    ("y", json!({"type": "integer", "minimum": 0})),
                    ("width", json!({"type": "integer", "minimum": 1})),
                    ("height", json!({"type": "integer", "minimum": 1})),
                    (
                        "max_edge",
                        json!({"type": "integer", "minimum": 1, "description": "Output max edge in pixels."}),
                    ),
                ],
                vec!["output", "x", "y", "width", "height"],
            ),
        ),
        tool(
            "plasma.wait_for_change",
            "Wait For Change",
            "Poll bounded screenshots until the normalized pixel delta reaches a threshold or the timeout expires.",
            object_schema(
                vec![
                    (
                        "output",
                        json!({"type": "string", "description": "PNG output path for the latest bounded screenshot."}),
                    ),
                    (
                        "max_edge",
                        json!({"type": "integer", "minimum": 1, "description": "Screenshot preview max edge in pixels. Defaults to 1600."}),
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
                ],
                vec!["output"],
            ),
        ),
        tool(
            "plasma.focus_window",
            "Focus Window",
            "Focus a listed KWin window by id. This is policy-gated control and usually requires explicit daemon approval mode.",
            object_schema(
                with_guard_properties(vec![(
                    "window_id",
                    json!({"type": "string", "description": "KWin window id from plasma.list_windows."}),
                )]),
                vec!["window_id"],
            ),
        ),
        tool(
            "plasma.type_text",
            "Type Text",
            "Type US-keyboard-mapped text through the Linux uinput backend. This is policy-gated keyboard control and summaries report text length only.",
            object_schema(
                with_guard_properties(vec![(
                    "text",
                    json!({"type": "string", "description": "Text to type. Current uinput backend supports US keyboard ASCII plus newline and tab."}),
                )]),
                vec!["text"],
            ),
        ),
        tool(
            "plasma.key_combo",
            "Key Combo",
            "Send a key combination through the Linux uinput backend, such as Ctrl+L or Alt+F4. This is policy-gated keyboard control.",
            object_schema(
                with_guard_properties(vec![(
                    "combo",
                    json!({"type": "string", "description": "Key combination, such as Ctrl+L, Shift+F4, or Super+Space."}),
                )]),
                vec!["combo"],
            ),
        ),
        tool(
            "plasma.move_pointer",
            "Move Pointer",
            "Move the pointer to an explicit coordinate. This is policy-gated pointer control; the daemon accepts physical_pixel or guarded active-window window_local coordinates.",
            object_schema(
                with_guard_properties(vec![
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
                        json!({"type": "string", "enum": ["physical_pixel", "logical_pixel", "window_local", "accessibility_node"], "description": "Coordinate space for x and y. Supported daemon spaces are physical_pixel and window_local. window_local is relative to the active window and requires an active-window guard."}),
                    ),
                ]),
                vec!["x", "y", "coordinate_space"],
            ),
        ),
        tool(
            "plasma.click_pointer",
            "Click Pointer",
            "Move the pointer to an explicit coordinate and click once or twice. This is policy-gated pointer control; the daemon accepts physical_pixel or guarded active-window window_local coordinates.",
            object_schema(
                with_guard_properties(vec![
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
                        json!({"type": "string", "enum": ["physical_pixel", "logical_pixel", "window_local", "accessibility_node"], "description": "Coordinate space for x and y. Supported daemon spaces are physical_pixel and window_local. window_local is relative to the active window and requires an active-window guard."}),
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
            "plasma.drag_pointer",
            "Drag Pointer",
            "Drag from one explicit coordinate to another by pressing, moving, and releasing a pointer button. This is policy-gated pointer control; the daemon accepts physical_pixel or guarded active-window window_local coordinates.",
            object_schema(
                with_guard_properties(vec![
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
                        json!({"type": "string", "enum": ["physical_pixel", "logical_pixel", "window_local", "accessibility_node"], "description": "Coordinate space for all coordinates. Supported daemon spaces are physical_pixel and window_local. window_local is relative to the active window and requires an active-window guard."}),
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
            "plasma.scroll_pointer",
            "Scroll Pointer",
            "Emit vertical and/or horizontal wheel deltas at the current pointer position. This is policy-gated pointer control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.click_button",
            "Click Button",
            "Find a named non-sensitive AT-SPI button and invoke its press action only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.set_text_field",
            "Set Text Field",
            "Find a named non-sensitive AT-SPI text field and replace its contents only when exactly one viable match is found. This is policy-gated semantic control and summaries report text length only.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.activate_tab",
            "Activate Tab",
            "Find a named non-sensitive AT-SPI tab and activate it only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.activate_link",
            "Activate Link",
            "Find a named non-sensitive AT-SPI link and activate it only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.toggle_check",
            "Toggle Check",
            "Find a named non-sensitive AT-SPI checkbox, radio button, or checkable menu item and press/select it only when exactly one viable match is found. Pass checked=true or checked=false to request a desired state and avoid an unnecessary toggle when AT-SPI state already matches. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.set_value",
            "Set Value",
            "Find a named non-sensitive AT-SPI slider, spin button, scrollbar, or dial and set its numeric CurrentValue only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.select_item",
            "Select Item",
            "Find a named non-sensitive AT-SPI list, tree, table-row, combo-box, or option item and select/press it only when exactly one viable match is found. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.select_menu",
            "Select Menu",
            "Select a visible AT-SPI menu path only when exactly one non-sensitive activatable item matches. This is policy-gated semantic control.",
            object_schema(
                with_guard_properties(vec![
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
            "plasma.clipboard_get_text",
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
            "plasma.clipboard_set_text",
            "Clipboard Set Text",
            "Set UTF-8 text on the Wayland clipboard. The daemon journals the action without echoing the text in summaries.",
            object_schema(
                vec![(
                    "text",
                    json!({"type": "string", "description": "UTF-8 text to place on the clipboard."}),
                )],
                vec!["text"],
            ),
        ),
        tool(
            "plasma.a11y_focused_tree",
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
            "plasma.a11y_find",
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
            "plasma.a11y_invoke",
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
            "plasma.a11y_set_text",
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
            "plasma.a11y_insert_text",
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
            "plasma.a11y_delete_text",
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
            "plasma.a11y_copy_text",
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
            "plasma.a11y_cut_text",
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
            "plasma.a11y_paste_text",
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
            "plasma.journal_tail",
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

fn with_guard_properties(mut properties: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
    properties.extend(guard_properties());
    properties
}

fn active_window_guard(arguments: &Value) -> Result<Option<ActiveWindowGuard>> {
    let expected_window_id = optional_string(arguments, "expected_active_window")?;
    let expected_app_id = optional_string(arguments, "expected_active_app")?;
    let title_contains = optional_string(arguments, "active_title_contains")?;
    if expected_window_id.is_none() && expected_app_id.is_none() && title_contains.is_none() {
        return Ok(None);
    }
    Ok(Some(ActiveWindowGuard {
        expected_window_id,
        expected_app_id,
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

    #[test]
    fn initialize_advertises_tools_capability() {
        let result = initialize_result();
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
    fn lists_compact_tool_definitions() {
        let tools = tool_definitions();
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.list_windows")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.focus_window")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.panic_stop_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.panic_stop_enable")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.panic_stop_disable")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.kwin_bridge_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.uinput_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.input_backend_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.capture_backend_status")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.pointer_calibration")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.wait_for_change")
        );
        assert!(tools.iter().any(|tool| tool["name"] == "plasma.type_text"));
        assert!(tools.iter().any(|tool| tool["name"] == "plasma.key_combo"));
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.move_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.click_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.drag_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.scroll_pointer")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.click_button")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.set_text_field")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.activate_tab")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.activate_link")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.toggle_check")
        );
        assert!(tools.iter().any(|tool| tool["name"] == "plasma.set_value"));
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.select_item")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.select_menu")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.clipboard_get_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.a11y_focused_tree")
        );
        assert!(tools.iter().any(|tool| tool["name"] == "plasma.a11y_find"));
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.a11y_invoke")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.a11y_set_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.a11y_copy_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.a11y_cut_text")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "plasma.a11y_paste_text")
        );
    }

    #[test]
    fn maps_screenshot_tile_arguments() {
        let request = daemon_request_for_tool(
            "plasma.screenshot_tile",
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
            })
        );
    }

    #[test]
    fn maps_wait_for_change_arguments() {
        let request = daemon_request_for_tool(
            "plasma.wait_for_change",
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
            "plasma.focus_window",
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
    fn maps_active_window_guard_arguments() {
        let request = daemon_request_for_tool(
            "plasma.focus_window",
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
                    expected_window_id: Some("current-window".to_string()),
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: Some("main.rs".to_string()),
                }),
            })
        );
    }

    #[test]
    fn maps_keyboard_input_arguments() {
        let type_text = daemon_request_for_tool("plasma.type_text", &json!({"text": "hello"}))
            .expect("type text maps");
        assert_eq!(
            type_text,
            DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
            })
        );

        let key_combo = daemon_request_for_tool("plasma.key_combo", &json!({"combo": "Ctrl+L"}))
            .expect("key combo maps");
        assert_eq!(
            key_combo,
            DaemonRequest::KeyCombo(KeyComboRequest {
                combo: "Ctrl+L".to_string(),
                guard: None,
            })
        );
    }

    #[test]
    fn maps_pointer_input_arguments() {
        let move_pointer = daemon_request_for_tool(
            "plasma.move_pointer",
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
                    expected_window_id: None,
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: None,
                }),
            })
        );

        let click_pointer = daemon_request_for_tool(
            "plasma.click_pointer",
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
            })
        );

        let drag_pointer = daemon_request_for_tool(
            "plasma.drag_pointer",
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
                    expected_window_id: None,
                    expected_app_id: None,
                    title_contains: Some("Canvas".to_string()),
                }),
            })
        );

        let scroll_pointer = daemon_request_for_tool(
            "plasma.scroll_pointer",
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
            })
        );
    }

    #[test]
    fn maps_panic_stop_tools() {
        assert_eq!(
            daemon_request_for_tool("plasma.panic_stop_status", &json!({}))
                .expect("panic-stop status maps"),
            DaemonRequest::PanicStopStatus
        );
        assert_eq!(
            daemon_request_for_tool("plasma.panic_stop_enable", &json!({}))
                .expect("panic-stop enable maps"),
            DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: true })
        );
        assert_eq!(
            daemon_request_for_tool("plasma.panic_stop_disable", &json!({}))
                .expect("panic-stop disable maps"),
            DaemonRequest::SetPanicStop(SetPanicStopRequest { enabled: false })
        );
    }

    #[test]
    fn maps_kwin_bridge_status_tool() {
        assert_eq!(
            daemon_request_for_tool("plasma.kwin_bridge_status", &json!({}))
                .expect("bridge status maps"),
            DaemonRequest::KwinBridgeStatus
        );
    }

    #[test]
    fn maps_uinput_status_tool() {
        assert_eq!(
            daemon_request_for_tool("plasma.uinput_status", &json!({}))
                .expect("uinput status maps"),
            DaemonRequest::UinputStatus
        );
    }

    #[test]
    fn maps_input_backend_status_tool() {
        assert_eq!(
            daemon_request_for_tool("plasma.input_backend_status", &json!({}))
                .expect("input backend status maps"),
            DaemonRequest::InputBackendStatus
        );
    }

    #[test]
    fn maps_capture_backend_status_tool() {
        assert_eq!(
            daemon_request_for_tool("plasma.capture_backend_status", &json!({}))
                .expect("capture backend status maps"),
            DaemonRequest::CaptureBackendStatus
        );
    }

    #[test]
    fn maps_pointer_calibration_tool() {
        assert_eq!(
            daemon_request_for_tool("plasma.pointer_calibration", &json!({}))
                .expect("pointer calibration maps"),
            DaemonRequest::PointerCalibration
        );
    }

    #[test]
    fn maps_click_button_arguments() {
        let request = daemon_request_for_tool(
            "plasma.click_button",
            &json!({
                "name": "OK",
                "app": "kate",
                "window_name_contains": "settings",
                "max_nodes": 256
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
            })
        );
    }

    #[test]
    fn maps_set_text_field_arguments() {
        let request = daemon_request_for_tool(
            "plasma.set_text_field",
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
            })
        );
    }

    #[test]
    fn maps_activate_tab_arguments() {
        let request = daemon_request_for_tool(
            "plasma.activate_tab",
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
            })
        );
    }

    #[test]
    fn maps_activate_link_arguments() {
        let request = daemon_request_for_tool(
            "plasma.activate_link",
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
            })
        );
    }

    #[test]
    fn maps_toggle_check_arguments() {
        let request = daemon_request_for_tool(
            "plasma.toggle_check",
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
            })
        );
    }

    #[test]
    fn maps_set_value_arguments() {
        let request = daemon_request_for_tool(
            "plasma.set_value",
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
            })
        );
    }

    #[test]
    fn maps_select_item_arguments() {
        let request = daemon_request_for_tool(
            "plasma.select_item",
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
            })
        );
    }

    #[test]
    fn maps_select_menu_arguments() {
        let request = daemon_request_for_tool(
            "plasma.select_menu",
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
            })
        );
    }

    #[test]
    fn maps_clipboard_set_arguments() {
        let request =
            daemon_request_for_tool("plasma.clipboard_set_text", &json!({"text": "copy this"}))
                .expect("clipboard set args map");
        assert_eq!(
            request,
            DaemonRequest::ClipboardSet(ClipboardSetRequest {
                text: "copy this".to_string(),
            })
        );
    }

    #[test]
    fn maps_clipboard_get_arguments() {
        let request =
            daemon_request_for_tool("plasma.clipboard_get_text", &json!({})).expect("get maps");
        assert_eq!(
            request,
            DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: Some(DEFAULT_CLIPBOARD_MAX_BYTES),
            })
        );
    }

    #[test]
    fn maps_unbounded_clipboard_get_arguments() {
        let request = daemon_request_for_tool("plasma.clipboard_get_text", &json!({"full": true}))
            .expect("full get maps");
        assert_eq!(
            request,
            DaemonRequest::ClipboardGet(ClipboardGetRequest { max_bytes: None })
        );
    }

    #[test]
    fn maps_filtered_journal_tail_arguments() {
        let request = daemon_request_for_tool(
            "plasma.journal_tail",
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
            "plasma.a11y_focused_tree",
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
            "plasma.a11y_find",
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
    fn maps_accessibility_invoke_arguments() {
        let request = daemon_request_for_tool(
            "plasma.a11y_invoke",
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
            "plasma.a11y_set_text",
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
            "plasma.a11y_insert_text",
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
            "plasma.a11y_delete_text",
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
            "plasma.a11y_copy_text",
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
            "plasma.a11y_cut_text",
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
            "plasma.a11y_paste_text",
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
    fn maps_observe_arguments_without_screenshot() {
        let request =
            daemon_request_for_tool("plasma.observe", &json!({})).expect("observe args map");
        assert_eq!(
            request,
            DaemonRequest::Observe(ObserveRequest { screenshot: None })
        );
    }

    #[test]
    fn maps_observe_arguments_with_screenshot() {
        let request = daemon_request_for_tool(
            "plasma.observe",
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
                }),
            })
        );
    }

    #[test]
    fn daemon_errors_become_tool_errors() {
        let response = DaemonResponse::Error {
            message: "policy denied".to_string(),
        };
        let result = tool_result_from_daemon("plasma.focus_window", &response);
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "policy denied");
    }
}
