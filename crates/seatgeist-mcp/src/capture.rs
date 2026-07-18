use anyhow::{Result, bail};
use libseatgeist::{
    CaptureOpenRequest, CaptureSessionRequest, CaptureSnapshotRequest, CaptureSourceKind,
    CaptureWaitRequest, DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS,
    DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS, DaemonRequest, WindowCaptureOpenRequest,
    WindowInventoryWaitRequest,
};
use serde_json::{Value, json};

use super::{
    object_schema, optional_output_path, optional_string, optional_u64, required_string, tool,
    u64_to_u32,
};

pub(crate) fn core_snapshot_request(arguments: &Value) -> Result<DaemonRequest> {
    snapshot_request(
        arguments,
        required_string(arguments, "session_id")?,
        "window-snapshot",
    )
}

pub(crate) fn core_wait_request(arguments: &Value) -> Result<DaemonRequest> {
    wait_request(
        arguments,
        required_string(arguments, "session_id")?,
        "window-wait",
    )
}

pub(crate) fn window_session_request(arguments: &Value) -> Result<DaemonRequest> {
    match optional_string(arguments, "operation")?
        .unwrap_or_else(|| "status".to_string())
        .as_str()
    {
        "open" => core_open_request(arguments),
        "status" => Ok(DaemonRequest::CaptureSessionStatus),
        "inventory" => Ok(DaemonRequest::WindowInventory),
        "wait_inventory" => Ok(DaemonRequest::WindowInventoryWait(
            WindowInventoryWaitRequest {
                after_revision: required_string(arguments, "after_revision")?,
                timeout_ms: optional_u64(arguments, "timeout_ms")?
                    .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS),
            },
        )),
        "renew" => renew_request(arguments),
        "close" => Ok(DaemonRequest::CaptureSessionClose(CaptureSessionRequest {
            session_id: required_string(arguments, "session_id")?,
        })),
        operation => bail!("unsupported window_session operation: {operation}"),
    }
}

fn core_open_request(arguments: &Value) -> Result<DaemonRequest> {
    Ok(DaemonRequest::WindowCaptureOpen(WindowCaptureOpenRequest {
        requested_window_id: Some(required_string(arguments, "requested_window_id")?),
        parent_window: String::new(),
        timeout_ms: optional_u64(arguments, "timeout_ms")?
            .unwrap_or(DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS),
    }))
}

pub(crate) fn open_request(arguments: &Value) -> Result<DaemonRequest> {
    Ok(DaemonRequest::WindowCaptureOpen(WindowCaptureOpenRequest {
        requested_window_id: optional_string(arguments, "requested_window_id")?,
        parent_window: optional_string(arguments, "parent_window")?.unwrap_or_default(),
        timeout_ms: optional_u64(arguments, "timeout_ms")?
            .unwrap_or(DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS),
    }))
}

pub(crate) fn generic_open_request(arguments: &Value) -> Result<DaemonRequest> {
    let source = match required_string(arguments, "source")?.as_str() {
        "window" => CaptureSourceKind::Window,
        "monitor" => CaptureSourceKind::Monitor,
        "virtual_output" => CaptureSourceKind::VirtualOutput,
        source => bail!("unsupported retained capture source: {source}"),
    };
    Ok(DaemonRequest::CaptureOpen(CaptureOpenRequest {
        source,
        requested_source_id: optional_string(arguments, "requested_source_id")?,
        parent_window: optional_string(arguments, "parent_window")?.unwrap_or_default(),
        timeout_ms: optional_u64(arguments, "timeout_ms")?
            .unwrap_or(DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS),
    }))
}

pub(crate) fn expert_snapshot_request(arguments: &Value) -> Result<DaemonRequest> {
    snapshot_request(
        arguments,
        required_string(arguments, "session_id")?,
        "window-snapshot",
    )
}

pub(crate) fn expert_wait_request(arguments: &Value) -> Result<DaemonRequest> {
    wait_request(
        arguments,
        required_string(arguments, "session_id")?,
        "window-wait",
    )
}

pub(crate) fn close_request(arguments: &Value) -> Result<DaemonRequest> {
    Ok(DaemonRequest::CaptureSessionClose(CaptureSessionRequest {
        session_id: required_string(arguments, "session_id")?,
    }))
}

pub(crate) fn renew_request(arguments: &Value) -> Result<DaemonRequest> {
    Ok(DaemonRequest::CaptureSessionRenew(CaptureSessionRequest {
        session_id: required_string(arguments, "session_id")?,
    }))
}

