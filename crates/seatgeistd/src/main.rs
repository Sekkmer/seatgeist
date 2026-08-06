#[cfg(test)]
use std::fmt::Display;
use std::{
    collections::BTreeMap,
    env, fs,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod activity;
mod agent_seat;
mod capture;
mod capture_backend;
mod capture_diagnostics;
mod capture_restore;
mod clipboard;
mod commands;
mod compatibility_capture_backend;
mod config;
mod eis_key_combo;
mod input_actions;
mod input_diagnostics;
mod input_execution;
mod interaction;
mod keymap;
mod kwin_bridge;
mod kwin_capture_backend;
mod observation;
mod observation_policy;
mod pointer_coordinates;
mod portal_eis_probe;
mod portal_eis_session;
mod post_action_capture;
mod safety_runtime;
mod screenshot;
mod screenshot_image;
mod semantic_handle;
mod semantic_settle;
mod session_execution;
mod session_owner;
mod target;
mod window_backend;
mod window_safety;
mod xdg;

use anyhow::{Context, Error, Result, bail};
use capture::capture_open;
use capture::{CaptureSessionStore, normalize_capture_frame_request};
use capture_backend::PortalScreenBackend;
use capture_diagnostics::{screenshot_portal_status, status as capture_backend_status};
use capture_restore::CaptureRestoreTokenStore;
use clap::Parser;
use commands::exists as command_exists;
use config::*;
#[cfg(test)]
use eis_key_combo::codes_with_keymap as eis_key_combo_codes_with_keymap;
use input_actions::{
    agent_click_pointer, agent_drag_pointer, agent_key_combo, agent_move_pointer,
    agent_scroll_pointer, agent_type_text, click_pointer, drag_pointer, key_combo, move_pointer,
    page_zoom, scroll_pointer, type_text,
};
use input_diagnostics::uinput_status;
#[cfg(test)]
use input_execution::backend_with_store as input_execution_backend_with_store;
#[cfg(test)]
use input_execution::session_backend as eis_session_input_execution_backend;
#[cfg(test)]
use keymap::Settings as XkbKeymapSettings;
use keymap::{Config as XkbKeymapConfig, resolve as effective_xkb_keymap_resolution};
use kwin_bridge::{
    ActiveWindowState, WindowActionQueue, WindowListState, start_kwin_bridge,
    status as kwin_bridge_status, supervise_kwin_bridge_ownership,
};
use kwin_capture_backend::RoutedScreenBackend;
use libseatgeist::{
    AccessibilityCopyTextRequest, AccessibilityCutTextRequest, AccessibilityDeleteTextRequest,
    AccessibilityFindRequest, AccessibilityInsertTextRequest, AccessibilityInvokeRequest,
    AccessibilityPasteTextRequest, AccessibilityQualityStatus, AccessibilitySetCaretRequest,
    AccessibilitySetSelectionRequest, AccessibilitySetTextRequest,
    AccessibilityTextAttributesRequest, ActionReadiness, ActionResult, ActionSettleCondition,
    ActionSettleResult, ActivateLinkRequest, ActivateTabRequest, ActiveWindowGuard,
    BackendCapability, CapabilitySet, CaptureOpenRequest, CaptureSessionRequest, CaptureSourceKind,
    ClickButtonRequest, ClickPointerRequest, CloseWindowRequest, ComputerUseReadinessStatus,
    CoordinateSpace, DaemonClientIdentity, DaemonRequest, DaemonRequestEnvelope, DaemonResponse,
    DaemonResponseOptions, DesktopSessionStatus, DragPointerRequest, ErrorKind,
    FocusTextFieldRequest, FocusWindowRequest, FocusedAccessibilityTreeRequest, HealthStatus,
    InputBackendStatus, JournalArtifactContext, JournalClientContext, JournalControlContext,
    JournalEntry, JournalRequestedTarget, JournalWindowContext, LaunchWindowRequest,
    MovePointerRequest, MoveWindowRequest, Observation, PanicStopStatus, Point, PolicyStatus,
    PostActionOptions, ResizeWindowRequest, SafetyClass, SafetyStatus, ScreenshotInfo,
    ScreenshotTransform, SelectItemRequest, SelectMenuRequest, SetPanicStopRequest,
    SetTextFieldRequest, SetValueRequest, ToggleCheckRequest, ToolApprovalLevel, WindowInfo,
    current_euid, default_capture_restore_path, default_journal_path, default_panic_stop_path,
    default_socket_path,
};
#[cfg(test)]
use libseatgeist::{
    AccessibilityNode, KeyComboRequest, ObserveRequest, PointerButton, PointerCalibrationPoint,
    PointerPhysicalBounds, RemoteDesktopSessionProbeRequest, ScreenshotRequest,
    ScrollPointerRequest, TypeTextRequest, WaitForChangeRequest, WaitForChangeResult,
    WindowGeometry,
};
#[cfg(test)]
use pointer_coordinates::{
    active_window_local_to_physical_point,
    calibration_from_monitors as pointer_calibration_status_from_monitors,
    logical_to_physical_point, physical_pointer_bounds_from_monitors,
    validate_physical_pointer_point,
};
#[cfg(test)]
use portal_eis_probe::{
    eis_capability_names, remote_desktop_device_types, remote_desktop_probe_timeout,
};
use portal_eis_probe::{remote_desktop_eis_probe, remote_desktop_session_probe};
#[cfg(test)]
use portal_eis_session::{DaemonPortalEisSession, DaemonPortalEisSessionMetadata};
use portal_eis_session::{
    PortalEisSessionStore, remote_desktop_eis_start, remote_desktop_eis_stop,
};
use safety_runtime::{ApprovalStore, ControlRateLimiter, PanicStopState};
use screenshot::{capture_screenshot, capture_screenshot_tile, wait_for_change};
use seatgeist_backend::{ScreenBackend, TargetedInputBackend, TargetedInputContext, WindowBackend};
use seatgeist_policy::{PolicyConfig, PolicyEngine};
use session_owner::SessionOwner;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};
use tracing::{error, info, warn};
use uuid::Uuid;
use window_backend::{KwinWindowBackend, active_window, list_monitors, list_windows_with_monitors};
#[cfg(test)]
use window_backend::{active_window_with_monitors, assign_monitor_id, merge_bridge_windows};
use window_safety::{enforce_active_window_guard, enforce_app_policy, enforce_app_policy_for_app};

const SEMANTIC_CHOICE_LIMIT: usize = 5;
const ACCESSIBILITY_QUALITY_SAMPLE_DEPTH: usize = 4;
const ACCESSIBILITY_QUALITY_SAMPLE_MAX_NODES: usize = 512;
const ACCESSIBILITY_QUALITY_TIMEOUT: Duration = Duration::from_millis(1_500);
const ACCESSIBILITY_TREE_TIMEOUT: Duration = Duration::from_secs(5);
const DAEMON_PROTOCOL_VERSION: &str = "1";

#[derive(Debug, Clone)]
struct ActionJournal {
    path: PathBuf,
    settings: JournalSettings,
    run_id: Uuid,
    build_id: String,
    sequence: Arc<Mutex<u64>>,
}

impl ActionJournal {
    fn new(path: PathBuf, settings: JournalSettings, binary_sha256: Option<&str>) -> Self {
        let build_id = binary_sha256
            .map(|digest| digest.chars().take(16).collect())
            .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
        Self {
            path,
            settings,
            run_id: Uuid::new_v4(),
            build_id,
            sequence: Arc::new(Mutex::new(0)),
        }
    }

    fn record_lifecycle(&self, method: &str, summary: &str) -> Result<()> {
        let entry = JournalEntry {
            sequence: self.next_sequence()?,
            unix_time_ms: unix_time_ms()?,
            method: method.to_string(),
            run_id: Some(self.run_id),
            build_id: Some(self.build_id.clone()),
            client: None,
            safety_class: Some(SafetyClass::Policy),
            guard_present: false,
            active_window_before: None,
            active_window_after: None,
            control: None,
            artifacts: Vec::new(),
            ok: true,
            summary: summary.to_string(),
        };
        append_journal_entry(&self.path, &entry)
    }

    fn record(
        &self,
        method: &str,
        context: JournalContext,
        response: &DaemonResponse,
    ) -> Result<()> {
        let control = finalize_journal_control_context(context.control, response);
        let artifacts = journal_artifacts_for_response(response, &self.settings);
        let entry = JournalEntry {
            sequence: self.next_sequence()?,
            unix_time_ms: unix_time_ms()?,
            method: method.to_string(),
            run_id: Some(self.run_id),
            build_id: Some(self.build_id.clone()),
            client: context.client,
            safety_class: Some(context.safety_class),
            guard_present: context.guard_present,
            active_window_before: context.active_window_before,
            active_window_after: context.active_window_after,
            control,
            artifacts,
            ok: response.ok(),
            summary: journal_response_summary(response, &self.settings),
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

    fn record_focus_lease_step(
        &self,
        method: &str,
        session_id: &str,
        lease_id: Uuid,
        window: &WindowInfo,
        backend: &str,
        ok: bool,
    ) -> Result<()> {
        let mut target = journal_target("sticky_focus_lease");
        target.add("session_id", session_id);
        target.add("window_id", &window.id);
        if let Some(app_id) = window.app_id.as_deref() {
            target.add("app_id", app_id);
        }
        if let Some(pid) = window.pid {
            target.add("pid", pid.to_string());
        }
        let entry = JournalEntry {
            sequence: self.next_sequence()?,
            unix_time_ms: unix_time_ms()?,
            method: method.to_string(),
            run_id: Some(self.run_id),
            build_id: Some(self.build_id.clone()),
            client: None,
            safety_class: Some(SafetyClass::ControlSemantic),
            guard_present: true,
            active_window_before: None,
            active_window_after: None,
            control: Some(JournalControlContext {
                action_id: Some(lease_id),
                policy: Some(if ok { "allow" } else { "checked" }.to_string()),
                backend: Some(backend.to_string()),
                requested_target: Some(target),
            }),
            artifacts: Vec::new(),
            ok,
            summary: format!(
                "sticky focus lease session={} lease={} window={} ok={}",
                session_id, lease_id, window.id, ok
            ),
        };
        append_journal_entry(&self.path, &entry)
    }

    fn record_agent_seat_delivery(
        &self,
        session_id: &str,
        lane_id: &str,
        action_id: Uuid,
        window: &WindowInfo,
        backend: &str,
        safety_class: SafetyClass,
    ) -> Result<()> {
        let mut target = journal_target("independent_agent_seat");
        target.add("session_id", session_id);
        target.add("lane_id", lane_id);
        target.add("window_id", &window.id);
        if let Some(app_id) = window.app_id.as_deref() {
            target.add("app_id", app_id);
        }
        if let Some(pid) = window.pid {
            target.add("pid", pid.to_string());
        }
        let entry = JournalEntry {
            sequence: self.next_sequence()?,
            unix_time_ms: unix_time_ms()?,
            method: "agent_seat_delivery".to_string(),
            run_id: Some(self.run_id),
            build_id: Some(self.build_id.clone()),
            client: None,
            safety_class: Some(safety_class),
            guard_present: true,
            active_window_before: None,
            active_window_after: None,
            control: Some(JournalControlContext {
                action_id: Some(action_id),
                policy: Some("allow".to_string()),
                backend: Some(backend.to_string()),
                requested_target: Some(target),
            }),
            artifacts: Vec::new(),
            ok: true,
            summary: format!(
                "independent agent seat session={} lane={} action={} window={} backend={} ok=true",
                session_id, lane_id, action_id, window.id, backend
            ),
        };
        append_journal_entry(&self.path, &entry)
    }

    fn record_post_action_capture_step(
        &self,
        method: &str,
        session_id: &str,
        action_id: Uuid,
        target_window_id: Option<&str>,
        ok: bool,
    ) -> Result<()> {
        let mut target = journal_target("post_action_capture");
        target.add("session_id", session_id);
        if let Some(window_id) = target_window_id {
            target.add("window_id", window_id);
        }
        let entry = JournalEntry {
            sequence: self.next_sequence()?,
            unix_time_ms: unix_time_ms()?,
            method: method.to_string(),
            run_id: Some(self.run_id),
            build_id: Some(self.build_id.clone()),
            client: None,
            safety_class: Some(SafetyClass::Observe),
            guard_present: true,
            active_window_before: None,
            active_window_after: None,
            control: Some(JournalControlContext {
                action_id: Some(action_id),
                policy: Some(if ok { "allow" } else { "checked" }.to_string()),
                backend: Some("portal_screencast_pipewire".to_string()),
                requested_target: Some(target),
            }),
            artifacts: Vec::new(),
            ok,
            summary: format!(
                "post-action capture session={} action={} ok={}",
                session_id, action_id, ok
            ),
        };
        append_journal_entry(&self.path, &entry)
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

#[derive(Debug, Clone)]
struct JournalContext {
    client: Option<JournalClientContext>,
    safety_class: SafetyClass,
    guard_present: bool,
    active_window_before: Option<JournalWindowContext>,
    active_window_after: Option<JournalWindowContext>,
    control: Option<JournalControlContext>,
}

#[derive(Debug, Parser)]
#[command(version, about = "Seatgeist local desktop-control daemon")]
struct Args {
    #[arg(long, env = "SEATGEIST_CONFIG")]
    config: Option<PathBuf>,

    #[arg(long, env = "SEATGEIST_SOCKET")]
    socket: Option<PathBuf>,

    #[arg(long, env = "SEATGEIST_JOURNAL")]
    journal: Option<PathBuf>,

    #[arg(long, env = "SEATGEIST_PANIC_STOP_FILE")]
    panic_stop_file: Option<PathBuf>,

    #[arg(long, env = "SEATGEIST_APPROVAL_FILE")]
    approval_file: Option<PathBuf>,

    #[arg(long, env = "SEATGEIST_CAPTURE_RESTORE_FILE")]
    capture_restore_file: Option<PathBuf>,

    #[arg(long, env = "SEATGEIST_DISABLE_KWIN_BRIDGE")]
    disable_kwin_bridge: bool,

    #[arg(long, env = "SEATGEIST_INPUT_BACKEND", value_enum)]
    input_backend: Option<InputBackendPreference>,

    #[arg(long, env = "SEATGEIST_ALLOW_CONTROL")]
    allow_control: bool,

    #[arg(long, env = "SEATGEIST_ALLOW_CLIPBOARD_READ")]
    allow_clipboard_read: bool,

    #[arg(long, env = "SEATGEIST_ALLOW_FULL_RESOLUTION_SCREENSHOT")]
    allow_full_resolution_screenshot: bool,

    #[arg(long)]
    print_capabilities: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let file_config = load_daemon_config(args.config.as_deref())?;
    let daemon_file_config = file_config.daemon.as_ref();
    let input_backend_preference = input_backend_preference(
        args.input_backend,
        file_config
            .backends
            .as_ref()
            .and_then(|config| config.input),
    );
    let xkb_keymap_config = keymap::config(
        file_config
            .backends
            .as_ref()
            .and_then(|backends| backends.keymap.as_ref()),
    );

    if args.print_capabilities {
        let portal_eis_session_store = PortalEisSessionStore::default();
        println!(
            "{}",
            serde_json::to_string_pretty(&capabilities(
                input_backend_preference,
                &portal_eis_session_store,
                false,
                false,
                false,
                false,
                false,
            ))?
        );
        return Ok(());
    }

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
    let approval_file = configured_optional_path(
        args.approval_file,
        daemon_file_config.and_then(|config| config.approval_file.as_deref()),
    )
    .context("resolve daemon approval file path")?;
    let capture_restore_file = configured_path(
        args.capture_restore_file,
        daemon_file_config.and_then(|config| config.capture_restore_file.as_deref()),
        default_capture_restore_path,
    )
    .context("resolve daemon capture restore-token path")?;
    let policy_config = policy_config(
        file_config.policy.as_ref(),
        args.allow_control,
        args.allow_clipboard_read,
        args.allow_full_resolution_screenshot,
    );
    let app_policy = app_policy(file_config.apps.as_ref());
    let safety_settings =
        safety_settings(file_config.safety.as_ref()).context("resolve safety settings")?;
    let journal_settings = journal_settings(file_config.journal.as_ref());

    run(RunSettings {
        socket,
        journal_path: journal,
        journal_settings,
        panic_stop_path: panic_stop_file,
        approval_file_path: approval_file,
        capture_restore_path: capture_restore_file,
        kwin_bridge_enabled: !args.disable_kwin_bridge,
        policy_config,
        app_policy,
        safety_settings,
        input_backend_preference,
        xkb_keymap_config,
    })
    .await
}

#[derive(Debug)]
struct RunSettings {
    socket: PathBuf,
    journal_path: PathBuf,
    journal_settings: JournalSettings,
    panic_stop_path: PathBuf,
    approval_file_path: Option<PathBuf>,
    capture_restore_path: PathBuf,
    kwin_bridge_enabled: bool,
    policy_config: PolicyConfig,
    app_policy: AppPolicy,
    safety_settings: SafetySettings,
    input_backend_preference: InputBackendPreference,
    xkb_keymap_config: XkbKeymapConfig,
}

async fn run(settings: RunSettings) -> Result<()> {
    let config_fingerprint = config_fingerprint(&settings);
    let RunSettings {
        socket,
        journal_path,
        journal_settings,
        panic_stop_path,
        approval_file_path,
        capture_restore_path,
        kwin_bridge_enabled,
        policy_config,
        app_policy,
        safety_settings,
        input_backend_preference,
        xkb_keymap_config,
    } = settings;
    // Hash the executable once per daemon run and reuse it for health and
    // journal provenance. Debug binaries are large enough that reading them
    // repeatedly can materially delay parallel private-daemon tests.
    let binary_sha256 = executable_sha256();
    let journal = ActionJournal::new(journal_path, journal_settings, binary_sha256.as_deref());
    let health_status = health(&journal, config_fingerprint, binary_sha256);
    let panic_stop = PanicStopState::new(panic_stop_path);
    let approval_store = ApprovalStore::new(approval_file_path);
    let policy = PolicyEngine::new(policy_config);
    let active_window_state = ActiveWindowState::default();
    let window_list_state = WindowListState::default();
    let portal_eis_session_store = PortalEisSessionStore::default();
    let capture_session_store = CaptureSessionStore::default();
    let session_execution_store = session_execution::SessionExecutionStore::default();
    let semantic_handle_store = semantic_handle::SemanticHandleStore::default();
    let capture_restore_store = CaptureRestoreTokenStore::new(capture_restore_path);
    let portal_screen_backend: Arc<dyn ScreenBackend> =
        Arc::new(PortalScreenBackend::new(capture_restore_store));
    let screen_backend: Arc<dyn ScreenBackend> =
        Arc::new(RoutedScreenBackend::new(portal_screen_backend));
    let interaction_session_store = interaction::InteractionSessionStore::default();
    let activity_tracker = activity::ActivityTracker::default();
    let window_action_queue = WindowActionQueue::default();
    let agent_seat_backend = agent_seat::KwinAgentSeatBackend::default();
    let focus_backend: Arc<dyn interaction::FocusBackend> = Arc::new(interaction::KwinFocusBackend);
    let _kwin_bridge_connection = if kwin_bridge_enabled {
        match start_kwin_bridge(
            active_window_state.clone(),
            window_list_state.clone(),
            activity_tracker.clone(),
            capture_session_store.clone(),
            window_action_queue.clone(),
            agent_seat_backend.clone(),
        )
        .await
        {
            Ok(connection) => Some(connection),
            Err(err) => {
                warn!(error = %err, "KWin bridge DBus service is unavailable");
                None
            }
        }
    } else {
        info!("KWin bridge disabled for isolated daemon");
        None
    };
    let kwin_bridge_registered = _kwin_bridge_connection.is_some();
    window_action_queue.set_registered(kwin_bridge_registered);
    if kwin_bridge_enabled {
        supervise_kwin_bridge_ownership(
            _kwin_bridge_connection,
            active_window_state.clone(),
            window_list_state.clone(),
            activity_tracker.clone(),
            capture_session_store.clone(),
            window_action_queue.clone(),
            agent_seat_backend.clone(),
        );
    }
    let window_backend: Arc<dyn WindowBackend> = Arc::new(KwinWindowBackend::new(
        active_window_state.clone(),
        window_list_state.clone(),
        focus_backend.clone(),
        window_action_queue.clone(),
    ));
    let runtime = DaemonRuntime {
        health_status,
        active_window_state,
        window_list_state,
        journal,
        panic_stop,
        control_rate_limiter: ControlRateLimiter::new(
            safety_settings.control_rate_limit_per_minute,
        ),
        approval_store,
        policy,
        app_policy,
        safety_settings,
        input_backend_preference,
        xkb_keymap_config,
        portal_eis_session_store,
        capture_session_store,
        session_execution_store,
        semantic_handle_store,
        screen_backend,
        interaction_session_store,
        activity_tracker,
        window_action_queue: window_action_queue.clone(),
        agent_seat_backend,
        window_backend,
    };

    prepare_socket_path(&socket)?;
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("bind daemon socket at {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("set socket permissions on {}", socket.display()))?;
    validate_socket_permissions(&socket)?;

    runtime.journal.record_lifecycle(
        "daemon_start",
        &format!(
            "daemon started run={} build={}",
            runtime.journal.run_id, runtime.journal.build_id
        ),
    )?;
    info!(socket = %socket.display(), "seatgeistd listening");

    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _addr) = accepted.context("accept client")?;
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_client(stream, runtime).await {
                        warn!(error = %err, "client request failed");
                    }
                });
            }
            signal = &mut shutdown => {
                signal?;
                break;
            }
        }
    }
    runtime.journal.record_lifecycle(
        "daemon_stop",
        &format!(
            "daemon stopped run={} build={}",
            runtime.journal.run_id, runtime.journal.build_id
        ),
    )?;
    info!("seatgeistd stopped");
    Ok(())
}

async fn shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("wait for SIGINT"),
        _ = terminate.recv() => Ok(()),
    }
}

#[derive(Debug, Clone)]
struct DaemonRuntime {
    health_status: HealthStatus,
    active_window_state: ActiveWindowState,
    window_list_state: WindowListState,
    journal: ActionJournal,
    panic_stop: PanicStopState,
    control_rate_limiter: ControlRateLimiter,
    approval_store: ApprovalStore,
    policy: PolicyEngine,
    app_policy: AppPolicy,
    safety_settings: SafetySettings,
    input_backend_preference: InputBackendPreference,
    xkb_keymap_config: XkbKeymapConfig,
    portal_eis_session_store: PortalEisSessionStore,
    capture_session_store: CaptureSessionStore,
    session_execution_store: session_execution::SessionExecutionStore,
    semantic_handle_store: semantic_handle::SemanticHandleStore,
    screen_backend: Arc<dyn ScreenBackend>,
    interaction_session_store: interaction::InteractionSessionStore,
    activity_tracker: activity::ActivityTracker,
    window_action_queue: WindowActionQueue,
    agent_seat_backend: agent_seat::KwinAgentSeatBackend,
    window_backend: Arc<dyn WindowBackend>,
}

async fn handle_client(stream: UnixStream, runtime: DaemonRuntime) -> Result<()> {
    let peer_client = validate_peer_client(&stream)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .await
        .context("read request line")?;
    if bytes == 0 {
        bail!("empty request");
    }

    let (request, request_client, response_options) = parse_daemon_request_line(&line)?;
    let client = merge_client_context(peer_client, request_client);
    let method = request.method_name();
    let mut journal_context = journal_context_for_request(&request, &runtime, client.clone());
    let response = handle_request(
        request,
        response_options.as_ref(),
        client.as_ref(),
        &runtime,
    )
    .await;
    journal_context.active_window_after = active_window_context_for_safety_class(
        &journal_context.safety_class,
        &runtime.active_window_state,
        &runtime.app_policy,
    );
    runtime
        .journal
        .record(method, journal_context, &response)
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

fn parse_daemon_request_line(
    line: &str,
) -> Result<(
    DaemonRequest,
    Option<JournalClientContext>,
    Option<DaemonResponseOptions>,
)> {
    match serde_json::from_str::<DaemonRequestEnvelope>(line) {
        Ok(envelope) => {
            let client = envelope.client.and_then(client_context_from_identity);
            Ok((envelope.request, client, envelope.response_options))
        }
        Err(_) => {
            let request =
                serde_json::from_str::<DaemonRequest>(line).context("parse daemon request")?;
            Ok((request, None, None))
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedPostAction {
    options: PostActionOptions,
    condition: ActionSettleCondition,
    delivery_ack: bool,
    expected_active_window: Option<String>,
    before: Option<Observation>,
}

async fn prepare_post_action(
    request: &DaemonRequest,
    response_options: Option<&DaemonResponseOptions>,
    runtime: &DaemonRuntime,
) -> Result<Option<PreparedPostAction>> {
    if !daemon_request_returns_action(request) {
        return Ok(None);
    }
    let Some(options) = response_options.and_then(|options| options.post_action.as_ref()) else {
        return Ok(None);
    };
    if !options.observe_after {
        return Ok(None);
    }
    validate_post_action_response_options(request, response_options)?;
    let mut options = options.clone();
    if let Some(image) = options.image.as_mut() {
        normalize_capture_frame_request(
            &mut image.max_edge,
            image.timeout_ms,
            runtime.safety_settings.preview_max_edge,
        )?;
        post_action_capture::validate(request, image, runtime).await?;
    }
    let target_event_settle = target_window_guard_for_request(request).is_some()
        && matches!(
            options.settle_condition,
            ActionSettleCondition::Auto
                | ActionSettleCondition::AccessibilityChange
                | ActionSettleCondition::AnyChange
        );
    let delivery_ack = options.settle_condition == ActionSettleCondition::Auto
        && uses_independent_agent_seat(request, runtime.input_backend_preference);
    let condition = resolve_post_action_condition(
        request,
        options.settle_condition,
        runtime.input_backend_preference,
        target_event_settle,
    );
    let expected_active_window = match request {
        DaemonRequest::FocusWindow(request) => Some(request.window_id.clone()),
        _ => None,
    };
    let before = if condition != ActionSettleCondition::None && !target_event_settle {
        Some(observation::post_action(runtime).await)
    } else {
        None
    };
    Ok(Some(PreparedPostAction {
        options,
        condition,
        delivery_ack,
        expected_active_window,
        before,
    }))
}

fn resolve_post_action_condition(
    request: &DaemonRequest,
    requested: ActionSettleCondition,
    preference: InputBackendPreference,
    target_event_settle: bool,
) -> ActionSettleCondition {
    match requested {
        ActionSettleCondition::Auto if uses_independent_agent_seat(request, preference) => {
            ActionSettleCondition::None
        }
        ActionSettleCondition::Auto if matches!(request, DaemonRequest::FocusWindow(_)) => {
            ActionSettleCondition::ActiveWindowChange
        }
        ActionSettleCondition::Auto if target_event_settle => {
            ActionSettleCondition::AccessibilityChange
        }
        ActionSettleCondition::Auto => ActionSettleCondition::Stable,
        condition => condition,
    }
}

fn validate_post_action_response_options(
    request: &DaemonRequest,
    response_options: Option<&DaemonResponseOptions>,
) -> Result<()> {
    if !daemon_request_returns_action(request) {
        return Ok(());
    }
    let Some(options) = response_options.and_then(|options| options.post_action.as_ref()) else {
        return Ok(());
    };
    if !options.observe_after {
        return Ok(());
    }
    if options.settle_timeout_ms == 0 || options.settle_timeout_ms > 10_000 {
        bail!("post-action settle_timeout_ms must be between 1 and 10000");
    }
    if options.settle_interval_ms < 10 || options.settle_interval_ms > 1_000 {
        bail!("post-action settle_interval_ms must be between 10 and 1000");
    }
    Ok(())
}

fn daemon_request_returns_action(request: &DaemonRequest) -> bool {
    request.returns_action()
}

async fn finish_post_action(
    response: DaemonResponse,
    prepared: Option<PreparedPostAction>,
    runtime: &DaemonRuntime,
) -> DaemonResponse {
    let Some(prepared) = prepared else {
        return response;
    };
    let mut action = match response {
        DaemonResponse::Action(action) => action,
        response => return response,
    };
    if let Some(observation) = action.observation.as_mut() {
        observation.active_window = runtime.window_backend.active_window().await.ok().flatten();
        if let Some(image) = prepared.options.image.as_ref() {
            post_action_capture::attach(image, &mut action, runtime).await;
        }
        return DaemonResponse::Action(action);
    }
    let started = Instant::now();
    let timeout = Duration::from_millis(prepared.options.settle_timeout_ms);
    let interval = Duration::from_millis(prepared.options.settle_interval_ms);
    let mut previous = None;
    let mut current = if prepared.condition == ActionSettleCondition::None || prepared.delivery_ack
    {
        observation::post_action_window_only(runtime).await
    } else {
        observation::post_action(runtime).await
    };
    let mut samples = 1_u32;
    let mut settled = post_action_condition_met(
        prepared.condition,
        prepared.expected_active_window.as_deref(),
        prepared.before.as_ref(),
        previous.as_ref(),
        &current,
    );

    while !settled && started.elapsed() < timeout {
        let remaining = timeout.saturating_sub(started.elapsed());
        tokio::time::sleep(interval.min(remaining)).await;
        previous = Some(current);
        current = observation::post_action(runtime).await;
        samples = samples.saturating_add(1);
        settled = post_action_condition_met(
            prepared.condition,
            prepared.expected_active_window.as_deref(),
            prepared.before.as_ref(),
            previous.as_ref(),
            &current,
        );
    }

    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    current.settle = Some(ActionSettleResult {
        confirmation: if settled {
            libseatgeist::ActionConfirmation::Confirmed
        } else {
            libseatgeist::ActionConfirmation::UnconfirmedTimeout
        },
        condition: prepared.condition,
        backend: if prepared.delivery_ack {
            libseatgeist::ActionSettleBackend::DeliveryAck
        } else {
            libseatgeist::ActionSettleBackend::Polling
        },
        target_scoped: prepared.delivery_ack,
        event: None,
        settled,
        timed_out: !settled,
        timeout_ms: prepared.options.settle_timeout_ms,
        interval_ms: prepared.options.settle_interval_ms,
        samples,
        elapsed_ms,
        before_revision: prepared.before.and_then(|observation| observation.revision),
        after_revision: current.revision.clone().unwrap_or_default(),
    });
    if !settled && let Some(target) = prepared.expected_active_window.as_deref() {
        action.ok = false;
        action.message = Some(format!(
            "focus dispatch accepted, but target window {target} was not confirmed active"
        ));
    }
    action.observation = Some(current);
    if let Some(image) = prepared.options.image.as_ref() {
        post_action_capture::attach(image, &mut action, runtime).await;
    }
    DaemonResponse::Action(action)
}

fn post_action_condition_met(
    condition: ActionSettleCondition,
    expected_active_window: Option<&str>,
    before: Option<&Observation>,
    previous: Option<&Observation>,
    current: &Observation,
) -> bool {
    match condition {
        ActionSettleCondition::None => true,
        ActionSettleCondition::Stable | ActionSettleCondition::Auto => {
            previous.is_some_and(|previous| observation_state_equal(previous, current))
        }
        ActionSettleCondition::ActiveWindowChange => expected_active_window.map_or_else(
            || {
                before.is_some_and(|before| {
                    before.active_window.as_ref().map(|window| &window.id)
                        != current.active_window.as_ref().map(|window| &window.id)
                })
            },
            |target| {
                current
                    .active_window
                    .as_ref()
                    .is_some_and(|window| window.id == target)
            },
        ),
        ActionSettleCondition::AccessibilityChange => before
            .is_some_and(|before| before.focused_accessibility != current.focused_accessibility),
        ActionSettleCondition::AnyChange => {
            before.is_some_and(|before| !observation_state_equal(before, current))
        }
    }
}

fn observation_state_equal(left: &Observation, right: &Observation) -> bool {
    left.active_window == right.active_window
        && left.focused_accessibility == right.focused_accessibility
}

fn client_context_from_identity(identity: DaemonClientIdentity) -> Option<JournalClientContext> {
    let tool = identity.tool.as_deref().and_then(compact_client_tool_name);
    tool.as_ref()?;
    Some(JournalClientContext {
        tool,
        pid: None,
        process_name: None,
    })
}

fn merge_client_context(
    peer_client: Option<JournalClientContext>,
    request_client: Option<JournalClientContext>,
) -> Option<JournalClientContext> {
    let tool = request_client.and_then(|client| client.tool);
    let pid = peer_client.as_ref().and_then(|client| client.pid);
    let process_name = peer_client.and_then(|client| client.process_name);
    if tool.is_none() && pid.is_none() && process_name.is_none() {
        return None;
    }
    Some(JournalClientContext {
        tool,
        pid,
        process_name,
    })
}

fn journal_context_for_request(
    request: &DaemonRequest,
    runtime: &DaemonRuntime,
    client: Option<JournalClientContext>,
) -> JournalContext {
    let safety_class = safety_class_for_request(request);
    let active_window_before = active_window_context_for_safety_class(
        &safety_class,
        &runtime.active_window_state,
        &runtime.app_policy,
    );
    let control = journal_control_context_for_request(
        request,
        &safety_class,
        runtime.input_backend_preference,
    );
    JournalContext {
        client,
        safety_class,
        guard_present: active_window_guard_for_request(request).is_some()
            || interaction_session_id_for_request(request).is_some(),
        active_window_before,
        active_window_after: None,
        control,
    }
}

fn active_window_context_for_safety_class(
    safety_class: &SafetyClass,
    active_window_state: &ActiveWindowState,
    app_policy: &AppPolicy,
) -> Option<JournalWindowContext> {
    if !is_control_safety_class(safety_class) {
        return None;
    }
    observation_policy::observable_journal_window(
        app_policy,
        active_window(active_window_state)
            .ok()
            .flatten()
            .map(journal_window_context),
    )
}

fn journal_control_context_for_request(
    request: &DaemonRequest,
    safety_class: &SafetyClass,
    input_backend_preference: InputBackendPreference,
) -> Option<JournalControlContext> {
    if !is_control_safety_class(safety_class) {
        return None;
    }
    Some(JournalControlContext {
        action_id: None,
        policy: None,
        backend: journal_backend_for_request(request, input_backend_preference),
        requested_target: journal_requested_target_for_request(request),
    })
}

fn finalize_journal_control_context(
    mut control: Option<JournalControlContext>,
    response: &DaemonResponse,
) -> Option<JournalControlContext> {
    let context = control.as_mut()?;
    if let DaemonResponse::Action(action) = response {
        context.action_id = Some(action.id);
    }
    if context.policy.is_none() {
        context.policy = Some(journal_policy_result(response));
    }
    if context.backend.is_none() {
        context.backend = journal_backend_from_response(response);
    }
    control
}

fn journal_policy_result(response: &DaemonResponse) -> String {
    match response {
        DaemonResponse::Error {
            kind: ErrorKind::PolicyDenied,
            ..
        } => "denied".to_string(),
        DaemonResponse::Error {
            kind: ErrorKind::PolicyPromptRequired,
            ..
        } => "prompt_required".to_string(),
        DaemonResponse::Error { .. } => "checked".to_string(),
        _ => "allow".to_string(),
    }
}

fn journal_backend_from_response(response: &DaemonResponse) -> Option<String> {
    let DaemonResponse::Action(action) = response else {
        return None;
    };
    let message = action.message.as_deref()?;
    let backend = message
        .split_whitespace()
        .find_map(|part| part.strip_prefix("backend="))?
        .trim_matches(|character: char| character == ',' || character == ';' || character == '.');
    if backend.is_empty() {
        None
    } else {
        Some(backend.to_string())
    }
}

fn journal_backend_for_request(
    request: &DaemonRequest,
    input_backend_preference: InputBackendPreference,
) -> Option<String> {
    let backend = match request {
        DaemonRequest::RemoteDesktopSessionProbe(_)
        | DaemonRequest::RemoteDesktopEisProbe(_)
        | DaemonRequest::RemoteDesktopEisStart(_) => "portal_remote_desktop",
        DaemonRequest::CaptureOpen(_)
        | DaemonRequest::WindowCaptureOpen(_)
        | DaemonRequest::CaptureSnapshot(_)
        | DaemonRequest::CaptureWait(_)
        | DaemonRequest::CaptureSessionClose(_) => "portal_screencast_pipewire",
        DaemonRequest::CaptureSessionRenew(_) => "interaction_session",
        DaemonRequest::FocusWindow(_) => "kwin",
        DaemonRequest::CloseWindow(_)
        | DaemonRequest::MoveWindow(_)
        | DaemonRequest::LaunchWindow(_)
        | DaemonRequest::ResizeWindow(_) => "kwin_script_bridge",
        DaemonRequest::PageZoom(_) => {
            return Some(input_backend_preference.status_name().to_string());
        }
        DaemonRequest::AccessibilityInvoke(_)
        | DaemonRequest::AccessibilitySetText(_)
        | DaemonRequest::AccessibilityInsertText(_)
        | DaemonRequest::AccessibilityDeleteText(_)
        | DaemonRequest::AccessibilityCopyText(_)
        | DaemonRequest::AccessibilityCutText(_)
        | DaemonRequest::AccessibilityPasteText(_)
        | DaemonRequest::AccessibilitySetCaret(_)
        | DaemonRequest::AccessibilitySetSelection(_)
        | DaemonRequest::ClickButton(_)
        | DaemonRequest::SetTextField(_)
        | DaemonRequest::FocusTextField(_)
        | DaemonRequest::ActivateTab(_)
        | DaemonRequest::ActivateLink(_)
        | DaemonRequest::ToggleCheck(_)
        | DaemonRequest::SetValue(_)
        | DaemonRequest::SelectItem(_)
        | DaemonRequest::SelectMenu(_) => "atspi",
        DaemonRequest::TypeText(_)
        | DaemonRequest::KeyCombo(_)
        | DaemonRequest::MovePointer(_)
        | DaemonRequest::ClickPointer(_)
        | DaemonRequest::DragPointer(_)
        | DaemonRequest::ScrollPointer(_) => {
            return Some(input_backend_preference.status_name().to_string());
        }
        _ => return None,
    };
    Some(backend.to_string())
}

fn journal_requested_target_for_request(request: &DaemonRequest) -> Option<JournalRequestedTarget> {
    let target = match request {
        DaemonRequest::RemoteDesktopSessionProbe(request)
        | DaemonRequest::RemoteDesktopEisProbe(request)
        | DaemonRequest::RemoteDesktopEisStart(request) => {
            let mut target = journal_target("remote_desktop_session");
            target.add_bool("keyboard", request.keyboard);
            target.add_bool("pointer", request.pointer);
            target.add_bool("touchscreen", request.touchscreen);
            if let Some(mode) = &request.persist_mode {
                target.add("persist_mode", format!("{mode:?}"));
            }
            target
        }
        DaemonRequest::WindowCaptureOpen(request) => {
            let mut target = journal_target("window_capture_session");
            if let Some(window_id) = request.requested_window_id.as_deref() {
                target.add("requested_window_id", window_id);
            }
            target.add("timeout_ms", request.timeout_ms.to_string());
            target
        }
        DaemonRequest::CaptureOpen(request) => {
            let mut target = journal_target("capture_session");
            target.add("source_type", format!("{:?}", request.source));
            if let Some(source_id) = request.requested_source_id.as_deref() {
                target.add("requested_source_id", source_id);
            }
            target.add("timeout_ms", request.timeout_ms.to_string());
            target
        }
        DaemonRequest::CaptureSnapshot(request) => {
            let mut target = journal_target("capture_snapshot");
            target.add("session_id", &request.session_id);
            if let Some(max_edge) = request.max_edge {
                target.add("max_edge", max_edge.to_string());
            }
            target.add("timeout_ms", request.timeout_ms.to_string());
            target
        }
        DaemonRequest::CaptureWait(request) => {
            let mut target = journal_target("capture_wait");
            target.add("session_id", &request.session_id);
            target.add_bool("after_revision_present", request.after_revision.is_some());
            if let Some(max_edge) = request.max_edge {
                target.add("max_edge", max_edge.to_string());
            }
            target.add("timeout_ms", request.timeout_ms.to_string());
            target
        }
        DaemonRequest::CaptureSessionClose(request) => {
            let mut target = journal_target("capture_session_close");
            target.add("session_id", &request.session_id);
            target
        }
        DaemonRequest::CaptureSessionRenew(request) => {
            let mut target = journal_target("capture_session_renew");
            target.add("session_id", &request.session_id);
            target
        }
        DaemonRequest::FocusWindow(request) => {
            let mut target = journal_target("window");
            target.add("window_id", &request.window_id);
            target
        }
        DaemonRequest::CloseWindow(request) => {
            let mut target = journal_target("window_close");
            target.add("window_id", &request.window_id);
            if let Some(session_id) = request.session_id.as_deref() {
                target.add("session_id", session_id);
            }
            target
        }
        DaemonRequest::MoveWindow(request) => {
            let mut target = journal_target("window_move");
            target.add("window_id", &request.window_id);
            target.add("x", request.x.to_string());
            target.add("y", request.y.to_string());
            target
        }
        DaemonRequest::LaunchWindow(request) => {
            let mut target = journal_target("window_launch");
            target.add("desktop_entry", &request.desktop_entry);
            target.add("anchor", format!("{:?}", request.anchor));
            target.add("activation", format!("{:?}", request.activation));
            if let Some(monitor_id) = request.monitor_id.as_deref() {
                target.add("monitor_id", monitor_id);
            }
            if let Some(width) = request.width {
                target.add("width", width.to_string());
            }
            if let Some(height) = request.height {
                target.add("height", height.to_string());
            }
            target.add("margin", request.margin.to_string());
            target.add("timeout_ms", request.timeout_ms.to_string());
            target
        }
        DaemonRequest::ResizeWindow(request) => {
            let mut target = journal_target("window_resize");
            target.add("window_id", &request.window_id);
            target.add("width", request.width.to_string());
            target.add("height", request.height.to_string());
            target
        }
        DaemonRequest::PageZoom(request) => {
            let mut target = journal_target("page_zoom");
            target.add("operation", format!("{:?}", request.operation));
            target.add("steps", request.steps.to_string());
            target
        }
        DaemonRequest::AccessibilityInvoke(request) => {
            let mut target = journal_target("accessibility_action");
            target.add("node_id", &request.node_id);
            target.add("action", format!("{:?}", request.action));
            target.add_bool("destructive", request.destructive);
            target
        }
        DaemonRequest::AccessibilitySetText(request) => {
            let mut target = journal_target("accessibility_text_replace");
            target.add("node_id", &request.node_id);
            target.add("text_chars", request.text.chars().count().to_string());
            target
        }
        DaemonRequest::AccessibilityInsertText(request) => {
            let mut target = journal_target("accessibility_text_insert");
            target.add("node_id", &request.node_id);
            target.add("offset", request.offset.to_string());
            target.add("text_chars", request.text.chars().count().to_string());
            target
        }
        DaemonRequest::AccessibilityDeleteText(request) => journal_text_range_target(
            "accessibility_text_delete",
            &request.node_id,
            request.start_offset,
            request.end_offset,
        ),
        DaemonRequest::AccessibilityCopyText(request) => journal_text_range_target(
            "accessibility_text_copy",
            &request.node_id,
            request.start_offset,
            request.end_offset,
        ),
        DaemonRequest::AccessibilityCutText(request) => journal_text_range_target(
            "accessibility_text_cut",
            &request.node_id,
            request.start_offset,
            request.end_offset,
        ),
        DaemonRequest::AccessibilityPasteText(request) => {
            let mut target = journal_target("accessibility_text_paste");
            target.add("node_id", &request.node_id);
            target.add("offset", request.offset.to_string());
            target
        }
        DaemonRequest::AccessibilitySetCaret(request) => {
            let mut target = journal_target("accessibility_caret");
            target.add("node_id", &request.node_id);
            target.add("offset", request.offset.to_string());
            target
        }
        DaemonRequest::AccessibilitySetSelection(request) => {
            let mut target = journal_target("accessibility_selection");
            target.add("node_id", &request.node_id);
            target.add("selection_num", request.selection_num.to_string());
            target.add("start_offset", request.start_offset.to_string());
            target.add("end_offset", request.end_offset.to_string());
            target
        }
        DaemonRequest::TypeText(request) => {
            let mut target = journal_target("keyboard_text");
            target.add("text_chars", request.text.chars().count().to_string());
            if let Some(session_id) = request.session_id.as_deref() {
                target.add("session_id", session_id);
            }
            target
        }
        DaemonRequest::KeyCombo(request) => {
            let mut target = journal_target("keyboard_combo");
            target.add(
                "key_count",
                request
                    .combo
                    .split('+')
                    .filter(|part| !part.trim().is_empty())
                    .count()
                    .to_string(),
            );
            target.add_bool(
                "destructive",
                request.destructive || destructive_key_combo(&request.combo),
            );
            if let Some(session_id) = request.session_id.as_deref() {
                target.add("session_id", session_id);
            }
            target
        }
        DaemonRequest::MovePointer(request) => {
            let mut target = journal_point_target("pointer_move", &request.point);
            if let Some(session_id) = request.session_id.as_deref() {
                target.add("session_id", session_id);
            }
            target
        }
        DaemonRequest::ClickPointer(request) => {
            let mut target = journal_point_target("pointer_click", &request.point);
            target.add("button", format!("{:?}", request.button));
            target.add("clicks", request.clicks.to_string());
            if let Some(session_id) = request.session_id.as_deref() {
                target.add("session_id", session_id);
            }
            target
        }
        DaemonRequest::DragPointer(request) => {
            let mut target = journal_target("pointer_drag");
            target.add_point("from", &request.from);
            target.add_point("to", &request.to);
            target.add("button", format!("{:?}", request.button));
            target.add("duration_ms", request.duration_ms.to_string());
            if let Some(session_id) = request.session_id.as_deref() {
                target.add("session_id", session_id);
            }
            target
        }
        DaemonRequest::ScrollPointer(request) => {
            let mut target = journal_target("pointer_scroll");
            target.add("vertical", request.vertical.to_string());
            target.add("horizontal", request.horizontal.to_string());
            if let Some(session_id) = request.session_id.as_deref() {
                target.add("session_id", session_id);
            }
            target
        }
        DaemonRequest::ClickButton(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_button",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            target.add_bool("destructive", request.destructive);
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::SetTextField(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_text_field_set",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            target.add("text_chars", request.text.chars().count().to_string());
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::FocusTextField(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_text_field_focus",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::ActivateTab(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_tab",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::ActivateLink(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_link",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::ToggleCheck(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_check",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            if let Some(checked) = request.checked {
                target.add_bool("checked", checked);
            }
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::SetValue(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_value",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            target.add("value", request.value.to_string());
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::SelectItem(request) => {
            let mut target = journal_named_semantic_target(
                "semantic_item",
                &request.name,
                request.app.as_deref(),
                request.window_name_contains.as_deref(),
                request.max_nodes,
            );
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        DaemonRequest::SelectMenu(request) => {
            let mut target = journal_target("semantic_menu");
            target.add("path_len", request.path.len().to_string());
            target.add_bool("destructive", request.destructive);
            target.add("max_nodes", request.max_nodes.to_string());
            if let Some(app) = request.app.as_deref() {
                target.add("app", app);
            }
            if let Some(window) = request.window_name_contains.as_deref() {
                target.add(
                    "window_name_contains_chars",
                    window.chars().count().to_string(),
                );
            }
            add_target_guard_to_journal(&mut target, request.target_guard.as_ref());
            target
        }
        _ => return None,
    };
    if target.fields.is_empty() && target.kind.is_empty() {
        None
    } else {
        Some(target)
    }
}

fn journal_text_range_target(
    kind: &str,
    node_id: &str,
    start_offset: i32,
    end_offset: i32,
) -> JournalRequestedTarget {
    let mut target = journal_target(kind);
    target.add("node_id", node_id);
    target.add("start_offset", start_offset.to_string());
    target.add("end_offset", end_offset.to_string());
    target
}

fn journal_point_target(kind: &str, point: &Point) -> JournalRequestedTarget {
    let mut target = journal_target(kind);
    target.add_point("", point);
    target
}

fn journal_named_semantic_target(
    kind: &str,
    name: &str,
    app: Option<&str>,
    window_name_contains: Option<&str>,
    max_nodes: usize,
) -> JournalRequestedTarget {
    let mut target = journal_target(kind);
    target.add("name_chars", name.chars().count().to_string());
    target.add("max_nodes", max_nodes.to_string());
    if let Some(app) = app {
        target.add("app", app);
    }
    if let Some(window) = window_name_contains {
        target.add(
            "window_name_contains_chars",
            window.chars().count().to_string(),
        );
    }
    target
}

fn add_target_guard_to_journal(
    target: &mut JournalRequestedTarget,
    guard: Option<&libseatgeist::TargetWindowGuard>,
) {
    let Some(guard) = guard else {
        return;
    };
    target.add("target_window_id", &guard.expected_window_id);
    if let Some(app_id) = guard.expected_app_id.as_deref() {
        target.add("target_app_id", app_id);
    }
    if let Some(pid) = guard.expected_pid {
        target.add("target_pid", pid.to_string());
    }
    target.add_bool("target_title_guard_present", guard.title_contains.is_some());
}

fn journal_target(kind: impl Into<String>) -> JournalRequestedTarget {
    JournalRequestedTarget {
        kind: kind.into(),
        fields: BTreeMap::new(),
    }
}

trait JournalRequestedTargetExt {
    fn add(&mut self, key: impl Into<String>, value: impl ToString);

    fn add_bool(&mut self, key: impl Into<String>, value: bool);

    fn add_point(&mut self, prefix: &str, point: &Point);
}

impl JournalRequestedTargetExt for JournalRequestedTarget {
    fn add(&mut self, key: impl Into<String>, value: impl ToString) {
        self.fields.insert(key.into(), value.to_string());
    }

    fn add_bool(&mut self, key: impl Into<String>, value: bool) {
        self.add(key, value.to_string());
    }

    fn add_point(&mut self, prefix: &str, point: &Point) {
        let separator = if prefix.is_empty() { "" } else { "_" };
        self.add(format!("{prefix}{separator}x"), format!("{:.0}", point.x));
        self.add(format!("{prefix}{separator}y"), format!("{:.0}", point.y));
        self.add(
            format!("{prefix}{separator}space"),
            format!("{:?}", point.space),
        );
    }
}

fn journal_window_context(window: WindowInfo) -> JournalWindowContext {
    JournalWindowContext {
        id: window.id,
        app_id: window.app_id,
        title: compact_journal_title(window.title),
        monitor_id: window.monitor_id,
    }
}

fn compact_journal_title(mut title: String) -> String {
    const MAX_TITLE_CHARS: usize = 160;
    if title.chars().count() <= MAX_TITLE_CHARS {
        return title;
    }
    let mut end = 0;
    for (count, (index, character)) in title.char_indices().enumerate() {
        if count == MAX_TITLE_CHARS {
            break;
        }
        end = index + character.len_utf8();
    }
    title.truncate(end);
    title.push_str("...");
    title
}

fn journal_artifacts_for_response(
    response: &DaemonResponse,
    settings: &JournalSettings,
) -> Vec<JournalArtifactContext> {
    if !settings.include_artifact_metadata {
        return Vec::new();
    }

    match response {
        DaemonResponse::Screenshot(info) => {
            vec![journal_artifact_for_screenshot("screenshot", info)]
        }
        DaemonResponse::Observation(observation) => observation
            .screenshot
            .as_ref()
            .map(|info| journal_artifact_for_screenshot("observe_screenshot", info))
            .into_iter()
            .collect(),
        DaemonResponse::WaitForChange(result) => vec![journal_artifact_for_screenshot(
            "wait_for_change_screenshot",
            &result.screenshot,
        )],
        DaemonResponse::CaptureFrame(frame) => vec![journal_artifact_for_screenshot(
            "capture_session_snapshot",
            &frame.screenshot,
        )],
        DaemonResponse::CaptureWait(result) => vec![journal_artifact_for_screenshot(
            "capture_session_wait",
            &result.frame.screenshot,
        )],
        DaemonResponse::Action(action) => action
            .screenshot
            .as_ref()
            .map(|info| journal_artifact_for_screenshot("post_action_screenshot", info))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn journal_artifact_for_screenshot(kind: &str, info: &ScreenshotInfo) -> JournalArtifactContext {
    let (bytes, sha256) = journal_artifact_file_metadata(&info.path);
    JournalArtifactContext {
        kind: kind.to_string(),
        path: info.path.clone(),
        sha256,
        bytes,
    }
}

fn journal_artifact_file_metadata(path: &Path) -> (Option<u64>, Option<String>) {
    let bytes = fs::metadata(path).ok().map(|metadata| metadata.len());
    let sha256 = sha256_file(path).ok();
    (bytes, sha256)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn handle_request(
    mut request: DaemonRequest,
    response_options: Option<&DaemonResponseOptions>,
    client: Option<&JournalClientContext>,
    runtime: &DaemonRuntime,
) -> DaemonResponse {
    if let Err(err) = validate_post_action_response_options(&request, response_options) {
        return daemon_error_with_kind(err, ErrorKind::Validation);
    }
    if let Err(err) = validate_targeted_key_combo(&request, runtime.input_backend_preference) {
        return daemon_error_with_kind(err, ErrorKind::Validation);
    }
    if let Err(err) =
        enforce_policy_with_approvals(&runtime.policy, &runtime.approval_store, &request)
    {
        let kind = policy_error_kind(&err);
        return daemon_error_with_kind(err, kind);
    }
    if let Err(err) = enforce_mcp_focus_isolation(&request, client) {
        return daemon_error_with_kind(err, ErrorKind::Validation);
    }
    if let Err(err) = enforce_panic_stop(&runtime.panic_stop, &request) {
        return daemon_error_with_kind(err, ErrorKind::PanicStop);
    }
    if !uses_independent_agent_seat(&request, runtime.input_backend_preference)
        && let Err(err) = enforce_human_input_pause(
            &runtime.safety_settings,
            &runtime.activity_tracker,
            &request,
        )
    {
        return daemon_error_with_kind(err, ErrorKind::HumanInputPause);
    }
    if let Err(err) = enforce_capture_session_owner(
        &request,
        response_options,
        client,
        &runtime.capture_session_store,
    )
    .await
    {
        return daemon_error_with_kind(err, ErrorKind::SessionOwnerMismatch);
    }
    if let Err(err) = validate_interaction_session_request(&request) {
        return daemon_error_with_kind(err, ErrorKind::Validation);
    }
    if let Err(err) = validate_capture_output_request(&request) {
        return daemon_error_with_kind(err, ErrorKind::Validation);
    }
    if let Err(err) =
        resolve_semantic_handle_for_request(&mut request, &runtime.semantic_handle_store, client)
    {
        return daemon_error_with_kind(err, ErrorKind::TargetMismatch);
    }
    if let Err(err) = enforce_required_focus_guard(&runtime.safety_settings, &request) {
        return daemon_error_with_kind(err, ErrorKind::FocusGuard);
    }
    if let Err(err) = enforce_active_window_guard(runtime.window_backend.as_ref(), &request).await {
        return daemon_error_with_kind(err, ErrorKind::FocusGuard);
    }
    if let Err(err) = enforce_app_policy(
        runtime.window_backend.as_ref(),
        &runtime.app_policy,
        &request,
    )
    .await
    {
        return daemon_error_with_kind(err, ErrorKind::AppDenied);
    }
    if let DaemonRequest::AccessibilityTextAttributes(request) = &request
        && let Err(err) =
            seatgeist_atspi::validate_text_attributes_request(&request.node_id, request.offset)
    {
        return daemon_error_with_kind(anyhow::anyhow!(err), ErrorKind::Validation);
    }
    if let Err(err) = observation_policy::enforce_observation_app_policy(
        &request,
        runtime.window_backend.as_ref(),
        &runtime.app_policy,
    )
    .await
    {
        return daemon_error_with_kind(err, ErrorKind::AppDenied);
    }
    if let Err(err) = enforce_control_rate_limit(&runtime.control_rate_limiter, &request) {
        return daemon_error_with_kind(err, ErrorKind::RateLimited);
    }

    let prepared_post_action = match prepare_post_action(&request, response_options, runtime).await
    {
        Ok(prepared) => prepared,
        Err(err) => return daemon_error_with_kind(err, ErrorKind::Validation),
    };
    let session_ids = capture_session_ids_for_request(&request, response_options)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let method = request.method_name().to_string();
    let safety_class = safety_class_for_request(&request);
    let backend = journal_backend_for_request(&request, runtime.input_backend_preference);
    let backend_role = session_backend_role_for_request(&request);
    let response = execute_request(request, runtime, response_options, client).await;
    let mut response = finish_post_action(response, prepared_post_action, runtime).await;
    if let Some(backend_role) = backend_role {
        record_session_execution_response(
            &session_ids,
            &method,
            safety_class,
            backend.as_deref(),
            backend_role,
            &response,
            &runtime.session_execution_store,
        )
        .await;
    }
    attach_session_execution_status(&mut response, runtime).await;
    response
}

async fn execute_request(
    request: DaemonRequest,
    runtime: &DaemonRuntime,
    response_options: Option<&DaemonResponseOptions>,
    client: Option<&JournalClientContext>,
) -> DaemonResponse {
    let post_action = response_options.and_then(|options| options.post_action.as_ref());
    match request {
        DaemonRequest::Health => {
            let mut status = runtime.health_status.clone();
            let (current, peak) = process_resident_memory();
            status.resident_memory_bytes = current;
            status.resident_memory_peak_bytes = peak;
            DaemonResponse::Health(status)
        }
        DaemonRequest::Capabilities => DaemonResponse::Capabilities(capabilities(
            runtime.input_backend_preference,
            &runtime.portal_eis_session_store,
            runtime.agent_seat_backend.ready(),
            runtime.window_action_queue.resize_ready(),
            runtime.window_action_queue.move_ready(),
            runtime.window_action_queue.launch_ready(),
            runtime.window_action_queue.close_ready(),
        )),
        DaemonRequest::PolicyStatus => {
            DaemonResponse::PolicyStatus(policy_status_from_config(runtime.policy.config()))
        }
        DaemonRequest::SafetyStatus => {
            match safety_status(
                &runtime.safety_settings,
                &runtime.journal.settings,
                &runtime.activity_tracker,
            ) {
                Ok(status) => DaemonResponse::SafetyStatus(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::DesktopSessionStatus => {
            DaemonResponse::DesktopSessionStatus(desktop_session_status_from_env(std::env::vars()))
        }
        DaemonRequest::ComputerUseReadiness => {
            DaemonResponse::ComputerUseReadiness(computer_use_readiness_status(runtime).await)
        }
        DaemonRequest::PanicStopStatus => DaemonResponse::PanicStop(runtime.panic_stop.status()),
        DaemonRequest::SetPanicStop(request) => {
            match set_panic_stop(&runtime.panic_stop, request) {
                Ok(status) => DaemonResponse::PanicStop(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::KwinBridgeStatus => {
            match kwin_bridge_status(
                &runtime.active_window_state,
                &runtime.window_list_state,
                &runtime.window_action_queue,
            ) {
                Ok(mut status) => {
                    status.active_window = observation_policy::observable_window(
                        &runtime.app_policy,
                        status.active_window,
                    );
                    if let Ok(Some(windows)) = runtime.window_list_state.snapshot() {
                        status.window_count =
                            observation_policy::observable_windows(&runtime.app_policy, windows)
                                .len();
                    }
                    DaemonResponse::KwinBridgeStatus(status)
                }
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::UinputStatus => match uinput_status() {
            Ok(status) => DaemonResponse::UinputStatus(status),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::InputBackendStatus => {
            match input_backend_status(
                runtime.input_backend_preference,
                &runtime.portal_eis_session_store,
                runtime.agent_seat_backend.ready(),
                &runtime.xkb_keymap_config,
            ) {
                Ok(status) => DaemonResponse::InputBackendStatus(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::RemoteDesktopSessionProbe(request) => {
            match remote_desktop_session_probe(request).await {
                Ok(status) => DaemonResponse::RemoteDesktopSessionProbe(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::RemoteDesktopEisProbe(request) => {
            match remote_desktop_eis_probe(request).await {
                Ok(status) => DaemonResponse::RemoteDesktopEisProbe(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::RemoteDesktopEisStart(request) => {
            match remote_desktop_eis_start(request, &runtime.portal_eis_session_store).await {
                Ok(status) => DaemonResponse::RemoteDesktopEisSessionStatus(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::RemoteDesktopEisSessionStatus => {
            match runtime.portal_eis_session_store.status() {
                Ok(status) => DaemonResponse::RemoteDesktopEisSessionStatus(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::RemoteDesktopEisStop => {
            match remote_desktop_eis_stop(&runtime.portal_eis_session_store) {
                Ok(status) => DaemonResponse::RemoteDesktopEisSessionStatus(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::CaptureBackendStatus => {
            DaemonResponse::CaptureBackendStatus(capture_backend_status())
        }
        DaemonRequest::CaptureOpen(request) => execute_capture_open(request, client, runtime).await,
        DaemonRequest::WindowCaptureOpen(request) => {
            execute_capture_open(
                CaptureOpenRequest {
                    source: CaptureSourceKind::Window,
                    requested_source_id: request.requested_window_id,
                    parent_window: request.parent_window,
                    timeout_ms: request.timeout_ms,
                },
                client,
                runtime,
            )
            .await
        }
        DaemonRequest::CaptureSessionStatus => {
            DaemonResponse::CaptureSessionStatus(capture_status(runtime, client).await)
        }
        DaemonRequest::CaptureSessionRenew(request) => {
            if let Err(err) = runtime
                .capture_session_store
                .require_active(&request.session_id)
                .await
            {
                let _ = runtime
                    .interaction_session_store
                    .clear(&request.session_id)
                    .await;
                return daemon_error_with_kind(err, ErrorKind::TargetLost);
            }
            let windows = match runtime.window_backend.list_windows().await {
                Ok(windows) => windows,
                Err(err) => return daemon_error(anyhow::Error::msg(err)),
            };
            let target = match runtime
                .interaction_session_store
                .resolve(&request.session_id, &windows)
                .await
            {
                Ok(target) => target,
                Err(err) => return daemon_error_with_kind(err, ErrorKind::TargetLost),
            };
            if let Err(err) = enforce_app_policy_for_app(
                &runtime.app_policy,
                target.window.app_id.as_deref(),
                "pinned interaction target",
            ) {
                return daemon_error(err);
            }
            if let Err(err) = runtime
                .interaction_session_store
                .renew(&request.session_id)
                .await
            {
                return daemon_error_with_kind(err, ErrorKind::TargetLost);
            }
            let capture = runtime
                .capture_session_store
                .status_for_session(&request.session_id)
                .await;
            let interaction = runtime
                .interaction_session_store
                .status(&request.session_id)
                .await;
            DaemonResponse::CaptureSessionStatus(merge_interaction_status(capture, interaction))
        }
        DaemonRequest::CaptureSnapshot(mut request) => {
            match normalize_capture_frame_request(
                &mut request.max_edge,
                request.timeout_ms,
                runtime.safety_settings.preview_max_edge,
            ) {
                Ok(()) => {}
                Err(err) => return daemon_error_with_kind(err, ErrorKind::Validation),
            }
            let session_id = request.session_id.clone();
            if let Err(err) = preflight_retained_capture_observation(&session_id, runtime).await {
                return daemon_error_with_kind(err, ErrorKind::AppDenied);
            }
            match runtime.capture_session_store.snapshot(request).await {
                Ok(mut frame) => {
                    if let Err(err) =
                        protect_retained_capture_frame(&session_id, &mut frame, runtime).await
                    {
                        fs::remove_file(&frame.screenshot.path).ok();
                        return daemon_error_with_kind(err, ErrorKind::AppDenied);
                    }
                    if let Err(err) = runtime
                        .capture_session_store
                        .update_latest_frame(&frame)
                        .await
                    {
                        return daemon_error_with_kind(err, ErrorKind::TargetLost);
                    }
                    DaemonResponse::CaptureFrame(frame)
                }
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::CaptureWait(mut request) => {
            match normalize_capture_frame_request(
                &mut request.max_edge,
                request.timeout_ms,
                runtime.safety_settings.preview_max_edge,
            ) {
                Ok(()) => {}
                Err(err) => return daemon_error_with_kind(err, ErrorKind::Validation),
            }
            let session_id = request.session_id.clone();
            if let Err(err) = preflight_retained_capture_observation(&session_id, runtime).await {
                return daemon_error_with_kind(err, ErrorKind::AppDenied);
            }
            match runtime.capture_session_store.wait(request).await {
                Ok(mut result) => {
                    if let Err(err) =
                        protect_retained_capture_frame(&session_id, &mut result.frame, runtime)
                            .await
                    {
                        fs::remove_file(&result.frame.screenshot.path).ok();
                        return daemon_error_with_kind(err, ErrorKind::AppDenied);
                    }
                    if let Err(err) = runtime
                        .capture_session_store
                        .update_latest_frame(&result.frame)
                        .await
                    {
                        return daemon_error_with_kind(err, ErrorKind::TargetLost);
                    }
                    DaemonResponse::CaptureWait(Box::new(result))
                }
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::CaptureSessionClose(request) => {
            let session_id = request.session_id.clone();
            let _seat_lease = match runtime.interaction_session_store.acquire_seat_lease().await {
                Ok(lease) => lease,
                Err(err) => {
                    return daemon_error_with_kind(err, ErrorKind::FocusLeaseConflict);
                }
            };
            match runtime.capture_session_store.close(request).await {
                Ok(status) => {
                    runtime
                        .interaction_session_store
                        .clear_if_present(&session_id)
                        .await;
                    if let Err(err) = runtime.session_execution_store.clear(&session_id).await {
                        return daemon_error(err);
                    }
                    DaemonResponse::CaptureSessionStatus(merge_interaction_status(
                        status,
                        runtime.interaction_session_store.status(&session_id).await,
                    ))
                }
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::PointerCalibration => {
            match pointer_coordinates::calibration(runtime.screen_backend.as_ref()).await {
                Ok(status) => DaemonResponse::PointerCalibration(status),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ListMonitors => match list_monitors() {
            Ok(monitors) => DaemonResponse::Monitors(monitors),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::ListWindows => match runtime.window_backend.list_windows().await {
            Ok(windows) => DaemonResponse::Windows(observation_policy::observable_windows(
                &runtime.app_policy,
                windows,
            )),
            Err(err) => daemon_error(anyhow::Error::msg(err)),
        },
        DaemonRequest::ActiveWindow => match runtime.window_backend.active_window().await {
            Ok(window) => DaemonResponse::ActiveWindow(observation_policy::observable_window(
                &runtime.app_policy,
                window,
            )),
            Err(err) => daemon_error(anyhow::Error::msg(err)),
        },
        DaemonRequest::WindowInventory => {
            match observation::window_inventory(
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
            )
            .await
            {
                Ok(mut inventory) => match runtime
                    .semantic_handle_store
                    .issue_for_windows(&inventory.windows, client)
                {
                    Ok(handles) => {
                        inventory.semantic_handles = handles;
                        DaemonResponse::WindowInventory(inventory)
                    }
                    Err(err) => daemon_error(err),
                },
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::WindowInventoryWait(request) => {
            match observation::wait_for_window_inventory(
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                request,
            )
            .await
            {
                Ok(mut result) => match runtime
                    .semantic_handle_store
                    .issue_for_windows(&result.inventory.windows, client)
                {
                    Ok(handles) => {
                        result.inventory.semantic_handles = handles;
                        DaemonResponse::WindowInventoryWait(result)
                    }
                    Err(err) => daemon_error(err),
                },
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::Observe(request) => {
            match observation::desktop(
                request,
                runtime.window_backend.as_ref(),
                runtime.screen_backend.as_ref(),
                &runtime.window_list_state,
                &runtime.safety_settings,
                &runtime.app_policy,
            )
            .await
            {
                Ok(observation) => DaemonResponse::Observation(Box::new(observation)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::Screenshot(request) => {
            match capture_screenshot(
                request,
                &runtime.safety_settings,
                &runtime.window_list_state,
                &runtime.app_policy,
            )
            .await
            {
                Ok(info) => DaemonResponse::Screenshot(info),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ScreenshotTile(request) => {
            match capture_screenshot_tile(
                request,
                &runtime.safety_settings,
                &runtime.window_list_state,
                &runtime.app_policy,
            )
            .await
            {
                Ok(info) => DaemonResponse::Screenshot(info),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::WaitForChange(request) => {
            match wait_for_change(
                request,
                &runtime.safety_settings,
                &runtime.window_list_state,
                &runtime.app_policy,
            )
            .await
            {
                Ok(result) => DaemonResponse::WaitForChange(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ClipboardBackendStatus => {
            DaemonResponse::ClipboardBackendStatus(clipboard::status())
        }
        DaemonRequest::ClipboardGet(request) => match clipboard::get_text(request).await {
            Ok(text) => DaemonResponse::ClipboardText(text),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::ClipboardSet(request) => match clipboard::set_text(&request.text).await {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilityQualityStatus => {
            DaemonResponse::AccessibilityQualityStatus(accessibility_quality_status().await)
        }
        DaemonRequest::FocusedAccessibilityTree(request) => {
            match focused_accessibility_tree_bounded(request, ACCESSIBILITY_TREE_TIMEOUT).await {
                Ok(tree) => DaemonResponse::AccessibilityTree(tree),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::AccessibilityFind(request) => match accessibility_find(request) {
            Ok(matches) => DaemonResponse::AccessibilityMatches(matches),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilityTextAttributes(request) => {
            match accessibility_text_attributes(request) {
                Ok(attributes) => DaemonResponse::AccessibilityTextAttributes(attributes),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::AccessibilityInvoke(request) => match accessibility_invoke(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilitySetText(request) => match accessibility_set_text(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilityInsertText(request) => {
            match accessibility_insert_text(request) {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::AccessibilityDeleteText(request) => {
            match accessibility_delete_text(request) {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::AccessibilityCopyText(request) => match accessibility_copy_text(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilityCutText(request) => match accessibility_cut_text(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilityPasteText(request) => match accessibility_paste_text(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilitySetCaret(request) => match accessibility_set_caret(request) {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::AccessibilitySetSelection(request) => {
            match accessibility_set_selection(request) {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::TypeText(request) => {
            let session_id = request.session_id.clone();
            let result =
                if runtime.input_backend_preference == InputBackendPreference::KwinAgentSeat {
                    interaction::execute_agent_seat_action(
                        runtime,
                        session_id.as_deref(),
                        SafetyClass::ControlKeyboard,
                        |target| async move {
                            let context = TargetedInputContext {
                                lane_id: target.lane_id,
                            };
                            agent_type_text(
                                request,
                                &context,
                                &target.window,
                                &runtime.agent_seat_backend,
                            )
                            .await
                        },
                    )
                    .await
                } else {
                    interaction::execute_raw_action(runtime, session_id.as_deref(), || async move {
                        type_text(
                            request,
                            runtime.input_backend_preference,
                            &runtime.portal_eis_session_store,
                        )
                    })
                    .await
                };
            match result {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::KeyCombo(request) => {
            let session_id = request.session_id.clone();
            let result =
                if runtime.input_backend_preference == InputBackendPreference::KwinAgentSeat {
                    interaction::execute_agent_seat_action(
                        runtime,
                        session_id.as_deref(),
                        SafetyClass::ControlKeyboard,
                        |target| async move {
                            let context = TargetedInputContext {
                                lane_id: target.lane_id,
                            };
                            agent_key_combo(
                                request,
                                &context,
                                &target.window,
                                &runtime.agent_seat_backend,
                            )
                            .await
                        },
                    )
                    .await
                } else {
                    interaction::execute_raw_action(runtime, session_id.as_deref(), || async move {
                        key_combo(
                            request,
                            runtime.input_backend_preference,
                            &runtime.xkb_keymap_config,
                            &runtime.portal_eis_session_store,
                        )
                    })
                    .await
                };
            match result {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::MovePointer(request) => {
            let request = match resolve_move_capture_coordinates(request, runtime).await {
                Ok(request) => request,
                Err(err) => return daemon_error_with_kind(err, ErrorKind::Validation),
            };
            let session_id = request.session_id.clone();
            let result =
                if runtime.input_backend_preference == InputBackendPreference::KwinAgentSeat {
                    interaction::execute_agent_seat_action(
                        runtime,
                        session_id.as_deref(),
                        SafetyClass::ControlPointer,
                        |target| async move {
                            let context = TargetedInputContext {
                                lane_id: target.lane_id,
                            };
                            agent_move_pointer(
                                request,
                                &context,
                                &target.window,
                                &runtime.agent_seat_backend,
                            )
                            .await
                        },
                    )
                    .await
                } else {
                    interaction::execute_raw_action(runtime, session_id.as_deref(), || async move {
                        move_pointer(
                            request,
                            runtime.window_backend.as_ref(),
                            runtime.screen_backend.as_ref(),
                            runtime.input_backend_preference,
                            &runtime.portal_eis_session_store,
                        )
                        .await
                    })
                    .await
                };
            match result {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ClickPointer(request) => {
            let request = match resolve_click_capture_coordinates(request, runtime).await {
                Ok(request) => request,
                Err(err) => return daemon_error_with_kind(err, ErrorKind::Validation),
            };
            let session_id = request.session_id.clone();
            let result =
                if runtime.input_backend_preference == InputBackendPreference::KwinAgentSeat {
                    interaction::execute_agent_seat_action(
                        runtime,
                        session_id.as_deref(),
                        SafetyClass::ControlPointer,
                        |target| async move {
                            let context = TargetedInputContext {
                                lane_id: target.lane_id,
                            };
                            agent_click_pointer(
                                request,
                                &context,
                                &target.window,
                                &runtime.agent_seat_backend,
                            )
                            .await
                        },
                    )
                    .await
                } else {
                    interaction::execute_raw_action(runtime, session_id.as_deref(), || async move {
                        click_pointer(
                            request,
                            runtime.window_backend.as_ref(),
                            runtime.screen_backend.as_ref(),
                            runtime.input_backend_preference,
                            &runtime.portal_eis_session_store,
                        )
                        .await
                    })
                    .await
                };
            match result {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::DragPointer(request) => {
            let request = match resolve_drag_capture_coordinates(request, runtime).await {
                Ok(request) => request,
                Err(err) => return daemon_error_with_kind(err, ErrorKind::Validation),
            };
            let session_id = request.session_id.clone();
            let result =
                if runtime.input_backend_preference == InputBackendPreference::KwinAgentSeat {
                    interaction::execute_agent_seat_action(
                        runtime,
                        session_id.as_deref(),
                        SafetyClass::ControlPointer,
                        |target| async move {
                            let context = TargetedInputContext {
                                lane_id: target.lane_id,
                            };
                            agent_drag_pointer(
                                request,
                                &context,
                                &target.window,
                                &runtime.agent_seat_backend,
                            )
                            .await
                        },
                    )
                    .await
                } else {
                    interaction::execute_raw_action(runtime, session_id.as_deref(), || async move {
                        drag_pointer(
                            request,
                            runtime.window_backend.as_ref(),
                            runtime.screen_backend.as_ref(),
                            runtime.input_backend_preference,
                            &runtime.portal_eis_session_store,
                        )
                        .await
                    })
                    .await
                };
            match result {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ScrollPointer(request) => {
            let session_id = request.session_id.clone();
            let result =
                if runtime.input_backend_preference == InputBackendPreference::KwinAgentSeat {
                    interaction::execute_agent_seat_action(
                        runtime,
                        session_id.as_deref(),
                        SafetyClass::ControlPointer,
                        |target| async move {
                            let context = TargetedInputContext {
                                lane_id: target.lane_id,
                            };
                            agent_scroll_pointer(
                                request,
                                &context,
                                &target.window,
                                &runtime.agent_seat_backend,
                            )
                            .await
                        },
                    )
                    .await
                } else {
                    interaction::execute_raw_action(runtime, session_id.as_deref(), || async move {
                        scroll_pointer(
                            request,
                            runtime.screen_backend.as_ref(),
                            runtime.input_backend_preference,
                            &runtime.portal_eis_session_store,
                        )
                        .await
                    })
                    .await
                };
            match result {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ClickButton(request) => {
            match click_button(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::SetTextField(request) => {
            match set_text_field(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::FocusTextField(request) => {
            match focus_text_field(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ActivateTab(request) => {
            match activate_tab(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ActivateLink(request) => {
            match activate_link(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::ToggleCheck(request) => {
            match toggle_check(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::SetValue(request) => {
            match set_value(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::SelectItem(request) => {
            match select_item(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::SelectMenu(request) => {
            match select_menu(
                request,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
                post_action,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::JournalTail(request) => {
            match runtime.journal.tail_filtered(
                request.limit,
                request.method_filter.as_deref(),
                request.ok,
            ) {
                Ok(mut entries) => {
                    for entry in &mut entries {
                        entry.active_window_before = observation_policy::observable_journal_window(
                            &runtime.app_policy,
                            entry.active_window_before.take(),
                        );
                        entry.active_window_after = observation_policy::observable_journal_window(
                            &runtime.app_policy,
                            entry.active_window_after.take(),
                        );
                    }
                    DaemonResponse::Journal(entries)
                }
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::FocusWindow(request) => {
            match focus_window(request, runtime.window_backend.as_ref()).await {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::CloseWindow(request) => match close_window(request, runtime).await {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error_with_kind(err, ErrorKind::TargetMismatch),
        },
        DaemonRequest::MoveWindow(request) => {
            match move_window(request, runtime.window_backend.as_ref()).await {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::LaunchWindow(request) => match launch_window(request, runtime).await {
            Ok(result) => DaemonResponse::Action(Box::new(result)),
            Err(err) => daemon_error(err),
        },
        DaemonRequest::ResizeWindow(request) => {
            match resize_window(request, runtime.window_backend.as_ref()).await {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
        DaemonRequest::PageZoom(request) => {
            match page_zoom(
                request,
                runtime.window_backend.as_ref(),
                runtime.input_backend_preference,
                &runtime.xkb_keymap_config,
                &runtime.portal_eis_session_store,
            )
            .await
            {
                Ok(result) => DaemonResponse::Action(Box::new(result)),
                Err(err) => daemon_error(err),
            }
        }
    }
}

async fn preflight_retained_capture_observation(
    session_id: &str,
    runtime: &DaemonRuntime,
) -> Result<()> {
    let status = runtime
        .capture_session_store
        .status_for_session(session_id)
        .await;
    verify_chooser_backed_window_capture_policy(&status, runtime).await
}

async fn verify_chooser_backed_window_capture_policy(
    status: &libseatgeist::CaptureSessionStatus,
    runtime: &DaemonRuntime,
) -> Result<()> {
    if status.source_type.as_deref() != Some("window") || status.requested_window_id.is_some() {
        return Ok(());
    }
    let windows = runtime
        .window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)
        .context("verify chooser-backed window capture against protected-app policy")?;
    if windows.iter().any(|window| {
        observation_policy::app_is_protected(&runtime.app_policy, window.app_id.as_deref())
    }) {
        bail!(
            "app policy denied an uncorrelated chooser-backed window capture while a protected \
             application is open; use an exact authorized window capture"
        );
    }
    Ok(())
}

async fn protect_retained_capture_frame(
    session_id: &str,
    frame: &mut libseatgeist::CaptureFrameResult,
    runtime: &DaemonRuntime,
) -> Result<()> {
    let status = runtime
        .capture_session_store
        .status_for_session(session_id)
        .await;
    if status.source_type.as_deref() == Some("window") && status.requested_window_id.is_some() {
        // Exact window sessions were app-authorized before opening and produce
        // window-local pixels rather than a composed desktop.
        annotate_exact_window_capture_transform(&status, frame, runtime).await?;
        return Ok(());
    }
    if status.source_type.as_deref() == Some("window") {
        verify_chooser_backed_window_capture_policy(&status, runtime).await?;
        return Ok(());
    }
    frame.screenshot = screenshot::apply_capture_frame_protection(
        frame.screenshot.clone(),
        &runtime.window_list_state,
        &runtime.app_policy,
    )?;
    let bytes = fs::read(&frame.screenshot.path).with_context(|| {
        format!(
            "hash protected capture frame {}",
            frame.screenshot.path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"seatgeist-protected-capture-v1\0");
    hasher.update(bytes);
    frame.revision = format!("protected:{:x}", hasher.finalize());
    Ok(())
}

async fn annotate_exact_window_capture_transform(
    status: &libseatgeist::CaptureSessionStatus,
    frame: &mut libseatgeist::CaptureFrameResult,
    runtime: &DaemonRuntime,
) -> Result<()> {
    let window_id = status
        .requested_window_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("exact capture status omitted its requested window id"))?;
    let windows = runtime
        .window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)
        .context("resolve exact capture window geometry")?;
    let window = windows
        .iter()
        .find(|window| window.id == window_id)
        .ok_or_else(|| anyhow::anyhow!("requested window does not exist"))?;
    let geometry = window
        .geometry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("exact capture window has no geometry metadata"))?;
    if geometry.space != CoordinateSpace::LogicalPixel
        || geometry.width == 0
        || geometry.height == 0
    {
        bail!("exact capture window has invalid logical geometry");
    }
    let monitors = runtime
        .screen_backend
        .list_monitors()
        .await
        .unwrap_or_default();
    let monitor_scale = window
        .monitor_id
        .as_deref()
        .and_then(|id| monitors.iter().find(|monitor| monitor.id == id))
        .or_else(|| {
            let center_x = f64::from(geometry.x) + f64::from(geometry.width) / 2.0;
            let center_y = f64::from(geometry.y) + f64::from(geometry.height) / 2.0;
            monitors.iter().find(|monitor| {
                let right = f64::from(monitor.logical_origin_x) + f64::from(monitor.logical_width);
                let bottom =
                    f64::from(monitor.logical_origin_y) + f64::from(monitor.logical_height);
                center_x >= f64::from(monitor.logical_origin_x)
                    && center_x < right
                    && center_y >= f64::from(monitor.logical_origin_y)
                    && center_y < bottom
            })
        })
        .map(|monitor| monitor.scale_factor)
        .filter(|scale| scale.is_finite() && *scale > 0.0);
    // ScreenShot2 excludes decorations and reports native-resolution source
    // pixels. Prefer its client-surface extent over the bridge's frame extent
    // so fractional output scaling and server-side decorations cannot shift a
    // preview-derived click.
    let logical_source_width = monitor_scale
        .map(|scale| (f64::from(frame.screenshot.source_width) / scale).round())
        .filter(|width| *width >= 1.0 && *width <= f64::from(u32::MAX))
        .map_or(geometry.width, |width| width as u32);
    let logical_source_height = monitor_scale
        .map(|scale| (f64::from(frame.screenshot.source_height) / scale).round())
        .filter(|height| *height >= 1.0 && *height <= f64::from(u32::MAX))
        .map_or(geometry.height, |height| height as u32);
    frame.screenshot.transform = ScreenshotTransform {
        source_coordinate_space: CoordinateSpace::WindowLocal,
        output_coordinate_space: CoordinateSpace::CaptureOutput,
        source_extent_width: Some(logical_source_width),
        source_extent_height: Some(logical_source_height),
        source_origin_x: 0,
        source_origin_y: 0,
        scale_x: f64::from(frame.screenshot.output_width) / f64::from(logical_source_width),
        scale_y: f64::from(frame.screenshot.output_height) / f64::from(logical_source_height),
    };
    frame.screenshot.coordinate_space = CoordinateSpace::WindowLocal;
    Ok(())
}

fn health(
    journal: &ActionJournal,
    config_fingerprint: String,
    binary_sha256: Option<String>,
) -> HealthStatus {
    HealthStatus {
        service: "seatgeistd".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        status: "ok".to_string(),
        protocol_version: Some(DAEMON_PROTOCOL_VERSION.to_string()),
        run_id: Some(journal.run_id),
        git_sha: option_env!("SEATGEIST_GIT_SHA").map(str::to_string),
        build_unix_ms: option_env!("SEATGEIST_BUILD_UNIX_MS")
            .and_then(|value| value.parse().ok())
            .or_else(executable_modified_unix_ms),
        binary_sha256,
        config_fingerprint: Some(config_fingerprint),
        resident_memory_bytes: None,
        resident_memory_peak_bytes: None,
    }
}

fn process_resident_memory() -> (Option<u64>, Option<u64>) {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    (
        proc_status_kib(&status, "VmRSS:"),
        proc_status_kib(&status, "VmHWM:"),
    )
}

fn proc_status_kib(status: &str, key: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with(key))?;
    let kib = line[key.len()..]
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kib.checked_mul(1024)
}

fn executable_sha256() -> Option<String> {
    env::current_exe()
        .ok()
        .and_then(|path| sha256_file(&path).ok())
}

fn executable_modified_unix_ms() -> Option<u64> {
    env::current_exe()
        .ok()
        .and_then(|path| fs::metadata(path).ok())
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn config_fingerprint(settings: &RunSettings) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(format!("{settings:?}").as_bytes());
    digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn capabilities(
    input_backend_preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
    agent_seat_ready: bool,
    window_resize_ready: bool,
    window_move_ready: bool,
    window_launch_ready: bool,
    window_close_ready: bool,
) -> CapabilitySet {
    let stored_session_active = portal_eis_session_store.active().unwrap_or(false);
    CapabilitySet {
        capabilities: current_capabilities(
            input_backend_preference,
            stored_session_active,
            agent_seat_ready,
            window_resize_ready,
            window_move_ready,
            window_launch_ready,
            window_close_ready,
        ),
    }
}

fn policy_status_from_config(config: &PolicyConfig) -> PolicyStatus {
    PolicyStatus {
        default_observe: config.default_observe.clone(),
        default_control: config.default_control.clone(),
        default_destructive_actions: config.default_destructive_actions.clone(),
        default_secret_fields: config.default_secret_fields.clone(),
        default_full_resolution_screenshot: config.default_full_resolution_screenshot.clone(),
        default_clipboard_read: config.default_clipboard_read.clone(),
        default_clipboard_write: config.default_clipboard_write.clone(),
    }
}

fn safety_status(
    settings: &SafetySettings,
    journal_settings: &JournalSettings,
    activity_tracker: &activity::ActivityTracker,
) -> Result<SafetyStatus> {
    let (human_input_signal_fresh, human_input_signal_age_ms) =
        human_input_signal_state(settings, activity_tracker)?;
    let activity = activity_tracker.status();
    Ok(SafetyStatus {
        require_focus_guard: settings.require_focus_guard,
        pause_on_human_input: settings.pause_on_human_input,
        human_input_activity_file: settings.human_input_activity_file.clone(),
        human_input_quiet_ms: settings.human_input_quiet_ms,
        human_input_signal_fresh,
        human_input_signal_age_ms,
        human_input_activity_backend: activity.backend,
        human_input_activity_trusted: activity.trusted,
        human_input_last_class: activity.last_class.map(str::to_string),
        human_input_last_provenance: activity.last_provenance.map(str::to_string),
        control_rate_limit_per_minute: settings.control_rate_limit_per_minute,
        preview_max_edge: settings.preview_max_edge,
        tile_max_edge: settings.tile_max_edge,
        screenshot_redaction_count: settings.screenshot_redactions.len(),
        journal_artifact_metadata_enabled: journal_settings.include_artifact_metadata,
    })
}

async fn computer_use_readiness_status(runtime: &DaemonRuntime) -> ComputerUseReadinessStatus {
    let mut issues = Vec::new();
    let mut next_steps = Vec::new();

    let safety = match safety_status(
        &runtime.safety_settings,
        &runtime.journal.settings,
        &runtime.activity_tracker,
    ) {
        Ok(status) => status,
        Err(err) => {
            issues.push(format!("safety status unavailable: {err}"));
            next_steps.push("check seatgeist.safety_status".to_string());
            SafetyStatus {
                require_focus_guard: runtime.safety_settings.require_focus_guard,
                pause_on_human_input: runtime.safety_settings.pause_on_human_input,
                human_input_activity_file: runtime
                    .safety_settings
                    .human_input_activity_file
                    .clone(),
                human_input_quiet_ms: runtime.safety_settings.human_input_quiet_ms,
                human_input_signal_fresh: false,
                human_input_signal_age_ms: None,
                human_input_activity_backend: None,
                human_input_activity_trusted: false,
                human_input_last_class: None,
                human_input_last_provenance: None,
                control_rate_limit_per_minute: runtime
                    .safety_settings
                    .control_rate_limit_per_minute,
                preview_max_edge: runtime.safety_settings.preview_max_edge,
                tile_max_edge: runtime.safety_settings.tile_max_edge,
                screenshot_redaction_count: runtime.safety_settings.screenshot_redactions.len(),
                journal_artifact_metadata_enabled: runtime
                    .journal
                    .settings
                    .include_artifact_metadata,
            }
        }
    };
    let desktop = desktop_session_status_from_env(std::env::vars());
    let panic_stop = runtime.panic_stop.status();
    let capture = capture_backend_status();
    let clipboard = clipboard::status();
    let accessibility = accessibility_quality_status().await;
    let input = match input_backend_status(
        runtime.input_backend_preference,
        &runtime.portal_eis_session_store,
        runtime.agent_seat_backend.ready(),
        &runtime.xkb_keymap_config,
    ) {
        Ok(status) => Some(status),
        Err(err) => {
            issues.push(format!("input backend status unavailable: {err}"));
            next_steps.push("check seatgeist.input_backend_status".to_string());
            None
        }
    };

    let desktop_session_ready =
        desktop.dbus_session_bus_address_present && desktop.xdg_runtime_dir_present;
    if !desktop_session_ready {
        issues.push("desktop session bus or runtime directory is missing".to_string());
        next_steps.push("check seatgeist.desktop_session_status".to_string());
    }
    if capture.implemented_available_backend.is_none() {
        issues.push("no executable screenshot backend is available".to_string());
        next_steps.push("check seatgeist.capture_backend_status".to_string());
    }
    if input
        .as_ref()
        .and_then(|status| status.implemented_available_backend.as_ref())
        .is_none()
    {
        issues.push("no executable input backend is available".to_string());
        next_steps.push("check seatgeist.input_backend_status".to_string());
    }
    if !accessibility.semantic_targeting_reliable {
        issues.push("accessibility tree is weak or unavailable for semantic targeting".to_string());
        next_steps.push("check seatgeist.a11y_quality_status".to_string());
    }
    if clipboard.read_backend.is_none() {
        issues.push("clipboard read backend is unavailable".to_string());
        next_steps.push("check seatgeist.clipboard_status".to_string());
    }
    if clipboard.write_backend.is_none() {
        issues.push("clipboard write backend is unavailable".to_string());
        next_steps.push("check seatgeist.clipboard_status".to_string());
    }
    if panic_stop.enabled {
        issues.push("panic-stop is enabled".to_string());
        next_steps.push("disable panic-stop only when control is safe".to_string());
    }
    if safety.pause_on_human_input && safety.human_input_signal_fresh {
        issues.push("fresh human input activity is blocking control".to_string());
        next_steps.push("wait for the configured human-input quiet interval".to_string());
    }

    let input_backend = input
        .as_ref()
        .and_then(|status| status.implemented_available_backend.clone());
    let control_blocked =
        panic_stop.enabled || (safety.pause_on_human_input && safety.human_input_signal_fresh);

    let policy = runtime.policy.config();
    let observe_state =
        action_readiness(desktop_session_ready, false, &policy.default_observe, false);
    let screenshot_state = action_readiness(
        capture.implemented_available_backend.is_some(),
        false,
        &policy.default_observe,
        false,
    );
    let window_control_state = action_readiness(
        desktop_session_ready,
        control_blocked,
        &policy.default_control,
        safety.require_focus_guard,
    );
    let keyboard_input_state = action_readiness(
        input_backend.is_some(),
        control_blocked,
        &policy.default_control,
        safety.require_focus_guard,
    );
    let pointer_input_state = keyboard_input_state.clone();
    let semantic_action_state = action_readiness(
        accessibility.semantic_targeting_reliable,
        control_blocked,
        &policy.default_control,
        safety.require_focus_guard,
    );
    let clipboard_read_state = action_readiness(
        clipboard.read_backend.is_some(),
        false,
        &policy.default_clipboard_read,
        false,
    );
    let clipboard_write_state = action_readiness(
        clipboard.write_backend.is_some(),
        false,
        &policy.default_clipboard_write,
        false,
    );
    if matches!(window_control_state, ActionReadiness::NeedsApproval) {
        issues.push("control policy requires an approval grant".to_string());
        next_steps.push("create a scoped seatgeist approval grant before control".to_string());
    } else if matches!(window_control_state, ActionReadiness::NeedsGuard) {
        issues.push("control requires a current desktop guard revision".to_string());
        next_steps
            .push("reuse desktop_revision from readiness or observe in the action".to_string());
    }
    let desktop_revision = match runtime.window_backend.active_window().await {
        Ok(active_window) => Some(observation::active_window_revision(&active_window)),
        Err(_) => None,
    };

    ComputerUseReadinessStatus {
        ready_for_observe: observe_state == ActionReadiness::Ready,
        ready_for_screenshot: screenshot_state == ActionReadiness::Ready,
        ready_for_window_control: window_control_state == ActionReadiness::Ready,
        ready_for_keyboard_input: keyboard_input_state == ActionReadiness::Ready,
        ready_for_pointer_input: pointer_input_state == ActionReadiness::Ready,
        ready_for_semantic_actions: semantic_action_state == ActionReadiness::Ready,
        ready_for_clipboard_read: clipboard_read_state == ActionReadiness::Ready,
        ready_for_clipboard_write: clipboard_write_state == ActionReadiness::Ready,
        observe_state,
        screenshot_state,
        window_control_state,
        keyboard_input_state,
        pointer_input_state,
        semantic_action_state,
        clipboard_read_state,
        clipboard_write_state,
        desktop_revision,
        focus_guard_required: safety.require_focus_guard,
        panic_stop_enabled: panic_stop.enabled,
        human_input_pause_enabled: safety.pause_on_human_input,
        human_input_signal_fresh: safety.human_input_signal_fresh,
        desktop_session_ready,
        dbus_session_bus_present: desktop.dbus_session_bus_address_present,
        runtime_dir_present: desktop.xdg_runtime_dir_present,
        capture_backend: capture.implemented_available_backend,
        input_backend,
        clipboard_read_backend: clipboard.read_backend,
        clipboard_write_backend: clipboard.write_backend,
        accessibility_backend: accessibility.recommended_fallback,
        issues,
        next_steps,
    }
}

fn action_readiness(
    available: bool,
    blocked: bool,
    policy: &ToolApprovalLevel,
    needs_guard: bool,
) -> ActionReadiness {
    if !available {
        return ActionReadiness::Unavailable;
    }
    if blocked || matches!(policy, ToolApprovalLevel::Deny) {
        return ActionReadiness::Blocked;
    }
    if matches!(policy, ToolApprovalLevel::Prompt) {
        return ActionReadiness::NeedsApproval;
    }
    if needs_guard {
        return ActionReadiness::NeedsGuard;
    }
    ActionReadiness::Ready
}

fn desktop_session_status_from_env<I, K, V>(vars: I) -> DesktopSessionStatus
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    let vars = vars
        .into_iter()
        .map(|(key, value)| (key.as_ref().to_string(), value.into()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut status = DesktopSessionStatus {
        xdg_session_type: clean_env_value(vars.get("XDG_SESSION_TYPE")),
        xdg_current_desktop: clean_env_value(vars.get("XDG_CURRENT_DESKTOP")),
        desktop_session: clean_env_value(vars.get("DESKTOP_SESSION")),
        kde_full_session: clean_env_value(vars.get("KDE_FULL_SESSION")),
        kde_session_version: clean_env_value(vars.get("KDE_SESSION_VERSION")),
        wayland_display: clean_env_value(vars.get("WAYLAND_DISPLAY")),
        display: clean_env_value(vars.get("DISPLAY")),
        dbus_session_bus_address_present: clean_env_value(vars.get("DBUS_SESSION_BUS_ADDRESS"))
            .is_some(),
        xdg_runtime_dir_present: clean_env_value(vars.get("XDG_RUNTIME_DIR")).is_some(),
        setup_hint: String::new(),
    };
    status.setup_hint = desktop_session_setup_hint(&status);
    status
}

fn clean_env_value(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn desktop_session_setup_hint(status: &DesktopSessionStatus) -> String {
    if !status.dbus_session_bus_address_present {
        return "DBUS_SESSION_BUS_ADDRESS is not present; Seatgeist DBus, portal, KWin, and AT-SPI probes need a user session bus".to_string();
    }
    if !status.xdg_runtime_dir_present {
        return "XDG_RUNTIME_DIR is not present; daemon socket, portal sessions, and Wayland runtime files need a user runtime directory".to_string();
    }
    let desktop = status
        .xdg_current_desktop
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_kde = desktop.contains("kde")
        || status.kde_full_session.as_deref() == Some("true")
        || status.kde_session_version.as_deref() == Some("6");
    if !is_kde {
        return "session bus and runtime directory are present, but KDE Plasma was not detected; KWin-specific tools may be unavailable".to_string();
    }
    match status.xdg_session_type.as_deref() {
        Some("wayland") if status.wayland_display.is_some() => {
            "KDE Wayland session detected; prefer portal/KWin/AT-SPI diagnostics before uinput fallback".to_string()
        }
        Some("wayland") => {
            "KDE Wayland session type detected, but WAYLAND_DISPLAY is missing; portal/KWin control may fail until the daemon inherits the session environment".to_string()
        }
        Some("x11") => {
            "KDE X11 session detected; Seatgeist targets KDE Wayland first, so Wayland portal/libei diagnostics may be unavailable".to_string()
        }
        Some(other) => format!(
            "KDE session detected with XDG_SESSION_TYPE={other}; verify portal, KWin, and input backend diagnostics before control"
        ),
        None => "KDE session detected, but XDG_SESSION_TYPE is missing; verify portal, KWin, and input backend diagnostics before control".to_string(),
    }
}

fn input_backend_status(
    preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
    agent_seat_ready: bool,
    xkb_keymap_config: &XkbKeymapConfig,
) -> Result<InputBackendStatus> {
    let xkb_keymap = effective_xkb_keymap_resolution(xkb_keymap_config).status;
    let stored_session_active = portal_eis_session_store.active()?;
    input_diagnostics::status(
        preference,
        stored_session_active,
        agent_seat_ready,
        xkb_keymap,
    )
}

#[cfg(test)]
fn enforce_policy(policy: &PolicyEngine, request: &DaemonRequest) -> Result<()> {
    enforce_policy_with_approvals(policy, &ApprovalStore::default(), request)
}

fn enforce_policy_with_approvals(
    policy: &PolicyEngine,
    approval_store: &ApprovalStore,
    request: &DaemonRequest,
) -> Result<()> {
    let safety_class = safety_class_for_request(request);
    let decision = policy.decide(&safety_class);
    match decision.level {
        ToolApprovalLevel::Allow => Ok(()),
        ToolApprovalLevel::Prompt => {
            if let Some(reason) =
                approval_store.matching_prompt_approval(&safety_class, request.method_name())?
            {
                info!(
                    method = request.method_name(),
                    safety_class = ?safety_class,
                    reason = %reason,
                    "prompt policy satisfied by approval file"
                );
                return Ok(());
            }
            bail!(
                "policy prompt required for {safety_class:?}, but no matching approval grant is available"
            )
        }
        ToolApprovalLevel::Deny => bail!("policy denied {safety_class:?}: {}", decision.reason),
    }
}

fn enforce_panic_stop(panic_stop: &PanicStopState, request: &DaemonRequest) -> Result<()> {
    let status = panic_stop.status();
    let safety_class = safety_class_for_request(request);
    if status.enabled && is_control_safety_class(&safety_class) {
        bail!(
            "panic-stop is active at {}; refusing {:?}",
            status.path.display(),
            safety_class
        );
    }
    Ok(())
}

fn enforce_human_input_pause(
    settings: &SafetySettings,
    activity_tracker: &activity::ActivityTracker,
    request: &DaemonRequest,
) -> Result<()> {
    if !settings.pause_on_human_input {
        return Ok(());
    }
    let safety_class = safety_class_for_request(request);
    if !is_control_safety_class(&safety_class) {
        return Ok(());
    }
    let (fresh, _) = human_input_signal_state(settings, activity_tracker)?;
    if fresh {
        let activity_backend = activity_tracker
            .status()
            .backend
            .unwrap_or_else(|| "legacy_file".to_string());
        bail!(
            "human input activity is fresh from {}; refusing {:?} until quiet for {}ms",
            activity_backend,
            safety_class,
            settings.human_input_quiet_ms
        );
    }
    Ok(())
}

async fn enforce_capture_session_owner(
    request: &DaemonRequest,
    response_options: Option<&DaemonResponseOptions>,
    client: Option<&JournalClientContext>,
    capture_session_store: &CaptureSessionStore,
) -> Result<()> {
    for session_id in capture_session_ids_for_request(request, response_options) {
        capture_session_store
            .require_owner(session_id, client)
            .await?;
    }
    Ok(())
}

fn capture_session_ids_for_request<'a>(
    request: &'a DaemonRequest,
    response_options: Option<&'a DaemonResponseOptions>,
) -> Vec<&'a str> {
    let mut session_ids = Vec::with_capacity(2);
    let lifecycle_session_id = match request {
        DaemonRequest::CaptureSessionRenew(request)
        | DaemonRequest::CaptureSessionClose(request) => Some(request.session_id.as_str()),
        DaemonRequest::CaptureSnapshot(request) => Some(request.session_id.as_str()),
        DaemonRequest::CaptureWait(request) => Some(request.session_id.as_str()),
        _ => interaction_session_id_for_request(request),
    };
    if let Some(session_id) = lifecycle_session_id {
        session_ids.push(session_id);
    }
    if let Some(session_id) = response_options
        .and_then(|options| options.post_action.as_ref())
        .filter(|options| options.observe_after)
        .and_then(|options| options.image.as_ref())
        .map(|image| image.session_id.as_str())
        && !session_ids.contains(&session_id)
    {
        session_ids.push(session_id);
    }
    session_ids
}

fn session_backend_role_for_request(
    request: &DaemonRequest,
) -> Option<session_execution::BackendRole> {
    match request {
        DaemonRequest::TypeText(_)
        | DaemonRequest::KeyCombo(_)
        | DaemonRequest::MovePointer(_)
        | DaemonRequest::ClickPointer(_)
        | DaemonRequest::DragPointer(_)
        | DaemonRequest::ScrollPointer(_) => Some(session_execution::BackendRole::RawInput),
        DaemonRequest::AccessibilityInvoke(_)
        | DaemonRequest::AccessibilitySetText(_)
        | DaemonRequest::AccessibilityInsertText(_)
        | DaemonRequest::AccessibilityDeleteText(_)
        | DaemonRequest::AccessibilityCopyText(_)
        | DaemonRequest::AccessibilityCutText(_)
        | DaemonRequest::AccessibilityPasteText(_)
        | DaemonRequest::AccessibilitySetCaret(_)
        | DaemonRequest::AccessibilitySetSelection(_)
        | DaemonRequest::ClickButton(_)
        | DaemonRequest::SetTextField(_)
        | DaemonRequest::FocusTextField(_)
        | DaemonRequest::ActivateTab(_)
        | DaemonRequest::ActivateLink(_)
        | DaemonRequest::ToggleCheck(_)
        | DaemonRequest::SetValue(_)
        | DaemonRequest::SelectItem(_)
        | DaemonRequest::SelectMenu(_) => Some(session_execution::BackendRole::Semantic),
        DaemonRequest::FocusWindow(_)
        | DaemonRequest::CloseWindow(_)
        | DaemonRequest::MoveWindow(_)
        | DaemonRequest::LaunchWindow(_)
        | DaemonRequest::ResizeWindow(_) => Some(session_execution::BackendRole::Other),
        DaemonRequest::PageZoom(_) => Some(session_execution::BackendRole::RawInput),
        _ => None,
    }
}

async fn record_session_execution_response(
    session_ids: &[String],
    method: &str,
    safety_class: SafetyClass,
    request_backend: Option<&str>,
    backend_role: session_execution::BackendRole,
    response: &DaemonResponse,
    store: &session_execution::SessionExecutionStore,
) {
    if !response.ok()
        || matches!(response, DaemonResponse::Action(action) if !action.ok)
        || method == "capture_session_close"
    {
        return;
    }
    let response_backend = journal_backend_from_response(response);
    let backend = response_backend.as_deref().or(request_backend);
    let (action_id, settle) = match response {
        DaemonResponse::Action(action) => (
            Some(action.id),
            action
                .observation
                .as_ref()
                .and_then(|observation| observation.settle.as_ref()),
        ),
        _ => (None, None),
    };
    for session_id in session_ids {
        if let Err(err) = store
            .record_success(
                session_id,
                session_execution::SuccessfulExecution {
                    method: method.to_string(),
                    safety_class: safety_class.clone(),
                    backend: backend.map(str::to_string),
                    backend_role,
                    action_id,
                    settle: settle.cloned(),
                },
            )
            .await
        {
            warn!(session_id, method, %err, "could not record session execution metadata");
        }
    }
}

async fn attach_session_execution_status(response: &mut DaemonResponse, runtime: &DaemonRuntime) {
    let DaemonResponse::CaptureSessionStatus(status) = response else {
        return;
    };
    let Some(session_id) = status.session_id.as_deref() else {
        status.execution = None;
        return;
    };
    status.execution = runtime
        .session_execution_store
        .status(session_id)
        .await
        .map(Box::new);
}

fn enforce_control_rate_limit(limiter: &ControlRateLimiter, request: &DaemonRequest) -> Result<()> {
    let safety_class = safety_class_for_request(request);
    if !is_control_safety_class(&safety_class) {
        return Ok(());
    }
    limiter.check(&safety_class)
}

fn human_input_signal_state(
    settings: &SafetySettings,
    activity_tracker: &activity::ActivityTracker,
) -> Result<(bool, Option<u64>)> {
    if !settings.pause_on_human_input {
        return Ok((false, None));
    }
    let quiet_for = Duration::from_millis(settings.human_input_quiet_ms);
    let tracked = activity_tracker.interference_state(quiet_for);
    if tracked.0 {
        return Ok(tracked);
    }
    let Some(path) = &settings.human_input_activity_file else {
        return Ok(tracked);
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((false, None)),
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("read mtime for {}", path.display()))?;
    let quiet_for = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    let age_ms = u64::try_from(quiet_for.as_millis()).unwrap_or(u64::MAX);
    Ok((
        quiet_for <= Duration::from_millis(settings.human_input_quiet_ms),
        Some(age_ms),
    ))
}

fn enforce_required_focus_guard(settings: &SafetySettings, request: &DaemonRequest) -> Result<()> {
    if pointer_request_uses_window_local(request)
        && active_window_guard_for_request(request).is_none()
        && interaction_session_id_for_request(request).is_none()
    {
        bail!("active-window guard is required for window_local pointer coordinates");
    }
    if !settings.require_focus_guard {
        return Ok(());
    }
    if !is_control_safety_class(&safety_class_for_request(request)) {
        return Ok(());
    }
    if target_window_guard_for_request(request).is_some() {
        return Ok(());
    }
    if interaction_session_id_for_request(request).is_some() {
        return Ok(());
    }
    if active_window_guard_for_request(request).is_some() {
        return Ok(());
    }
    bail!(
        "focus guard is required for {:?} by safety.require_focus_guard",
        safety_class_for_request(request)
    )
}

fn pointer_request_uses_window_local(request: &DaemonRequest) -> bool {
    match request {
        DaemonRequest::MovePointer(request) => matches!(
            request.point.space,
            CoordinateSpace::WindowLocal | CoordinateSpace::CaptureOutput
        ),
        DaemonRequest::ClickPointer(request) => matches!(
            request.point.space,
            CoordinateSpace::WindowLocal | CoordinateSpace::CaptureOutput
        ),
        DaemonRequest::DragPointer(request) => {
            matches!(
                request.from.space,
                CoordinateSpace::WindowLocal | CoordinateSpace::CaptureOutput
            ) || matches!(
                request.to.space,
                CoordinateSpace::WindowLocal | CoordinateSpace::CaptureOutput
            )
        }
        _ => false,
    }
}

async fn resolve_move_capture_coordinates(
    mut request: MovePointerRequest,
    runtime: &DaemonRuntime,
) -> Result<MovePointerRequest> {
    request.point = resolve_capture_output_point(
        request.point,
        request.session_id.as_deref(),
        request.capture_revision.as_deref(),
        runtime,
    )
    .await?;
    request.capture_revision = None;
    Ok(request)
}

async fn resolve_click_capture_coordinates(
    mut request: ClickPointerRequest,
    runtime: &DaemonRuntime,
) -> Result<ClickPointerRequest> {
    request.point = resolve_capture_output_point(
        request.point,
        request.session_id.as_deref(),
        request.capture_revision.as_deref(),
        runtime,
    )
    .await?;
    request.capture_revision = None;
    Ok(request)
}

async fn resolve_drag_capture_coordinates(
    mut request: DragPointerRequest,
    runtime: &DaemonRuntime,
) -> Result<DragPointerRequest> {
    request.from = resolve_capture_output_point(
        request.from,
        request.session_id.as_deref(),
        request.capture_revision.as_deref(),
        runtime,
    )
    .await?;
    request.to = resolve_capture_output_point(
        request.to,
        request.session_id.as_deref(),
        request.capture_revision.as_deref(),
        runtime,
    )
    .await?;
    request.capture_revision = None;
    Ok(request)
}

async fn resolve_capture_output_point(
    point: Point,
    session_id: Option<&str>,
    capture_revision: Option<&str>,
    runtime: &DaemonRuntime,
) -> Result<Point> {
    if point.space != CoordinateSpace::CaptureOutput {
        if capture_revision.is_some() {
            bail!("capture_revision requires capture_output coordinates");
        }
        return Ok(point);
    }
    let session_id =
        session_id.ok_or_else(|| anyhow::anyhow!("capture_output requires session_id"))?;
    let capture_revision = capture_revision
        .ok_or_else(|| anyhow::anyhow!("capture_output requires capture_revision"))?;
    runtime
        .capture_session_store
        .resolve_capture_output_point(session_id, capture_revision, point)
        .await
}

fn is_control_safety_class(safety_class: &SafetyClass) -> bool {
    matches!(
        safety_class,
        SafetyClass::ControlPointer
            | SafetyClass::ControlKeyboard
            | SafetyClass::ControlSemantic
            | SafetyClass::DestructiveAction
            | SafetyClass::SecretField
    )
}

fn validate_interaction_session_request(request: &DaemonRequest) -> Result<()> {
    if interaction_session_id_for_request(request).is_some()
        && active_window_guard_for_request(request).is_some()
    {
        bail!("session_id cannot be combined with an active-window guard");
    }
    Ok(())
}

fn validate_capture_output_request(request: &DaemonRequest) -> Result<()> {
    let (points, revision, session_id): (&[Point], Option<&str>, Option<&str>) = match request {
        DaemonRequest::MovePointer(request) => (
            std::slice::from_ref(&request.point),
            request.capture_revision.as_deref(),
            request.session_id.as_deref(),
        ),
        DaemonRequest::ClickPointer(request) => (
            std::slice::from_ref(&request.point),
            request.capture_revision.as_deref(),
            request.session_id.as_deref(),
        ),
        DaemonRequest::DragPointer(request) => (
            std::slice::from_ref(&request.from),
            request.capture_revision.as_deref(),
            request.session_id.as_deref(),
        ),
        _ => return Ok(()),
    };
    let uses_capture_output = match request {
        DaemonRequest::DragPointer(request) => {
            request.from.space == CoordinateSpace::CaptureOutput
                || request.to.space == CoordinateSpace::CaptureOutput
        }
        _ => points
            .iter()
            .any(|point| point.space == CoordinateSpace::CaptureOutput),
    };
    if uses_capture_output && session_id.is_none() {
        bail!("capture_output coordinates require session_id");
    }
    if uses_capture_output && revision.is_none() {
        bail!("capture_output coordinates require capture_revision");
    }
    if !uses_capture_output && revision.is_some() {
        bail!("capture_revision requires capture_output coordinates");
    }
    if let DaemonRequest::DragPointer(request) = request
        && (request.from.space == CoordinateSpace::CaptureOutput)
            != (request.to.space == CoordinateSpace::CaptureOutput)
    {
        bail!("drag endpoints must both use capture_output coordinates");
    }
    Ok(())
}

fn interaction_session_id_for_request(request: &DaemonRequest) -> Option<&str> {
    match request {
        DaemonRequest::CloseWindow(request) => request.session_id.as_deref(),
        DaemonRequest::TypeText(request) => request.session_id.as_deref(),
        DaemonRequest::KeyCombo(request) => request.session_id.as_deref(),
        DaemonRequest::MovePointer(request) => request.session_id.as_deref(),
        DaemonRequest::ClickPointer(request) => request.session_id.as_deref(),
        DaemonRequest::DragPointer(request) => request.session_id.as_deref(),
        DaemonRequest::ScrollPointer(request) => request.session_id.as_deref(),
        _ => None,
    }
}

fn uses_independent_agent_seat(
    request: &DaemonRequest,
    preference: InputBackendPreference,
) -> bool {
    preference == InputBackendPreference::KwinAgentSeat
        && interaction_session_id_for_request(request).is_some()
        && matches!(
            request,
            DaemonRequest::TypeText(_)
                | DaemonRequest::KeyCombo(_)
                | DaemonRequest::MovePointer(_)
                | DaemonRequest::ClickPointer(_)
                | DaemonRequest::DragPointer(_)
                | DaemonRequest::ScrollPointer(_)
        )
}

fn active_window_guard_for_request(request: &DaemonRequest) -> Option<&ActiveWindowGuard> {
    match request {
        DaemonRequest::FocusWindow(request) => request.guard.as_ref(),
        DaemonRequest::CloseWindow(request) => request.guard.as_ref(),
        DaemonRequest::MoveWindow(request) => request.guard.as_ref(),
        DaemonRequest::LaunchWindow(request) => request.guard.as_ref(),
        DaemonRequest::ResizeWindow(request) => request.guard.as_ref(),
        DaemonRequest::PageZoom(request) => Some(&request.guard),
        DaemonRequest::AccessibilityInvoke(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilitySetText(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilityInsertText(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilityDeleteText(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilityCopyText(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilityCutText(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilityPasteText(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilitySetCaret(request) => request.guard.as_ref(),
        DaemonRequest::AccessibilitySetSelection(request) => request.guard.as_ref(),
        DaemonRequest::TypeText(request) => request.guard.as_ref(),
        DaemonRequest::KeyCombo(request) => request.guard.as_ref(),
        DaemonRequest::RemoteDesktopSessionProbe(request) => request.guard.as_ref(),
        DaemonRequest::RemoteDesktopEisProbe(request) => request.guard.as_ref(),
        DaemonRequest::RemoteDesktopEisStart(request) => request.guard.as_ref(),
        DaemonRequest::MovePointer(request) => request.guard.as_ref(),
        DaemonRequest::ClickPointer(request) => request.guard.as_ref(),
        DaemonRequest::DragPointer(request) => request.guard.as_ref(),
        DaemonRequest::ScrollPointer(request) => request.guard.as_ref(),
        DaemonRequest::ClickButton(request) => request.guard.as_ref(),
        DaemonRequest::SetTextField(request) => request.guard.as_ref(),
        DaemonRequest::FocusTextField(request) => request.guard.as_ref(),
        DaemonRequest::ActivateTab(request) => request.guard.as_ref(),
        DaemonRequest::ActivateLink(request) => request.guard.as_ref(),
        DaemonRequest::ToggleCheck(request) => request.guard.as_ref(),
        DaemonRequest::SetValue(request) => request.guard.as_ref(),
        DaemonRequest::SelectItem(request) => request.guard.as_ref(),
        DaemonRequest::SelectMenu(request) => request.guard.as_ref(),
        _ => None,
    }
}

fn target_window_guard_for_request(
    request: &DaemonRequest,
) -> Option<&libseatgeist::TargetWindowGuard> {
    match request {
        DaemonRequest::ClickButton(request) => request.target_guard.as_ref(),
        DaemonRequest::SetTextField(request) => request.target_guard.as_ref(),
        DaemonRequest::FocusTextField(request) => request.target_guard.as_ref(),
        DaemonRequest::ActivateTab(request) => request.target_guard.as_ref(),
        DaemonRequest::ActivateLink(request) => request.target_guard.as_ref(),
        DaemonRequest::ToggleCheck(request) => request.target_guard.as_ref(),
        DaemonRequest::SetValue(request) => request.target_guard.as_ref(),
        DaemonRequest::SelectItem(request) => request.target_guard.as_ref(),
        DaemonRequest::SelectMenu(request) => request.target_guard.as_ref(),
        _ => None,
    }
}

fn resolve_semantic_handle_for_request(
    request: &mut DaemonRequest,
    store: &semantic_handle::SemanticHandleStore,
    client: Option<&JournalClientContext>,
) -> Result<()> {
    let guard = match request {
        DaemonRequest::ClickButton(request) => request.target_guard.as_mut(),
        DaemonRequest::SetTextField(request) => request.target_guard.as_mut(),
        DaemonRequest::FocusTextField(request) => request.target_guard.as_mut(),
        DaemonRequest::ActivateTab(request) => request.target_guard.as_mut(),
        DaemonRequest::ActivateLink(request) => request.target_guard.as_mut(),
        DaemonRequest::ToggleCheck(request) => request.target_guard.as_mut(),
        DaemonRequest::SetValue(request) => request.target_guard.as_mut(),
        DaemonRequest::SelectItem(request) => request.target_guard.as_mut(),
        DaemonRequest::SelectMenu(request) => request.target_guard.as_mut(),
        _ => None,
    };
    let Some(guard) = guard else {
        return Ok(());
    };
    let Some(handle) = semantic_handle::encoded_handle(guard).map(str::to_string) else {
        return Ok(());
    };
    *guard = store.consume(&handle, client)?;
    Ok(())
}

fn safety_class_for_request(request: &DaemonRequest) -> SafetyClass {
    match request {
        DaemonRequest::Health
        | DaemonRequest::Capabilities
        | DaemonRequest::PolicyStatus
        | DaemonRequest::SafetyStatus
        | DaemonRequest::DesktopSessionStatus
        | DaemonRequest::ComputerUseReadiness
        | DaemonRequest::PanicStopStatus
        | DaemonRequest::SetPanicStop(_)
        | DaemonRequest::UinputStatus
        | DaemonRequest::InputBackendStatus
        | DaemonRequest::RemoteDesktopEisSessionStatus
        | DaemonRequest::RemoteDesktopEisStop
        | DaemonRequest::CaptureBackendStatus
        | DaemonRequest::CaptureSessionStatus
        | DaemonRequest::CaptureSessionRenew(_)
        | DaemonRequest::CaptureSessionClose(_)
        | DaemonRequest::PointerCalibration
        | DaemonRequest::ClipboardBackendStatus
        | DaemonRequest::AccessibilityQualityStatus
        | DaemonRequest::JournalTail(_) => SafetyClass::Policy,
        DaemonRequest::ListMonitors
        | DaemonRequest::ListWindows
        | DaemonRequest::WindowInventory
        | DaemonRequest::WindowInventoryWait(_)
        | DaemonRequest::KwinBridgeStatus
        | DaemonRequest::ActiveWindow
        | DaemonRequest::ScreenshotTile(_)
        | DaemonRequest::WaitForChange(_)
        | DaemonRequest::CaptureOpen(_)
        | DaemonRequest::WindowCaptureOpen(_)
        | DaemonRequest::CaptureSnapshot(_)
        | DaemonRequest::CaptureWait(_)
        | DaemonRequest::FocusedAccessibilityTree(_)
        | DaemonRequest::AccessibilityFind(_)
        | DaemonRequest::AccessibilityTextAttributes(_) => SafetyClass::Observe,
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
        DaemonRequest::RemoteDesktopSessionProbe(request)
        | DaemonRequest::RemoteDesktopEisProbe(request)
        | DaemonRequest::RemoteDesktopEisStart(request) => {
            if request.pointer || request.touchscreen {
                SafetyClass::ControlPointer
            } else {
                SafetyClass::ControlKeyboard
            }
        }
        DaemonRequest::MovePointer(_)
        | DaemonRequest::ClickPointer(_)
        | DaemonRequest::DragPointer(_)
        | DaemonRequest::ScrollPointer(_) => SafetyClass::ControlPointer,
        DaemonRequest::TypeText(_) | DaemonRequest::PageZoom(_) => SafetyClass::ControlKeyboard,
        DaemonRequest::KeyCombo(request) => {
            if request.destructive || destructive_key_combo(&request.combo) {
                SafetyClass::DestructiveAction
            } else {
                SafetyClass::ControlKeyboard
            }
        }
        DaemonRequest::CloseWindow(_) => SafetyClass::DestructiveAction,
        DaemonRequest::FocusWindow(_)
        | DaemonRequest::MoveWindow(_)
        | DaemonRequest::LaunchWindow(_)
        | DaemonRequest::ResizeWindow(_)
        | DaemonRequest::AccessibilitySetText(_)
        | DaemonRequest::AccessibilityInsertText(_)
        | DaemonRequest::AccessibilityDeleteText(_)
        | DaemonRequest::AccessibilityCopyText(_)
        | DaemonRequest::AccessibilityCutText(_)
        | DaemonRequest::AccessibilityPasteText(_)
        | DaemonRequest::AccessibilitySetCaret(_)
        | DaemonRequest::AccessibilitySetSelection(_) => SafetyClass::ControlSemantic,
        DaemonRequest::AccessibilityInvoke(request) => {
            if request.destructive {
                SafetyClass::DestructiveAction
            } else {
                SafetyClass::ControlSemantic
            }
        }
        DaemonRequest::ClickButton(request) => {
            if request.destructive || destructive_label(&request.name) {
                SafetyClass::DestructiveAction
            } else {
                SafetyClass::ControlSemantic
            }
        }
        DaemonRequest::SelectMenu(request) => {
            if request.destructive || request.path.iter().any(|label| destructive_label(label)) {
                SafetyClass::DestructiveAction
            } else {
                SafetyClass::ControlSemantic
            }
        }
        DaemonRequest::SetTextField(request) => {
            if secret_field_label(&request.name) {
                SafetyClass::SecretField
            } else {
                SafetyClass::ControlSemantic
            }
        }
        DaemonRequest::FocusTextField(request) => {
            if secret_field_label(&request.name) {
                SafetyClass::SecretField
            } else {
                SafetyClass::ControlSemantic
            }
        }
        DaemonRequest::ActivateTab(_)
        | DaemonRequest::ActivateLink(_)
        | DaemonRequest::ToggleCheck(_)
        | DaemonRequest::SetValue(_)
        | DaemonRequest::SelectItem(_) => SafetyClass::ControlSemantic,
    }
}

fn secret_field_label(label: &str) -> bool {
    let normalized = label.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "password"
            | "passcode"
            | "pin"
            | "secret"
            | "token"
            | "api key"
            | "api token"
            | "access token"
            | "private key"
            | "recovery key"
            | "seed phrase"
            | "mnemonic"
            | "credential"
            | "credentials"
            | "security code"
            | "cvv"
            | "cvc"
            | "card number"
    ) || normalized.contains("password")
        || normalized.contains("passcode")
        || normalized.contains("api key")
        || normalized.contains("access token")
        || normalized.contains("secret")
        || normalized.contains("private key")
        || normalized.contains("recovery key")
        || normalized.contains("seed phrase")
}

fn destructive_label(label: &str) -> bool {
    let normalized = label.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "delete"
            | "delete..."
            | "delete permanently"
            | "remove"
            | "remove..."
            | "discard"
            | "discard changes"
            | "clear"
            | "empty trash"
            | "trash"
            | "uninstall"
            | "format"
            | "erase"
            | "reset"
            | "factory reset"
            | "close"
            | "close without saving"
            | "quit"
            | "exit"
            | "shutdown"
            | "shut down"
            | "restart"
            | "reboot"
    ) || normalized.starts_with("delete ")
        || normalized.starts_with("remove ")
        || normalized.starts_with("discard ")
        || normalized.starts_with("erase ")
}

fn destructive_key_combo(combo: &str) -> bool {
    let parts = combo
        .split('+')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let has = |name: &str| parts.iter().any(|part| part == name);
    let control = has("ctrl") || has("control");
    let close_key = has("q") || has("w");
    (parts.len() == 2 && has("alt") && has("f4"))
        || ((parts.len() == 2 || (parts.len() == 3 && has("shift"))) && control && close_key)
        || (parts.len() == 2 && has("q") && (has("meta") || has("super")))
}

fn enforce_mcp_focus_isolation(
    request: &DaemonRequest,
    client: Option<&JournalClientContext>,
) -> Result<()> {
    if client.and_then(|client| client.tool.as_deref()) != Some("seatgeist-mcp") {
        return Ok(());
    }
    match request {
        DaemonRequest::FocusWindow(_) => bail!(
            "MCP focus_window is disabled because it changes the physical user's workspace focus; open and use an exact retained window session instead"
        ),
        DaemonRequest::LaunchWindow(request)
            if request.activation != libseatgeist::WindowActivationMode::PreserveFocus =>
        {
            bail!("MCP launch_window must preserve the physical user's workspace focus")
        }
        _ => Ok(()),
    }
}

fn validate_targeted_key_combo(
    request: &DaemonRequest,
    preference: InputBackendPreference,
) -> Result<()> {
    let DaemonRequest::KeyCombo(request) = request else {
        return Ok(());
    };
    if preference == InputBackendPreference::KwinAgentSeat
        && request.session_id.is_some()
        && (request.destructive || destructive_key_combo(&request.combo))
    {
        bail!(
            "destructive or window-global key combinations are not target-safe on an independent agent seat; use close_window with the exact retained session and KWin window id"
        );
    }
    Ok(())
}

fn set_panic_stop(
    panic_stop: &PanicStopState,
    request: SetPanicStopRequest,
) -> Result<PanicStopStatus> {
    panic_stop.set_enabled(request.enabled)
}

fn current_capabilities(
    input_backend_preference: InputBackendPreference,
    stored_session_active: bool,
    agent_seat_ready: bool,
    window_resize_ready: bool,
    window_move_ready: bool,
    window_launch_ready: bool,
    window_close_ready: bool,
) -> Vec<BackendCapability> {
    let mut capabilities = vec![
        BackendCapability::DaemonHealth,
        BackendCapability::DaemonPolicyStatus,
        BackendCapability::DaemonSafetyStatus,
        BackendCapability::DaemonDesktopSessionStatus,
        BackendCapability::DaemonComputerUseReadiness,
    ];
    if command_exists("spectacle") || screenshot_portal_status().screenshot_interface_available {
        capabilities.push(BackendCapability::Screenshot);
    }
    if command_exists("qdbus6") {
        capabilities.push(BackendCapability::MonitorMetadata);
        capabilities.push(BackendCapability::WindowList);
        capabilities.push(BackendCapability::WindowFocus);
        if window_resize_ready {
            capabilities.push(BackendCapability::WindowResize);
        }
        if window_move_ready {
            capabilities.push(BackendCapability::WindowMove);
        }
        if window_launch_ready {
            capabilities.push(BackendCapability::WindowLaunch);
        }
        if window_close_ready {
            capabilities.push(BackendCapability::WindowClose);
        }
    }
    if clipboard::available() {
        capabilities.push(BackendCapability::ClipboardText);
    }
    if raw_input_capability_available(
        input_backend_preference,
        stored_session_active,
        agent_seat_ready,
    ) {
        capabilities.push(BackendCapability::KeyboardInput);
        capabilities.push(BackendCapability::PointerInput);
    }
    if command_exists("busctl") && seatgeist_atspi::available() {
        capabilities.push(BackendCapability::AccessibilityTree);
        capabilities.push(BackendCapability::SemanticActions);
    }
    capabilities
}

fn raw_input_capability_available(
    input_backend_preference: InputBackendPreference,
    stored_session_active: bool,
    agent_seat_ready: bool,
) -> bool {
    match input_backend_preference {
        InputBackendPreference::Auto | InputBackendPreference::Uinput => {
            seatgeist_uinput::available()
        }
        InputBackendPreference::PortalRemoteDesktop | InputBackendPreference::Libei => {
            stored_session_active
        }
        InputBackendPreference::KwinAgentSeat => agent_seat_ready,
    }
}

async fn resolve_interaction_window(
    window_id: &str,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
) -> Result<WindowInfo> {
    let window = window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .find(|window| window.id == window_id)
        .ok_or_else(|| {
            anyhow::anyhow!("interaction target lost: requested window does not exist")
        })?;
    enforce_app_policy_for_app(
        app_policy,
        window.app_id.as_deref(),
        "pinned interaction target",
    )?;
    if window.pid.is_none() {
        bail!("interaction target lost: requested window has no process id");
    }
    Ok(window)
}

fn merge_interaction_status(
    mut capture: libseatgeist::CaptureSessionStatus,
    interaction: interaction::InteractionStatus,
) -> libseatgeist::CaptureSessionStatus {
    let same_session = interaction.session_id.as_deref() == capture.session_id.as_deref();
    capture.sticky_target_bound = same_session && interaction.bound;
    if same_session {
        capture.target_window_id = interaction.window_id;
        capture.target_app_id = interaction.app_id;
        capture.target_pid = interaction.pid;
        capture.target_expires_in_ms = interaction.expires_in_ms;
    }
    capture
}

async fn execute_capture_open(
    request: CaptureOpenRequest,
    client: Option<&JournalClientContext>,
    runtime: &DaemonRuntime,
) -> DaemonResponse {
    let owner = match SessionOwner::from_client(client) {
        Ok(owner) => owner,
        Err(err) => return daemon_error_with_kind(err, ErrorKind::SessionOwnerMismatch),
    };
    let sticky_target = if request.source == CaptureSourceKind::Window {
        match request.requested_source_id.as_deref() {
            Some(window_id) => match resolve_interaction_window(
                window_id,
                runtime.window_backend.as_ref(),
                &runtime.app_policy,
            )
            .await
            {
                Ok(window) => Some(window),
                Err(err) => return daemon_error(err),
            },
            None => None,
        }
    } else {
        None
    };
    let open_result = capture_open(
        request,
        owner.clone(),
        &runtime.capture_session_store,
        runtime.screen_backend.as_ref(),
        runtime.safety_settings.preview_max_edge,
    )
    .await;
    match open_result {
        Ok(status) => {
            let Some(session_id) = status.session_id.clone() else {
                return daemon_error(anyhow::anyhow!(
                    "capture session opened without a session id"
                ));
            };
            let Some(capture_backend) = status.backend.clone() else {
                return daemon_error(anyhow::anyhow!(
                    "capture session opened without backend metadata"
                ));
            };
            runtime
                .session_execution_store
                .open(session_id.clone(), capture_backend, sticky_target.is_some())
                .await;
            if let Some(window) = sticky_target {
                let active_session_ids = runtime.capture_session_store.active_session_ids().await;
                runtime
                    .interaction_session_store
                    .retain_active_capture_sessions(&active_session_ids)
                    .await;
                if let Err(err) = runtime
                    .interaction_session_store
                    .bind(session_id.clone(), &window, owner)
                    .await
                {
                    let cleanup = runtime
                        .capture_session_store
                        .close(CaptureSessionRequest {
                            session_id: session_id.clone(),
                        })
                        .await;
                    let _ = runtime.session_execution_store.clear(&session_id).await;
                    if let Err(cleanup_error) = cleanup {
                        return daemon_error(err.context(format!(
                            "could not close rejected capture session: {cleanup_error}"
                        )));
                    }
                    return daemon_error(err);
                }
                if let Err(err) = runtime
                    .session_execution_store
                    .record_target_policy(&session_id, "allow")
                    .await
                {
                    return daemon_error(err);
                }
            }
            let capture = runtime
                .capture_session_store
                .status_for_session(&session_id)
                .await;
            let interaction = runtime.interaction_session_store.status(&session_id).await;
            DaemonResponse::CaptureSessionStatus(merge_interaction_status(capture, interaction))
        }
        Err(err) => daemon_error(err),
    }
}

async fn capture_status(
    runtime: &DaemonRuntime,
    client: Option<&JournalClientContext>,
) -> libseatgeist::CaptureSessionStatus {
    let mut capture = match SessionOwner::from_client(client) {
        Ok(owner) if owner.tool() == Some("seatgeist-cli") => {
            runtime.capture_session_store.status().await
        }
        Ok(owner) => runtime.capture_session_store.status_for_owner(&owner).await,
        Err(_) => runtime.capture_session_store.status().await,
    };
    let session_id = capture.session_id.clone().unwrap_or_default();
    let mut interaction = runtime.interaction_session_store.status(&session_id).await;
    if capture.active
        && interaction.bound
        && let Ok(windows) = runtime.window_backend.list_windows().await
        && runtime
            .interaction_session_store
            .clear_if_target_invalid(&session_id, &windows)
            .await
            .unwrap_or(false)
    {
        interaction = runtime.interaction_session_store.status(&session_id).await;
    }
    if let Some(session_id) = capture.session_id.as_deref() {
        capture.execution = runtime
            .session_execution_store
            .status(session_id)
            .await
            .map(Box::new);
    }
    merge_interaction_status(capture, interaction)
}

async fn focus_window(
    request: FocusWindowRequest,
    window_backend: &dyn WindowBackend,
) -> Result<ActionResult> {
    if request.window_id.trim().is_empty() {
        bail!("window id must not be empty");
    }
    window_backend
        .focus_window(request.window_id.clone())
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!("focused window {}", request.window_id)),
    })
}

async fn close_window(
    request: CloseWindowRequest,
    runtime: &DaemonRuntime,
) -> Result<ActionResult> {
    if request.window_id.trim().is_empty() {
        bail!("window id must not be empty");
    }
    let windows_before = runtime
        .window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)?;
    let target = if let Some(session_id) = request.session_id.as_deref() {
        runtime
            .capture_session_store
            .require_active(session_id)
            .await?;
        let pinned = runtime
            .interaction_session_store
            .resolve(session_id, &windows_before)
            .await?;
        if pinned.window.id != request.window_id {
            bail!(
                "close target mismatch: retained session is pinned to {}, not {}",
                pinned.window.id,
                request.window_id
            );
        }
        pinned.window
    } else {
        windows_before
            .iter()
            .find(|window| window.id == request.window_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("close target window was not found"))?
    };
    enforce_app_policy_for_app(
        &runtime.app_policy,
        target.app_id.as_deref(),
        "exact close target",
    )?;
    runtime
        .window_backend
        .close_window(target.id.clone())
        .await
        .map_err(anyhow::Error::msg)?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let windows_after = loop {
        let windows = runtime
            .window_backend
            .list_windows()
            .await
            .map_err(anyhow::Error::msg)?;
        if !windows.iter().any(|window| window.id == target.id) {
            break windows;
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "exact close was not confirmed: target KWin window {} remains present",
                target.id
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    if let Some(session_id) = request.session_id.as_deref() {
        runtime
            .interaction_session_store
            .clear_if_present(session_id)
            .await;
    }
    let active_window = runtime
        .window_backend
        .active_window()
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: Some(Observation {
            active_window,
            target_window: Some(target.clone()),
            windows: windows_after,
            monitors: Vec::new(),
            focused_accessibility: None,
            target_accessibility: None,
            screenshot_path: None,
            revision: None,
            issues: Vec::new(),
            settle: None,
        }),
        screenshot: None,
        message: Some(format!(
            "closed exact window {} app={} pid={} backend={} confirmation=target_absent",
            target.id,
            target.app_id.as_deref().unwrap_or("unknown"),
            target
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            runtime.window_backend.backend_name(),
        )),
    })
}

async fn move_window(
    request: MoveWindowRequest,
    window_backend: &dyn WindowBackend,
) -> Result<ActionResult> {
    if request.window_id.trim().is_empty() {
        bail!("window id must not be empty");
    }
    let geometry = window_backend
        .move_window(request.window_id.clone(), request.x, request.y)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "moved window {} requested={},{} actual={},{} size={}x{} backend={}",
            request.window_id,
            request.x,
            request.y,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            window_backend.backend_name()
        )),
    })
}

fn normalize_desktop_entry(value: &str) -> Result<String> {
    let value = value.trim();
    let value = value.strip_suffix(".desktop").unwrap_or(value);
    if value.is_empty() || value.len() > 255 {
        bail!("desktop entry id must contain between 1 and 255 characters");
    }
    if value.starts_with('.')
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        bail!("desktop entry id may contain only ASCII letters, digits, '.', '_' and '-'");
    }
    Ok(value.to_string())
}

async fn launch_window(
    request: LaunchWindowRequest,
    runtime: &DaemonRuntime,
) -> Result<ActionResult> {
    let desktop_entry = normalize_desktop_entry(&request.desktop_entry)?;
    if request
        .monitor_id
        .as_ref()
        .is_some_and(|id| id.trim().is_empty())
    {
        bail!("monitor id must not be empty");
    }
    for (label, value) in [("width", request.width), ("height", request.height)] {
        if let Some(value) = value
            && !(64..=32_768).contains(&value)
        {
            bail!("launch window {label} must be between 64 and 32768 logical pixels");
        }
    }
    if request.margin > 32_768 {
        bail!("launch window margin must be at most 32768 logical pixels");
    }
    if !(1_000..=30_000).contains(&request.timeout_ms) {
        bail!("launch window timeout must be between 1000 and 30000 milliseconds");
    }

    let ticket = runtime
        .window_action_queue
        .arm_launch_window(
            &desktop_entry,
            request.anchor,
            request.monitor_id.as_deref(),
            request.width,
            request.height,
            request.margin,
            request.activation,
            request.timeout_ms,
        )
        .await?;
    let launch_id = ticket.id().to_string();
    let mut child = match std::process::Command::new("gtk-launch")
        .arg(&desktop_entry)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            runtime
                .window_action_queue
                .cancel_launch_window(&launch_id)?;
            return Err(err).context("start desktop entry through gtk-launch");
        }
    };
    let launcher_status = tokio::task::spawn_blocking(move || child.wait());
    if let Ok(joined) = tokio::time::timeout(Duration::from_secs(2), launcher_status).await {
        let status = joined.context("join gtk-launch status task")??;
        if !status.success() {
            runtime
                .window_action_queue
                .cancel_launch_window(&launch_id)?;
            bail!("gtk-launch rejected desktop entry {desktop_entry}");
        }
    }

    let outcome = runtime
        .window_action_queue
        .finish_launch_window(ticket, Duration::from_millis(request.timeout_ms + 1_000))
        .await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let active_window = runtime.active_window_state.snapshot()?.flatten();
    let mut windows = runtime.window_list_state.snapshot()?.unwrap_or_default();
    if !windows.iter().any(|window| window.id == outcome.window.id) {
        windows.push(outcome.window.clone());
    }
    let geometry = outcome
        .window
        .geometry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("launch confirmation omitted window geometry"))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: Some(Observation {
            active_window,
            target_window: Some(outcome.window.clone()),
            windows,
            monitors: Vec::new(),
            focused_accessibility: None,
            target_accessibility: None,
            screenshot_path: None,
            revision: None,
            issues: Vec::new(),
            settle: None,
        }),
        screenshot: None,
        message: Some(format!(
            "launched desktop entry {} window={} position={},{} size={}x{} anchor={:?} activation={:?} focus_preserved={} backend=kwin_script_bridge",
            desktop_entry,
            outcome.window.id,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            request.anchor,
            request.activation,
            outcome.focus_preserved,
        )),
    })
}

async fn resize_window(
    request: ResizeWindowRequest,
    window_backend: &dyn WindowBackend,
) -> Result<ActionResult> {
    if request.window_id.trim().is_empty() {
        bail!("window id must not be empty");
    }
    if request.width < 64 || request.height < 64 {
        bail!("window width and height must each be at least 64 logical pixels");
    }
    if request.width > 32_768 || request.height > 32_768 {
        bail!("window width and height must each be at most 32768 logical pixels");
    }
    let geometry = window_backend
        .resize_window(request.window_id.clone(), request.width, request.height)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "resized window {} requested={}x{} actual={}x{} position={},{} backend={}",
            request.window_id,
            request.width,
            request.height,
            geometry.width,
            geometry.height,
            geometry.x,
            geometry.y,
            window_backend.backend_name()
        )),
    })
}

async fn focused_accessibility_tree_bounded(
    request: FocusedAccessibilityTreeRequest,
    timeout: Duration,
) -> Result<Option<libseatgeist::AccessibilityNode>> {
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }
    tokio::task::spawn_blocking(move || {
        seatgeist_atspi::focused_tree_bounded(request.depth, request.max_nodes, timeout)
            .map_err(|err| anyhow::anyhow!(err))
    })
    .await
    .context("join bounded AT-SPI focused-tree worker")?
}

fn accessibility_find(
    request: AccessibilityFindRequest,
) -> Result<Vec<libseatgeist::AccessibilityNode>> {
    seatgeist_atspi::find(request).map_err(|err| anyhow::anyhow!(err))
}

fn accessibility_find_with_context(
    request: AccessibilityFindRequest,
) -> Result<Vec<seatgeist_atspi::AccessibilityMatch>> {
    seatgeist_atspi::find_with_context(request).map_err(|err| anyhow::anyhow!(err))
}

#[derive(Default)]
struct AccessibilityQualityCounts {
    sampled_node_count: usize,
    named_node_count: usize,
    actionable_node_count: usize,
    text_node_count: usize,
    sensitive_node_count: usize,
    generic_role_count: usize,
    max_depth_seen: usize,
}

async fn accessibility_quality_status() -> AccessibilityQualityStatus {
    let sample_depth = ACCESSIBILITY_QUALITY_SAMPLE_DEPTH;
    let sample_max_nodes = ACCESSIBILITY_QUALITY_SAMPLE_MAX_NODES;
    let sample = focused_accessibility_tree_bounded(
        FocusedAccessibilityTreeRequest {
            depth: sample_depth,
            max_nodes: sample_max_nodes,
        },
        ACCESSIBILITY_QUALITY_TIMEOUT,
    )
    .await;
    let status = match sample {
        Err(err) => accessibility_quality_unavailable_status(
            sample_depth,
            sample_max_nodes,
            Some(&err.to_string()),
        ),
        sample => accessibility_quality_status_from_sample(sample_depth, sample_max_nodes, sample),
    };
    with_accessibility_registry_diagnostics(status)
}

fn with_accessibility_registry_diagnostics(
    mut status: AccessibilityQualityStatus,
) -> AccessibilityQualityStatus {
    let count = current_euid()
        .ok()
        .and_then(|uid| accessibility_registry_process_count(Path::new("/proc"), uid));
    status.registry_process_count = count;
    status.extra_registry_process_count = count.map(|count| count.saturating_sub(1));
    if let Some(extra) = status
        .extra_registry_process_count
        .filter(|extra| *extra > 0)
    {
        status.setup_hint.push_str(&format!(
            "; detected {extra} extra same-user AT-SPI registry process(es), so stale accessibility bus generations may be present"
        ));
    }
    status
}

fn accessibility_registry_process_count(proc_root: &Path, uid: u32) -> Option<usize> {
    let entries = fs::read_dir(proc_root).ok()?;
    let mut count = 0;
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .chars()
            .all(|character| character.is_ascii_digit())
        {
            continue;
        }
        let process_dir = entry.path();
        let comm = match fs::read_to_string(process_dir.join("comm")) {
            Ok(comm) => comm,
            Err(_) => continue,
        };
        if !comm.trim().starts_with("at-spi2-registr") {
            continue;
        }
        let status = match fs::read_to_string(process_dir.join("status")) {
            Ok(status) => status,
            Err(_) => continue,
        };
        let process_uid = status.lines().find_map(|line| {
            line.strip_prefix("Uid:")
                .and_then(|ids| ids.split_whitespace().next())
                .and_then(|id| id.parse::<u32>().ok())
        });
        if process_uid == Some(uid) {
            count += 1;
        }
    }
    Some(count)
}

fn accessibility_quality_unavailable_status(
    sample_depth: usize,
    sample_max_nodes: usize,
    reason: Option<&str>,
) -> AccessibilityQualityStatus {
    let setup_hint = match reason {
        Some(reason) => format!(
            "AT-SPI Registry is unreachable ({reason}); check or restart the user accessibility bus before semantic UI control"
        ),
        None => "AT-SPI Registry is unreachable; check or restart the user accessibility bus before semantic UI control".to_string(),
    };
    AccessibilityQualityStatus {
        atspi_available: false,
        registry_process_count: None,
        extra_registry_process_count: None,
        target_event_settle_available: false,
        event_backend: "atspi_registry".to_string(),
        target_event_classes: target_event_classes(),
        focused_node_present: false,
        sample_depth,
        sample_max_nodes,
        sampled_node_count: 0,
        named_node_count: 0,
        actionable_node_count: 0,
        text_node_count: 0,
        sensitive_node_count: 0,
        generic_role_count: 0,
        max_depth_seen: 0,
        tree_flat: false,
        semantic_targeting_reliable: false,
        recommended_fallback: "desktop_session_status".to_string(),
        setup_hint,
    }
}

fn accessibility_quality_status_from_sample(
    sample_depth: usize,
    sample_max_nodes: usize,
    focused: Result<Option<libseatgeist::AccessibilityNode>>,
) -> AccessibilityQualityStatus {
    let focused = match focused {
        Ok(focused) => focused,
        Err(err) => {
            return AccessibilityQualityStatus {
                atspi_available: true,
                registry_process_count: None,
                extra_registry_process_count: None,
                target_event_settle_available: true,
                event_backend: "atspi_registry".to_string(),
                target_event_classes: target_event_classes(),
                focused_node_present: false,
                sample_depth,
                sample_max_nodes,
                sampled_node_count: 0,
                named_node_count: 0,
                actionable_node_count: 0,
                text_node_count: 0,
                sensitive_node_count: 0,
                generic_role_count: 0,
                max_depth_seen: 0,
                tree_flat: false,
                semantic_targeting_reliable: false,
                recommended_fallback: "screenshot_tile_or_structured_integration".to_string(),
                setup_hint: format!(
                    "AT-SPI is reachable but focused-tree sampling failed: {}",
                    format_error_chain(&err)
                ),
            };
        }
    };

    let Some(root) = focused else {
        return AccessibilityQualityStatus {
            atspi_available: true,
            registry_process_count: None,
            extra_registry_process_count: None,
            target_event_settle_available: true,
            event_backend: "atspi_registry".to_string(),
            target_event_classes: target_event_classes(),
            focused_node_present: false,
            sample_depth,
            sample_max_nodes,
            sampled_node_count: 0,
            named_node_count: 0,
            actionable_node_count: 0,
            text_node_count: 0,
            sensitive_node_count: 0,
            generic_role_count: 0,
            max_depth_seen: 0,
            tree_flat: false,
            semantic_targeting_reliable: false,
            recommended_fallback: "focus_target_window_or_screenshot".to_string(),
            setup_hint: "AT-SPI is reachable but no focused accessibility node was found; focus the target app or use screenshot/window diagnostics first".to_string(),
        };
    };

    let mut counts = AccessibilityQualityCounts::default();
    collect_accessibility_quality_counts(&root, 0, &mut counts);
    let semantic_signal_count =
        counts.named_node_count + counts.actionable_node_count + counts.text_node_count;
    let tree_flat = counts.sampled_node_count <= 1 || counts.max_depth_seen == 0;
    let mostly_generic =
        counts.sampled_node_count > 0 && counts.generic_role_count == counts.sampled_node_count;
    let semantic_targeting_reliable = !tree_flat && semantic_signal_count > 0 && !mostly_generic;
    let recommended_fallback = if semantic_targeting_reliable {
        "atspi_semantic"
    } else if semantic_signal_count > 0 {
        "atspi_find_with_screenshot_confirmation"
    } else {
        "screenshot_tile_or_structured_integration"
    };
    let setup_hint = if semantic_targeting_reliable {
        "AT-SPI tree has names/actions/text in a non-flat subtree; prefer semantic actions before pixel fallback".to_string()
    } else if tree_flat {
        "AT-SPI tree is flat or only exposes the focused node; use semantic actions cautiously and confirm with screenshot or structured app integration".to_string()
    } else if mostly_generic {
        "AT-SPI tree is mostly generic roles; prefer screenshot, app API, or explicit pointer fallback for this surface".to_string()
    } else {
        "AT-SPI tree has limited semantic signals; confirm targets with screenshot or structured app integration before control".to_string()
    };

    AccessibilityQualityStatus {
        atspi_available: true,
        registry_process_count: None,
        extra_registry_process_count: None,
        target_event_settle_available: true,
        event_backend: "atspi_registry".to_string(),
        target_event_classes: target_event_classes(),
        focused_node_present: true,
        sample_depth,
        sample_max_nodes,
        sampled_node_count: counts.sampled_node_count,
        named_node_count: counts.named_node_count,
        actionable_node_count: counts.actionable_node_count,
        text_node_count: counts.text_node_count,
        sensitive_node_count: counts.sensitive_node_count,
        generic_role_count: counts.generic_role_count,
        max_depth_seen: counts.max_depth_seen,
        tree_flat,
        semantic_targeting_reliable,
        recommended_fallback: recommended_fallback.to_string(),
        setup_hint,
    }
}

fn target_event_classes() -> Vec<String> {
    ["object", "window", "focus"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn collect_accessibility_quality_counts(
    node: &libseatgeist::AccessibilityNode,
    depth: usize,
    counts: &mut AccessibilityQualityCounts,
) {
    counts.sampled_node_count += 1;
    counts.max_depth_seen = counts.max_depth_seen.max(depth);
    if node
        .name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
    {
        counts.named_node_count += 1;
    }
    if !node.actions.is_empty() || !node.available_actions.is_empty() {
        counts.actionable_node_count += 1;
    }
    if is_accessibility_text_role(&node.role) {
        counts.text_node_count += 1;
    }
    if node.sensitive {
        counts.sensitive_node_count += 1;
    }
    if is_generic_accessibility_role(&node.role) {
        counts.generic_role_count += 1;
    }
    for child in &node.children {
        collect_accessibility_quality_counts(child, depth + 1, counts);
    }
}

fn is_accessibility_text_role(role: &str) -> bool {
    let role = role.to_ascii_lowercase();
    role.contains("text")
        || role.contains("entry")
        || role.contains("paragraph")
        || role.contains("document")
}

fn is_generic_accessibility_role(role: &str) -> bool {
    matches!(
        role.to_ascii_lowercase().as_str(),
        "unknown" | "filler" | "panel" | "section" | "layer" | "canvas"
    )
}

fn accessibility_text_attributes(
    request: AccessibilityTextAttributesRequest,
) -> Result<libseatgeist::AccessibilityTextAttributes> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.offset < 0 {
        bail!("offset must be greater than or equal to zero");
    }
    seatgeist_atspi::text_attributes(&request.node_id, request.offset, request.include_defaults)
        .map_err(|err| anyhow::anyhow!(err))
}

fn accessibility_invoke(request: AccessibilityInvokeRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    seatgeist_atspi::invoke(&request.node_id, request.action.clone())
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
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
    seatgeist_atspi::set_text(&request.node_id, &request.text)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "set accessibility text length={} node={}",
            request.text.chars().count(),
            request.node_id
        )),
    })
}

fn accessibility_insert_text(request: AccessibilityInsertTextRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.offset < 0 {
        bail!("offset must be greater than or equal to zero");
    }
    seatgeist_atspi::insert_text(&request.node_id, request.offset, &request.text)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "inserted accessibility text length={} offset={} node={}",
            request.text.chars().count(),
            request.offset,
            request.node_id
        )),
    })
}

fn accessibility_delete_text(request: AccessibilityDeleteTextRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.start_offset < 0 {
        bail!("start_offset must be greater than or equal to zero");
    }
    if request.end_offset <= request.start_offset {
        bail!("end_offset must be greater than start_offset");
    }
    seatgeist_atspi::delete_text(&request.node_id, request.start_offset, request.end_offset)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "deleted accessibility text range={}..{} node={}",
            request.start_offset, request.end_offset, request.node_id
        )),
    })
}

fn accessibility_copy_text(request: AccessibilityCopyTextRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.start_offset < 0 {
        bail!("start_offset must be greater than or equal to zero");
    }
    if request.end_offset <= request.start_offset {
        bail!("end_offset must be greater than start_offset");
    }
    seatgeist_atspi::copy_text(&request.node_id, request.start_offset, request.end_offset)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "copied accessibility text range={}..{} node={}",
            request.start_offset, request.end_offset, request.node_id
        )),
    })
}

fn accessibility_cut_text(request: AccessibilityCutTextRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.start_offset < 0 {
        bail!("start_offset must be greater than or equal to zero");
    }
    if request.end_offset <= request.start_offset {
        bail!("end_offset must be greater than start_offset");
    }
    seatgeist_atspi::cut_text(&request.node_id, request.start_offset, request.end_offset)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "cut accessibility text range={}..{} node={}",
            request.start_offset, request.end_offset, request.node_id
        )),
    })
}

fn accessibility_paste_text(request: AccessibilityPasteTextRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.offset < 0 {
        bail!("offset must be greater than or equal to zero");
    }
    seatgeist_atspi::paste_text(&request.node_id, request.offset)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "pasted accessibility clipboard text offset={} node={}",
            request.offset, request.node_id
        )),
    })
}

fn accessibility_set_caret(request: AccessibilitySetCaretRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.offset < 0 {
        bail!("offset must be greater than or equal to zero");
    }
    seatgeist_atspi::set_caret(&request.node_id, request.offset)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "set accessibility caret offset={} node={}",
            request.offset, request.node_id
        )),
    })
}

fn accessibility_set_selection(request: AccessibilitySetSelectionRequest) -> Result<ActionResult> {
    if request.node_id.trim().is_empty() {
        bail!("node_id must be non-empty");
    }
    if request.selection_num < 0 {
        bail!("selection_num must be greater than or equal to zero");
    }
    if request.start_offset < 0 {
        bail!("start_offset must be greater than or equal to zero");
    }
    if request.end_offset <= request.start_offset {
        bail!("end_offset must be greater than start_offset");
    }
    seatgeist_atspi::set_selection(
        &request.node_id,
        request.selection_num,
        request.start_offset,
        request.end_offset,
    )
    .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,

        screenshot: None,
        message: Some(format!(
            "set accessibility selection index={} range={}..{} node={}",
            request.selection_num, request.start_offset, request.end_offset, request.node_id
        )),
    })
}

async fn prepare_semantic_settle(
    target: &target::SemanticActionTarget,
    options: Option<&PostActionOptions>,
) -> Option<semantic_settle::PreparedSemanticSettle> {
    let (Some(event_target), Some(window)) = (&target.event_target, &target.window) else {
        return None;
    };
    match semantic_settle::prepare(event_target.clone(), window.clone(), options).await {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::warn!(%error, "AT-SPI event subscription unavailable; using polling settle fallback");
            None
        }
    }
}

async fn semantic_action_result(
    message: String,
    prepared: Option<semantic_settle::PreparedSemanticSettle>,
) -> ActionResult {
    ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: match prepared {
            Some(prepared) => Some(semantic_settle::finish(prepared).await),
            None => None,
        },
        screenshot: None,
        message: Some(message),
    }
}

async fn click_button(
    request: ClickButtonRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("button name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: Some("button".to_string()),
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 5,
        max_nodes: request.max_nodes,
    })?;
    let target = resolve_click_button_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::invoke(&target.id, libseatgeist::AccessibilityAction::Press)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "clicked button name={} node={}",
            target.name.as_deref().unwrap_or(name),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn set_text_field(
    request: SetTextFieldRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("text field name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let target = resolve_text_field_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::set_text(&target.id, &request.text).map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "set text field name={} length={} node={}",
            target.name.as_deref().unwrap_or(name),
            request.text.chars().count(),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn focus_text_field(
    request: FocusTextFieldRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("text field name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let target = resolve_focus_text_field_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::invoke(&target.id, libseatgeist::AccessibilityAction::Focus)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "focused text field name={} node={}",
            target.name.as_deref().unwrap_or(name),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn activate_tab(
    request: ActivateTabRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("tab name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let (target, action) = resolve_tab_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::invoke(&target.id, action.clone()).map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "activated tab name={} action={} node={}",
            target.name.as_deref().unwrap_or(name),
            action.as_str(),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn activate_link(
    request: ActivateLinkRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("link name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: Some("link".to_string()),
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let (target, action) = resolve_link_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::invoke(&target.id, action.clone()).map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "activated link name={} action={} node={}",
            target.name.as_deref().unwrap_or(name),
            action.as_str(),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn toggle_check(
    request: ToggleCheckRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("check name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let (target, action) = resolve_check_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let was_checked = node_checked_state(&target);
    if request
        .checked
        .is_some_and(|desired| desired == was_checked)
    {
        let observation = post_action
            .filter(|options| options.observe_after)
            .and_then(|options| {
                target.window.clone().map(|window| {
                    semantic_settle::unchanged_observation(window, target.node.clone(), options)
                })
            });
        return Ok(ActionResult {
            id: Uuid::new_v4(),
            ok: true,
            observation,
            screenshot: None,
            message: Some(format!(
                "check state already name={} checked={} node={}",
                target.name.as_deref().unwrap_or(name),
                was_checked,
                target.id
            )),
        });
    }

    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::invoke(&target.id, action.clone()).map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "toggled check name={} action={} previous_checked={} requested_checked={} node={}",
            target.name.as_deref().unwrap_or(name),
            action.as_str(),
            was_checked,
            request
                .checked
                .map(|checked| checked.to_string())
                .unwrap_or_else(|| "toggle".to_string()),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn set_value(
    request: SetValueRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("value control name must be non-empty");
    }
    if !request.value.is_finite() {
        bail!("value must be finite");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let target = resolve_value_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::set_current_value(&target.id, request.value)
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "set value name={} value={} previous_value={} node={}",
            target.name.as_deref().unwrap_or(name),
            request.value,
            target.value.as_deref().unwrap_or("unknown"),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn select_item(
    request: SelectItemRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let name = request.name.trim();
    if name.is_empty() {
        bail!("item name must be non-empty");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }

    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: None,
        name_contains: Some(name.to_string()),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: 0,
        max_results: 10,
        max_nodes: request.max_nodes,
    })?;
    let (target, action) = resolve_select_item_match(
        name,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::invoke(&target.id, action.clone()).map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "selected item name={} action={} node={}",
            target.name.as_deref().unwrap_or(name),
            action.as_str(),
            target.id
        ),
        prepared,
    )
    .await)
}

async fn select_menu(
    request: SelectMenuRequest,
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    post_action: Option<&PostActionOptions>,
) -> Result<ActionResult> {
    let path = normalize_semantic_path(&request.path);
    if path.is_empty() {
        bail!("menu path must contain at least one non-empty segment");
    }
    if request.max_nodes == 0 {
        bail!("max_nodes must be greater than zero");
    }
    let first = path[0].clone();
    let search_depth = path.len().saturating_add(2);
    let matches = accessibility_find_with_context(AccessibilityFindRequest {
        role: None,
        name_contains: Some(first),
        app: request.app.clone(),
        window_name_contains: request.window_name_contains.clone(),
        depth: search_depth,
        max_results: 20,
        max_nodes: request.max_nodes,
    })?;
    let (target, action) = resolve_menu_path_match(
        &path,
        matches
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect(),
    )?;
    let target = target::authorize_semantic_target(
        target,
        matches,
        request.target_guard.as_ref(),
        window_backend,
        app_policy,
    )
    .await?;
    let prepared = prepare_semantic_settle(&target, post_action).await;
    seatgeist_atspi::invoke(&target.id, action.clone()).map_err(|err| anyhow::anyhow!(err))?;
    Ok(semantic_action_result(
        format!(
            "selected menu path={} action={} node={}",
            path.join("/"),
            action.as_str(),
            target.id
        ),
        prepared,
    )
    .await)
}

fn resolve_click_button_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<libseatgeist::AccessibilityNode> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(|node| {
            node.actions
                .contains(&libseatgeist::AccessibilityAction::Press)
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

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous button match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn resolve_menu_path_match(
    path: &[String],
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<(
    libseatgeist::AccessibilityNode,
    libseatgeist::AccessibilityAction,
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

    let choices = semantic_menu_choice_summary(path, &candidates);
    bail!(
        "ambiguous menu path={} matched {} candidates; choices=[{choices}]",
        path.join("/"),
        candidates.len()
    );
}

fn resolve_tab_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<(
    libseatgeist::AccessibilityNode,
    libseatgeist::AccessibilityAction,
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

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous tab match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn resolve_link_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<(
    libseatgeist::AccessibilityNode,
    libseatgeist::AccessibilityAction,
)> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(is_link_candidate)
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive activatable link matched name={name}");
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
        let action = link_activation_action(&node)
            .ok_or_else(|| anyhow::anyhow!("link has no press or select action"))?;
        return Ok((node, action));
    }

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous link match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn resolve_check_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<(
    libseatgeist::AccessibilityNode,
    libseatgeist::AccessibilityAction,
)> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(is_check_candidate)
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive activatable check matched name={name}");
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
        let action = check_activation_action(&node)
            .ok_or_else(|| anyhow::anyhow!("check has no press or select action"))?;
        return Ok((node, action));
    }

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous check match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn resolve_value_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<libseatgeist::AccessibilityNode> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(is_value_candidate)
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive writable value control matched name={name}");
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

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous value control match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn resolve_select_item_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<(
    libseatgeist::AccessibilityNode,
    libseatgeist::AccessibilityAction,
)> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(is_select_item_candidate)
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive selectable item matched name={name}");
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
        let action = select_item_activation_action(&node)
            .ok_or_else(|| anyhow::anyhow!("item has no select or press action"))?;
        return Ok((node, action));
    }

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous item match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn collect_menu_path_candidates(
    node: &libseatgeist::AccessibilityNode,
    path: &[String],
    index: usize,
    candidates: &mut Vec<(
        libseatgeist::AccessibilityNode,
        libseatgeist::AccessibilityAction,
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

fn node_name_matches(node: &libseatgeist::AccessibilityNode, name: &str) -> bool {
    node.name
        .as_deref()
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
}

fn menu_activation_action(
    node: &libseatgeist::AccessibilityNode,
) -> Option<libseatgeist::AccessibilityAction> {
    if !is_menu_item_candidate(node) {
        return None;
    }
    if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Select)
    {
        Some(libseatgeist::AccessibilityAction::Select)
    } else if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Press)
    {
        Some(libseatgeist::AccessibilityAction::Press)
    } else {
        None
    }
}

fn resolve_text_field_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<libseatgeist::AccessibilityNode> {
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

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous text field match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn resolve_focus_text_field_match(
    name: &str,
    matches: Vec<libseatgeist::AccessibilityNode>,
) -> Result<libseatgeist::AccessibilityNode> {
    let mut viable = matches
        .into_iter()
        .filter(|node| !node.sensitive)
        .filter(is_text_field_candidate)
        .filter(|node| {
            node.actions
                .contains(&libseatgeist::AccessibilityAction::Focus)
        })
        .collect::<Vec<_>>();
    if viable.is_empty() {
        bail!("no non-sensitive focusable text field matched name={name}");
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

    let choices = semantic_choice_summary(name, &viable);
    bail!(
        "ambiguous focusable text field match for name={name}: {} candidates; choices=[{choices}]",
        viable.len()
    );
}

fn semantic_choice_summary(query: &str, nodes: &[libseatgeist::AccessibilityNode]) -> String {
    let mut choices = nodes
        .iter()
        .take(SEMANTIC_CHOICE_LIMIT)
        .enumerate()
        .map(|(index, node)| semantic_node_choice(index + 1, query, node))
        .collect::<Vec<_>>();
    append_omitted_choice_count(&mut choices, nodes.len());
    choices.join("; ")
}

fn semantic_menu_choice_summary(
    path: &[String],
    candidates: &[(
        libseatgeist::AccessibilityNode,
        libseatgeist::AccessibilityAction,
    )],
) -> String {
    let query = path.last().map(String::as_str).unwrap_or_default();
    let mut choices = candidates
        .iter()
        .take(SEMANTIC_CHOICE_LIMIT)
        .enumerate()
        .map(|(index, (node, action))| {
            format!(
                "{} action={}",
                semantic_node_choice(index + 1, query, node),
                action.as_str()
            )
        })
        .collect::<Vec<_>>();
    append_omitted_choice_count(&mut choices, candidates.len());
    choices.join("; ")
}

fn append_omitted_choice_count(choices: &mut Vec<String>, total: usize) {
    if total > SEMANTIC_CHOICE_LIMIT {
        choices.push(format!("+{} more", total - SEMANTIC_CHOICE_LIMIT));
    }
}

fn semantic_node_choice(
    choice_index: usize,
    query: &str,
    node: &libseatgeist::AccessibilityNode,
) -> String {
    let name = node.name.as_deref().unwrap_or("<unnamed>");
    let score = semantic_name_match_score(query, node.name.as_deref());
    let actions = if node.actions.is_empty() {
        "none".to_string()
    } else {
        node.actions
            .iter()
            .map(libseatgeist::AccessibilityAction::as_str)
            .collect::<Vec<_>>()
            .join("|")
    };
    let candidate_id = semantic_candidate_id(choice_index, node);
    format!(
        "choice={choice_index} candidate_id={candidate_id} id={} role={} name={} score={score:.2} actions={actions}",
        node.id, node.role, name
    )
}

fn semantic_candidate_id(choice_index: usize, node: &libseatgeist::AccessibilityNode) -> String {
    let mut hash = FNV64_OFFSET_BASIS;
    fnv64_update(&mut hash, node.role.trim().to_ascii_lowercase().as_bytes());
    fnv64_update(&mut hash, b"\0");
    if let Some(name) = node.name.as_deref() {
        fnv64_update(&mut hash, name.trim().to_ascii_lowercase().as_bytes());
    }
    fnv64_update(&mut hash, b"\0");
    for action in &node.actions {
        fnv64_update(&mut hash, action.as_str().as_bytes());
        fnv64_update(&mut hash, b"\0");
    }
    format!("c{choice_index}-{hash:016x}")
}

const FNV64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x00000100000001b3;

fn fnv64_update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV64_PRIME);
    }
}

fn semantic_name_match_score(query: &str, candidate: Option<&str>) -> f64 {
    let query = query.trim().to_ascii_lowercase();
    let Some(candidate) = candidate else {
        return 0.0;
    };
    let candidate = candidate.trim().to_ascii_lowercase();
    if query.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    if candidate == query {
        return 1.0;
    }
    if candidate.starts_with(&query) {
        return 0.85;
    }
    if candidate.contains(&query) {
        return 0.65;
    }
    0.0
}

fn is_menu_item_candidate(node: &libseatgeist::AccessibilityNode) -> bool {
    matches!(
        node.role.to_ascii_lowercase().as_str(),
        "menu item" | "check menu item" | "radio menu item"
    )
}

fn is_select_item_candidate(node: &libseatgeist::AccessibilityNode) -> bool {
    matches!(
        node.role.to_ascii_lowercase().as_str(),
        "list item"
            | "tree item"
            | "table row"
            | "row"
            | "combo box"
            | "option"
            | "menu item"
            | "check menu item"
            | "radio menu item"
    ) && select_item_activation_action(node).is_some()
}

fn select_item_activation_action(
    node: &libseatgeist::AccessibilityNode,
) -> Option<libseatgeist::AccessibilityAction> {
    if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Select)
    {
        Some(libseatgeist::AccessibilityAction::Select)
    } else if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Press)
    {
        Some(libseatgeist::AccessibilityAction::Press)
    } else {
        None
    }
}

fn is_check_candidate(node: &libseatgeist::AccessibilityNode) -> bool {
    matches!(
        node.role.to_ascii_lowercase().as_str(),
        "check box" | "checkbox" | "check menu item" | "radio button" | "radio menu item"
    ) && check_activation_action(node).is_some()
}

fn check_activation_action(
    node: &libseatgeist::AccessibilityNode,
) -> Option<libseatgeist::AccessibilityAction> {
    if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Press)
    {
        Some(libseatgeist::AccessibilityAction::Press)
    } else if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Select)
    {
        Some(libseatgeist::AccessibilityAction::Select)
    } else {
        None
    }
}

fn is_link_candidate(node: &libseatgeist::AccessibilityNode) -> bool {
    matches!(node.role.to_ascii_lowercase().as_str(), "link")
        && link_activation_action(node).is_some()
}

fn link_activation_action(
    node: &libseatgeist::AccessibilityNode,
) -> Option<libseatgeist::AccessibilityAction> {
    if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Press)
    {
        Some(libseatgeist::AccessibilityAction::Press)
    } else if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Select)
    {
        Some(libseatgeist::AccessibilityAction::Select)
    } else {
        None
    }
}

fn node_checked_state(node: &libseatgeist::AccessibilityNode) -> bool {
    node.states.iter().any(|state| {
        let state = state.to_ascii_lowercase();
        matches!(state.as_str(), "checked" | "selected")
    })
}

fn is_value_candidate(node: &libseatgeist::AccessibilityNode) -> bool {
    matches!(
        node.role.to_ascii_lowercase().as_str(),
        "slider" | "spin button" | "scroll bar" | "dial"
    ) && node
        .value
        .as_deref()
        .is_some_and(|value| parse_node_value(value).is_some())
}

fn parse_node_value(value: &str) -> Option<f64> {
    let parsed = value.parse::<f64>().ok()?;
    parsed.is_finite().then_some(parsed)
}

fn is_tab_candidate(node: &libseatgeist::AccessibilityNode) -> bool {
    let role = node.role.to_ascii_lowercase();
    matches!(
        role.as_str(),
        "page tab" | "tab" | "tab item" | "page tab list item"
    ) && tab_activation_action(node).is_some()
}

fn tab_activation_action(
    node: &libseatgeist::AccessibilityNode,
) -> Option<libseatgeist::AccessibilityAction> {
    if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Select)
    {
        Some(libseatgeist::AccessibilityAction::Select)
    } else if node
        .actions
        .contains(&libseatgeist::AccessibilityAction::Press)
    {
        Some(libseatgeist::AccessibilityAction::Press)
    } else {
        None
    }
}

fn is_text_field_candidate(node: &libseatgeist::AccessibilityNode) -> bool {
    let role = node.role.to_ascii_lowercase();
    role == "text"
        || role == "entry"
        || role == "text input"
        || role == "editable text"
        || node
            .actions
            .contains(&libseatgeist::AccessibilityAction::SetText)
}

fn format_error_chain(err: &Error) -> String {
    err.chain()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

fn daemon_error(err: Error) -> DaemonResponse {
    let message = format_error_chain(&err);
    let kind = classify_error_message(&message);
    DaemonResponse::Error {
        kind,
        reason_code: Some(classify_error_reason(&message, kind).to_string()),
        message,
    }
}

fn daemon_error_with_kind(err: Error, kind: ErrorKind) -> DaemonResponse {
    let message = format_error_chain(&err);
    DaemonResponse::Error {
        kind,
        reason_code: Some(classify_error_reason(&message, kind).to_string()),
        message,
    }
}

fn classify_error_reason(message: &str, kind: ErrorKind) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("protected application") {
        "protected_application"
    } else if lower.contains("did not allow control of application") {
        "application_not_allowlisted"
    } else if lower.contains("no active capture session with that id")
        || lower.contains("capture session ended or is not active")
    {
        "capture_session_inactive"
    } else if lower.contains("capture open reservation was lost") {
        "capture_open_reservation_lost"
    } else if lower.contains("already has an opening or active capture session")
        || lower.contains("capture session is already opening or active")
    {
        "capture_session_already_active"
    } else if lower.contains("exact-window capture session quota") {
        "capture_session_quota"
    } else if lower.contains("capture revision is stale") {
        "capture_revision_stale"
    } else if lower.contains("capture_output") && lower.contains("requires session_id") {
        "capture_session_required"
    } else if lower.contains("capture_output") && lower.contains("requires capture_revision") {
        "capture_revision_required"
    } else if lower.contains("capture session has no captured frame") {
        "capture_frame_missing"
    } else if lower.contains("capture frame invalidated by user input") {
        "capture_frame_invalidated_by_user"
    } else if lower.contains("no non-sensitive activatable tab matched") {
        "semantic_target_not_actionable"
    } else if lower.contains("launch intent expired before a matching window appeared") {
        "launch_no_new_window"
    } else if lower.contains("capture_output pointer coordinate")
        && lower.contains("outside preview")
    {
        "capture_coordinate_out_of_bounds"
    } else if lower.contains("invalid coordinate transform") {
        "capture_transform_invalid"
    } else if lower.contains("invalid at-spi node id") {
        "invalid_accessibility_node_id"
    } else if lower.contains("clicks must be 1 or 2") {
        "invalid_click_count"
    } else if lower.contains("window_local pointer coordinate") && lower.contains("outside") {
        "pointer_coordinate_out_of_bounds"
    } else if lower.contains("agent-seat plugin did not complete") {
        "agent_seat_timeout"
    } else if lower.contains("agent-seat plugin is not registered") {
        "agent_seat_unavailable"
    } else if lower.contains("agent target in use") {
        "agent_target_in_use"
    } else if lower.contains("agent lane quota reached") {
        "agent_lane_quota"
    } else if lower.contains("agent target user active")
        || lower.contains("agent target received physical user input")
    {
        "agent_target_user_active"
    } else if lower.contains("requested window does not exist")
        || lower.contains("pinned window closed")
    {
        "window_not_found"
    } else if lower.contains("pinned window identity changed") {
        "window_identity_changed"
    } else if lower.contains("kwin script bridge dbus receiver is unavailable") {
        "kwin_bridge_unavailable"
    } else if lower.contains("kwin script bridge heartbeat is stale") {
        "kwin_bridge_stale"
    } else if lower.contains("channel closed") {
        "backend_channel_closed"
    } else if lower.contains("without geometry metadata")
        || lower.contains("without a window id")
        || lower.contains("omitted window geometry")
    {
        "backend_confirmation_incomplete"
    } else if kind == ErrorKind::Validation {
        "validation"
    } else if lower.contains("at-spi") || lower.contains("accessibility bus") {
        "atspi_registry_unreachable"
    } else {
        kind.as_str()
    }
}

fn policy_error_kind(err: &Error) -> ErrorKind {
    let message = format_error_chain(err);
    if message.starts_with("policy prompt required") {
        ErrorKind::PolicyPromptRequired
    } else if message.starts_with("policy denied") {
        ErrorKind::PolicyDenied
    } else {
        ErrorKind::Unknown
    }
}

fn classify_error_message(message: &str) -> ErrorKind {
    let lower = message.to_ascii_lowercase();
    if lower.contains("app policy") {
        ErrorKind::AppDenied
    } else if lower.contains("policy prompt required") {
        ErrorKind::PolicyPromptRequired
    } else if lower.contains("policy denied") {
        ErrorKind::PolicyDenied
    } else if lower.contains("target-window guard") || lower.contains("target-window correlation") {
        ErrorKind::TargetMismatch
    } else if lower.contains("session owner mismatch")
        || lower.contains("capture session owner requires")
        || lower.contains("active capture session has no owner")
    {
        ErrorKind::SessionOwnerMismatch
    } else if lower.contains("interaction target lost")
        || lower.contains("no active capture session with that id")
        || lower.contains("capture open reservation was lost")
    {
        ErrorKind::TargetLost
    } else if lower.contains("focus lease conflict")
        || lower.contains("agent target in use")
        || lower.contains("agent lane quota reached")
    {
        ErrorKind::FocusLeaseConflict
    } else if lower.contains("active-window guard") || lower.contains("focus guard") {
        ErrorKind::FocusGuard
    } else if lower.contains("human input activity")
        || lower.contains("agent target user active")
        || lower.contains("agent target received physical user input")
    {
        ErrorKind::HumanInputPause
    } else if lower.contains("capture frame invalidated by user input")
        || lower.contains("no non-sensitive activatable tab matched")
        || lower.contains("launch intent expired before a matching window appeared")
    {
        ErrorKind::Validation
    } else if lower.contains("panic-stop is active") {
        ErrorKind::PanicStop
    } else if lower.contains("rate limit") || lower.contains("rate-limited") {
        ErrorKind::RateLimited
    } else if (lower.contains("portal") || lower.contains("screencast consent"))
        && (lower.contains("cancelled")
            || lower.contains("canceled")
            || lower.contains("consent was cancelled")
            || lower.contains("consent was denied"))
    {
        ErrorKind::ConsentCancelled
    } else if lower.contains("xdg-desktop-portal remotedesktop is not available")
        || lower.contains("portal remotedesktop is not available")
        || lower.contains("portal screenshot target")
        || lower.contains("availabletargets")
    {
        ErrorKind::PortalUnavailable
    } else if lower.contains("at-spi")
        || lower.contains("accessibility bus")
        || lower.contains("accessibility tree")
    {
        if lower.contains("max_nodes exhausted")
            || lower.contains("generic")
            || lower.contains("flat")
            || lower.contains("weak")
        {
            ErrorKind::AccessibilityWeakTree
        } else {
            ErrorKind::AccessibilityUnavailable
        }
    } else if lower.contains("not available")
        || lower.contains("agent-seat plugin is not registered")
        || lower.contains("dbus receiver is unavailable")
        || lower.contains("bridge heartbeat is stale")
        || lower.contains("no executable")
        || lower.contains("requires a stored remotedesktop eis session")
        || lower.contains("/dev/uinput")
    {
        ErrorKind::BackendUnavailable
    } else if lower.contains("already has an opening or active capture session")
        || lower.contains("capture session is already opening or active")
        || lower.starts_with("invalid ")
        || lower.starts_with("unsupported ")
        || lower.starts_with("expected ")
        || lower.contains(" must ")
        || lower.contains(" out of bounds")
    {
        ErrorKind::Validation
    } else if lower.contains("failed")
        || lower.contains("timed out")
        || lower.contains("could not")
        || lower.contains("channel closed")
        || lower.contains("without geometry metadata")
        || lower.contains("without a window id")
        || lower.contains("omitted window geometry")
        || lower.contains("rejected desktop entry")
    {
        ErrorKind::BackendFailed
    } else {
        ErrorKind::Unknown
    }
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
        DaemonResponse::SafetyStatus(status) => format!(
            "safety focus_guard={} human_pause={} human_fresh={} activity_backend={} activity_trusted={} control_rate_limit_per_minute={} preview_max_edge={} tile_max_edge={} redactions={} journal_artifacts={}",
            status.require_focus_guard,
            status.pause_on_human_input,
            status.human_input_signal_fresh,
            status
                .human_input_activity_backend
                .as_deref()
                .unwrap_or("none"),
            status.human_input_activity_trusted,
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
            "desktop session type={} desktop={} dbus={} runtime={}",
            status.xdg_session_type.as_deref().unwrap_or("unknown"),
            status.xdg_current_desktop.as_deref().unwrap_or("unknown"),
            status.dbus_session_bus_address_present,
            status.xdg_runtime_dir_present
        ),
        DaemonResponse::ComputerUseReadiness(status) => format!(
            "computer_use_readiness observe={} screenshot={} window_control={} keyboard={} pointer={} semantic={} clipboard_read={} clipboard_write={} desktop_revision={} focus_guard={} panic_stop={} issues={} capture_backend={} input_backend={} a11y={}",
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
            "kwin bridge dbus={} ownership_retries={} ownership_retry_in_ms={} ownership_error={} active_update_seen={} active_age_ms={} window_list_update_seen={} window_list_age_ms={} stale={} window_count={} installed={} enabled={}",
            status.dbus_service_registered,
            status.ownership_retry_count,
            status
                .ownership_retry_in_ms
                .map(|delay| delay.to_string())
                .unwrap_or_else(|| "none".to_string()),
            status.ownership_last_error.is_some(),
            status.active_window_update_seen,
            status
                .active_window_update_age_ms
                .map(|age| age.to_string())
                .unwrap_or_else(|| "none".to_string()),
            status.window_list_update_seen,
            status
                .window_list_update_age_ms
                .map(|age| age.to_string())
                .unwrap_or_else(|| "none".to_string()),
            status.snapshot_stale,
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
            "input backends configured={} preferred={} implemented={} portal_remote_desktop={} libei={} uinput={}",
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
            status.uinput_available
        ),
        DaemonResponse::RemoteDesktopSessionProbe(status) => format!(
            "remote desktop session probe started={} requested={} selected={} clipboard={} transient_closed={}",
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
        DaemonResponse::CaptureSessionStatus(status) => format!(
            "capture_session active={} opening={} id={} backend={} source={} occlusion_possible={} requested_source={} requested_id={} owner_tool={} owner_scope={} owner_pid={} restore_ref={} revision={} sticky_target={} target_window={} expires_ms={} end_reason={}",
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
            status.last_end_reason.as_deref().unwrap_or("none")
        ),
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
        DaemonResponse::ActiveWindow(Some(window)) => {
            format!(
                "active window app={}",
                window.app_id.as_deref().unwrap_or("")
            )
        }
        DaemonResponse::ActiveWindow(None) => "no active window".to_string(),
        DaemonResponse::WindowInventory(inventory) => format!(
            "window inventory revision={} windows={} active={}",
            inventory.revision,
            inventory.windows.len(),
            inventory
                .active_window
                .as_ref()
                .map(|window| window.id.as_str())
                .unwrap_or("none")
        ),
        DaemonResponse::WindowInventoryWait(result) => format!(
            "window inventory wait changed={} timed_out={} elapsed_ms={} revision={} windows={}",
            result.changed,
            result.timed_out,
            result.elapsed_ms,
            result.inventory.revision,
            result.inventory.windows.len()
        ),
        DaemonResponse::Screenshot(info) => format!(
            "screenshot {}x{} from {}x{} backend={} occlusion_possible={} path={}",
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
            "accessibility quality atspi={} registries={} extra_registries={} focused={} reliable={} nodes={} named={} actionable={} text={} generic={} flat={} fallback={}",
            status.atspi_available,
            status
                .registry_process_count
                .map_or_else(|| "unknown".to_string(), |count| count.to_string()),
            status
                .extra_registry_process_count
                .map_or_else(|| "unknown".to_string(), |count| count.to_string()),
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
            "accessibility text attributes range={}..{} count={} node={}",
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
                .unwrap_or_else(|| format!("action {}", result.id));
            match result
                .observation
                .as_ref()
                .and_then(|observation| observation.settle.as_ref())
            {
                Some(settle) => format!(
                    "{message} settle={:?} backend={:?} target_scoped={} event={} settled={} timed_out={} samples={} elapsed_ms={}",
                    settle.condition,
                    settle.backend,
                    settle.target_scoped,
                    settle.event.as_deref().unwrap_or("none"),
                    settle.settled,
                    settle.timed_out,
                    settle.samples,
                    settle.elapsed_ms
                ),
                None => message,
            }
        }
        DaemonResponse::Error {
            kind,
            reason_code,
            message,
        } => format!(
            "error kind={kind:?} reason={}: {message}",
            reason_code.as_deref().unwrap_or("unspecified")
        ),
    }
}

fn journal_response_summary(response: &DaemonResponse, settings: &JournalSettings) -> String {
    match response {
        DaemonResponse::Error {
            kind, reason_code, ..
        } if !settings.include_error_details => {
            format!(
                "error kind={} reason={}",
                kind.as_str(),
                reason_code.as_deref().unwrap_or("unspecified")
            )
        }
        _ => summarize_response(response),
    }
}

fn unix_time_ms() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before Unix epoch")?;
    Ok(duration.as_millis().try_into().unwrap_or(u64::MAX))
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

fn validate_peer_client(stream: &UnixStream) -> Result<Option<JournalClientContext>> {
    let credentials = stream.peer_cred().context("read peer credentials")?;
    let peer_uid = credentials.uid();
    let daemon_uid = current_euid().context("read daemon uid")?;
    if peer_uid != daemon_uid {
        error!(peer_uid, daemon_uid, "rejecting client from different uid");
        bail!("peer uid {peer_uid} does not match daemon uid {daemon_uid}");
    }
    let pid = credentials
        .pid()
        .filter(|pid| *pid > 0)
        .and_then(|pid| u32::try_from(pid).ok());
    let process_name = pid.and_then(client_process_name);
    if pid.is_none() && process_name.is_none() {
        return Ok(None);
    }
    Ok(Some(JournalClientContext {
        tool: None,
        pid,
        process_name,
    }))
}

fn client_process_name(pid: u32) -> Option<String> {
    let path = PathBuf::from(format!("/proc/{pid}/comm"));
    fs::read_to_string(path)
        .ok()
        .and_then(|name| compact_client_process_name(&name))
}

fn compact_client_process_name(name: &str) -> Option<String> {
    const MAX_CLIENT_NAME_CHARS: usize = 64;
    let cleaned = name
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_CLIENT_NAME_CHARS)
        .collect::<String>();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn compact_client_tool_name(name: &str) -> Option<String> {
    const MAX_CLIENT_TOOL_CHARS: usize = 64;
    let cleaned = name
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_CLIENT_TOOL_CHARS)
        .collect::<String>();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, os::fd::RawFd};

    #[derive(Default)]
    struct MockEisSource {
        event_fd: RawFd,
        pending_batches: VecDeque<Vec<seatgeist_eis::LibeiEventSnapshot>>,
        plan_batches: VecDeque<Vec<seatgeist_eis::LibeiEventSnapshot>>,
        executed_plans: Vec<MockEisExecutedPlan>,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct MockEisExecutedPlan {
        selection: seatgeist_eis::EisDeviceSelection,
        events: Vec<seatgeist_eis::EisEvent>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum MockEisExecutionError {
        RejectedDevice,
    }

    impl Display for MockEisExecutionError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::RejectedDevice => {
                    write!(formatter, "mock EIS executor rejected selected device")
                }
            }
        }
    }

    impl MockEisSource {
        fn push_pending(&mut self, snapshots: Vec<seatgeist_eis::LibeiEventSnapshot>) {
            self.pending_batches.push_back(snapshots);
        }

        fn push_plan(&mut self, snapshots: Vec<seatgeist_eis::LibeiEventSnapshot>) {
            self.plan_batches.push_back(snapshots);
        }
    }

    impl seatgeist_eis::EisEventSource for MockEisSource {
        fn event_fd(&self) -> RawFd {
            self.event_fd
        }

        fn dispatch_pending(&mut self) -> Vec<seatgeist_eis::LibeiEventSnapshot> {
            self.pending_batches.pop_front().unwrap_or_default()
        }

        fn dispatch_pending_for_plan(
            &mut self,
            _plan: &seatgeist_eis::EisActionPlan,
        ) -> Vec<seatgeist_eis::LibeiEventSnapshot> {
            self.plan_batches.pop_front().unwrap_or_default()
        }
    }

    impl seatgeist_eis::EisSelectedDeviceExecutor for MockEisSource {
        type Error = MockEisExecutionError;

        fn apply_plan_to_selected_device(
            &mut self,
            selection: &seatgeist_eis::EisDeviceSelection,
            plan: &seatgeist_eis::EisActionPlan,
        ) -> std::result::Result<(), Self::Error> {
            if selection.device_id == "rejected" {
                return Err(MockEisExecutionError::RejectedDevice);
            }
            self.executed_plans.push(MockEisExecutedPlan {
                selection: selection.clone(),
                events: plan.events.clone(),
            });
            Ok(())
        }
    }

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
    fn window_list_state_accepts_kwin_payload() {
        let state = WindowListState::default();
        assert!(
            state
                .snapshot()
                .expect("initial snapshot succeeds")
                .is_none()
        );

        state
            .update_from_payload(
                r#"{
                    "windows": [
                        {
                            "id": "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}",
                            "title": "Konsole",
                            "app_id": "org.kde.konsole",
                            "pid": 1234,
                            "geometry": {"x": 10, "y": 20, "width": 800, "height": 600}
                        },
                        {
                            "id": "{7d6c2ae6-38a9-4a99-8b65-5bbd2aa6d7d4}",
                            "title": "Kate",
                            "app_id": "org.kde.kate",
                            "pid": 4321,
                            "geometry": {"x": 900, "y": 40, "width": 1200, "height": 900}
                        }
                    ]
                }"#,
            )
            .expect("payload updates window-list state");

        let windows = state
            .snapshot()
            .expect("state snapshot succeeds")
            .expect("bridge reported");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].title, "Konsole");
        assert_eq!(windows[0].pid, Some(1234));
        assert_eq!(
            windows[1].geometry.as_ref().map(|geometry| geometry.space),
            Some(CoordinateSpace::LogicalPixel)
        );
    }

    #[test]
    fn bridge_windows_enrich_runner_window_list() {
        let mut windows = vec![
            WindowInfo {
                id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                app_id: Some("utilities-terminal".to_string()),
                title: "Runner Konsole".to_string(),
                pid: None,
                monitor_id: None,
                geometry: None,
            },
            WindowInfo {
                id: "{runner-only}".to_string(),
                app_id: Some("org.example.RunnerOnly".to_string()),
                title: "Runner Only".to_string(),
                pid: None,
                monitor_id: None,
                geometry: None,
            },
        ];
        let bridge_windows = vec![
            WindowInfo {
                id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                app_id: Some("org.kde.konsole".to_string()),
                title: "Bridge Konsole".to_string(),
                pid: Some(1234),
                monitor_id: None,
                geometry: Some(WindowGeometry {
                    x: 10,
                    y: 20,
                    width: 800,
                    height: 600,
                    space: CoordinateSpace::LogicalPixel,
                }),
            },
            WindowInfo {
                id: "{bridge-only}".to_string(),
                app_id: Some("org.example.BridgeOnly".to_string()),
                title: "Bridge Only".to_string(),
                pid: Some(777),
                monitor_id: None,
                geometry: None,
            },
        ];

        merge_bridge_windows(&mut windows, bridge_windows);

        assert_eq!(windows.len(), 3);
        let enriched = windows
            .iter()
            .find(|window| window.id == "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}")
            .expect("matching runner window is preserved");
        assert_eq!(enriched.app_id.as_deref(), Some("org.kde.konsole"));
        assert_eq!(enriched.title, "Bridge Konsole");
        assert_eq!(enriched.pid, Some(1234));
        assert!(enriched.geometry.is_some());
        assert!(
            windows.iter().any(|window| window.id == "{runner-only}"),
            "runner-only fallback windows remain visible"
        );
        assert!(
            windows.iter().any(|window| window.id == "{bridge-only}"),
            "bridge-only windows are included"
        );
    }

    #[test]
    fn journal_active_window_context_is_control_only() {
        let state = ActiveWindowState::default();
        state
            .update_from_payload(
                r#"{
                    "active": true,
                    "id": "window-1",
                    "title": "Test Window",
                    "app_id": "org.kde.test",
                    "geometry": {"x": 10, "y": 20, "width": 800, "height": 600}
                }"#,
            )
            .expect("payload updates active-window state");

        assert!(
            active_window_context_for_safety_class(
                &SafetyClass::Observe,
                &state,
                &AppPolicy::default(),
            )
            .is_none(),
            "observe requests should not add active-window journal context"
        );

        let context = active_window_context_for_safety_class(
            &SafetyClass::ControlSemantic,
            &state,
            &AppPolicy::default(),
        )
        .expect("control requests include active-window context");
        assert_eq!(context.id, "window-1");
        assert_eq!(context.app_id.as_deref(), Some("org.kde.test"));
        assert_eq!(context.title, "Test Window");
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
        let path = temp_test_path("journal-test").with_extension("jsonl");
        let journal = ActionJournal::new(path.clone(), JournalSettings::default(), None);

        journal
            .record(
                "health",
                JournalContext {
                    client: Some(JournalClientContext {
                        tool: Some("seatgeist-cli".to_string()),
                        pid: Some(1111),
                        process_name: Some("seatgeist-cl".to_string()),
                    }),
                    safety_class: SafetyClass::Policy,
                    guard_present: false,
                    active_window_before: None,
                    active_window_after: None,
                    control: None,
                },
                &DaemonResponse::Health(HealthStatus {
                    service: "seatgeistd".to_string(),
                    version: "0.1.0".to_string(),
                    status: "ok".to_string(),
                    protocol_version: None,
                    run_id: None,
                    git_sha: None,
                    build_unix_ms: None,
                    binary_sha256: None,
                    config_fingerprint: None,
                    resident_memory_bytes: None,
                    resident_memory_peak_bytes: None,
                }),
            )
            .expect("health record appends");
        journal
            .record(
                "focus_window",
                JournalContext {
                    client: Some(JournalClientContext {
                        tool: Some("seatgeist-mcp".to_string()),
                        pid: Some(2222),
                        process_name: Some("seatgeist-mc".to_string()),
                    }),
                    safety_class: SafetyClass::ControlSemantic,
                    guard_present: true,
                    active_window_before: Some(JournalWindowContext {
                        id: "window-1".to_string(),
                        app_id: Some("org.kde.kate".to_string()),
                        title: "main.rs".to_string(),
                        monitor_id: Some("main".to_string()),
                    }),
                    active_window_after: Some(JournalWindowContext {
                        id: "window-2".to_string(),
                        app_id: Some("org.kde.konsole".to_string()),
                        title: "shell".to_string(),
                        monitor_id: Some("main".to_string()),
                    }),
                    control: Some(JournalControlContext {
                        action_id: None,
                        policy: None,
                        backend: Some("kwin".to_string()),
                        requested_target: Some({
                            let mut target = journal_target("window");
                            target.add("window_id", "window-2");
                            target
                        }),
                    }),
                },
                &DaemonResponse::Action(Box::new(ActionResult {
                    id: Uuid::nil(),
                    ok: true,
                    observation: None,

                    screenshot: None,
                    message: Some("focused window".to_string()),
                })),
            )
            .expect("focus record appends");

        let entries = journal
            .tail_filtered(1, None, None)
            .expect("journal tail succeeds");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sequence, 2);
        assert_eq!(entries[0].method, "focus_window");
        assert!(entries[0].artifacts.is_empty());
        assert_eq!(
            entries[0]
                .client
                .as_ref()
                .and_then(|client| client.tool.as_deref()),
            Some("seatgeist-mcp")
        );
        assert_eq!(
            entries[0]
                .client
                .as_ref()
                .and_then(|client| client.process_name.as_deref()),
            Some("seatgeist-mc")
        );
        assert_eq!(entries[0].safety_class, Some(SafetyClass::ControlSemantic));
        assert!(entries[0].guard_present);
        assert_eq!(
            entries[0]
                .active_window_before
                .as_ref()
                .and_then(|window| window.app_id.as_deref()),
            Some("org.kde.kate")
        );
        assert_eq!(
            entries[0]
                .active_window_after
                .as_ref()
                .and_then(|window| window.app_id.as_deref()),
            Some("org.kde.konsole")
        );
        let control = entries[0]
            .control
            .as_ref()
            .expect("control journal context is present");
        assert_eq!(control.action_id, Some(Uuid::nil()));
        assert_eq!(control.policy.as_deref(), Some("allow"));
        assert_eq!(control.backend.as_deref(), Some("kwin"));
        assert_eq!(
            control
                .requested_target
                .as_ref()
                .and_then(|target| target.fields.get("window_id"))
                .map(String::as_str),
            Some("window-2")
        );
        assert!(entries[0].ok);

        let entries = journal
            .tail_filtered(10, Some("health"), Some(true))
            .expect("filtered journal tail succeeds");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "health");
        assert_eq!(
            entries[0].client.as_ref().and_then(|client| client.pid),
            Some(1111)
        );
        assert!(entries[0].ok);

        fs::remove_file(&path).ok();
    }

    #[test]
    fn journal_redacts_error_details_unless_privately_enabled() {
        let response = DaemonResponse::Error {
            kind: ErrorKind::TargetMismatch,
            reason_code: Some("window_identity_changed".to_string()),
            message: "target Secret Project window uuid-123 disappeared".to_string(),
        };
        let context = || JournalContext {
            client: None,
            safety_class: SafetyClass::ControlSemantic,
            guard_present: true,
            active_window_before: None,
            active_window_after: None,
            control: None,
        };

        let private_default = temp_test_path("journal-redacted").with_extension("jsonl");
        ActionJournal::new(private_default.clone(), JournalSettings::default(), None)
            .record("click_button", context(), &response)
            .expect("redacted error journals");
        let entry = tail_journal_entries(&private_default, 1, None, None)
            .expect("redacted journal reads")
            .pop()
            .expect("redacted entry exists");
        assert_eq!(
            entry.summary,
            "error kind=target_mismatch reason=window_identity_changed"
        );
        assert!(!entry.summary.contains("Secret Project"));

        let diagnostic = temp_test_path("journal-diagnostic").with_extension("jsonl");
        ActionJournal::new(
            diagnostic.clone(),
            JournalSettings {
                include_artifact_metadata: false,
                include_error_details: true,
            },
            None,
        )
        .record("click_button", context(), &response)
        .expect("diagnostic error journals");
        let entry = tail_journal_entries(&diagnostic, 1, None, None)
            .expect("diagnostic journal reads")
            .pop()
            .expect("diagnostic entry exists");
        assert!(entry.summary.contains("Secret Project"));
        fs::remove_file(private_default).ok();
        fs::remove_file(diagnostic).ok();
    }

    #[test]
    fn actionable_readiness_prioritizes_policy_then_guard() {
        assert_eq!(
            action_readiness(true, false, &ToolApprovalLevel::Prompt, true),
            ActionReadiness::NeedsApproval
        );
        assert_eq!(
            action_readiness(true, false, &ToolApprovalLevel::Allow, true),
            ActionReadiness::NeedsGuard
        );
        assert_eq!(
            action_readiness(false, false, &ToolApprovalLevel::Allow, false),
            ActionReadiness::Unavailable
        );
    }

    #[test]
    fn journal_artifact_metadata_is_opt_in_and_hashes_written_files() {
        let path = temp_test_path("journal-artifact-test").with_extension("jsonl");
        let artifact_path = temp_test_path("journal-artifact").with_extension("png");
        fs::write(&artifact_path, b"abc").expect("artifact fixture is written");
        let mut screenshot = sample_screenshot_info("test");
        screenshot.path = artifact_path.clone();

        let disabled = ActionJournal::new(path.clone(), JournalSettings::default(), None);
        disabled
            .record(
                "screenshot",
                JournalContext {
                    client: None,
                    safety_class: SafetyClass::Observe,
                    guard_present: false,
                    active_window_before: None,
                    active_window_after: None,
                    control: None,
                },
                &DaemonResponse::Screenshot(screenshot.clone()),
            )
            .expect("disabled artifact record appends");
        let entries = disabled
            .tail_filtered(1, None, None)
            .expect("disabled journal tail succeeds");
        assert!(entries[0].artifacts.is_empty());

        let enabled = ActionJournal::new(
            path.clone(),
            JournalSettings {
                include_artifact_metadata: true,
                include_error_details: false,
            },
            None,
        );
        enabled
            .record(
                "screenshot",
                JournalContext {
                    client: None,
                    safety_class: SafetyClass::Observe,
                    guard_present: false,
                    active_window_before: None,
                    active_window_after: None,
                    control: None,
                },
                &DaemonResponse::Screenshot(screenshot),
            )
            .expect("enabled artifact record appends");
        let entries = enabled
            .tail_filtered(1, None, None)
            .expect("enabled journal tail succeeds");
        assert_eq!(entries[0].artifacts.len(), 1);
        let artifact = &entries[0].artifacts[0];
        assert_eq!(artifact.kind, "screenshot");
        assert_eq!(artifact.path, artifact_path);
        assert_eq!(artifact.bytes, Some(3));
        assert_eq!(
            artifact.sha256.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );

        let mut action_screenshot = sample_screenshot_info("test");
        action_screenshot.path = artifact_path.clone();
        let action_artifacts = journal_artifacts_for_response(
            &DaemonResponse::Action(Box::new(ActionResult {
                id: Uuid::nil(),
                ok: true,
                observation: None,
                screenshot: Some(action_screenshot),
                message: None,
            })),
            &JournalSettings {
                include_artifact_metadata: true,
                include_error_details: false,
            },
        );
        assert_eq!(action_artifacts.len(), 1);
        assert_eq!(action_artifacts[0].kind, "post_action_screenshot");

        fs::remove_file(&path).ok();
        fs::remove_file(&artifact_path).ok();
    }

    #[test]
    fn compact_client_process_name_removes_controls_and_bounds_length() {
        let name =
            compact_client_process_name("seatgeist-cli\n").expect("valid client name is retained");
        assert_eq!(name, "seatgeist-cli");

        let long = format!("{}{}", "a".repeat(80), "\n");
        let name = compact_client_process_name(&long).expect("long client name is retained");
        assert_eq!(name.len(), 64);
        assert!(compact_client_process_name("\n\t").is_none());
    }

    #[test]
    fn compact_client_tool_name_removes_controls_and_bounds_length() {
        let name =
            compact_client_tool_name("seatgeist-mcp\n").expect("valid tool name is retained");
        assert_eq!(name, "seatgeist-mcp");

        let long = format!("{}{}", "m".repeat(80), "\n");
        let name = compact_client_tool_name(&long).expect("long tool name is retained");
        assert_eq!(name.len(), 64);
        assert!(compact_client_tool_name("\n\t").is_none());
    }

    #[test]
    fn parse_daemon_request_line_accepts_legacy_and_enveloped_requests() {
        let (legacy, legacy_client, legacy_options) =
            parse_daemon_request_line(r#"{"method":"health"}"#).expect("legacy request parses");
        assert_eq!(legacy, DaemonRequest::Health);
        assert_eq!(legacy_client, None);
        assert_eq!(legacy_options, None);

        let (enveloped, client, response_options) = parse_daemon_request_line(
            r#"{"request":{"method":"health"},"client":{"tool":"seatgeist-mcp"},"response_options":{"post_action":{"observe_after":true,"settle_condition":"stable","settle_timeout_ms":1000,"settle_interval_ms":100}}}"#,
        )
        .expect("enveloped request parses");
        assert_eq!(enveloped, DaemonRequest::Health);
        assert_eq!(
            client.and_then(|client| client.tool),
            Some("seatgeist-mcp".to_string())
        );
        assert_eq!(
            response_options
                .and_then(|options| options.post_action)
                .map(|options| options.settle_condition),
            Some(ActionSettleCondition::Stable)
        );
    }

    #[test]
    fn post_action_conditions_distinguish_stability_and_change() {
        let observation = |id: &str| Observation {
            active_window: Some(WindowInfo {
                id: id.to_string(),
                app_id: Some("org.example.App".to_string()),
                title: "Window".to_string(),
                pid: Some(42),
                monitor_id: None,
                geometry: None,
            }),
            target_window: None,
            windows: Vec::new(),
            monitors: Vec::new(),
            focused_accessibility: None,
            target_accessibility: None,
            screenshot_path: None,
            revision: Some(id.to_string()),
            issues: Vec::new(),
            settle: None,
        };
        let before = observation("window-a");
        let same = observation("window-a");
        let changed = observation("window-b");

        assert!(post_action_condition_met(
            ActionSettleCondition::None,
            None,
            Some(&before),
            None,
            &same
        ));
        assert!(post_action_condition_met(
            ActionSettleCondition::Stable,
            None,
            Some(&before),
            Some(&before),
            &same
        ));
        assert!(post_action_condition_met(
            ActionSettleCondition::ActiveWindowChange,
            None,
            Some(&before),
            None,
            &changed
        ));
        assert!(post_action_condition_met(
            ActionSettleCondition::ActiveWindowChange,
            Some("window-a"),
            Some(&before),
            None,
            &same
        ));
        assert!(!post_action_condition_met(
            ActionSettleCondition::ActiveWindowChange,
            Some("window-c"),
            Some(&before),
            None,
            &changed
        ));
        assert!(post_action_condition_met(
            ActionSettleCondition::AnyChange,
            None,
            Some(&before),
            None,
            &changed
        ));
        assert!(!post_action_condition_met(
            ActionSettleCondition::AnyChange,
            None,
            Some(&before),
            None,
            &same
        ));
    }

    #[test]
    fn merge_client_context_keeps_trusted_peer_metadata_and_request_tool() {
        let merged = merge_client_context(
            Some(JournalClientContext {
                tool: None,
                pid: Some(1234),
                process_name: Some("seatgeist-mc".to_string()),
            }),
            Some(JournalClientContext {
                tool: Some("seatgeist-mcp".to_string()),
                pid: Some(9999),
                process_name: Some("spoofed".to_string()),
            }),
        )
        .expect("merged client context is present");

        assert_eq!(merged.tool.as_deref(), Some("seatgeist-mcp"));
        assert_eq!(merged.pid, Some(1234));
        assert_eq!(merged.process_name.as_deref(), Some("seatgeist-mc"));
    }

    #[test]
    fn journal_control_context_redacts_text_payloads() {
        let request = DaemonRequest::TypeText(TypeTextRequest {
            text: "secret text".to_string(),
            guard: Some(ActiveWindowGuard {
                desktop_revision: None,
                expected_window_id: None,
                expected_app_id: Some("org.kde.kate".to_string()),
                title_contains: None,
            }),
            session_id: None,
        });
        let context = journal_control_context_for_request(
            &request,
            &SafetyClass::ControlKeyboard,
            InputBackendPreference::Uinput,
        )
        .expect("keyboard control context is present");
        let target = context
            .requested_target
            .expect("keyboard target metadata is present");

        assert_eq!(context.backend.as_deref(), Some("uinput"));
        assert_eq!(target.kind, "keyboard_text");
        assert_eq!(
            target.fields.get("text_chars").map(String::as_str),
            Some("11")
        );
        assert!(
            !target.fields.values().any(|value| value.contains("secret")),
            "journal target metadata must not contain typed text"
        );
    }

    #[test]
    fn internal_focus_lease_steps_are_correlated_and_title_free() {
        let path = temp_test_path("sticky-focus-journal");
        let journal = ActionJournal::new(
            path.clone(),
            JournalSettings {
                include_artifact_metadata: false,
                include_error_details: false,
            },
            None,
        );
        let lease_id = Uuid::new_v4();
        let window = WindowInfo {
            id: "firefox-window-1".to_string(),
            app_id: Some("org.mozilla.firefox".to_string()),
            title: "Private browsing title".to_string(),
            pid: Some(4242),
            monitor_id: None,
            geometry: None,
        };
        journal
            .record_focus_lease_step(
                "interaction_focus",
                "capture-1",
                lease_id,
                &window,
                "kwin",
                true,
            )
            .expect("internal focus step journals");
        let entries = journal
            .tail_filtered(4, Some("interaction_focus"), Some(true))
            .expect("internal focus entry reads");

        assert_eq!(entries.len(), 1);
        let control = entries[0]
            .control
            .as_ref()
            .expect("control metadata exists");
        assert_eq!(control.action_id, Some(lease_id));
        assert_eq!(control.backend.as_deref(), Some("kwin"));
        let encoded = serde_json::to_string(&entries).expect("journal serializes");
        assert!(encoded.contains("capture-1"));
        assert!(encoded.contains("firefox-window-1"));
        assert!(!encoded.contains("Private browsing title"));
        fs::remove_file(path).ok();
    }

    #[test]
    fn independent_agent_seat_deliveries_are_correlated_and_title_free() {
        let path = temp_test_path("agent-seat-journal");
        let journal = ActionJournal::new(path.clone(), JournalSettings::default(), None);
        let action_id = Uuid::new_v4();
        let window = WindowInfo {
            id: "45837f40-43a8-4be5-b9d7-50d2ff8f79b3".to_string(),
            app_id: Some("org.kde.kate".to_string()),
            title: "Sensitive document title".to_string(),
            pid: Some(4242),
            monitor_id: None,
            geometry: None,
        };
        journal
            .record_agent_seat_delivery(
                "capture-1",
                "0194e9f8-1910-7e24-b5bd-52d184b6427f",
                action_id,
                &window,
                agent_seat::KWIN_AGENT_SEAT_BACKEND,
                SafetyClass::ControlKeyboard,
            )
            .expect("agent-seat delivery journals");
        let entries = journal
            .tail_filtered(4, Some("agent_seat_delivery"), Some(true))
            .expect("agent-seat entry reads");

        assert_eq!(entries.len(), 1);
        let control = entries[0].control.as_ref().expect("control metadata");
        assert_eq!(control.action_id, Some(action_id));
        assert_eq!(
            control.backend.as_deref(),
            Some(agent_seat::KWIN_AGENT_SEAT_BACKEND)
        );
        let encoded = serde_json::to_string(&entries).expect("journal serializes");
        assert!(encoded.contains("capture-1"));
        assert!(encoded.contains("0194e9f8-1910-7e24-b5bd-52d184b6427f"));
        assert!(encoded.contains(&window.id));
        assert!(!encoded.contains("Sensitive document title"));
        fs::remove_file(path).ok();
    }

    #[test]
    fn internal_post_action_capture_steps_share_the_parent_action_id() {
        let path = temp_test_path("post-action-capture-journal");
        let journal = ActionJournal::new(path.clone(), JournalSettings::default(), None);
        let action_id = Uuid::new_v4();
        journal
            .record_post_action_capture_step(
                "interaction_post_action_capture_start",
                "capture-1",
                action_id,
                Some("window-1"),
                true,
            )
            .expect("post-action capture step journals");
        let entries = journal
            .tail_filtered(2, Some("interaction_post_action_capture_start"), Some(true))
            .expect("post-action capture entry reads");
        assert_eq!(entries.len(), 1);
        let control = entries[0].control.as_ref().expect("control context");
        assert_eq!(control.action_id, Some(action_id));
        assert_eq!(
            control.backend.as_deref(),
            Some("portal_screencast_pipewire")
        );
        let target = control.requested_target.as_ref().expect("target context");
        assert_eq!(
            target.fields.get("session_id"),
            Some(&"capture-1".to_string())
        );
        assert_eq!(
            target.fields.get("window_id"),
            Some(&"window-1".to_string())
        );
        fs::remove_file(path).ok();
    }

    #[test]
    fn retained_capture_journal_target_omits_output_paths_and_revisions() {
        let request = DaemonRequest::CaptureWait(libseatgeist::CaptureWaitRequest {
            session_id: "capture-1".to_string(),
            after_revision: Some("private-revision".to_string()),
            output: PathBuf::from("/tmp/private-window-name.png"),
            max_edge: Some(800),
            timeout_ms: 5_000,
        });
        let target = journal_requested_target_for_request(&request)
            .expect("capture wait has compact journal target metadata");

        assert_eq!(target.kind, "capture_wait");
        assert_eq!(
            target.fields.get("session_id").map(String::as_str),
            Some("capture-1")
        );
        assert_eq!(
            target
                .fields
                .get("after_revision_present")
                .map(String::as_str),
            Some("true")
        );
        let encoded = serde_json::to_string(&target).expect("target serializes");
        assert!(!encoded.contains("private-revision"));
        assert!(!encoded.contains("private-window-name"));

        let monitor = DaemonRequest::CaptureOpen(CaptureOpenRequest {
            source: CaptureSourceKind::Monitor,
            requested_source_id: Some("DP-1".to_string()),
            parent_window: "wayland:private-parent".to_string(),
            timeout_ms: 30_000,
        });
        let target = journal_requested_target_for_request(&monitor)
            .expect("generic capture open has compact target metadata");
        assert_eq!(
            target.fields.get("source_type").map(String::as_str),
            Some("Monitor")
        );
        assert_eq!(
            target.fields.get("requested_source_id").map(String::as_str),
            Some("DP-1")
        );
        let encoded = serde_json::to_string(&target).expect("target serializes");
        assert!(!encoded.contains("private-parent"));
    }

    #[test]
    fn semantic_target_guard_journal_omits_title_text() {
        let request = DaemonRequest::ClickButton(ClickButtonRequest {
            name: "Continue".to_string(),
            destructive: false,
            app: Some("Firefox".to_string()),
            window_name_contains: Some("Meeting".to_string()),
            max_nodes: 128,
            guard: None,
            target_guard: Some(libseatgeist::TargetWindowGuard {
                expected_window_id: "kwin-firefox-1".to_string(),
                expected_app_id: Some("org.mozilla.firefox".to_string()),
                expected_pid: Some(4242),
                title_contains: Some("private meeting title".to_string()),
            }),
        });
        let target = journal_requested_target_for_request(&request)
            .expect("semantic action has compact target metadata");

        assert_eq!(
            target.fields.get("target_window_id").map(String::as_str),
            Some("kwin-firefox-1")
        );
        assert_eq!(
            target.fields.get("target_app_id").map(String::as_str),
            Some("org.mozilla.firefox")
        );
        assert_eq!(
            target.fields.get("target_pid").map(String::as_str),
            Some("4242")
        );
        assert_eq!(
            target
                .fields
                .get("target_title_guard_present")
                .map(String::as_str),
            Some("true")
        );
        let encoded = serde_json::to_string(&target).expect("target serializes");
        assert!(!encoded.contains("private meeting title"));
        assert!(!encoded.contains("Continue"));
        assert!(!encoded.contains("Meeting"));
    }

    #[test]
    fn retained_capture_lifecycle_uses_policy_engine_and_bounded_safety_classes() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let open = DaemonRequest::WindowCaptureOpen(libseatgeist::WindowCaptureOpenRequest {
            requested_window_id: Some("kwin-window-7".to_string()),
            parent_window: String::new(),
            timeout_ms: 30_000,
        });
        assert_eq!(safety_class_for_request(&open), SafetyClass::Observe);
        enforce_policy(&policy, &open).expect("capture open passes observe policy");

        let monitor = DaemonRequest::CaptureOpen(CaptureOpenRequest {
            source: CaptureSourceKind::Monitor,
            requested_source_id: Some("DP-1".to_string()),
            parent_window: String::new(),
            timeout_ms: 30_000,
        });
        assert_eq!(safety_class_for_request(&monitor), SafetyClass::Observe);
        enforce_policy(&policy, &monitor).expect("monitor capture open passes observe policy");

        let status = DaemonRequest::CaptureSessionStatus;
        assert_eq!(safety_class_for_request(&status), SafetyClass::Policy);
        enforce_policy(&policy, &status).expect("capture status passes policy");

        let renew = DaemonRequest::CaptureSessionRenew(libseatgeist::CaptureSessionRequest {
            session_id: "capture-1".to_string(),
        });
        assert_eq!(safety_class_for_request(&renew), SafetyClass::Policy);
        enforce_policy(&policy, &renew).expect("capture renew passes policy");

        let close = DaemonRequest::CaptureSessionClose(libseatgeist::CaptureSessionRequest {
            session_id: "capture-1".to_string(),
        });
        assert_eq!(safety_class_for_request(&close), SafetyClass::Policy);
        enforce_policy(&policy, &close).expect("capture close passes policy");
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
                portal_interactive: false,
                portal_target: None,
                visible_window_crop_id: None,
            }),
        )
        .expect("bounded screenshot requests are allowed by default");
    }

    #[test]
    fn screenshot_summaries_include_backend_provenance() {
        let screenshot = sample_screenshot_info("spectacle");
        let summary = summarize_response(&DaemonResponse::Screenshot(screenshot.clone()));
        assert!(summary.contains("backend=spectacle"));

        let summary = summarize_response(&DaemonResponse::WaitForChange(Box::new(
            WaitForChangeResult {
                changed: true,
                timed_out: false,
                timeout_ms: 5_000,
                interval_ms: 250,
                captures: 2,
                elapsed_ms: 250,
                score: 0.25,
                threshold: 0.01,
                screenshot: screenshot.clone(),
            },
        )));
        assert!(summary.contains("backend=spectacle"));
        assert!(summary.contains("timed_out=false"));
        assert!(summary.contains("elapsed_ms=250"));

        let summary = summarize_response(&DaemonResponse::WaitForChange(Box::new(
            WaitForChangeResult {
                changed: false,
                timed_out: true,
                timeout_ms: 5_000,
                interval_ms: 250,
                captures: 20,
                elapsed_ms: 5_000,
                score: 0.0,
                threshold: 0.01,
                screenshot,
            },
        )));
        assert!(summary.contains("changed=false"));
        assert!(summary.contains("timed_out=true"));
        assert!(summary.contains("timeout_ms=5000"));
    }

    #[test]
    fn action_settle_summary_excludes_observation_content() {
        let sensitive_value = "do-not-journal-this-value";
        let summary = summarize_response(&DaemonResponse::Action(Box::new(ActionResult {
            id: Uuid::nil(),
            ok: true,
            observation: Some(Observation {
                active_window: None,
                target_window: None,
                windows: Vec::new(),
                monitors: Vec::new(),
                focused_accessibility: Some(AccessibilityNode {
                    id: "node-1".to_string(),
                    role: "text".to_string(),
                    name: Some("field".to_string()),
                    value: Some(sensitive_value.to_string()),
                    value_truncated: false,
                    sensitive: false,
                    states: Vec::new(),
                    bounds: None,
                    available_actions: Vec::new(),
                    actions: Vec::new(),
                    children: Vec::new(),
                }),
                target_accessibility: None,
                screenshot_path: None,
                revision: Some("after-revision".to_string()),
                issues: Vec::new(),
                settle: Some(ActionSettleResult {
                    confirmation: libseatgeist::ActionConfirmation::Confirmed,
                    condition: ActionSettleCondition::Stable,
                    backend: libseatgeist::ActionSettleBackend::Polling,
                    target_scoped: false,
                    event: None,
                    settled: true,
                    timed_out: false,
                    timeout_ms: 1_500,
                    interval_ms: 100,
                    samples: 2,
                    elapsed_ms: 100,
                    before_revision: Some("before-revision".to_string()),
                    after_revision: "after-revision".to_string(),
                }),
            }),
            screenshot: None,
            message: Some("set text length=25".to_string()),
        })));

        assert!(summary.contains("settle=Stable"));
        assert!(summary.contains("backend=Polling"));
        assert!(summary.contains("target_scoped=false"));
        assert!(summary.contains("settled=true"));
        assert!(summary.contains("samples=2"));
        assert!(!summary.contains(sensitive_value));
        assert!(!summary.contains("before-revision"));
        assert!(!summary.contains("after-revision"));
    }

    #[tokio::test]
    async fn successful_response_records_resolved_session_backend_and_settle_only() {
        let store = session_execution::SessionExecutionStore::default();
        store
            .open(
                "capture-1".to_string(),
                "portal_screencast_pipewire".to_string(),
                true,
            )
            .await;
        let sensitive_value = "do-not-persist-this-input";
        let response = DaemonResponse::Action(Box::new(ActionResult {
            id: Uuid::nil(),
            ok: true,
            observation: Some(Observation {
                active_window: None,
                target_window: None,
                windows: Vec::new(),
                monitors: Vec::new(),
                focused_accessibility: Some(AccessibilityNode {
                    id: "node-1".to_string(),
                    role: "text".to_string(),
                    name: Some("field".to_string()),
                    value: Some(sensitive_value.to_string()),
                    value_truncated: false,
                    sensitive: true,
                    states: Vec::new(),
                    bounds: None,
                    available_actions: Vec::new(),
                    actions: Vec::new(),
                    children: Vec::new(),
                }),
                target_accessibility: None,
                screenshot_path: None,
                revision: Some("after-revision".to_string()),
                issues: Vec::new(),
                settle: Some(ActionSettleResult {
                    confirmation: libseatgeist::ActionConfirmation::Confirmed,
                    condition: ActionSettleCondition::AccessibilityChange,
                    backend: libseatgeist::ActionSettleBackend::AtspiEvent,
                    target_scoped: true,
                    event: Some("object:text-changed".to_string()),
                    settled: true,
                    timed_out: false,
                    timeout_ms: 1_000,
                    interval_ms: 100,
                    samples: 1,
                    elapsed_ms: 12,
                    before_revision: Some("before-revision".to_string()),
                    after_revision: "after-revision".to_string(),
                }),
            }),
            screenshot: None,
            message: Some("typed text backend=uinput".to_string()),
        }));

        record_session_execution_response(
            &["capture-1".to_string()],
            "type_text",
            SafetyClass::ControlKeyboard,
            Some("auto"),
            session_execution::BackendRole::RawInput,
            &response,
            &store,
        )
        .await;

        let status = store.status("capture-1").await.expect("status records");
        assert_eq!(status.raw_input_backend.as_deref(), Some("uinput"));
        assert_eq!(status.last_action_backend.as_deref(), Some("uinput"));
        assert_eq!(status.last_action_method.as_deref(), Some("type_text"));
        assert_eq!(status.last_policy_result.as_deref(), Some("allow"));
        assert_eq!(status.last_action_id, Some(Uuid::nil()));
        assert_eq!(
            status.settle.as_ref().map(|settle| settle.backend),
            Some(libseatgeist::ActionSettleBackend::AtspiEvent)
        );
        let serialized = serde_json::to_string(&status).expect("status serializes");
        assert!(!serialized.contains(sensitive_value));
        assert!(!serialized.contains("focused_accessibility"));
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
                portal_interactive: false,
                portal_target: None,
                visible_window_crop_id: None,
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
                    portal_interactive: false,
                    portal_target: None,
                    visible_window_crop_id: None,
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
                portal_interactive: false,
                portal_target: None,
                visible_window_crop_id: None,
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
            destructive_actions: Some(ToolApprovalLevel::Allow),
            secret_fields: Some(ToolApprovalLevel::Prompt),
            default_clipboard_read: Some(ToolApprovalLevel::Allow),
            default_clipboard_write: Some(ToolApprovalLevel::Prompt),
            default_full_resolution_screenshot: Some(ToolApprovalLevel::Deny),
        };

        let config = policy_config(Some(&file_policy), false, false, false);

        assert_eq!(config.default_observe, ToolApprovalLevel::Deny);
        assert_eq!(config.default_control, ToolApprovalLevel::Deny);
        assert_eq!(config.default_destructive_actions, ToolApprovalLevel::Allow);
        assert_eq!(config.default_secret_fields, ToolApprovalLevel::Prompt);
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
            destructive_actions: Some(ToolApprovalLevel::Allow),
            secret_fields: Some(ToolApprovalLevel::Deny),
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
    fn input_backend_preference_uses_cli_then_config_then_auto() {
        assert_eq!(
            input_backend_preference(None, None),
            InputBackendPreference::Auto
        );
        assert_eq!(
            input_backend_preference(None, Some(InputBackendPreference::Uinput)),
            InputBackendPreference::Uinput
        );
        assert_eq!(
            input_backend_preference(
                Some(InputBackendPreference::Libei),
                Some(InputBackendPreference::Uinput),
            ),
            InputBackendPreference::Libei
        );

        let config: DaemonConfigFile = toml::from_str(
            r#"
            [backends]
            input = "portal_remote_desktop"
            "#,
        )
        .expect("backend config parses");
        assert_eq!(
            config.backends.and_then(|backends| backends.input),
            Some(InputBackendPreference::PortalRemoteDesktop)
        );
        let config: DaemonConfigFile = toml::from_str(
            r#"
            [backends]
            input = "kwin_agent_seat"
            "#,
        )
        .expect("agent-seat backend config parses");
        assert_eq!(
            config.backends.and_then(|backends| backends.input),
            Some(InputBackendPreference::KwinAgentSeat)
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
    fn app_policy_without_config_protects_keepassxc() {
        let policy = app_policy(None);

        assert!(policy.allow.is_empty());
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

        assert!(
            err.to_string()
                .contains("denied control of protected application")
        );
        assert!(err.to_string().contains("do not retry"));
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

        assert!(
            err.to_string()
                .contains("denied control of protected application")
        );
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
                .contains("did not allow control of application")
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
    fn safety_settings_from_config_defaults_focus_guard_to_true() {
        assert!(
            safety_settings(None)
                .expect("default safety settings resolve")
                .require_focus_guard
        );
        assert!(
            safety_settings(Some(&SafetyFileConfig {
                require_focus_guard: None,
                pause_on_human_input: None,
                human_input_activity_file: None,
                human_input_quiet_ms: None,
                control_rate_limit_per_minute: None,
                preview_max_edge: None,
                tile_max_edge: None,
                redact_regions: None,
            }))
            .expect("empty safety config resolves")
            .require_focus_guard
        );
        assert!(
            !safety_settings(Some(&SafetyFileConfig {
                require_focus_guard: Some(false),
                pause_on_human_input: None,
                human_input_activity_file: None,
                human_input_quiet_ms: None,
                control_rate_limit_per_minute: None,
                preview_max_edge: None,
                tile_max_edge: None,
                redact_regions: None,
            }))
            .expect("explicit focus guard opt-out resolves")
            .require_focus_guard
        );
    }

    #[test]
    fn safety_settings_from_config_normalizes_redaction_regions() {
        let settings = safety_settings(Some(&SafetyFileConfig {
            require_focus_guard: Some(true),
            pause_on_human_input: None,
            human_input_activity_file: None,
            human_input_quiet_ms: None,
            control_rate_limit_per_minute: None,
            preview_max_edge: None,
            tile_max_edge: None,
            redact_regions: Some(vec![
                RedactRegionFileConfig {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                },
                RedactRegionFileConfig {
                    x: 1,
                    y: 2,
                    width: 0,
                    height: 4,
                },
            ]),
        }))
        .expect("redaction safety settings resolve");

        assert!(settings.require_focus_guard);
        assert_eq!(
            settings.screenshot_redactions,
            vec![RedactRegion {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            }]
        );
    }

    #[test]
    fn safety_settings_from_config_resolves_human_input_pause() {
        let path = temp_test_path("human-input-signal");
        let path_text = path.to_string_lossy().to_string();
        let settings = safety_settings(Some(&SafetyFileConfig {
            require_focus_guard: None,
            pause_on_human_input: Some(true),
            human_input_activity_file: Some(path_text.clone()),
            human_input_quiet_ms: Some(2500),
            control_rate_limit_per_minute: None,
            preview_max_edge: None,
            tile_max_edge: None,
            redact_regions: None,
        }))
        .expect("human input pause settings resolve");

        assert!(settings.pause_on_human_input);
        assert_eq!(
            settings.human_input_activity_file,
            Some(PathBuf::from(path_text))
        );
        assert_eq!(settings.human_input_quiet_ms, 2500);
    }

    #[test]
    fn safety_settings_from_config_resolves_control_rate_limit() {
        assert_eq!(
            safety_settings(None)
                .expect("default safety settings resolve")
                .control_rate_limit_per_minute,
            Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE)
        );

        let disabled = safety_settings(Some(&SafetyFileConfig {
            require_focus_guard: None,
            pause_on_human_input: None,
            human_input_activity_file: None,
            human_input_quiet_ms: None,
            control_rate_limit_per_minute: Some(0),
            preview_max_edge: None,
            tile_max_edge: None,
            redact_regions: None,
        }))
        .expect("disabled rate-limit setting resolves");
        assert_eq!(disabled.control_rate_limit_per_minute, None);

        let custom = safety_settings(Some(&SafetyFileConfig {
            require_focus_guard: None,
            pause_on_human_input: None,
            human_input_activity_file: None,
            human_input_quiet_ms: None,
            control_rate_limit_per_minute: Some(3),
            preview_max_edge: None,
            tile_max_edge: None,
            redact_regions: None,
        }))
        .expect("custom rate-limit setting resolves");
        assert_eq!(custom.control_rate_limit_per_minute, Some(3));
    }

    #[test]
    fn safety_settings_from_config_resolves_screenshot_max_edges() {
        let defaults = safety_settings(None).expect("default safety settings resolve");
        assert_eq!(defaults.preview_max_edge, DEFAULT_PREVIEW_MAX_EDGE);
        assert_eq!(defaults.tile_max_edge, DEFAULT_TILE_MAX_EDGE);

        let custom = safety_settings(Some(&SafetyFileConfig {
            require_focus_guard: None,
            pause_on_human_input: None,
            human_input_activity_file: None,
            human_input_quiet_ms: None,
            control_rate_limit_per_minute: None,
            preview_max_edge: Some(1200),
            tile_max_edge: Some(2400),
            redact_regions: None,
        }))
        .expect("custom screenshot max-edge settings resolve");
        assert_eq!(custom.preview_max_edge, 1200);
        assert_eq!(custom.tile_max_edge, 2400);

        let preview_err = safety_settings(Some(&SafetyFileConfig {
            require_focus_guard: None,
            pause_on_human_input: None,
            human_input_activity_file: None,
            human_input_quiet_ms: None,
            control_rate_limit_per_minute: None,
            preview_max_edge: Some(0),
            tile_max_edge: None,
            redact_regions: None,
        }))
        .expect_err("zero preview max edge is rejected");
        assert!(
            preview_err
                .to_string()
                .contains("safety.preview_max_edge must be greater than zero")
        );

        let tile_err = safety_settings(Some(&SafetyFileConfig {
            require_focus_guard: None,
            pause_on_human_input: None,
            human_input_activity_file: None,
            human_input_quiet_ms: None,
            control_rate_limit_per_minute: None,
            preview_max_edge: None,
            tile_max_edge: Some(0),
            redact_regions: None,
        }))
        .expect_err("zero tile max edge is rejected");
        assert!(
            tile_err
                .to_string()
                .contains("safety.tile_max_edge must be greater than zero")
        );
    }

    #[test]
    fn require_focus_guard_blocks_unguarded_control() {
        let settings = SafetySettings {
            require_focus_guard: true,
            pause_on_human_input: false,
            human_input_activity_file: None,
            human_input_quiet_ms: DEFAULT_HUMAN_INPUT_QUIET_MS,
            control_rate_limit_per_minute: Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE),
            preview_max_edge: DEFAULT_PREVIEW_MAX_EDGE,
            tile_max_edge: DEFAULT_TILE_MAX_EDGE,
            screenshot_redactions: Vec::new(),
        };

        let err = enforce_required_focus_guard(
            &settings,
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "guarded only".to_string(),
                guard: None,
                session_id: None,
            }),
        )
        .expect_err("unguarded control is rejected");

        assert!(err.to_string().contains("focus guard is required"));
    }

    #[test]
    fn sticky_session_replaces_active_guard_only_for_raw_action() {
        let settings = SafetySettings {
            require_focus_guard: true,
            ..SafetySettings::default()
        };
        let sticky = DaemonRequest::TypeText(TypeTextRequest {
            text: "hello".to_string(),
            guard: None,
            session_id: Some("capture-1".to_string()),
        });
        enforce_required_focus_guard(&settings, &sticky)
            .expect("sticky session satisfies raw focus guard");
        validate_interaction_session_request(&sticky)
            .expect("session without active guard is valid");

        let ambiguous = DaemonRequest::TypeText(TypeTextRequest {
            text: "hello".to_string(),
            guard: Some(ActiveWindowGuard {
                desktop_revision: None,
                expected_window_id: Some("firefox-window-1".to_string()),
                expected_app_id: None,
                title_contains: None,
            }),
            session_id: Some("capture-1".to_string()),
        });
        assert!(validate_interaction_session_request(&ambiguous).is_err());
    }

    #[test]
    fn retained_session_execution_tracks_controls_but_not_observation_lifecycle() {
        let raw = DaemonRequest::TypeText(TypeTextRequest {
            text: "hello".to_string(),
            guard: None,
            session_id: Some("capture-1".to_string()),
        });
        let focus = DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: "window-1".to_string(),
            guard: None,
        });
        let snapshot = DaemonRequest::CaptureSnapshot(libseatgeist::CaptureSnapshotRequest {
            session_id: "capture-1".to_string(),
            output: temp_test_path("execution-observation.png"),
            max_edge: Some(800),
            timeout_ms: 1_000,
        });

        assert_eq!(
            session_backend_role_for_request(&raw),
            Some(session_execution::BackendRole::RawInput)
        );
        assert_eq!(
            session_backend_role_for_request(&focus),
            Some(session_execution::BackendRole::Other)
        );
        assert_eq!(session_backend_role_for_request(&snapshot), None);
    }

    #[test]
    fn only_session_bound_raw_actions_use_the_independent_agent_lane() {
        let bound = DaemonRequest::KeyCombo(libseatgeist::KeyComboRequest {
            combo: "Ctrl+L".to_string(),
            destructive: false,
            guard: None,
            session_id: Some("capture-1".to_string()),
        });
        let unbound = DaemonRequest::KeyCombo(libseatgeist::KeyComboRequest {
            combo: "Ctrl+L".to_string(),
            destructive: false,
            guard: None,
            session_id: None,
        });
        assert!(uses_independent_agent_seat(
            &bound,
            InputBackendPreference::KwinAgentSeat
        ));
        assert!(!uses_independent_agent_seat(
            &unbound,
            InputBackendPreference::KwinAgentSeat
        ));
        assert!(!uses_independent_agent_seat(
            &bound,
            InputBackendPreference::Uinput
        ));
        assert_eq!(
            resolve_post_action_condition(
                &bound,
                ActionSettleCondition::Auto,
                InputBackendPreference::KwinAgentSeat,
                false,
            ),
            ActionSettleCondition::None
        );
        assert_eq!(
            resolve_post_action_condition(
                &bound,
                ActionSettleCondition::Stable,
                InputBackendPreference::KwinAgentSeat,
                false,
            ),
            ActionSettleCondition::Stable
        );
    }

    #[test]
    fn destructive_browser_shortcuts_are_classified_and_rejected_on_retained_seats() {
        let close_combo = DaemonRequest::KeyCombo(KeyComboRequest {
            combo: "CTRL+SHIFT+W".to_string(),
            destructive: false,
            guard: None,
            session_id: Some("capture-firefox".to_string()),
        });
        assert_eq!(
            safety_class_for_request(&close_combo),
            SafetyClass::DestructiveAction
        );
        assert!(
            validate_targeted_key_combo(&close_combo, InputBackendPreference::KwinAgentSeat)
                .expect_err("process-global close shortcut fails before delivery")
                .to_string()
                .contains("not target-safe")
        );

        let address_bar = DaemonRequest::KeyCombo(KeyComboRequest {
            combo: "Ctrl+L".to_string(),
            destructive: false,
            guard: None,
            session_id: Some("capture-firefox".to_string()),
        });
        assert_eq!(
            safety_class_for_request(&address_bar),
            SafetyClass::ControlKeyboard
        );
        validate_targeted_key_combo(&address_bar, InputBackendPreference::KwinAgentSeat)
            .expect("ordinary targeted browser shortcut remains available");

        let exact_close = DaemonRequest::CloseWindow(CloseWindowRequest {
            window_id: "firefox-agent".to_string(),
            session_id: Some("capture-firefox".to_string()),
            guard: None,
        });
        assert_eq!(
            safety_class_for_request(&exact_close),
            SafetyClass::DestructiveAction
        );
        assert!(
            enforce_policy(&PolicyEngine::new(PolicyConfig::default()), &exact_close)
                .expect_err("exact close requires destructive approval by default")
                .to_string()
                .contains("prompt required")
        );
    }

    #[test]
    fn mcp_clients_cannot_change_the_physical_workspace_focus() {
        let mcp = test_client("seatgeist-mcp", 7, "seatgeist-mcp");
        let focus = DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: "firefox-agent".to_string(),
            guard: None,
        });
        assert!(
            enforce_mcp_focus_isolation(&focus, Some(&mcp))
                .expect_err("MCP focus is refused")
                .to_string()
                .contains("physical user's workspace focus")
        );

        let activate = DaemonRequest::LaunchWindow(LaunchWindowRequest {
            desktop_entry: "firefox".to_string(),
            anchor: libseatgeist::WindowPlacementAnchor::Center,
            monitor_id: None,
            width: None,
            height: None,
            margin: 0,
            activation: libseatgeist::WindowActivationMode::Activate,
            timeout_ms: 10_000,
            guard: None,
        });
        assert!(enforce_mcp_focus_isolation(&activate, Some(&mcp)).is_err());

        let cli = test_client("seatgeist-cli", 8, "seatgeist-cli");
        enforce_mcp_focus_isolation(&focus, Some(&cli))
            .expect("explicit operator CLI focus remains available");
    }

    #[tokio::test]
    async fn exact_close_distinguishes_same_process_firefox_windows_by_kwin_uuid() {
        let user_window = WindowInfo {
            id: "firefox-user".to_string(),
            app_id: Some("firefox".to_string()),
            title: "AOF — Mozilla Firefox".to_string(),
            pid: Some(727_994),
            monitor_id: None,
            geometry: None,
        };
        let agent_window = WindowInfo {
            id: "firefox-agent".to_string(),
            app_id: Some("firefox".to_string()),
            title: "LocaleWeave — Mozilla Firefox".to_string(),
            pid: Some(727_994),
            monitor_id: None,
            geometry: None,
        };
        let backend = seatgeist_testkit::MockWindowBackend::new(
            vec![user_window.clone(), agent_window.clone()],
            Some(user_window.clone()),
        );

        backend
            .close_window(agent_window.id.clone())
            .await
            .expect("exact UUID closes");
        let remaining = backend.list_windows().await.expect("windows list");
        assert_eq!(remaining, vec![user_window.clone()]);
        assert_eq!(
            backend.active_window().await.expect("active"),
            Some(user_window)
        );
        assert_eq!(
            backend.closed_windows().expect("close journal"),
            vec!["firefox-agent".to_string()]
        );
    }

    fn test_client(tool: &str, pid: u32, process_name: &str) -> JournalClientContext {
        JournalClientContext {
            tool: Some(tool.to_string()),
            pid: Some(pid),
            process_name: Some(process_name.to_string()),
        }
    }

    #[tokio::test]
    async fn owner_gate_rejects_cross_process_capture_raw_and_post_action_uses() {
        let capture_session_store = CaptureSessionStore::default();
        let backend = seatgeist_testkit::MockScreenBackend::default();
        let owner_client = test_client("seatgeist-mcp", 100, "seatgeist-mcp");
        let intruder = test_client("seatgeist-mcp", 101, "seatgeist-mcp");
        let status = capture_open(
            CaptureOpenRequest {
                source: CaptureSourceKind::Monitor,
                requested_source_id: Some("monitor-1".to_string()),
                parent_window: String::new(),
                timeout_ms: 30_000,
            },
            SessionOwner::from_client(Some(&owner_client)).expect("owner constructs"),
            &capture_session_store,
            &backend,
            DEFAULT_PREVIEW_MAX_EDGE,
        )
        .await
        .expect("mock capture opens");
        let session_id = status.session_id.expect("capture session id");

        let snapshot = DaemonRequest::CaptureSnapshot(libseatgeist::CaptureSnapshotRequest {
            session_id: session_id.clone(),
            output: temp_test_path("owner-gate-snapshot.png"),
            max_edge: Some(800),
            timeout_ms: 1_000,
        });
        enforce_capture_session_owner(&snapshot, None, Some(&owner_client), &capture_session_store)
            .await
            .expect("the opening process may use capture");
        let error =
            enforce_capture_session_owner(&snapshot, None, Some(&intruder), &capture_session_store)
                .await
                .expect_err("another MCP process cannot snapshot");
        assert!(error.to_string().contains("session owner mismatch"));

        let raw = DaemonRequest::TypeText(TypeTextRequest {
            text: "x".to_string(),
            guard: None,
            session_id: Some(session_id.clone()),
        });
        assert!(
            enforce_capture_session_owner(&raw, None, Some(&intruder), &capture_session_store,)
                .await
                .is_err()
        );

        let focus = DaemonRequest::FocusWindow(FocusWindowRequest {
            window_id: "window-1".to_string(),
            guard: None,
        });
        let post_action = DaemonResponseOptions {
            post_action: Some(PostActionOptions {
                observe_after: true,
                settle_condition: ActionSettleCondition::None,
                settle_timeout_ms: 1_000,
                settle_interval_ms: 100,
                image: Some(libseatgeist::PostActionImageOptions {
                    session_id,
                    output: temp_test_path("owner-gate-post-action.png"),
                    max_edge: Some(800),
                    timeout_ms: 1_000,
                }),
            }),
        };
        assert!(
            enforce_capture_session_owner(
                &focus,
                Some(&post_action),
                Some(&intruder),
                &capture_session_store,
            )
            .await
            .is_err()
        );
        assert!(
            backend
                .snapshot_requests()
                .expect("mock snapshot calls read")
                .is_empty(),
            "owner denials happen before capture side effects"
        );
    }

    #[tokio::test]
    async fn sticky_focus_verification_waits_for_pinned_target() {
        let initial = WindowInfo {
            id: "kate-1".to_string(),
            title: "Kate".to_string(),
            app_id: Some("org.kde.kate".to_string()),
            pid: Some(7),
            monitor_id: None,
            geometry: None,
        };
        let target = WindowInfo {
            id: "firefox-1".to_string(),
            title: "Firefox".to_string(),
            app_id: Some("org.mozilla.firefox".to_string()),
            pid: Some(42),
            monitor_id: None,
            geometry: None,
        };
        let backend = seatgeist_testkit::MockWindowBackend::new(
            vec![initial.clone(), target.clone()],
            Some(initial),
        );
        let updated = backend.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            updated
                .set_active_window(Some(target))
                .expect("focused target update loads");
        });

        assert!(
            interaction::wait_for_active_target(&backend, "firefox-1", Duration::from_millis(100))
                .await
                .expect("focus verification succeeds")
        );
    }

    #[test]
    fn window_local_pointer_requires_active_window_guard() {
        let settings = SafetySettings {
            require_focus_guard: false,
            pause_on_human_input: false,
            human_input_activity_file: None,
            human_input_quiet_ms: DEFAULT_HUMAN_INPUT_QUIET_MS,
            control_rate_limit_per_minute: Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE),
            preview_max_edge: DEFAULT_PREVIEW_MAX_EDGE,
            tile_max_edge: DEFAULT_TILE_MAX_EDGE,
            screenshot_redactions: Vec::new(),
        };

        let err = enforce_required_focus_guard(
            &settings,
            &DaemonRequest::ClickPointer(ClickPointerRequest {
                point: Point {
                    x: 10.0,
                    y: 20.0,
                    space: CoordinateSpace::WindowLocal,
                },
                button: PointerButton::Left,
                clicks: 1,
                capture_revision: None,
                guard: None,
                session_id: None,
            }),
        )
        .expect_err("window-local pointer requests need a guard");
        assert!(err.to_string().contains("window_local pointer coordinates"));

        enforce_required_focus_guard(
            &settings,
            &DaemonRequest::ClickPointer(ClickPointerRequest {
                point: Point {
                    x: 10.0,
                    y: 20.0,
                    space: CoordinateSpace::WindowLocal,
                },
                button: PointerButton::Left,
                clicks: 1,
                capture_revision: None,
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: Some("window-1".to_string()),
                    expected_app_id: None,
                    title_contains: None,
                }),
                session_id: None,
            }),
        )
        .expect("guarded window-local pointer request passes precheck");
    }

    #[test]
    fn require_focus_guard_allows_guarded_control_and_observe() {
        let settings = SafetySettings {
            require_focus_guard: true,
            pause_on_human_input: false,
            human_input_activity_file: None,
            human_input_quiet_ms: DEFAULT_HUMAN_INPUT_QUIET_MS,
            control_rate_limit_per_minute: Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE),
            preview_max_edge: DEFAULT_PREVIEW_MAX_EDGE,
            tile_max_edge: DEFAULT_TILE_MAX_EDGE,
            screenshot_redactions: Vec::new(),
        };

        enforce_required_focus_guard(&settings, &DaemonRequest::ListWindows)
            .expect("observe requests do not need active-window guards");
        enforce_required_focus_guard(
            &settings,
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "guarded only".to_string(),
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: None,
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: None,
                }),
                session_id: None,
            }),
        )
        .expect("guarded control is accepted by require-focus-guard precheck");
    }

    #[test]
    fn human_input_pause_blocks_control_on_fresh_signal() {
        let path = temp_test_path("human-input-blocks-control");
        fs::write(&path, "activity").expect("human input signal fixture is written");
        let settings = SafetySettings {
            require_focus_guard: false,
            pause_on_human_input: true,
            human_input_activity_file: Some(path.clone()),
            human_input_quiet_ms: 60_000,
            control_rate_limit_per_minute: Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE),
            preview_max_edge: DEFAULT_PREVIEW_MAX_EDGE,
            tile_max_edge: DEFAULT_TILE_MAX_EDGE,
            screenshot_redactions: Vec::new(),
        };

        let err = enforce_human_input_pause(
            &settings,
            &activity::ActivityTracker::default(),
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
                session_id: None,
            }),
        )
        .expect_err("fresh human input signal blocks control");

        assert!(err.to_string().contains("human input activity is fresh"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn human_input_pause_uses_trusted_activity_and_ignores_own_injection() {
        let settings = SafetySettings {
            require_focus_guard: false,
            pause_on_human_input: true,
            human_input_activity_file: None,
            human_input_quiet_ms: 60_000,
            control_rate_limit_per_minute: Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE),
            preview_max_edge: DEFAULT_PREVIEW_MAX_EDGE,
            tile_max_edge: DEFAULT_TILE_MAX_EDGE,
            screenshot_redactions: Vec::new(),
        };
        let request = DaemonRequest::TypeText(TypeTextRequest {
            text: "hello".to_string(),
            guard: None,
            session_id: None,
        });
        let injected = activity::ActivityTracker::default();
        injected
            .record_payload(
                r#"{"backend":"kwin_input_spy_v1","seat":"default","class":"keyboard","provenance":"seatgeist_injected","monotonic_ms":1}"#,
            )
            .expect("injected activity records");
        enforce_human_input_pause(&settings, &injected, &request)
            .expect("Seatgeist injection does not trigger human pause");

        let physical = activity::ActivityTracker::default();
        physical
            .record_payload(
                r#"{"backend":"kwin_input_spy_v1","seat":"default","class":"pointer","provenance":"trusted_physical","monotonic_ms":2}"#,
            )
            .expect("physical activity records");
        let err = enforce_human_input_pause(&settings, &physical, &request)
            .expect_err("trusted physical activity triggers human pause");
        assert!(err.to_string().contains("kwin_input_spy_v1"));
    }

    #[test]
    fn classifies_operator_visible_error_causes() {
        let cases = [
            (
                "policy prompt required for ControlPointer, but no matching approval grant is available",
                ErrorKind::PolicyPromptRequired,
            ),
            (
                "policy denied ClipboardRead: configured deny",
                ErrorKind::PolicyDenied,
            ),
            (
                "app policy denied control of protected application org.keepassxc.KeePassXC for active window; do not retry through another backend",
                ErrorKind::AppDenied,
            ),
            (
                "active-window guard failed: expected app id org.kde.kate, got org.kde.konsole",
                ErrorKind::FocusGuard,
            ),
            (
                "target-window correlation failed: KWin title does not match accessibility window",
                ErrorKind::TargetMismatch,
            ),
            (
                "interaction target lost: pinned window closed",
                ErrorKind::TargetLost,
            ),
            ("session owner mismatch", ErrorKind::SessionOwnerMismatch),
            (
                "focus lease conflict: pinned target did not become active before input",
                ErrorKind::FocusLeaseConflict,
            ),
            (
                "agent target in use: another agent owns the window interaction lease",
                ErrorKind::FocusLeaseConflict,
            ),
            (
                "agent target user active: physical input reached this window 40ms ago",
                ErrorKind::HumanInputPause,
            ),
            (
                "capture frame invalidated by user input; acquire a fresh frame before preview-derived pointer input",
                ErrorKind::Validation,
            ),
            (
                "no non-sensitive activatable tab matched name=AOF",
                ErrorKind::Validation,
            ),
            (
                "KWin launch placement failed: launch intent expired before a matching window appeared",
                ErrorKind::Validation,
            ),
            (
                "human input activity is fresh from kwin_input_spy_v1; refusing ControlKeyboard until quiet for 1000ms",
                ErrorKind::HumanInputPause,
            ),
            (
                "backend unavailable: ScreenCast consent was cancelled or denied",
                ErrorKind::ConsentCancelled,
            ),
            (
                "xdg-desktop-portal RemoteDesktop is not available: org.freedesktop.portal.Desktop is missing",
                ErrorKind::PortalUnavailable,
            ),
            (
                "portal screenshot target active_window requires xdg-desktop-portal Screenshot v3/AvailableTargets; current Screenshot interface version is 2",
                ErrorKind::PortalUnavailable,
            ),
            (
                "accessibility tree max_nodes exhausted",
                ErrorKind::AccessibilityWeakTree,
            ),
            (
                "invalid AT-SPI node id: bad",
                ErrorKind::AccessibilityUnavailable,
            ),
            (
                "no active capture session with that id",
                ErrorKind::TargetLost,
            ),
            (
                "this client already has an opening or active capture session",
                ErrorKind::Validation,
            ),
            (
                "KWin script bridge DBus receiver is unavailable",
                ErrorKind::BackendUnavailable,
            ),
            (
                "KWin script bridge heartbeat is stale; reload only the seatgeist-bridge script",
                ErrorKind::BackendUnavailable,
            ),
            (
                "launch succeeded without geometry metadata",
                ErrorKind::BackendFailed,
            ),
            ("unsupported key name: Hyper", ErrorKind::Validation),
        ];

        for (message, kind) in cases {
            assert_eq!(classify_error_message(message), kind, "{message}");
        }
    }

    #[test]
    fn error_reason_codes_preserve_actionable_backend_causes() {
        assert_eq!(
            classify_error_reason(
                "no active capture session with that id",
                ErrorKind::TargetLost
            ),
            "capture_session_inactive"
        );
        assert_eq!(
            classify_error_reason(
                "KWin script bridge DBus receiver is unavailable",
                ErrorKind::BackendUnavailable
            ),
            "kwin_bridge_unavailable"
        );
        assert_eq!(
            classify_error_reason(
                "KWin script bridge heartbeat is stale; reload only the seatgeist-bridge script",
                ErrorKind::BackendUnavailable
            ),
            "kwin_bridge_stale"
        );
        assert_eq!(
            classify_error_reason(
                "app policy denied control of protected application org.keepassxc.KeePassXC",
                ErrorKind::AppDenied
            ),
            "protected_application"
        );
        assert_eq!(
            classify_error_reason(
                "app policy did not allow control of application org.example.Editor",
                ErrorKind::AppDenied
            ),
            "application_not_allowlisted"
        );
        assert_eq!(
            classify_error_reason("unsupported key name: Hyper", ErrorKind::Validation),
            "validation"
        );
        assert_eq!(
            classify_error_reason(
                "invalid request: invalid AT-SPI node id: invalid-atspi-node",
                ErrorKind::Validation
            ),
            "invalid_accessibility_node_id"
        );
        assert_eq!(
            classify_error_reason(
                "capture revision is stale; acquire a fresh frame before pointer input",
                ErrorKind::Validation
            ),
            "capture_revision_stale"
        );
        assert_eq!(
            classify_error_reason(
                "capture frame invalidated by user input; acquire a fresh frame before preview-derived pointer input",
                ErrorKind::Validation
            ),
            "capture_frame_invalidated_by_user"
        );
        assert_eq!(
            classify_error_reason(
                "no non-sensitive activatable tab matched name=AOF",
                ErrorKind::Validation
            ),
            "semantic_target_not_actionable"
        );
        assert_eq!(
            classify_error_reason(
                "KWin launch placement failed: launch intent expired before a matching window appeared",
                ErrorKind::Validation
            ),
            "launch_no_new_window"
        );
        assert_eq!(
            classify_error_reason(
                "capture_output pointer coordinate 1280,360 is outside preview 1280x720",
                ErrorKind::Validation
            ),
            "capture_coordinate_out_of_bounds"
        );
        assert_eq!(
            classify_error_reason(
                "KWin agent-seat plugin is not registered",
                ErrorKind::BackendUnavailable
            ),
            "agent_seat_unavailable"
        );
        assert_eq!(
            classify_error_reason(
                "agent target in use: another agent owns the window interaction lease",
                ErrorKind::FocusLeaseConflict
            ),
            "agent_target_in_use"
        );
        assert_eq!(
            classify_error_reason(
                "agent lane quota reached: at most 4 agent owners may hold interaction sessions",
                ErrorKind::FocusLeaseConflict
            ),
            "agent_lane_quota"
        );
        assert_eq!(
            classify_error_reason(
                "agent target user active: physical input reached this window 40ms ago",
                ErrorKind::HumanInputPause
            ),
            "agent_target_user_active"
        );
    }

    #[test]
    fn parses_linux_resident_memory_status_in_bytes() {
        let status = "Name:\tseatgeistd\nVmHWM:\t  410624 kB\nVmRSS:\t  181392 kB\n";
        assert_eq!(proc_status_kib(status, "VmRSS:"), Some(185_745_408));
        assert_eq!(proc_status_kib(status, "VmHWM:"), Some(420_478_976));
        assert_eq!(proc_status_kib(status, "VmSwap:"), None);
    }

    #[test]
    fn app_policy_errors_keep_their_kind_for_capture_targets() {
        let response = daemon_error(anyhow::anyhow!(
            "app policy denied control of protected application org.keepassxc.KeePassXC for pinned interaction target; do not retry through another backend"
        ));

        assert!(matches!(
            response,
            DaemonResponse::Error {
                kind: ErrorKind::AppDenied,
                reason_code: Some(ref reason),
                ..
            } if reason == "protected_application"
        ));
    }

    #[test]
    fn human_input_pause_allows_observe_and_missing_or_quiet_signal() {
        let missing_path = temp_test_path("human-input-missing");
        let settings = SafetySettings {
            require_focus_guard: false,
            pause_on_human_input: true,
            human_input_activity_file: Some(missing_path),
            human_input_quiet_ms: 60_000,
            control_rate_limit_per_minute: Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE),
            preview_max_edge: DEFAULT_PREVIEW_MAX_EDGE,
            tile_max_edge: DEFAULT_TILE_MAX_EDGE,
            screenshot_redactions: Vec::new(),
        };
        enforce_human_input_pause(
            &settings,
            &activity::ActivityTracker::default(),
            &DaemonRequest::ListWindows,
        )
        .expect("human input pause does not block observe requests");
        enforce_human_input_pause(
            &settings,
            &activity::ActivityTracker::default(),
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
                session_id: None,
            }),
        )
        .expect("missing human input signal does not block control");

        let path = temp_test_path("human-input-quiet");
        fs::write(&path, "activity").expect("human input signal fixture is written");
        let settings = SafetySettings {
            require_focus_guard: false,
            pause_on_human_input: true,
            human_input_activity_file: Some(path.clone()),
            human_input_quiet_ms: 0,
            control_rate_limit_per_minute: Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE),
            preview_max_edge: DEFAULT_PREVIEW_MAX_EDGE,
            tile_max_edge: DEFAULT_TILE_MAX_EDGE,
            screenshot_redactions: Vec::new(),
        };
        std::thread::sleep(Duration::from_millis(2));
        enforce_human_input_pause(
            &settings,
            &activity::ActivityTracker::default(),
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
                session_id: None,
            }),
        )
        .expect("quiet human input signal does not block control");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn control_rate_limiter_blocks_excess_control_requests_only() {
        let limiter = ControlRateLimiter::new(Some(2));
        enforce_control_rate_limit(&limiter, &DaemonRequest::ListWindows)
            .expect("observe request is not rate-limited");
        let request = DaemonRequest::TypeText(TypeTextRequest {
            text: "hello".to_string(),
            guard: None,
            session_id: None,
        });

        enforce_control_rate_limit(&limiter, &request).expect("first control request is allowed");
        enforce_control_rate_limit(&limiter, &request).expect("second control request is allowed");
        let err = enforce_control_rate_limit(&limiter, &request)
            .expect_err("third control request exceeds limit");
        assert!(err.to_string().contains("control rate limit exceeded"));

        let disabled = ControlRateLimiter::new(None);
        for _ in 0..3 {
            enforce_control_rate_limit(&disabled, &request)
                .expect("disabled control rate limit never blocks");
        }
    }

    #[test]
    fn parses_daemon_config_file() {
        let path = temp_test_path("daemon-config.toml");
        fs::write(
            &path,
            r#"
[daemon]
socket = "$XDG_RUNTIME_DIR/seatgeist/configured.sock"
journal = "$XDG_STATE_HOME/seatgeist/configured.jsonl"
panic_stop_file = "$XDG_RUNTIME_DIR/seatgeist/configured-panic-stop"
approval_file = "$XDG_RUNTIME_DIR/seatgeist/approvals.jsonl"
capture_restore_file = "$XDG_STATE_HOME/seatgeist/capture-restore.json"

[journal]
include_artifact_metadata = true

[backends]
input = "portal_remote_desktop"

[backends.keymap]
rules = "evdev"
model = "pc105"
layout = "de"
variant = "nodeadkeys"
options = ""

[policy]
default_observe = "allow"
default_control = "deny"
destructive_actions = "deny"
secret_fields = "prompt"
default_clipboard_read = "allow"
default_clipboard_write = "prompt"
full_resolution_screenshot = "deny"

[apps]
allow = ["org.kde.kate"]
deny = ["org.keepassxc.KeePassXC"]

[safety]
require_focus_guard = true
pause_on_human_input = true
human_input_activity_file = "$XDG_RUNTIME_DIR/seatgeist/human-input-active"
human_input_quiet_ms = 2500
control_rate_limit_per_minute = 60
preview_max_edge = 1200
tile_max_edge = 2400

[[safety.redact_regions]]
x = 10
y = 20
width = 30
height = 40
"#,
        )
        .expect("config fixture is written");

        let config = load_daemon_config(Some(&path)).expect("config file parses");
        let daemon = config.daemon.expect("daemon section is present");
        assert_eq!(
            daemon.socket.as_deref(),
            Some("$XDG_RUNTIME_DIR/seatgeist/configured.sock")
        );
        assert_eq!(
            daemon.approval_file.as_deref(),
            Some("$XDG_RUNTIME_DIR/seatgeist/approvals.jsonl")
        );
        assert_eq!(
            daemon.capture_restore_file.as_deref(),
            Some("$XDG_STATE_HOME/seatgeist/capture-restore.json")
        );
        let journal = config.journal.expect("journal section is present");
        assert_eq!(journal.include_artifact_metadata, Some(true));
        assert!(journal_settings(Some(&journal)).include_artifact_metadata);
        let backends = config.backends.expect("backends section is present");
        assert_eq!(
            backends.input,
            Some(InputBackendPreference::PortalRemoteDesktop)
        );
        let keymap = backends.keymap.expect("keymap section is present");
        assert_eq!(keymap.rules.as_deref(), Some("evdev"));
        assert_eq!(keymap.model.as_deref(), Some("pc105"));
        assert_eq!(keymap.layout.as_deref(), Some("de"));
        assert_eq!(keymap.variant.as_deref(), Some("nodeadkeys"));
        assert_eq!(keymap.options.as_deref(), Some(""));

        let policy = config.policy.expect("policy section is present");
        assert_eq!(policy.default_control, Some(ToolApprovalLevel::Deny));
        assert_eq!(policy.destructive_actions, Some(ToolApprovalLevel::Deny));
        assert_eq!(policy.secret_fields, Some(ToolApprovalLevel::Prompt));
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
        let safety = config.safety.expect("safety section is present");
        assert_eq!(safety.require_focus_guard, Some(true));
        assert_eq!(safety.pause_on_human_input, Some(true));
        assert_eq!(
            safety.human_input_activity_file.as_deref(),
            Some("$XDG_RUNTIME_DIR/seatgeist/human-input-active")
        );
        assert_eq!(safety.human_input_quiet_ms, Some(2500));
        assert_eq!(safety.control_rate_limit_per_minute, Some(60));
        assert_eq!(safety.preview_max_edge, Some(1200));
        assert_eq!(safety.tile_max_edge, Some(2400));
        assert_eq!(
            safety
                .redact_regions
                .as_ref()
                .and_then(|regions| regions.first())
                .map(|region| (region.x, region.y, region.width, region.height)),
            Some((10, 20, 30, 40))
        );
        fs::remove_file(&path).ok();
    }

    #[test]
    fn configured_path_prefers_cli_over_config() {
        let path = configured_path(
            Some(PathBuf::from("/tmp/seatgeist-cli.sock")),
            Some("/tmp/seatgeist-config.sock"),
            || Ok(PathBuf::from("/tmp/seatgeist-default.sock")),
        )
        .expect("configured path resolves");

        assert_eq!(path, PathBuf::from("/tmp/seatgeist-cli.sock"));
    }

    #[test]
    fn prompt_policy_fails_closed_without_approval_channel() {
        let policy = PolicyEngine::new(PolicyConfig {
            default_observe: ToolApprovalLevel::Prompt,
            ..PolicyConfig::default()
        });
        let err = enforce_policy(&policy, &DaemonRequest::ListWindows)
            .expect_err("prompt requires approval channel");
        assert!(
            err.to_string()
                .contains("no matching approval grant is available")
        );
    }

    #[test]
    fn approval_file_grant_allows_matching_prompt_policy() {
        let root = temp_test_private_dir("approval-grant");
        let path = root.join("approvals.jsonl");
        write_test_approval_grant(
            &path,
            SafetyClass::ControlSemantic,
            "focus_window",
            unix_time_ms().expect("time is available") + 60_000,
        );
        let policy = PolicyEngine::new(PolicyConfig::default());
        let approval_store = ApprovalStore::new(Some(path.clone()));

        enforce_policy_with_approvals(
            &policy,
            &approval_store,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "window-1".to_string(),
                guard: None,
            }),
        )
        .expect("matching approval grant allows prompt policy");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn approval_file_rejects_insecure_permissions() {
        let root = temp_test_private_dir("approval-insecure");
        let path = root.join("approvals.jsonl");
        write_test_approval_grant(
            &path,
            SafetyClass::ControlSemantic,
            "focus_window",
            unix_time_ms().expect("time is available") + 60_000,
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("permissions update");
        let policy = PolicyEngine::new(PolicyConfig::default());
        let approval_store = ApprovalStore::new(Some(path.clone()));

        let err = enforce_policy_with_approvals(
            &policy,
            &approval_store,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "window-1".to_string(),
                guard: None,
            }),
        )
        .expect_err("insecure approval file is rejected");
        assert!(err.to_string().contains("must not be readable"));

        fs::remove_dir_all(&root).ok();
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

    #[tokio::test]
    async fn active_window_guard_allows_matching_injected_window() {
        let window = WindowInfo {
            id: "current-window".to_string(),
            title: "main.rs - Kate".to_string(),
            app_id: Some("org.kde.kate".to_string()),
            pid: Some(1234),
            monitor_id: None,
            geometry: None,
        };
        let backend = seatgeist_testkit::MockWindowBackend::new(vec![window.clone()], Some(window));

        enforce_active_window_guard(
            &backend,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: Some("current-window".to_string()),
                    expected_app_id: Some("org.kde.kate".to_string()),
                    title_contains: Some("main.rs".to_string()),
                }),
            }),
        )
        .await
        .expect("matching active-window guard passes");
    }

    #[tokio::test]
    async fn active_window_guard_rejects_changed_injected_window() {
        let window = WindowInfo {
            id: "other-window".to_string(),
            title: "Terminal".to_string(),
            app_id: Some("org.kde.konsole".to_string()),
            pid: None,
            monitor_id: None,
            geometry: None,
        };
        let backend = seatgeist_testkit::MockWindowBackend::new(vec![window.clone()], Some(window));

        let err = enforce_active_window_guard(
            &backend,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: Some(ActiveWindowGuard {
                    desktop_revision: None,
                    expected_window_id: Some("current-window".to_string()),
                    expected_app_id: None,
                    title_contains: None,
                }),
            }),
        )
        .await
        .expect_err("stale active-window guard fails");
        assert!(err.to_string().contains("active-window guard failed"));
    }

    #[tokio::test]
    async fn opaque_desktop_revision_guards_the_active_window() {
        let window = seatgeist_testkit::sample_window();
        let backend =
            seatgeist_testkit::MockWindowBackend::new(vec![window.clone()], Some(window.clone()));
        let revision = observation::active_window_revision(&Some(window));
        enforce_active_window_guard(
            &backend,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: Some(ActiveWindowGuard {
                    desktop_revision: Some(revision),
                    expected_window_id: None,
                    expected_app_id: None,
                    title_contains: None,
                }),
            }),
        )
        .await
        .expect("current opaque revision passes");

        let err = enforce_active_window_guard(
            &backend,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: Some(ActiveWindowGuard {
                    desktop_revision: Some("aw1:stale".to_string()),
                    expected_window_id: None,
                    expected_app_id: None,
                    title_contains: None,
                }),
            }),
        )
        .await
        .expect_err("stale opaque revision fails");
        assert_eq!(
            err.to_string(),
            "active-window guard failed: desktop revision changed"
        );
    }

    #[tokio::test]
    async fn app_policy_blocks_control_for_denied_injected_active_app() {
        let window = WindowInfo {
            id: "secrets-window".to_string(),
            title: "Vault".to_string(),
            app_id: Some("org.keepassxc.KeePassXC".to_string()),
            pid: None,
            monitor_id: None,
            geometry: None,
        };
        let backend = seatgeist_testkit::MockWindowBackend::new(vec![window.clone()], Some(window));
        let policy = AppPolicy {
            allow: Vec::new(),
            deny: vec!["org.keepassxc.KeePassXC".to_string()],
        };

        let err = enforce_app_policy(
            &backend,
            &policy,
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "should-not-type".to_string(),
                guard: None,
                session_id: None,
            }),
        )
        .await
        .expect_err("denied active app blocks keyboard control");

        assert!(
            err.to_string()
                .contains("denied control of protected application")
        );
    }

    #[tokio::test]
    async fn app_policy_checks_focus_target_through_injected_window_backend() {
        let target = WindowInfo {
            id: "target-window".to_string(),
            title: "Vault".to_string(),
            app_id: Some("org.keepassxc.KeePassXC".to_string()),
            pid: None,
            monitor_id: None,
            geometry: None,
        };
        let backend = seatgeist_testkit::MockWindowBackend::new(vec![target], None);
        let policy = AppPolicy {
            allow: Vec::new(),
            deny: vec!["org.keepassxc.KeePassXC".to_string()],
        };

        let err = enforce_app_policy(
            &backend,
            &policy,
            &DaemonRequest::FocusWindow(FocusWindowRequest {
                window_id: "target-window".to_string(),
                guard: None,
            }),
        )
        .await
        .expect_err("denied focus target is checked through injected window list");

        assert!(
            err.to_string()
                .contains("protected application org.keepassxc.KeePassXC for focus target")
        );
    }

    #[tokio::test]
    async fn resolved_semantic_target_is_authorized_through_injected_window_backend() {
        let window = WindowInfo {
            id: "kwin-firefox-1".to_string(),
            title: "Example - Mozilla Firefox".to_string(),
            app_id: Some("org.mozilla.firefox".to_string()),
            pid: Some(4242),
            monitor_id: None,
            geometry: None,
        };
        let window_backend = seatgeist_testkit::MockWindowBackend::new(vec![window], None);
        let guard = libseatgeist::TargetWindowGuard {
            expected_window_id: "kwin-firefox-1".to_string(),
            expected_app_id: Some("org.mozilla.firefox".to_string()),
            expected_pid: Some(4242),
            title_contains: Some("Example".to_string()),
        };
        let request = DaemonRequest::ClickButton(ClickButtonRequest {
            name: "Continue".to_string(),
            destructive: false,
            app: Some("Firefox".to_string()),
            window_name_contains: Some("Example".to_string()),
            max_nodes: 256,
            guard: None,
            target_guard: Some(guard.clone()),
        });
        let safety = SafetySettings {
            require_focus_guard: true,
            ..SafetySettings::default()
        };
        enforce_required_focus_guard(&safety, &request)
            .expect("target guard replaces active guard for semantic operation");

        let mut candidate_node = button_node("button-1", "Continue");
        candidate_node.id =
            "atspi://org.mozilla.firefox/org/a11y/atspi/accessible/button-1".to_string();
        let candidate = seatgeist_atspi::AccessibilityMatch {
            node: candidate_node,
            application_name: "Firefox".to_string(),
            application_bus_name: "org.mozilla.firefox".to_string(),
            process_id: Some(4242),
            window_name: Some("Example - Mozilla Firefox".to_string()),
            window_node_id: Some(
                "atspi://org.mozilla.firefox/org/a11y/atspi/accessible/window-1".to_string(),
            ),
        };
        let denied = AppPolicy {
            allow: Vec::new(),
            deny: vec!["org.mozilla.firefox".to_string()],
        };
        let err = target::authorize_semantic_target(
            candidate.node.clone(),
            vec![candidate.clone()],
            Some(&guard),
            &window_backend,
            &denied,
        )
        .await
        .expect_err("resolved target app policy runs before the caller invokes AT-SPI");
        assert!(
            err.to_string()
                .contains("denied control of protected application")
        );

        let allowed = target::authorize_semantic_target(
            candidate.node.clone(),
            vec![candidate],
            Some(&guard),
            &window_backend,
            &AppPolicy::default(),
        )
        .await
        .expect("matching target resolves without consulting active window");
        assert!(allowed.id.ends_with("button-1"));
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
                threshold: libseatgeist::DEFAULT_WAIT_FOR_CHANGE_THRESHOLD,
            }),
        )
        .expect("wait_for_change is observe policy");
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
        enforce_policy(&policy, &DaemonRequest::CaptureBackendStatus)
            .expect("capture backend status is allowed as policy diagnostics");
        enforce_policy(&policy, &DaemonRequest::PointerCalibration)
            .expect("pointer calibration is allowed as policy diagnostics");
        enforce_policy(&policy, &DaemonRequest::DesktopSessionStatus)
            .expect("desktop session status is allowed as policy diagnostics");
        enforce_policy(&policy, &DaemonRequest::AccessibilityQualityStatus)
            .expect("accessibility quality status is allowed as policy diagnostics");
    }

    #[test]
    fn remote_desktop_session_probe_is_control_pointer_policy() {
        let request = DaemonRequest::RemoteDesktopSessionProbe(RemoteDesktopSessionProbeRequest {
            keyboard: true,
            pointer: true,
            touchscreen: false,
            restore_token: None,
            persist_mode: None,
            parent_window: None,
            timeout_ms: 30_000,
            guard: None,
        });
        assert_eq!(
            safety_class_for_request(&request),
            SafetyClass::ControlPointer
        );
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(&policy, &request)
            .expect_err("remote desktop session probe prompts by default");
        assert!(err.to_string().contains("ControlPointer"));

        let keyboard_only =
            DaemonRequest::RemoteDesktopSessionProbe(RemoteDesktopSessionProbeRequest {
                keyboard: true,
                pointer: false,
                touchscreen: false,
                restore_token: None,
                persist_mode: None,
                parent_window: None,
                timeout_ms: 30_000,
                guard: None,
            });
        assert_eq!(
            safety_class_for_request(&keyboard_only),
            SafetyClass::ControlKeyboard
        );

        let eis_request = DaemonRequest::RemoteDesktopEisProbe(RemoteDesktopSessionProbeRequest {
            keyboard: true,
            pointer: true,
            touchscreen: false,
            restore_token: None,
            persist_mode: None,
            parent_window: None,
            timeout_ms: 30_000,
            guard: None,
        });
        assert_eq!(
            safety_class_for_request(&eis_request),
            SafetyClass::ControlPointer
        );
        let err = enforce_policy(&policy, &eis_request)
            .expect_err("remote desktop EIS probe prompts by default");
        assert!(err.to_string().contains("ControlPointer"));

        let keyboard_only_eis =
            DaemonRequest::RemoteDesktopEisProbe(RemoteDesktopSessionProbeRequest {
                keyboard: true,
                pointer: false,
                touchscreen: false,
                restore_token: None,
                persist_mode: None,
                parent_window: None,
                timeout_ms: 30_000,
                guard: None,
            });
        assert_eq!(
            safety_class_for_request(&keyboard_only_eis),
            SafetyClass::ControlKeyboard
        );

        let start = DaemonRequest::RemoteDesktopEisStart(RemoteDesktopSessionProbeRequest {
            keyboard: true,
            pointer: true,
            touchscreen: false,
            restore_token: None,
            persist_mode: None,
            parent_window: None,
            timeout_ms: 30_000,
            guard: None,
        });
        assert_eq!(
            safety_class_for_request(&start),
            SafetyClass::ControlPointer
        );
        assert!(enforce_policy(&policy, &start).is_err());
        assert_eq!(
            safety_class_for_request(&DaemonRequest::RemoteDesktopEisSessionStatus),
            SafetyClass::Policy
        );
        enforce_policy(&policy, &DaemonRequest::RemoteDesktopEisSessionStatus)
            .expect("EIS session status is a policy diagnostic");
        assert_eq!(
            safety_class_for_request(&DaemonRequest::RemoteDesktopEisStop),
            SafetyClass::Policy
        );
        enforce_policy(&policy, &DaemonRequest::RemoteDesktopEisStop)
            .expect("EIS session stop is a policy action");
    }

    #[test]
    fn remote_desktop_session_probe_validates_devices_and_timeout() {
        let request = RemoteDesktopSessionProbeRequest {
            keyboard: true,
            pointer: false,
            touchscreen: true,
            restore_token: None,
            persist_mode: None,
            parent_window: None,
            timeout_ms: 30_000,
            guard: None,
        };
        let devices = remote_desktop_device_types(&request).expect("device bitmask builds");
        assert!(devices.contains(seatgeist_portal::RemoteDesktopDeviceTypes::KEYBOARD));
        assert!(devices.contains(seatgeist_portal::RemoteDesktopDeviceTypes::TOUCHSCREEN));
        assert!(!devices.contains(seatgeist_portal::RemoteDesktopDeviceTypes::POINTER));

        let empty = RemoteDesktopSessionProbeRequest {
            keyboard: false,
            pointer: false,
            touchscreen: false,
            ..request.clone()
        };
        assert!(remote_desktop_device_types(&empty).is_err());
        assert!(remote_desktop_probe_timeout(0).is_err());
        assert!(remote_desktop_probe_timeout(300_001).is_err());
        assert_eq!(
            remote_desktop_probe_timeout(30_000).expect("valid timeout"),
            Duration::from_secs(30)
        );
    }

    fn portal_session_start_fixture() -> seatgeist_portal::PortalRemoteDesktopSessionStart {
        seatgeist_portal::PortalRemoteDesktopSessionStart {
            create_request_path: "/org/freedesktop/portal/desktop/request/1/create".to_string(),
            select_request_path: "/org/freedesktop/portal/desktop/request/1/select".to_string(),
            start_request_path: "/org/freedesktop/portal/desktop/request/1/start".to_string(),
            session: seatgeist_portal::PortalRemoteDesktopSession {
                expected_session_path: "/org/freedesktop/portal/desktop/session/1/session"
                    .to_string(),
                actual_session_path: "/org/freedesktop/portal/desktop/session/1/session"
                    .to_string(),
            },
            start: seatgeist_portal::PortalRemoteDesktopStart {
                devices: seatgeist_portal::RemoteDesktopDeviceTypes::keyboard_pointer(),
                clipboard_enabled: true,
                restore_token: Some("restore-token".to_string()),
            },
        }
    }

    #[test]
    fn daemon_portal_eis_session_preserves_metadata_and_updates_state() {
        let mut source = MockEisSource::default();
        source.push_pending(vec![
            seatgeist_eis::LibeiEventSnapshot::Connect,
            seatgeist_eis::LibeiEventSnapshot::SeatAdded {
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                bound_capabilities: vec![seatgeist_eis::EisCapability::Text],
            },
        ]);
        let runtime = seatgeist_eis::EisSessionRuntime::new(source);
        let mut session = DaemonPortalEisSession::from_runtime(
            portal_session_start_fixture(),
            "/org/freedesktop/portal/desktop/session/1/session".to_string(),
            runtime,
        );

        assert_eq!(
            session.metadata(),
            &DaemonPortalEisSessionMetadata {
                selected_devices: vec!["keyboard".to_string(), "pointer".to_string()],
                clipboard_enabled: true,
                restore_token: Some("restore-token".to_string()),
                session_handle: "/org/freedesktop/portal/desktop/session/1/session".to_string(),
                create_request_path: "/org/freedesktop/portal/desktop/request/1/create".to_string(),
                select_request_path: "/org/freedesktop/portal/desktop/request/1/select".to_string(),
                start_request_path: "/org/freedesktop/portal/desktop/request/1/start".to_string(),
            }
        );

        let snapshots = session.dispatch_pending();
        assert_eq!(snapshots.len(), 2);
        assert!(session.state().connected());
        assert_eq!(
            session.state().bound_capabilities(),
            &[seatgeist_eis::EisCapability::Text]
        );
    }

    #[test]
    fn daemon_portal_eis_session_reports_plan_readiness() {
        let plan = seatgeist_eis::plan_text_utf8(1, "hello").expect("text plan");
        let mut source = MockEisSource::default();
        source.push_plan(vec![
            seatgeist_eis::LibeiEventSnapshot::Connect,
            seatgeist_eis::LibeiEventSnapshot::SeatAdded {
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                bound_capabilities: vec![seatgeist_eis::EisCapability::Text],
            },
            seatgeist_eis::LibeiEventSnapshot::DeviceResumed(seatgeist_eis::EisDeviceInfo {
                id: "text-device".to_string(),
                name: Some("Text Device".to_string()),
                kind: seatgeist_eis::EisDeviceKind::Virtual,
                resumed: true,
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                regions: Vec::new(),
            }),
        ]);
        let runtime = seatgeist_eis::EisSessionRuntime::new(source);
        let mut session = DaemonPortalEisSession::from_runtime(
            portal_session_start_fixture(),
            "/org/freedesktop/portal/desktop/session/1/session".to_string(),
            runtime,
        );

        let readiness = session.refresh_execution_readiness(&plan);

        assert_eq!(
            readiness.selection.expect("selection"),
            seatgeist_eis::EisDeviceSelection {
                device_id: "text-device".to_string(),
                device_name: Some("Text Device".to_string()),
                matched_region: None,
            }
        );
    }

    #[test]
    fn portal_eis_session_store_reports_inactive_status() {
        let store = PortalEisSessionStore::<MockEisSource>::default();

        let status = store.status().expect("inactive status");

        assert!(!status.active);
        assert!(!status.runtime_connected);
        assert!(status.bound_capabilities.is_empty());
        assert!(status.selected_devices.is_empty());
        assert!(status.setup_hint.contains("no stored"));
    }

    #[test]
    fn portal_eis_session_store_replaces_and_clears_session() {
        let mut source = MockEisSource::default();
        source.push_pending(vec![
            seatgeist_eis::LibeiEventSnapshot::Connect,
            seatgeist_eis::LibeiEventSnapshot::SeatAdded {
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                bound_capabilities: vec![seatgeist_eis::EisCapability::Text],
            },
            seatgeist_eis::LibeiEventSnapshot::DeviceResumed(seatgeist_eis::EisDeviceInfo {
                id: "text-device".to_string(),
                name: Some("Text Device".to_string()),
                kind: seatgeist_eis::EisDeviceKind::Virtual,
                resumed: true,
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                regions: Vec::new(),
            }),
        ]);
        let runtime = seatgeist_eis::EisSessionRuntime::new(source);
        let mut session = DaemonPortalEisSession::from_runtime(
            portal_session_start_fixture(),
            "/org/freedesktop/portal/desktop/session/1/session".to_string(),
            runtime,
        );
        session.dispatch_pending();
        let store = PortalEisSessionStore::default();

        store.replace(session).expect("store session");
        let status = store.status().expect("active status");

        assert!(status.active);
        assert!(status.runtime_connected);
        assert_eq!(status.bound_capabilities, vec!["text".to_string()]);
        assert_eq!(status.resumed_device_count, 1);
        assert_eq!(
            status.selected_devices,
            vec!["keyboard".to_string(), "pointer".to_string()]
        );
        assert!(status.session_handle.is_some());

        assert!(store.clear().expect("clear active session"));
        let status = store.status().expect("inactive status after clear");
        assert!(!status.active);
        assert!(!store.clear().expect("clear already inactive store"));
    }

    #[test]
    fn eis_session_input_backend_executes_ready_text_plan() {
        let mut source = MockEisSource::default();
        source.push_plan(vec![
            seatgeist_eis::LibeiEventSnapshot::Connect,
            seatgeist_eis::LibeiEventSnapshot::SeatAdded {
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                bound_capabilities: vec![seatgeist_eis::EisCapability::Text],
            },
            seatgeist_eis::LibeiEventSnapshot::DeviceResumed(seatgeist_eis::EisDeviceInfo {
                id: "text-device".to_string(),
                name: Some("Text Device".to_string()),
                kind: seatgeist_eis::EisDeviceKind::Virtual,
                resumed: true,
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                regions: Vec::new(),
            }),
        ]);
        let runtime = seatgeist_eis::EisSessionRuntime::new(source);
        let mut session = DaemonPortalEisSession::from_runtime(
            portal_session_start_fixture(),
            "/org/freedesktop/portal/desktop/session/1/session".to_string(),
            runtime,
        );

        {
            let mut backend =
                eis_session_input_execution_backend("portal_remote_desktop", &mut session);
            backend
                .type_text("hello")
                .expect("ready EIS text execution");
        }

        assert_eq!(session.runtime.source().executed_plans.len(), 1);
        let executed = &session.runtime.source().executed_plans[0];
        assert_eq!(
            executed.selection,
            seatgeist_eis::EisDeviceSelection {
                device_id: "text-device".to_string(),
                device_name: Some("Text Device".to_string()),
                matched_region: None,
            }
        );
        assert!(matches!(
            executed.events.as_slice(),
            [
                seatgeist_eis::EisEvent::StartEmulating { .. },
                seatgeist_eis::EisEvent::TextUtf8 { .. },
                seatgeist_eis::EisEvent::Frame,
                seatgeist_eis::EisEvent::StopEmulating,
            ]
        ));
    }

    #[test]
    fn eis_session_input_backend_fails_without_ready_device() {
        let mut source = MockEisSource::default();
        source.push_plan(vec![
            seatgeist_eis::LibeiEventSnapshot::Connect,
            seatgeist_eis::LibeiEventSnapshot::SeatAdded {
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                bound_capabilities: vec![seatgeist_eis::EisCapability::Text],
            },
        ]);
        let runtime = seatgeist_eis::EisSessionRuntime::new(source);
        let mut session = DaemonPortalEisSession::from_runtime(
            portal_session_start_fixture(),
            "/org/freedesktop/portal/desktop/session/1/session".to_string(),
            runtime,
        );

        let mut backend =
            eis_session_input_execution_backend("portal_remote_desktop", &mut session);
        let err = backend
            .type_text("hello")
            .expect_err("EIS execution must require a ready selected device");

        assert!(
            err.to_string()
                .contains("no resumed EIS device provides the required capabilities")
        );
        drop(backend);
        assert!(session.runtime.source().executed_plans.is_empty());
    }

    #[test]
    fn eis_capability_names_are_compact_and_stable() {
        assert_eq!(
            eis_capability_names(&[
                seatgeist_eis::EisCapability::PointerAbsolute,
                seatgeist_eis::EisCapability::Keyboard,
                seatgeist_eis::EisCapability::Button,
                seatgeist_eis::EisCapability::Scroll,
                seatgeist_eis::EisCapability::Text,
            ]),
            vec![
                "pointer_absolute".to_string(),
                "keyboard".to_string(),
                "button".to_string(),
                "scroll".to_string(),
                "text".to_string(),
            ]
        );
    }

    #[test]
    fn desktop_session_status_detects_kde_wayland() {
        let status = desktop_session_status_from_env([
            ("XDG_SESSION_TYPE", "wayland"),
            ("XDG_CURRENT_DESKTOP", "KDE"),
            ("DESKTOP_SESSION", "plasma"),
            ("KDE_FULL_SESSION", "true"),
            ("KDE_SESSION_VERSION", "6"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":0"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);

        assert_eq!(status.xdg_session_type.as_deref(), Some("wayland"));
        assert_eq!(status.xdg_current_desktop.as_deref(), Some("KDE"));
        assert!(status.dbus_session_bus_address_present);
        assert!(status.xdg_runtime_dir_present);
        assert!(status.setup_hint.contains("KDE Wayland"));
    }

    #[test]
    fn desktop_session_status_reports_missing_bus_first() {
        let status = desktop_session_status_from_env([
            ("XDG_SESSION_TYPE", "wayland"),
            ("XDG_CURRENT_DESKTOP", "KDE"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);

        assert!(!status.dbus_session_bus_address_present);
        assert!(status.xdg_runtime_dir_present);
        assert!(status.setup_hint.contains("DBUS_SESSION_BUS_ADDRESS"));
    }

    #[test]
    fn desktop_session_status_reports_non_kde_session() {
        let status = desktop_session_status_from_env([
            ("XDG_SESSION_TYPE", "wayland"),
            ("XDG_CURRENT_DESKTOP", "GNOME"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);

        assert_eq!(status.xdg_current_desktop.as_deref(), Some("GNOME"));
        assert!(status.setup_hint.contains("KDE Plasma was not detected"));
    }

    #[test]
    fn input_execution_backend_routes_current_backends() {
        let store = PortalEisSessionStore::<MockEisSource>::default();
        let keymap = XkbKeymapSettings::default();
        assert_eq!(
            input_execution_backend_with_store(InputBackendPreference::Auto, &store, &keymap)
                .expect("auto currently executes through uinput")
                .name(),
            "uinput"
        );
        assert_eq!(
            input_execution_backend_with_store(InputBackendPreference::Uinput, &store, &keymap)
                .expect("explicit uinput currently executes through uinput")
                .name(),
            "uinput"
        );

        assert_eq!(
            input_execution_backend_with_store(
                InputBackendPreference::PortalRemoteDesktop,
                &store,
                &keymap,
            )
            .expect("explicit portal backend routes through stored EIS sessions")
            .name(),
            "portal_remote_desktop"
        );
        assert_eq!(
            input_execution_backend_with_store(InputBackendPreference::Libei, &store, &keymap)
                .expect("explicit libei backend routes through stored EIS sessions")
                .name(),
            "libei"
        );
    }

    #[test]
    fn eis_key_combo_codes_use_xkb_for_symbol_parts() {
        let keymap =
            seatgeist_eis::XkbKeymap::new_from_names(seatgeist_eis::XkbKeymapNames::us_pc105())
                .expect("us xkb keymap");

        assert_eq!(
            eis_key_combo_codes_with_keymap("Ctrl+L", &keymap)
                .expect("named combo still parses through evdev names"),
            vec![29, 38]
        );
        assert_eq!(
            eis_key_combo_codes_with_keymap("Ctrl+;", &keymap)
                .expect("symbol combo parses through xkb"),
            vec![29, 39]
        );
        assert_eq!(
            eis_key_combo_codes_with_keymap("Alt+,", &keymap)
                .expect("punctuation combo parses through xkb"),
            vec![56, 51]
        );

        let err = eis_key_combo_codes_with_keymap("Ctrl+NotAKey", &keymap)
            .expect_err("unsupported multi-character key is rejected");
        assert!(err.to_string().contains("unsupported key name"));
    }

    #[test]
    fn explicit_eis_backends_require_stored_session() {
        let store = PortalEisSessionStore::<MockEisSource>::default();
        let keymap = XkbKeymapSettings::default();
        let mut libei =
            input_execution_backend_with_store(InputBackendPreference::Libei, &store, &keymap)
                .expect("libei backend routes through stored EIS sessions");
        let err = libei
            .type_text("hello")
            .expect_err("libei text execution requires a stored session");
        let err = err.to_string();
        assert!(err.contains("libei"));
        assert!(err.contains("requires a stored RemoteDesktop EIS session"));
        assert!(err.contains("remote_desktop_eis_start"));

        let mut portal = input_execution_backend_with_store(
            InputBackendPreference::PortalRemoteDesktop,
            &store,
            &keymap,
        )
        .expect("portal backend routes through stored EIS sessions");
        let err = portal
            .click_pointer(
                Point {
                    x: 25.0,
                    y: 50.0,
                    space: CoordinateSpace::PhysicalPixel,
                },
                seatgeist_uinput::PointerBounds {
                    min_x: 0,
                    min_y: 0,
                    width: 100,
                    height: 100,
                },
                PointerButton::Left,
                1,
            )
            .expect_err("portal EIS pointer execution requires a stored session");
        let err = err.to_string();
        assert!(err.contains("portal_remote_desktop"));
        assert!(err.contains("requires a stored RemoteDesktop EIS session"));
    }

    #[test]
    fn explicit_eis_backend_executes_through_stored_session() {
        let mut source = MockEisSource::default();
        source.push_plan(vec![
            seatgeist_eis::LibeiEventSnapshot::Connect,
            seatgeist_eis::LibeiEventSnapshot::SeatAdded {
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                bound_capabilities: vec![seatgeist_eis::EisCapability::Text],
            },
            seatgeist_eis::LibeiEventSnapshot::DeviceResumed(seatgeist_eis::EisDeviceInfo {
                id: "text-device".to_string(),
                name: Some("Text Device".to_string()),
                kind: seatgeist_eis::EisDeviceKind::Virtual,
                resumed: true,
                capabilities: vec![seatgeist_eis::EisCapability::Text],
                regions: Vec::new(),
            }),
        ]);
        let runtime = seatgeist_eis::EisSessionRuntime::new(source);
        let session = DaemonPortalEisSession::from_runtime(
            portal_session_start_fixture(),
            "/org/freedesktop/portal/desktop/session/1/session".to_string(),
            runtime,
        );
        let store = PortalEisSessionStore::default();
        store.replace(session).expect("store ready session");

        {
            let keymap = XkbKeymapSettings::default();
            let mut backend = input_execution_backend_with_store(
                InputBackendPreference::PortalRemoteDesktop,
                &store,
                &keymap,
            )
            .expect("portal backend routes through stored session");
            backend
                .type_text("hello")
                .expect("stored session executes text plan");
            assert_eq!(backend.name(), "portal_remote_desktop");
        }

        let stored = store.inner.lock().expect("store lock");
        let session = stored.as_ref().expect("stored session remains active");
        assert_eq!(session.runtime.source().executed_plans.len(), 1);
        assert_eq!(
            session.runtime.source().executed_plans[0]
                .selection
                .device_id,
            "text-device"
        );
    }

    #[test]
    fn explicit_eis_key_combos_execute_through_stored_session() {
        let mut source = MockEisSource::default();
        source.push_plan(vec![
            seatgeist_eis::LibeiEventSnapshot::Connect,
            seatgeist_eis::LibeiEventSnapshot::SeatAdded {
                capabilities: vec![seatgeist_eis::EisCapability::Keyboard],
                bound_capabilities: vec![seatgeist_eis::EisCapability::Keyboard],
            },
            seatgeist_eis::LibeiEventSnapshot::DeviceResumed(seatgeist_eis::EisDeviceInfo {
                id: "keyboard-device".to_string(),
                name: Some("Keyboard Device".to_string()),
                kind: seatgeist_eis::EisDeviceKind::Virtual,
                resumed: true,
                capabilities: vec![seatgeist_eis::EisCapability::Keyboard],
                regions: Vec::new(),
            }),
        ]);
        let runtime = seatgeist_eis::EisSessionRuntime::new(source);
        let session = DaemonPortalEisSession::from_runtime(
            portal_session_start_fixture(),
            "/org/freedesktop/portal/desktop/session/1/session".to_string(),
            runtime,
        );
        let store = PortalEisSessionStore::default();
        store.replace(session).expect("store ready session");

        {
            let keymap = XkbKeymapSettings {
                model: Some("pc105".to_string()),
                layout: Some("us".to_string()),
                options: Some("".to_string()),
                ..XkbKeymapSettings::default()
            };
            let mut libei =
                input_execution_backend_with_store(InputBackendPreference::Libei, &store, &keymap)
                    .expect("libei backend routes through stored EIS sessions");
            assert_eq!(
                libei
                    .key_combo("Ctrl+;")
                    .expect("stored session executes configured-keymap symbol combo"),
                2
            );
            assert_eq!(libei.name(), "libei");
        }

        let stored = store.inner.lock().expect("store lock");
        let session = stored.as_ref().expect("stored session remains active");
        assert_eq!(session.runtime.source().executed_plans.len(), 1);
        let executed = &session.runtime.source().executed_plans[0];
        assert_eq!(executed.selection.device_id, "keyboard-device");
        assert!(matches!(
            executed.events.as_slice(),
            [
                seatgeist_eis::EisEvent::StartEmulating { .. },
                seatgeist_eis::EisEvent::KeyboardKey {
                    keycode: 29,
                    is_press: true,
                },
                seatgeist_eis::EisEvent::KeyboardKey {
                    keycode: 39,
                    is_press: true,
                },
                seatgeist_eis::EisEvent::Frame,
                seatgeist_eis::EisEvent::KeyboardKey {
                    keycode: 39,
                    is_press: false,
                },
                seatgeist_eis::EisEvent::KeyboardKey {
                    keycode: 29,
                    is_press: false,
                },
                seatgeist_eis::EisEvent::Frame,
                seatgeist_eis::EisEvent::StopEmulating,
            ]
        ));
    }

    #[test]
    fn inactive_eis_backend_selection_removes_raw_input_capabilities() {
        let capabilities = current_capabilities(
            InputBackendPreference::PortalRemoteDesktop,
            false,
            false,
            false,
            false,
            false,
            false,
        );

        assert!(
            !capabilities
                .iter()
                .any(|capability| capability == &BackendCapability::KeyboardInput)
        );
        assert!(
            !capabilities
                .iter()
                .any(|capability| capability == &BackendCapability::PointerInput)
        );
    }

    #[test]
    fn active_eis_backend_selection_reports_raw_input_capabilities() {
        let capabilities = current_capabilities(
            InputBackendPreference::PortalRemoteDesktop,
            true,
            false,
            false,
            false,
            false,
            false,
        );

        assert!(
            capabilities
                .iter()
                .any(|capability| capability == &BackendCapability::KeyboardInput)
        );
        assert!(
            capabilities
                .iter()
                .any(|capability| capability == &BackendCapability::PointerInput)
        );
    }

    #[test]
    fn agent_seat_capabilities_require_plugin_registration() {
        let unavailable = current_capabilities(
            InputBackendPreference::KwinAgentSeat,
            false,
            false,
            false,
            false,
            false,
            false,
        );
        assert!(!unavailable.contains(&BackendCapability::KeyboardInput));

        let available = current_capabilities(
            InputBackendPreference::KwinAgentSeat,
            false,
            true,
            false,
            false,
            false,
            false,
        );
        assert!(available.contains(&BackendCapability::KeyboardInput));
        assert!(available.contains(&BackendCapability::PointerInput));
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
    fn window_geometry_launch_and_page_zoom_are_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let launch = DaemonRequest::LaunchWindow(LaunchWindowRequest {
            desktop_entry: "org.kde.kcalc".to_string(),
            anchor: libseatgeist::WindowPlacementAnchor::TopRight,
            monitor_id: None,
            width: Some(400),
            height: Some(300),
            margin: 20,
            activation: libseatgeist::WindowActivationMode::PreserveFocus,
            timeout_ms: 10_000,
            guard: None,
        });
        assert_eq!(
            safety_class_for_request(&launch),
            SafetyClass::ControlSemantic
        );
        assert!(enforce_policy(&policy, &launch).is_err());

        let resize = DaemonRequest::ResizeWindow(ResizeWindowRequest {
            window_id: "window-1".to_string(),
            width: 1280,
            height: 720,
            guard: None,
        });
        assert_eq!(
            safety_class_for_request(&resize),
            SafetyClass::ControlSemantic
        );
        assert!(enforce_policy(&policy, &resize).is_err());

        let zoom = DaemonRequest::PageZoom(libseatgeist::PageZoomRequest {
            operation: libseatgeist::PageZoomOperation::Out,
            steps: 2,
            guard: ActiveWindowGuard {
                desktop_revision: None,
                expected_window_id: Some("window-1".to_string()),
                expected_app_id: Some("org.mozilla.firefox".to_string()),
                title_contains: None,
            },
        });
        assert_eq!(
            safety_class_for_request(&zoom),
            SafetyClass::ControlKeyboard
        );
        assert!(enforce_policy(&policy, &zoom).is_err());
        assert_eq!(
            normalize_desktop_entry("org.kde.kcalc.desktop").expect("desktop suffix accepted"),
            "org.kde.kcalc"
        );
        assert!(normalize_desktop_entry("/usr/bin/kcalc").is_err());
        assert!(normalize_desktop_entry("kcalc --evil").is_err());
    }

    #[tokio::test]
    async fn direct_focus_uses_shared_injected_backend_after_validation() {
        let backend = seatgeist_testkit::MockWindowBackend::default();
        let result = focus_window(
            FocusWindowRequest {
                window_id: "{96d3c5da-75ec-4a2a-b75f-05c4c077153b}".to_string(),
                guard: None,
            },
            &backend,
        )
        .await
        .expect("valid focus request reaches injected backend");
        assert!(result.ok);
        assert_eq!(
            backend
                .focused_windows()
                .expect("focus calls are available")
                .as_slice(),
            ["{96d3c5da-75ec-4a2a-b75f-05c4c077153b}"]
        );

        let error = focus_window(
            FocusWindowRequest {
                window_id: "  ".to_string(),
                guard: None,
            },
            &backend,
        )
        .await
        .expect_err("empty id fails before backend execution");
        assert!(error.to_string().contains("must not be empty"));
        assert_eq!(
            backend
                .focused_windows()
                .expect("focus calls are available")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn direct_resize_uses_shared_backend_and_reports_actual_geometry() {
        let backend = seatgeist_testkit::MockWindowBackend::default();
        let result = resize_window(
            ResizeWindowRequest {
                window_id: "window-1".to_string(),
                width: 1280,
                height: 720,
                guard: None,
            },
            &backend,
        )
        .await
        .expect("valid resize reaches injected backend");
        assert!(result.ok);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|message| message.contains("actual=1280x720"))
        );
        assert_eq!(
            backend
                .resized_windows()
                .expect("resize calls are available"),
            vec![("window-1".to_string(), 1280, 720)]
        );

        let error = resize_window(
            ResizeWindowRequest {
                window_id: "window-1".to_string(),
                width: 32,
                height: 720,
                guard: None,
            },
            &backend,
        )
        .await
        .expect_err("undersized geometry fails before backend execution");
        assert!(error.to_string().contains("at least 64"));
        assert_eq!(
            backend
                .resized_windows()
                .expect("resize calls persist")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn direct_move_uses_shared_backend_and_reports_actual_geometry() {
        let backend = seatgeist_testkit::MockWindowBackend::default();
        let result = move_window(
            MoveWindowRequest {
                window_id: "window-1".to_string(),
                x: -20,
                y: 40,
                guard: None,
            },
            &backend,
        )
        .await
        .expect("valid move reaches injected backend");
        assert!(result.ok);
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|message| message.contains("actual=-20,40"))
        );
    }

    #[test]
    fn keyboard_input_is_control_keyboard_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::TypeText(TypeTextRequest {
                text: "hello".to_string(),
                guard: None,
                session_id: None,
            }),
        )
        .expect_err("type_text requires keyboard control approval by default");
        assert!(err.to_string().contains("ControlKeyboard"));

        let err = enforce_policy(
            &policy,
            &DaemonRequest::KeyCombo(KeyComboRequest {
                combo: "Ctrl+L".to_string(),
                destructive: false,
                guard: None,
                session_id: None,
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
                capture_revision: None,
                guard: None,
                session_id: None,
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
                capture_revision: None,
                guard: None,
                session_id: None,
            }),
        )
        .expect_err("click pointer requires pointer control approval by default");
        assert!(err.to_string().contains("ControlPointer"));

        let err = enforce_policy(
            &policy,
            &DaemonRequest::DragPointer(DragPointerRequest {
                from: physical_point(100.0, 200.0),
                to: physical_point(300.0, 400.0),
                button: PointerButton::Left,
                duration_ms: 250,
                capture_revision: None,
                guard: None,
                session_id: None,
            }),
        )
        .expect_err("drag pointer requires pointer control approval by default");
        assert!(err.to_string().contains("ControlPointer"));

        let err = enforce_policy(
            &policy,
            &DaemonRequest::ScrollPointer(ScrollPointerRequest {
                vertical: -1,
                horizontal: 0,
                guard: None,
                session_id: None,
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
                session_id: None,
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
                capture_revision: None,
                guard: None,
                session_id: None,
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
        let bounds = seatgeist_uinput::PointerBounds {
            min_x: -1920,
            min_y: 0,
            width: 9600,
            height: 4320,
        };

        validate_physical_pointer_point(physical_point(-1920.0, 0.0), bounds)
            .expect("minimum physical point is valid");
        validate_physical_pointer_point(physical_point(7679.0, 4319.0), bounds)
            .expect("maximum physical point is valid");

        let err = validate_physical_pointer_point(
            Point {
                x: 10.0,
                y: 10.0,
                space: CoordinateSpace::LogicalPixel,
            },
            bounds,
        )
        .expect_err("unresolved logical coordinate space is rejected");
        assert!(err.to_string().contains("physical_pixel"));

        let err = validate_physical_pointer_point(physical_point(7680.0, 4319.0), bounds)
            .expect_err("out-of-bounds coordinate is rejected");
        assert!(err.to_string().contains("outside physical desktop bounds"));
    }

    #[test]
    fn maps_logical_pointer_points_to_scaled_physical_pixels() {
        let monitors = vec![monitor("main-8k", 0, 0, 7680, 4320, 5120, 2880, 1.5)];

        let point = logical_to_physical_point(
            Point {
                x: 3200.0,
                y: 1600.0,
                space: CoordinateSpace::LogicalPixel,
            },
            &monitors,
        )
        .expect("logical point maps");

        assert_eq!(point.space, CoordinateSpace::PhysicalPixel);
        assert_eq!(point.x, 4800.0);
        assert_eq!(point.y, 2400.0);
    }

    #[test]
    fn maps_logical_pointer_points_on_negative_origin_monitor() {
        let monitors = vec![
            monitor("left", -1920, 0, 1920, 1080, 1920, 1080, 1.0),
            monitor("main", 0, 0, 3840, 2160, 3840, 2160, 1.0),
        ];

        let point = logical_to_physical_point(
            Point {
                x: -100.0,
                y: 20.0,
                space: CoordinateSpace::LogicalPixel,
            },
            &monitors,
        )
        .expect("negative-origin logical point maps");

        assert_eq!(point.space, CoordinateSpace::PhysicalPixel);
        assert_eq!(point.x, -100.0);
        assert_eq!(point.y, 20.0);
    }

    #[test]
    fn rejects_logical_pointer_points_outside_monitors() {
        let monitors = vec![monitor("main", 0, 0, 1920, 1080, 1920, 1080, 1.0)];

        let err = logical_to_physical_point(
            Point {
                x: 1920.0,
                y: 10.0,
                space: CoordinateSpace::LogicalPixel,
            },
            &monitors,
        )
        .expect_err("right edge is outside logical monitor bounds");

        assert!(err.to_string().contains("does not map to a known monitor"));
    }

    #[test]
    fn maps_window_local_pointer_points_to_scaled_physical_pixels() {
        let state = active_window_state_fixture();
        state
            .update_from_payload(
                r#"{
                    "active": true,
                    "id": "window-1",
                    "title": "Editor",
                    "app_id": "org.kde.kate",
                    "geometry": {"x": 100, "y": 200, "width": 800, "height": 600}
                }"#,
            )
            .expect("active window updates");
        let monitors = vec![monitor("main-8k", 0, 0, 7680, 4320, 5120, 2880, 1.5)];
        let active = state
            .snapshot()
            .expect("active-window snapshot succeeds")
            .flatten();

        let point = active_window_local_to_physical_point(
            Point {
                x: 50.0,
                y: 60.0,
                space: CoordinateSpace::WindowLocal,
            },
            active.as_ref(),
            &monitors,
        )
        .expect("window-local point maps");

        assert_eq!(point.space, CoordinateSpace::PhysicalPixel);
        assert_eq!(point.x, 225.0);
        assert_eq!(point.y, 390.0);
    }

    #[test]
    fn rejects_window_local_pointer_points_outside_active_window() {
        let state = active_window_state_fixture();
        state
            .update_from_payload(
                r#"{
                    "active": true,
                    "id": "window-1",
                    "title": "Editor",
                    "app_id": "org.kde.kate",
                    "geometry": {"x": 100, "y": 200, "width": 800, "height": 600}
                }"#,
            )
            .expect("active window updates");
        let monitors = vec![monitor("main", 0, 0, 1920, 1080, 1920, 1080, 1.0)];
        let active = state
            .snapshot()
            .expect("active-window snapshot succeeds")
            .flatten();

        let err = active_window_local_to_physical_point(
            Point {
                x: 800.0,
                y: 10.0,
                space: CoordinateSpace::WindowLocal,
            },
            active.as_ref(),
            &monitors,
        )
        .expect_err("edge outside active window is rejected");
        assert!(err.to_string().contains("outside active window"));
    }

    #[test]
    fn rejects_window_local_pointer_points_without_active_window() {
        let monitors = vec![monitor("main", 0, 0, 1920, 1080, 1920, 1080, 1.0)];

        let err = active_window_local_to_physical_point(
            Point {
                x: 10.0,
                y: 10.0,
                space: CoordinateSpace::WindowLocal,
            },
            None,
            &monitors,
        )
        .expect_err("missing active window is rejected");
        assert!(err.to_string().contains("require an active window"));
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
                action: libseatgeist::AccessibilityAction::Press,
                destructive: false,
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
    fn accessibility_insert_text_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityInsertText(AccessibilityInsertTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 5,
                text: "hello".to_string(),
                guard: None,
            }),
        )
        .expect_err("accessibility insert-text requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_delete_text_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityDeleteText(AccessibilityDeleteTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                start_offset: 2,
                end_offset: 5,
                guard: None,
            }),
        )
        .expect_err("accessibility delete-text requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_copy_text_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityCopyText(AccessibilityCopyTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                start_offset: 2,
                end_offset: 5,
                guard: None,
            }),
        )
        .expect_err("accessibility copy-text requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_cut_text_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityCutText(AccessibilityCutTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                start_offset: 2,
                end_offset: 5,
                guard: None,
            }),
        )
        .expect_err("accessibility cut-text requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_paste_text_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityPasteText(AccessibilityPasteTextRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 5,
                guard: None,
            }),
        )
        .expect_err("accessibility paste-text requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_set_caret_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilitySetCaret(AccessibilitySetCaretRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 5,
                guard: None,
            }),
        )
        .expect_err("accessibility set-caret requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn accessibility_set_selection_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::AccessibilitySetSelection(AccessibilitySetSelectionRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                selection_num: 0,
                start_offset: 2,
                end_offset: 8,
                guard: None,
            }),
        )
        .expect_err("accessibility set-selection requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn click_button_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::ClickButton(ClickButtonRequest {
                name: "OK".to_string(),
                destructive: false,
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
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
                target_guard: None,
            }),
        )
        .expect_err("set text field requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn focus_text_field_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::FocusTextField(FocusTextFieldRequest {
                name: "Search".to_string(),
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("focus text field requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn secret_text_field_uses_secret_field_policy() {
        let policy = PolicyEngine::new(PolicyConfig {
            default_control: ToolApprovalLevel::Allow,
            default_secret_fields: ToolApprovalLevel::Deny,
            ..PolicyConfig::default()
        });

        let err = enforce_policy(
            &policy,
            &DaemonRequest::SetTextField(SetTextFieldRequest {
                name: "Password".to_string(),
                text: "not logged".to_string(),
                app: Some("login".to_string()),
                window_name_contains: Some("sign in".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("secret-looking text fields use secret-field policy");
        assert!(err.to_string().contains("SecretField"));

        enforce_policy(
            &policy,
            &DaemonRequest::SetTextField(SetTextFieldRequest {
                name: "Search".to_string(),
                text: "query".to_string(),
                app: Some("kate".to_string()),
                window_name_contains: Some("settings".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect("non-secret text fields still use default control policy");

        let err = enforce_policy(
            &policy,
            &DaemonRequest::FocusTextField(FocusTextFieldRequest {
                name: "Password".to_string(),
                app: Some("login".to_string()),
                window_name_contains: Some("sign in".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("secret-looking focus targets use secret-field policy");
        assert!(err.to_string().contains("SecretField"));
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
                target_guard: None,
            }),
        )
        .expect_err("activate tab requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn activate_link_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::ActivateLink(ActivateLinkRequest {
                name: "Release notes".to_string(),
                app: Some("firefox".to_string()),
                window_name_contains: Some("docs".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("activate link requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn toggle_check_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::ToggleCheck(ToggleCheckRequest {
                name: "Enable feature".to_string(),
                checked: Some(true),
                app: Some("settings".to_string()),
                window_name_contains: Some("preferences".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("toggle check requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn set_value_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::SetValue(SetValueRequest {
                name: "Volume".to_string(),
                value: 0.75,
                app: Some("settings".to_string()),
                window_name_contains: Some("sound".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("set value requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn select_item_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::SelectItem(SelectItemRequest {
                name: "Printer".to_string(),
                app: Some("settings".to_string()),
                window_name_contains: Some("devices".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("select item requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn select_menu_is_control_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let err = enforce_policy(
            &policy,
            &DaemonRequest::SelectMenu(SelectMenuRequest {
                path: vec!["File".to_string(), "Open".to_string()],
                destructive: false,
                app: Some("kate".to_string()),
                window_name_contains: Some("editor".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("select menu requires control approval by default");
        assert!(err.to_string().contains("ControlSemantic"));
    }

    #[test]
    fn destructive_semantic_actions_use_destructive_policy() {
        let policy = PolicyEngine::new(PolicyConfig {
            default_control: ToolApprovalLevel::Allow,
            default_destructive_actions: ToolApprovalLevel::Deny,
            ..PolicyConfig::default()
        });

        let err = enforce_policy(
            &policy,
            &DaemonRequest::ClickButton(ClickButtonRequest {
                name: "OK".to_string(),
                destructive: true,
                app: Some("kate".to_string()),
                window_name_contains: Some("confirm".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("explicit destructive button uses destructive policy");
        assert!(err.to_string().contains("DestructiveAction"));

        let err = enforce_policy(
            &policy,
            &DaemonRequest::SelectMenu(SelectMenuRequest {
                path: vec!["File".to_string(), "Delete".to_string()],
                destructive: false,
                app: Some("kate".to_string()),
                window_name_contains: Some("editor".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect_err("destructive menu label uses destructive policy");
        assert!(err.to_string().contains("DestructiveAction"));

        enforce_policy(
            &policy,
            &DaemonRequest::ClickButton(ClickButtonRequest {
                name: "Open".to_string(),
                destructive: false,
                app: Some("kate".to_string()),
                window_name_contains: Some("dialog".to_string()),
                max_nodes: 256,
                guard: None,
                target_guard: None,
            }),
        )
        .expect("non-destructive semantic control still uses default control policy");
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
    fn semantic_resolvers_do_not_depend_on_stable_node_ids() {
        let first = resolve_click_button_match("Save", vec![button_node("__rg-1::42", "Save")])
            .expect("first dynamic id resolves");
        let second = resolve_click_button_match("Save", vec![button_node("__rg-9::7", "Save")])
            .expect("second dynamic id resolves");
        assert_ne!(first.id, second.id);
        assert_eq!(first.role, second.role);
        assert_eq!(first.name, second.name);

        let first_menu = resolve_menu_path_match(
            &["File".to_string(), "Open".to_string()],
            vec![menu_node(
                "__menu-session-a",
                "File",
                vec![menu_item_node("__item-session-a", "Open")],
            )],
        )
        .expect("first dynamic menu path resolves");
        let second_menu = resolve_menu_path_match(
            &["File".to_string(), "Open".to_string()],
            vec![menu_node(
                "__menu-session-b",
                "File",
                vec![menu_item_node("__item-session-b", "Open")],
            )],
        )
        .expect("second dynamic menu path resolves");
        assert_ne!(first_menu.0.id, second_menu.0.id);
        assert_eq!(first_menu.0.role, second_menu.0.role);
        assert_eq!(first_menu.0.name, second_menu.0.name);
        assert_eq!(first_menu.1, second_menu.1);
    }

    #[test]
    fn semantic_candidate_ids_survive_dynamic_node_ids() {
        let first_session = vec![
            button_node("__rg-1::42", "Save"),
            button_node("__rg-1::43", "Save As"),
        ];
        let second_session = vec![
            button_node("__rg-9::7", "Save"),
            button_node("__rg-9::8", "Save As"),
        ];

        assert_ne!(first_session[0].id, second_session[0].id);
        assert_eq!(
            semantic_candidate_id(1, &first_session[0]),
            semantic_candidate_id(1, &second_session[0])
        );
        assert_eq!(
            semantic_candidate_id(2, &first_session[1]),
            semantic_candidate_id(2, &second_session[1])
        );
        assert_ne!(
            semantic_candidate_id(1, &first_session[0]),
            semantic_candidate_id(2, &first_session[1])
        );

        let first_summary = semantic_choice_summary("Save", &first_session);
        let second_summary = semantic_choice_summary("Save", &second_session);
        let stable_save_id = semantic_candidate_id(1, &first_session[0]);
        assert!(first_summary.contains(&format!(
            "choice=1 candidate_id={stable_save_id} id=__rg-1::42"
        )));
        assert!(second_summary.contains(&format!(
            "choice=1 candidate_id={stable_save_id} id=__rg-9::7"
        )));
    }

    #[test]
    fn accessibility_quality_counts_semantic_signals() {
        let mut root = libseatgeist::AccessibilityNode {
            id: "root".to_string(),
            role: "frame".to_string(),
            name: Some("Editor".to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: vec!["focused".to_string()],
            bounds: None,
            available_actions: Vec::new(),
            actions: Vec::new(),
            children: vec![
                button_node("save", "Save"),
                text_node("body", "Document Body"),
                libseatgeist::AccessibilityNode {
                    id: "canvas".to_string(),
                    role: "canvas".to_string(),
                    name: None,
                    value: None,
                    value_truncated: false,
                    sensitive: false,
                    states: Vec::new(),
                    bounds: None,
                    available_actions: Vec::new(),
                    actions: Vec::new(),
                    children: Vec::new(),
                },
            ],
        };
        root.children[1].sensitive = true;
        let mut counts = AccessibilityQualityCounts::default();
        collect_accessibility_quality_counts(&root, 0, &mut counts);
        assert_eq!(counts.sampled_node_count, 4);
        assert_eq!(counts.named_node_count, 3);
        assert_eq!(counts.actionable_node_count, 2);
        assert_eq!(counts.text_node_count, 1);
        assert_eq!(counts.sensitive_node_count, 1);
        assert_eq!(counts.generic_role_count, 1);
        assert_eq!(counts.max_depth_seen, 1);
    }

    #[test]
    fn accessibility_registry_process_count_ignores_other_users_and_processes() {
        let proc_root = temp_test_private_dir("atspi-proc");
        for (pid, comm, uid) in [
            ("101", "at-spi2-registryd\n", 1000),
            ("102", "at-spi2-registr\n", 1000),
            ("103", "at-spi2-registryd\n", 1001),
            ("104", "firefox\n", 1000),
        ] {
            let process_dir = proc_root.join(pid);
            fs::create_dir_all(&process_dir).expect("process fixture directory is created");
            fs::write(process_dir.join("comm"), comm).expect("process comm fixture is written");
            fs::write(
                process_dir.join("status"),
                format!("Name:\tfixture\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .expect("process status fixture is written");
        }
        fs::create_dir_all(proc_root.join("not-a-pid")).expect("non-pid fixture is created");

        assert_eq!(
            accessibility_registry_process_count(&proc_root, 1000),
            Some(2)
        );
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn accessibility_quality_reports_flat_weak_tree_fixture() {
        let status = accessibility_quality_status_from_sample(
            4,
            512,
            Ok(Some(generic_node("canvas", "canvas", vec![]))),
        );
        assert!(status.atspi_available);
        assert!(status.focused_node_present);
        assert_eq!(status.sampled_node_count, 1);
        assert_eq!(status.generic_role_count, 1);
        assert_eq!(status.max_depth_seen, 0);
        assert!(status.tree_flat);
        assert!(!status.semantic_targeting_reliable);
        assert_eq!(
            status.recommended_fallback,
            "screenshot_tile_or_structured_integration"
        );
        assert!(status.setup_hint.contains("flat"));

        let summary = summarize_response(&DaemonResponse::AccessibilityQualityStatus(status));
        assert!(summary.contains("accessibility quality"));
        assert!(summary.contains("reliable=false"));
        assert!(summary.contains("nodes=1"));
        assert!(summary.contains("generic=1"));
        assert!(summary.contains("flat=true"));
    }

    #[test]
    fn accessibility_quality_reports_mostly_generic_weak_tree_fixture() {
        let status = accessibility_quality_status_from_sample(
            4,
            512,
            Ok(Some(generic_node(
                "root-panel",
                "panel",
                vec![generic_node(
                    "section",
                    "section",
                    vec![generic_node("layer", "layer", vec![])],
                )],
            ))),
        );
        assert!(status.atspi_available);
        assert!(status.focused_node_present);
        assert_eq!(status.sampled_node_count, 3);
        assert_eq!(status.generic_role_count, 3);
        assert_eq!(status.max_depth_seen, 2);
        assert!(!status.tree_flat);
        assert!(!status.semantic_targeting_reliable);
        assert_eq!(
            status.recommended_fallback,
            "screenshot_tile_or_structured_integration"
        );
        assert!(status.setup_hint.contains("mostly generic"));

        let summary = summarize_response(&DaemonResponse::AccessibilityQualityStatus(status));
        assert!(summary.contains("reliable=false"));
        assert!(summary.contains("nodes=3"));
        assert!(summary.contains("generic=3"));
        assert!(summary.contains("flat=false"));
    }

    #[test]
    fn click_button_resolver_refuses_ambiguous_matches() {
        let err = resolve_click_button_match(
            "Open",
            vec![button_node("1", "Open"), button_node("2", "Open")],
        )
        .expect_err("multiple exact matches are ambiguous");
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=button name=Open score=1.00 actions=press"));
        assert!(err.contains("id=2 role=button name=Open score=1.00 actions=press"));
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
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=text name=Search score=1.00 actions=set_text"));
        assert!(err.contains("id=2 role=text name=Search score=1.00 actions=set_text"));
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
    fn focus_text_field_resolver_requires_focus_action() {
        let err = resolve_focus_text_field_match("Search", vec![text_node("1", "Search")])
            .expect_err("text fields without focus actions are not viable");
        assert!(err.to_string().contains("focusable"));
    }

    #[test]
    fn focus_text_field_resolver_prefers_exact_match() {
        let target = resolve_focus_text_field_match(
            "Search",
            vec![
                focusable_text_node("1", "Search everywhere"),
                focusable_text_node("2", "Search"),
            ],
        )
        .expect("exact focusable text field resolves");
        assert_eq!(target.id, "2");
    }

    #[test]
    fn focus_text_field_resolver_refuses_ambiguous_matches() {
        let err = resolve_focus_text_field_match(
            "Search",
            vec![
                focusable_text_node("1", "Search"),
                focusable_text_node("2", "Search"),
            ],
        )
        .expect_err("multiple exact focusable text fields are ambiguous");
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=text name=Search score=1.00 actions=set_text|focus"));
        assert!(err.contains("id=2 role=text name=Search score=1.00 actions=set_text|focus"));
    }

    #[test]
    fn tab_resolver_prefers_exact_match_and_select_action() {
        let (target, action) = resolve_tab_match(
            "General",
            vec![tab_node("1", "General settings"), tab_node("2", "General")],
        )
        .expect("exact tab resolves");
        assert_eq!(target.id, "2");
        assert_eq!(action, libseatgeist::AccessibilityAction::Select);
    }

    #[test]
    fn tab_resolver_uses_press_when_select_is_unavailable() {
        let (target, action) = resolve_tab_match("General", vec![press_tab_node("1", "General")])
            .expect("pressable tab resolves");
        assert_eq!(target.id, "1");
        assert_eq!(action, libseatgeist::AccessibilityAction::Press);
    }

    #[test]
    fn tab_resolver_refuses_ambiguous_matches() {
        let err = resolve_tab_match(
            "General",
            vec![tab_node("1", "General"), tab_node("2", "General")],
        )
        .expect_err("multiple exact tabs are ambiguous");
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=page tab name=General score=1.00 actions=press|select"));
        assert!(err.contains("id=2 role=page tab name=General score=1.00 actions=press|select"));
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
    fn link_resolver_prefers_exact_match_and_press_action() {
        let (target, action) = resolve_link_match(
            "Release notes",
            vec![
                link_node("1", "Release notes archive"),
                link_node("2", "Release notes"),
            ],
        )
        .expect("exact link resolves");
        assert_eq!(target.id, "2");
        assert_eq!(action, libseatgeist::AccessibilityAction::Press);
    }

    #[test]
    fn link_resolver_uses_select_when_press_is_unavailable() {
        let (target, action) = resolve_link_match("Docs", vec![select_link_node("1", "Docs")])
            .expect("selectable link resolves");
        assert_eq!(target.id, "1");
        assert_eq!(action, libseatgeist::AccessibilityAction::Select);
    }

    #[test]
    fn link_resolver_refuses_ambiguous_matches() {
        let err = resolve_link_match("Open", vec![link_node("1", "Open"), link_node("2", "Open")])
            .expect_err("multiple exact links are ambiguous");
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=link name=Open score=1.00 actions=press"));
        assert!(err.contains("id=2 role=link name=Open score=1.00 actions=press"));
    }

    #[test]
    fn link_resolver_requires_non_sensitive_activatable_link() {
        let mut sensitive = link_node("1", "Secret report");
        sensitive.sensitive = true;
        let err = resolve_link_match("Secret report", vec![sensitive])
            .expect_err("sensitive links are not viable");
        assert!(err.to_string().contains("no non-sensitive"));
    }

    #[test]
    fn check_resolver_prefers_exact_match_and_press_action() {
        let (target, action) = resolve_check_match(
            "Enable feature",
            vec![
                check_node("1", "Enable feature later", false),
                check_node("2", "Enable feature", true),
            ],
        )
        .expect("exact check resolves");
        assert_eq!(target.id, "2");
        assert_eq!(action, libseatgeist::AccessibilityAction::Press);
        assert!(node_checked_state(&target));
    }

    #[test]
    fn check_resolver_refuses_ambiguous_matches() {
        let err = resolve_check_match(
            "Enable feature",
            vec![
                check_node("1", "Enable feature", false),
                check_node("2", "Enable feature", false),
            ],
        )
        .expect_err("multiple exact checks are ambiguous");
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=check box name=Enable feature score=1.00 actions=press"));
        assert!(err.contains("id=2 role=check box name=Enable feature score=1.00 actions=press"));
    }

    #[test]
    fn check_resolver_requires_non_sensitive_activatable_match() {
        let mut sensitive = check_node("1", "Enable feature", false);
        sensitive.sensitive = true;
        let err = resolve_check_match("Enable feature", vec![sensitive])
            .expect_err("sensitive checks are not viable");
        assert!(err.to_string().contains("no non-sensitive"));
    }

    #[test]
    fn value_resolver_prefers_exact_numeric_match() {
        let target = resolve_value_match(
            "Volume",
            vec![
                value_node("1", "Volume control", "0.25"),
                value_node("2", "Volume", "0.50"),
            ],
        )
        .expect("exact value control resolves");
        assert_eq!(target.id, "2");
    }

    #[test]
    fn value_resolver_refuses_ambiguous_matches() {
        let err = resolve_value_match(
            "Volume",
            vec![
                value_node("1", "Volume", "0.25"),
                value_node("2", "Volume", "0.50"),
            ],
        )
        .expect_err("multiple exact value controls are ambiguous");
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=slider name=Volume"));
        assert!(err.contains("id=2 role=slider name=Volume"));
    }

    #[test]
    fn value_resolver_requires_non_sensitive_numeric_value_control() {
        let mut sensitive = value_node("1", "Volume", "0.25");
        sensitive.sensitive = true;
        let non_numeric = value_node("2", "Volume", "loud");
        let err = resolve_value_match("Volume", vec![sensitive, non_numeric])
            .expect_err("sensitive and non-numeric value controls are not viable");
        assert!(err.to_string().contains("no non-sensitive"));
    }

    #[test]
    fn select_item_resolver_prefers_exact_match_and_select_action() {
        let (target, action) = resolve_select_item_match(
            "Printer",
            vec![
                list_item_node("1", "Printer settings"),
                list_item_node("2", "Printer"),
            ],
        )
        .expect("exact item resolves");
        assert_eq!(target.id, "2");
        assert_eq!(action, libseatgeist::AccessibilityAction::Select);
    }

    #[test]
    fn select_item_resolver_uses_press_when_select_is_unavailable() {
        let mut item = list_item_node("1", "Printer");
        item.actions = vec![libseatgeist::AccessibilityAction::Press];
        item.available_actions = vec!["press".to_string()];
        let (target, action) =
            resolve_select_item_match("Printer", vec![item]).expect("pressable item resolves");
        assert_eq!(target.id, "1");
        assert_eq!(action, libseatgeist::AccessibilityAction::Press);
    }

    #[test]
    fn select_item_resolver_refuses_ambiguous_matches() {
        let err = resolve_select_item_match(
            "Printer",
            vec![
                list_item_node("1", "Printer"),
                list_item_node("2", "Printer"),
            ],
        )
        .expect_err("multiple exact items are ambiguous");
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=1 role=list item name=Printer score=1.00 actions=select"));
        assert!(err.contains("id=2 role=list item name=Printer score=1.00 actions=select"));
    }

    #[test]
    fn select_item_resolver_requires_non_sensitive_selectable_match() {
        let mut sensitive = list_item_node("1", "Secret network");
        sensitive.sensitive = true;
        let mut inert = list_item_node("2", "Secret network");
        inert.actions.clear();
        inert.available_actions.clear();
        let err = resolve_select_item_match("Secret network", vec![sensitive, inert])
            .expect_err("sensitive and inert items are not viable");
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
        assert_eq!(action, libseatgeist::AccessibilityAction::Select);
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
        assert_eq!(action, libseatgeist::AccessibilityAction::Press);
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
        let err = err.to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("choices=[choice=1 candidate_id="));
        assert!(err.contains("id=open1 role=menu item name=Open score=1.00 actions=press|select"));
        assert!(err.contains("id=open2 role=menu item name=Open score=1.00 actions=press|select"));
        assert!(err.contains("action=select"));
    }

    #[test]
    fn semantic_choice_summary_is_bounded() {
        let choices = (1..=7)
            .map(|index| button_node(&format!("button-{index}"), "Open"))
            .collect::<Vec<_>>();

        let summary = semantic_choice_summary("Open", &choices);
        assert!(summary.contains("id=button-1 role=button name=Open score=1.00 actions=press"));
        assert!(summary.contains("id=button-5 role=button name=Open score=1.00 actions=press"));
        assert!(!summary.contains("id=button-6"));
        assert!(summary.contains("+2 more"));
    }

    #[test]
    fn semantic_choice_summary_scores_name_quality() {
        let choices = vec![
            button_node("exact", "Open"),
            button_node("prefix", "Open Recent"),
            button_node("contains", "Reopen"),
            button_node("missing-name", ""),
        ];

        let summary = semantic_choice_summary("Open", &choices);
        assert!(summary.contains("id=exact role=button name=Open score=1.00 actions=press"));
        assert!(
            summary.contains("id=prefix role=button name=Open Recent score=0.85 actions=press")
        );
        assert!(summary.contains("id=contains role=button name=Reopen score=0.65 actions=press"));
        assert!(summary.contains("id=missing-name role=button name= score=0.00 actions=press"));
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
            &DaemonRequest::ClipboardGet(libseatgeist::ClipboardGetRequest {
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
            &DaemonRequest::ClipboardGet(libseatgeist::ClipboardGetRequest {
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
    fn clipboard_write_is_allowed_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(
            &policy,
            &DaemonRequest::ClipboardSet(libseatgeist::ClipboardSetRequest {
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

    #[test]
    fn accessibility_text_attributes_is_observe_policy() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        enforce_policy(
            &policy,
            &DaemonRequest::AccessibilityTextAttributes(AccessibilityTextAttributesRequest {
                node_id: "atspi://:1.42/org/a11y/atspi/accessible/7".to_string(),
                offset: 3,
                include_defaults: false,
            }),
        )
        .expect("accessibility text attributes are observe policy");
    }

    fn button_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "button".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["click".to_string()],
            actions: vec![libseatgeist::AccessibilityAction::Press],
            children: Vec::new(),
        }
    }

    fn text_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "text".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: Vec::new(),
            actions: vec![libseatgeist::AccessibilityAction::SetText],
            children: Vec::new(),
        }
    }

    fn generic_node(
        id: &str,
        role: &str,
        children: Vec<libseatgeist::AccessibilityNode>,
    ) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: role.to_string(),
            name: None,
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

    fn focusable_text_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        let mut node = text_node(id, name);
        node.available_actions = vec!["set text".to_string(), "grab focus".to_string()];
        node.actions = vec![
            libseatgeist::AccessibilityAction::SetText,
            libseatgeist::AccessibilityAction::Focus,
        ];
        node
    }

    fn tab_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        let mut node = press_tab_node(id, name);
        node.actions = vec![
            libseatgeist::AccessibilityAction::Press,
            libseatgeist::AccessibilityAction::Select,
        ];
        node.available_actions = vec!["press".to_string(), "select".to_string()];
        node
    }

    fn press_tab_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "page tab".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["press".to_string()],
            actions: vec![libseatgeist::AccessibilityAction::Press],
            children: Vec::new(),
        }
    }

    fn link_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "link".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["press".to_string()],
            actions: vec![libseatgeist::AccessibilityAction::Press],
            children: Vec::new(),
        }
    }

    fn select_link_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        let mut node = link_node(id, name);
        node.available_actions = vec!["select".to_string()];
        node.actions = vec![libseatgeist::AccessibilityAction::Select];
        node
    }

    fn check_node(id: &str, name: &str, checked: bool) -> libseatgeist::AccessibilityNode {
        let mut states = Vec::new();
        if checked {
            states.push("checked".to_string());
        }
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "check box".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states,
            bounds: None,
            available_actions: vec!["press".to_string()],
            actions: vec![libseatgeist::AccessibilityAction::Press],
            children: Vec::new(),
        }
    }

    fn value_node(id: &str, name: &str, value: &str) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "slider".to_string(),
            name: Some(name.to_string()),
            value: Some(value.to_string()),
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: Vec::new(),
            actions: Vec::new(),
            children: Vec::new(),
        }
    }

    fn list_item_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "list item".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["select".to_string()],
            actions: vec![libseatgeist::AccessibilityAction::Select],
            children: Vec::new(),
        }
    }

    fn menu_node(
        id: &str,
        name: &str,
        children: Vec<libseatgeist::AccessibilityNode>,
    ) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
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

    fn menu_item_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        let mut node = press_menu_item_node(id, name);
        node.actions = vec![
            libseatgeist::AccessibilityAction::Press,
            libseatgeist::AccessibilityAction::Select,
        ];
        node.available_actions = vec!["press".to_string(), "select".to_string()];
        node
    }

    fn press_menu_item_node(id: &str, name: &str) -> libseatgeist::AccessibilityNode {
        libseatgeist::AccessibilityNode {
            id: id.to_string(),
            role: "menu item".to_string(),
            name: Some(name.to_string()),
            value: None,
            value_truncated: false,
            sensitive: false,
            states: Vec::new(),
            bounds: None,
            available_actions: vec!["press".to_string()],
            actions: vec![libseatgeist::AccessibilityAction::Press],
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

    fn active_window_state_fixture() -> ActiveWindowState {
        ActiveWindowState::default()
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
    ) -> libseatgeist::MonitorInfo {
        libseatgeist::MonitorInfo {
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
                output_coordinate_space: CoordinateSpace::CaptureOutput,
                source_extent_width: Some(7680),
                source_extent_height: Some(4320),
                source_origin_x: 0,
                source_origin_y: 0,
                scale_x: 1600.0 / 7680.0,
                scale_y: 900.0 / 4320.0,
            },
            coordinate_space: CoordinateSpace::PhysicalPixel,
            monitors: Vec::new(),
        }
    }

    fn temp_test_path(name: &str) -> PathBuf {
        static NEXT_TEST_PATH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!("seatgeist-tests-{}", std::process::id()));
        fs::create_dir_all(&root).expect("private test root is created");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("private test root permissions are strict");
        let sequence = NEXT_TEST_PATH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        root.join(format!("{name}-{sequence}"))
    }

    fn temp_test_private_dir(name: &str) -> PathBuf {
        let path = temp_test_path(name);
        fs::create_dir_all(&path).expect("private test dir is created");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("private test dir permissions are strict");
        path
    }

    fn write_test_approval_grant(
        path: &Path,
        safety_class: SafetyClass,
        method: &str,
        expires_unix_ms: u64,
    ) {
        let grant = serde_json::json!({
            "safety_class": safety_class,
            "method": method,
            "expires_unix_ms": expires_unix_ms,
            "reason": "test approval",
        });
        fs::write(
            path,
            format!(
                "{}\n",
                serde_json::to_string(&grant).expect("approval grant serializes")
            ),
        )
        .expect("approval grant is written");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .expect("approval file permissions are strict");
    }
}