fn snapshot_request(arguments: &Value, session_id: String, kind: &str) -> Result<DaemonRequest> {
    Ok(DaemonRequest::CaptureSnapshot(CaptureSnapshotRequest {
        session_id,
        output: optional_output_path(arguments, "output", kind)?,
        max_edge: optional_u64(arguments, "max_edge")?
            .map(u64_to_u32)
            .transpose()?,
        timeout_ms: optional_u64(arguments, "timeout_ms")?.unwrap_or(1_500),
    }))
}

fn wait_request(arguments: &Value, session_id: String, kind: &str) -> Result<DaemonRequest> {
    Ok(DaemonRequest::CaptureWait(CaptureWaitRequest {
        session_id,
        after_revision: optional_string(arguments, "after_revision")?,
        output: optional_output_path(arguments, "output", kind)?,
        max_edge: optional_u64(arguments, "max_edge")?
            .map(u64_to_u32)
            .transpose()?,
        timeout_ms: optional_u64(arguments, "timeout_ms")?
            .unwrap_or(DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS),
    }))
}

pub(crate) fn window_session_tool() -> Value {
    tool(
        "seatgeist.window_session",
        "Window Session",
        "Read or wait on the revisioned KWin inventory, or open, inspect, renew, and close one exact window session. Exact opens use KWin screenshots without a chooser; status is the default operation.",
        object_schema(
            vec![
                (
                    "operation",
                    json!({"type": "string", "enum": ["inventory", "wait_inventory", "open", "status", "renew", "close"], "description": "Inventory returns windows plus an opaque revision; wait_inventory blocks until that revision changes or times out. Session status is the default."}),
                ),
                (
                    "session_id",
                    json!({"type": "string", "description": "Required for renew or close; use the id returned by open or status."}),
                ),
                (
                    "requested_window_id",
                    json!({"type": "string", "description": "Exact KWin UUID required for operation=open. The daemon policy-checks and pins it, then uses authorized KWin exact-window screenshots without a chooser."}),
                ),
                (
                    "parent_window",
                    json!({"type": "string", "description": "Compatibility-only portal parent identifier; ignored by core exact-window opens."}),
                ),
                (
                    "timeout_ms",
                    json!({"type": "integer", "minimum": 1, "maximum": 300000, "description": "Bounded open or inventory-wait timeout."}),
                ),
                (
                    "after_revision",
                    json!({"type": "string", "description": "Required for wait_inventory; use the revision returned by inventory or the previous wait."}),
                ),
            ],
            vec![],
        ),
    )
}

pub(crate) fn core_snapshot_schema() -> Value {
    object_schema(
        vec![
            (
                "session_id",
                json!({"type": "string", "description": "Required retained window-session id returned by window_session open or status."}),
            ),
            (
                "output",
                json!({"type": "string", "description": "Optional PNG output path; a private runtime path is generated when omitted."}),
            ),
            (
                "max_edge",
                json!({"type": "integer", "minimum": 1, "maximum": 2048, "description": "Maximum output edge. The daemon preview bound is used when omitted."}),
            ),
            (
                "include_image",
                json!({"type": "boolean", "description": "Attach the bounded PNG to this MCP result. Defaults to true."}),
            ),
            (
                "timeout_ms",
                json!({"type": "integer", "minimum": 1, "maximum": 30000, "description": "Frame acquisition timeout."}),
            ),
        ],
        vec!["session_id"],
    )
}

pub(crate) fn core_wait_schema() -> Value {
    object_schema(
        vec![
            (
                "session_id",
                json!({"type": "string", "description": "Required retained window-session id returned by window_session open or status."}),
            ),
            (
                "after_revision",
                json!({"type": "string", "description": "Return changed=true only for content newer than this revision."}),
            ),
            (
                "output",
                json!({"type": "string", "description": "Optional PNG output path."}),
            ),
            (
                "max_edge",
                json!({"type": "integer", "minimum": 1, "maximum": 2048}),
            ),
            (
                "timeout_ms",
                json!({"type": "integer", "minimum": 1, "maximum": 30000}),
            ),
            (
                "include_image",
                json!({"type": "boolean", "description": "Attach the latest bounded PNG. Defaults to true."}),
            ),
        ],
        vec!["session_id"],
    )
}

pub(crate) fn open_tool() -> Value {
    tool(
        "seatgeist.window_capture_open",
        "Window Capture Open",
        "Open one daemon-retained window session. An exact KWin UUID uses direct KWin screenshots with no chooser; omitting it explicitly uses portal ScreenCast selection. Only one session may be active.",
        object_schema(
            vec![
                (
                    "requested_window_id",
                    json!({"type": "string", "description": "Optional exact KWin window UUID. When present, routes to direct KWin screenshots and bypasses portal ScreenCast."}),
                ),
                (
                    "parent_window",
                    json!({"type": "string", "description": "Optional xdg-foreign parent window identifier used only when requested_window_id is omitted and the portal dialog opens."}),
                ),
                (
                    "timeout_ms",
                    json!({"type": "integer", "minimum": 1, "maximum": 300000}),
                ),
            ],
            vec![],
        ),
    )
}

pub(crate) fn generic_open_tool() -> Value {
    tool(
        "seatgeist.capture_open",
        "Retained Capture Open",
        "Expert-only retained capture open. An exact window UUID uses direct KWin screenshots; generic windows, monitors, and virtual outputs use chooser-authoritative portal ScreenCast. Only one session may be active.",
        object_schema(
            vec![
                (
                    "source",
                    json!({"type": "string", "enum": ["window", "monitor", "virtual_output"], "description": "Exact retained portal source contract."}),
                ),
                (
                    "requested_source_id",
                    json!({"type": "string", "description": "Exact KWin UUID for a direct window capture, optional monitor intent for portal capture, or omitted for generic window/virtual_output portal selection."}),
                ),
                (
                    "parent_window",
                    json!({"type": "string", "description": "Optional xdg-foreign parent window identifier for portal consent."}),
                ),
                (
                    "timeout_ms",
                    json!({"type": "integer", "minimum": 1, "maximum": 300000}),
                ),
            ],
            vec!["source"],
        ),
    )
}

pub(crate) fn status_tool() -> Value {
    tool(
        "seatgeist.capture_session_status",
        "Capture Session Status",
        "Inspect the daemon-retained capture session without opening a portal dialog or acquiring a new frame.",
        object_schema(vec![], vec![]),
    )
}

pub(crate) fn renew_tool() -> Value {
    tool(
        "seatgeist.capture_session_renew",
        "Capture Session Renew",
        "Extend the bounded lifetime of a still-live, identity-validated pinned interaction target without opening a portal dialog or sending input.",
        object_schema(
            vec![("session_id", json!({"type": "string"}))],
            vec!["session_id"],
        ),
    )
}

pub(crate) fn snapshot_tool() -> Value {
    tool(
        "seatgeist.capture_snapshot",
        "Capture Snapshot",
        "Read one bounded PNG from an existing retained window stream and return its content revision.",
        frame_schema(false),
    )
}

pub(crate) fn wait_tool() -> Value {
    tool(
        "seatgeist.capture_wait",
        "Capture Wait",
        "Wait for a retained window stream to produce content newer than an optional revision and return explicit changed/timeout metadata.",
        frame_schema(true),
    )
}

pub(crate) fn close_tool() -> Value {
    tool(
        "seatgeist.capture_session_close",
        "Capture Session Close",
        "Close the named daemon-retained capture session and release its PipeWire and portal resources.",
        object_schema(
            vec![("session_id", json!({"type": "string"}))],
            vec!["session_id"],
        ),
    )
}

fn frame_schema(include_revision: bool) -> Value {
    let mut properties = vec![
        (
            "session_id",
            json!({"type": "string", "description": "Id returned by window_capture_open or capture_session_status."}),
        ),
        (
            "output",
            json!({"type": "string", "description": "Optional PNG output path; a private runtime path is generated when omitted."}),
        ),
        (
            "max_edge",
            json!({"type": "integer", "minimum": 1, "maximum": 2048, "description": "Maximum output edge, additionally capped by daemon safety configuration."}),
        ),
        (
            "timeout_ms",
            json!({"type": "integer", "minimum": 1, "maximum": 30000}),
        ),
        (
            "include_image",
            json!({"type": "boolean", "description": "Attach the bounded PNG to this MCP result. Defaults to true."}),
        ),
    ];
    if include_revision {
        properties.push((
            "after_revision",
            json!({"type": "string", "description": "Optional last-seen content revision."}),
        ));
    }
    object_schema(properties, vec!["session_id"])
}
