use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use libplasma_pilot::{
    BackendCapability, CapabilitySet, ClipboardGetRequest, DaemonRequest, DaemonResponse,
    DesktopSessionStatus, HealthStatus, JournalEntry, PanicStopStatus, PolicyStatus,
    RemoteDesktopEisSessionStatus, ReplayTrace, SafetyStatus, ToolApprovalLevel,
    TraceJsonExpectation, TraceStep, UinputStatus,
};
use std::os::unix::fs::PermissionsExt;

struct DaemonFixture {
    child: Child,
    socket: PathBuf,
    root: PathBuf,
}

impl DaemonFixture {
    fn start() -> Result<Self> {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).context("create integration temp dir")?;
        let socket = root.join("plasma-pilotd.sock");
        let journal = root.join("journal.jsonl");
        let panic_stop = root.join("panic-stop");
        let child = Command::new(daemon_binary()?)
            .arg("--socket")
            .arg(&socket)
            .arg("--journal")
            .arg(&journal)
            .arg("--panic-stop-file")
            .arg(&panic_stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn plasma-pilotd")?;
        wait_for_socket(&socket)?;
        Ok(Self {
            child,
            socket,
            root,
        })
    }

    fn start_with_config(config_contents: &str) -> Result<Self> {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).context("create integration temp dir")?;
        let socket = root.join("configured.sock");
        let journal = root.join("configured-journal.jsonl");
        let panic_stop = root.join("configured-panic-stop");
        let config = root.join("config.toml");
        fs::write(
            &config,
            config_contents
                .replace("__SOCKET__", &socket.display().to_string())
                .replace("__JOURNAL__", &journal.display().to_string())
                .replace("__PANIC_STOP__", &panic_stop.display().to_string()),
        )
        .context("write daemon config fixture")?;

        let child = Command::new(daemon_binary()?)
            .arg("--config")
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn configured plasma-pilotd")?;
        wait_for_socket(&socket)?;
        Ok(Self {
            child,
            socket,
            root,
        })
    }

    fn cli_json(&self, args: &[&str]) -> Result<DaemonResponse> {
        let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
            .arg("--socket")
            .arg(&self.socket)
            .args(args)
            .output()
            .with_context(|| format!("run plasma-pilot-cli {}", args.join(" ")))?;
        require_success(args, &output)?;
        serde_json::from_slice(&output.stdout).context("parse CLI JSON response")
    }

    fn cli_value(&self, args: &[&str]) -> Result<serde_json::Value> {
        let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
            .arg("--socket")
            .arg(&self.socket)
            .args(args)
            .output()
            .with_context(|| format!("run plasma-pilot-cli {}", args.join(" ")))?;
        require_success(args, &output)?;
        serde_json::from_slice(&output.stdout).context("parse CLI JSON value")
    }

    fn cli_output(&self, args: &[&str]) -> Result<Output> {
        Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
            .arg("--socket")
            .arg(&self.socket)
            .args(args)
            .output()
            .with_context(|| format!("run plasma-pilot-cli {}", args.join(" ")))
    }
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn cli_talks_to_real_daemon_for_status_commands() -> Result<()> {
    let daemon = DaemonFixture::start()?;

    let health = daemon.cli_json(&["doctor"])?;
    assert_eq!(
        health,
        DaemonResponse::Health(HealthStatus {
            service: "plasma-pilotd".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            status: "ok".to_string(),
        })
    );

    let capabilities = daemon.cli_json(&["capabilities"])?;
    let DaemonResponse::Capabilities(CapabilitySet { capabilities }) = capabilities else {
        bail!("expected capabilities response, got {capabilities:?}");
    };
    assert!(capabilities.contains(&BackendCapability::DaemonHealth));
    assert!(capabilities.contains(&BackendCapability::DaemonPolicyStatus));
    assert!(capabilities.contains(&BackendCapability::DaemonSafetyStatus));
    assert!(capabilities.contains(&BackendCapability::DaemonDesktopSessionStatus));

    let policy = daemon.cli_json(&["policy-status"])?;
    assert_eq!(
        policy,
        DaemonResponse::PolicyStatus(PolicyStatus {
            default_observe: ToolApprovalLevel::Allow,
            default_control: ToolApprovalLevel::Prompt,
            default_destructive_actions: ToolApprovalLevel::Prompt,
            default_secret_fields: ToolApprovalLevel::Deny,
            default_full_resolution_screenshot: ToolApprovalLevel::Prompt,
            default_clipboard_read: ToolApprovalLevel::Prompt,
            default_clipboard_write: ToolApprovalLevel::Allow,
        })
    );

    let safety = daemon.cli_json(&["safety-status"])?;
    assert_eq!(
        safety,
        DaemonResponse::SafetyStatus(SafetyStatus {
            require_focus_guard: true,
            pause_on_human_input: false,
            human_input_activity_file: None,
            human_input_quiet_ms: 1500,
            human_input_signal_fresh: false,
            human_input_signal_age_ms: None,
            control_rate_limit_per_minute: Some(120),
            preview_max_edge: 1600,
            tile_max_edge: 1600,
            screenshot_redaction_count: 0,
            journal_artifact_metadata_enabled: false,
        })
    );

    let desktop_session = daemon.cli_json(&["desktop-session-status"])?;
    let DaemonResponse::DesktopSessionStatus(DesktopSessionStatus { setup_hint, .. }) =
        desktop_session
    else {
        bail!("expected desktop session status response, got {desktop_session:?}");
    };
    assert!(!setup_hint.is_empty());

    let uinput = daemon.cli_json(&["input", "status"])?;
    let DaemonResponse::UinputStatus(UinputStatus {
        path, setup_hint, ..
    }) = uinput
    else {
        bail!("expected uinput status response, got {uinput:?}");
    };
    assert_eq!(path, PathBuf::from("/dev/uinput"));
    assert!(!setup_hint.is_empty());

    let input_backends = daemon.cli_json(&["input", "backends"])?;
    let DaemonResponse::InputBackendStatus(status) = input_backends else {
        bail!("expected input backend status response, got {input_backends:?}");
    };
    assert!(!status.remote_desktop_portal.setup_hint.is_empty());
    assert!(!status.libei.setup_hint.is_empty());
    if status.implemented_available_backend.is_some() {
        assert_eq!(
            status.implemented_available_backend.as_deref(),
            Some("uinput")
        );
    }
    assert!(!status.setup_hint.is_empty());

    let eis_session_status = daemon.cli_json(&["input", "remote-desktop-eis-session-status"])?;
    assert_eq!(
        eis_session_status,
        DaemonResponse::RemoteDesktopEisSessionStatus(RemoteDesktopEisSessionStatus {
            active: false,
            runtime_connected: false,
            bound_capabilities: vec![],
            resumed_device_count: 0,
            selected_devices: vec![],
            clipboard_enabled: false,
            restore_token: None,
            session_handle: None,
            create_request_path: None,
            select_request_path: None,
            start_request_path: None,
            setup_hint:
                "no stored portal RemoteDesktop EIS session; start one before selecting portal/libei execution"
                    .to_string(),
        })
    );

    let stopped_eis_session = daemon.cli_json(&["input", "remote-desktop-eis-stop"])?;
    assert_eq!(
        stopped_eis_session,
        DaemonResponse::RemoteDesktopEisSessionStatus(RemoteDesktopEisSessionStatus {
            active: false,
            runtime_connected: false,
            bound_capabilities: vec![],
            resumed_device_count: 0,
            selected_devices: vec![],
            clipboard_enabled: false,
            restore_token: None,
            session_handle: None,
            create_request_path: None,
            select_request_path: None,
            start_request_path: None,
            setup_hint: "no stored portal RemoteDesktop EIS session was active".to_string(),
        })
    );

    let capture_backends = daemon.cli_json(&["capture-backends"])?;
    let DaemonResponse::CaptureBackendStatus(status) = capture_backends else {
        bail!("expected capture backend status response, got {capture_backends:?}");
    };
    assert!(!status.screenshot_portal.setup_hint.is_empty());
    assert!(!status.kwin_metadata.setup_hint.is_empty());
    assert!(!status.spectacle.setup_hint.is_empty());
    if status.implemented_available_backend.is_some() {
        assert_eq!(
            status.implemented_available_backend.as_deref(),
            Some("spectacle")
        );
    }
    assert!(!status.setup_hint.is_empty());

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "20"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(
        &entries,
        &[
            "capabilities",
            "policy_status",
            "safety_status",
            "desktop_session_status",
            "uinput_status",
            "input_backend_status",
            "capture_backend_status",
        ],
    );
    assert!(entries.iter().all(|entry| entry.ok));
    assert!(entries.iter().all(|entry| {
        entry
            .client
            .as_ref()
            .and_then(|client| client.tool.as_deref())
            == Some("plasma-pilot-cli")
    }));
    Ok(())
}

#[test]
fn cli_reports_configured_safety_bounds() -> Result<()> {
    let daemon = DaemonFixture::start_with_config(
        r#"
[daemon]
socket = "__SOCKET__"
journal = "__JOURNAL__"
panic_stop_file = "__PANIC_STOP__"

[journal]
include_artifact_metadata = true

[safety]
require_focus_guard = false
human_input_quiet_ms = 2500
control_rate_limit_per_minute = 42
preview_max_edge = 1024
tile_max_edge = 2048
"#,
    )?;

    let safety = daemon.cli_json(&["safety-status"])?;
    assert_eq!(
        safety,
        DaemonResponse::SafetyStatus(SafetyStatus {
            require_focus_guard: false,
            pause_on_human_input: false,
            human_input_activity_file: None,
            human_input_quiet_ms: 2500,
            human_input_signal_fresh: false,
            human_input_signal_age_ms: None,
            control_rate_limit_per_minute: Some(42),
            preview_max_edge: 1024,
            tile_max_edge: 2048,
            screenshot_redaction_count: 0,
            journal_artifact_metadata_enabled: true,
        })
    );
    Ok(())
}

#[test]
fn cli_routes_atspi_text_attribute_validation_to_daemon() -> Result<()> {
    let daemon = DaemonFixture::start()?;

    let output = daemon.cli_output(&[
        "atspi",
        "text-attributes",
        "--node",
        "",
        "--offset",
        "0",
        "--include-defaults",
    ])?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("node_id must be non-empty"),
        "stderr did not contain validation error: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "20"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(&entries, &["accessibility_text_attributes"]);
    let Some(entry) = entries
        .iter()
        .find(|entry| entry.method == "accessibility_text_attributes")
    else {
        bail!("missing accessibility_text_attributes journal entry: {entries:?}");
    };
    assert!(!entry.ok);
    assert_eq!(
        entry.safety_class,
        Some(libplasma_pilot::SafetyClass::Observe)
    );
    Ok(())
}

#[test]
fn cli_toggles_private_panic_stop_file() -> Result<()> {
    let daemon = DaemonFixture::start()?;

    let status = daemon.cli_json(&["panic-stop", "status"])?;
    let DaemonResponse::PanicStop(PanicStopStatus { enabled, path }) = status else {
        bail!("expected panic-stop response, got {status:?}");
    };
    assert!(!enabled);
    assert!(path.starts_with(&daemon.root));

    let enabled_response = daemon.cli_json(&["panic-stop", "enable"])?;
    let DaemonResponse::PanicStop(PanicStopStatus { enabled, path }) = enabled_response else {
        bail!("expected panic-stop enable response, got {enabled_response:?}");
    };
    assert!(enabled);
    assert!(path.exists());

    let disabled_response = daemon.cli_json(&["panic-stop", "disable"])?;
    let DaemonResponse::PanicStop(PanicStopStatus { enabled, path }) = disabled_response else {
        bail!("expected panic-stop disable response, got {disabled_response:?}");
    };
    assert!(!enabled);
    assert!(!path.exists());

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "10"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(&entries, &["panic_stop_status", "set_panic_stop"]);
    assert!(entries.iter().all(|entry| entry.ok));

    let journal = daemon.cli_json(&[
        "journal",
        "tail",
        "--limit",
        "10",
        "--method",
        "set_panic_stop",
        "--ok",
        "true",
    ])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected filtered journal response, got {journal:?}");
    };
    assert!(!entries.is_empty());
    assert!(entries.iter().all(|entry| entry.method == "set_panic_stop"));
    assert!(entries.iter().all(|entry| entry.ok));
    Ok(())
}

#[test]
fn cli_replays_trace_against_real_daemon() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = workspace_root().join("examples/traces/status-smoke.json");
    let trace: ReplayTrace = serde_json::from_str(
        &fs::read_to_string(&trace_path).context("read checked-in status trace fixture")?,
    )
    .context("parse checked-in status trace fixture")?;
    assert_eq!(trace.version, 1);
    let trace_arg = trace_path.to_string_lossy().into_owned();
    let report = daemon.cli_value(&["trace", "replay", "--file", &trace_arg])?;
    assert_eq!(report["type"], "trace_replay");
    assert_eq!(report["trace_version"], 1);
    assert_eq!(
        report["steps"]
            .as_array()
            .context("trace report steps are an array")?
            .len(),
        trace.steps.len()
    );
    assert_eq!(report["steps"][0]["method"], "health");
    assert_eq!(report["steps"][2]["response_type"], "policy_status");
    assert_eq!(
        report["steps"][4]["response_type"],
        "desktop_session_status"
    );
    assert_eq!(report["steps"][4]["ok"], true);
    assert_eq!(report["steps"][5]["method"], "kwin_bridge_status");
    assert_eq!(report["steps"][5]["response_type"], "kwin_bridge_status");
    assert_eq!(report["steps"][5]["ok"], true);
    assert_eq!(report["steps"][6]["method"], "uinput_status");
    assert_eq!(report["steps"][6]["response_type"], "uinput_status");
    assert_eq!(report["steps"][6]["ok"], true);
    assert_eq!(report["steps"][7]["method"], "capture_backend_status");
    assert_eq!(
        report["steps"][7]["response_type"],
        "capture_backend_status"
    );
    assert_eq!(report["steps"][7]["ok"], true);
    assert_eq!(report["steps"][8]["method"], "clipboard_backend_status");
    assert_eq!(
        report["steps"][8]["response_type"],
        "clipboard_backend_status"
    );
    assert_eq!(report["steps"][8]["ok"], true);
    assert_eq!(report["steps"][9]["method"], "input_backend_status");
    assert_eq!(report["steps"][9]["response_type"], "input_backend_status");
    assert_eq!(report["steps"][9]["ok"], true);

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "12"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(
        &entries,
        &[
            "capabilities",
            "policy_status",
            "safety_status",
            "desktop_session_status",
            "kwin_bridge_status",
            "uinput_status",
            "capture_backend_status",
            "clipboard_backend_status",
            "input_backend_status",
        ],
    );
    assert!(entries.iter().all(|entry| entry.ok));
    Ok(())
}

#[test]
fn cli_replays_policy_denial_trace_against_real_daemon() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let denied_screenshot = Path::new("/tmp/plasma-pilot-denied-full-resolution.png");
    fs::remove_file(denied_screenshot).ok();
    let trace_path = workspace_root().join("examples/traces/policy-denials-smoke.json");
    let trace: ReplayTrace = serde_json::from_str(
        &fs::read_to_string(&trace_path).context("read checked-in policy denial trace fixture")?,
    )
    .context("parse checked-in policy denial trace fixture")?;
    assert_eq!(trace.version, 1);
    assert!(
        trace
            .steps
            .iter()
            .all(|step| step.expect_error_contains.is_some())
    );
    let trace_arg = trace_path.to_string_lossy().into_owned();
    let report = daemon.cli_value(&["trace", "replay", "--file", &trace_arg])?;
    assert_eq!(report["type"], "trace_replay");
    assert_eq!(report["trace_version"], 1);
    assert_eq!(
        report["steps"]
            .as_array()
            .context("trace report steps are an array")?
            .len(),
        trace.steps.len()
    );
    assert!(
        report["steps"]
            .as_array()
            .context("trace report steps are an array")?
            .iter()
            .all(|step| step["response_type"] == "error" && step["ok"] == false)
    );
    assert_eq!(report["steps"][0]["method"], "screenshot");
    assert_eq!(report["steps"][1]["method"], "clipboard_get");
    assert_eq!(report["steps"][2]["method"], "focus_window");

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "10", "--ok", "false"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(&entries, &["screenshot", "clipboard_get", "focus_window"]);
    assert!(entries.iter().all(|entry| !entry.ok));
    assert!(
        !denied_screenshot.exists(),
        "denied full-resolution screenshot trace wrote {}",
        denied_screenshot.display()
    );
    fs::remove_file(denied_screenshot).ok();
    Ok(())
}

#[test]
fn cli_replays_input_denial_trace_against_real_daemon() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = workspace_root().join("examples/traces/input-denials-smoke.json");
    let trace: ReplayTrace = serde_json::from_str(
        &fs::read_to_string(&trace_path).context("read checked-in input denial trace fixture")?,
    )
    .context("parse checked-in input denial trace fixture")?;
    assert_eq!(trace.version, 1);
    assert_eq!(trace.steps.len(), 9);
    assert!(
        trace
            .steps
            .iter()
            .all(|step| step.expect_response_type.as_deref() == Some("error"))
    );

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let report = daemon.cli_value(&["trace", "replay", "--file", &trace_arg])?;
    assert_eq!(report["type"], "trace_replay");
    assert_eq!(report["trace_version"], 1);
    let steps = report["steps"]
        .as_array()
        .context("trace report steps are an array")?;
    assert_eq!(steps.len(), trace.steps.len());
    assert!(
        steps
            .iter()
            .all(|step| step["response_type"] == "error" && step["ok"] == false)
    );
    assert_eq!(steps[0]["method"], "type_text");
    assert_eq!(steps[1]["method"], "key_combo");
    assert_eq!(steps[2]["method"], "move_pointer");
    assert_eq!(steps[3]["method"], "click_pointer");
    assert_eq!(steps[4]["method"], "drag_pointer");
    assert_eq!(steps[5]["method"], "scroll_pointer");

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "10", "--ok", "false"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(
        &entries,
        &[
            "type_text",
            "key_combo",
            "move_pointer",
            "click_pointer",
            "drag_pointer",
            "scroll_pointer",
        ],
    );
    assert!(entries.iter().all(|entry| !entry.ok));
    assert!(entries.iter().all(|entry| entry.summary.contains("policy")));
    Ok(())
}

#[test]
fn cli_replays_panic_stop_trace_against_real_daemon() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = workspace_root().join("examples/traces/panic-stop-smoke.json");
    let trace: ReplayTrace = serde_json::from_str(
        &fs::read_to_string(&trace_path).context("read checked-in panic-stop trace fixture")?,
    )
    .context("parse checked-in panic-stop trace fixture")?;
    assert_eq!(trace.version, 1);
    assert!(trace.steps.iter().all(|step| !step.expect_json.is_empty()));

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let report = daemon.cli_value(&["trace", "replay", "--file", &trace_arg])?;
    assert_eq!(report["type"], "trace_replay");
    assert_eq!(report["trace_version"], 1);
    assert_eq!(
        report["steps"]
            .as_array()
            .context("trace report steps are an array")?
            .len(),
        trace.steps.len()
    );
    assert_eq!(report["steps"][0]["method"], "panic_stop_status");
    assert_eq!(report["steps"][1]["method"], "set_panic_stop");
    assert_eq!(report["steps"][2]["method"], "panic_stop_status");
    assert_eq!(report["steps"][3]["method"], "set_panic_stop");
    assert_eq!(report["steps"][4]["method"], "panic_stop_status");
    assert!(
        report["steps"]
            .as_array()
            .context("trace report steps are an array")?
            .iter()
            .all(|step| step["response_type"] == "panic_stop" && step["ok"] == true)
    );

    let final_status = daemon.cli_json(&["panic-stop", "status"])?;
    let DaemonResponse::PanicStop(PanicStopStatus { enabled, .. }) = final_status else {
        bail!("expected panic-stop response, got {final_status:?}");
    };
    assert!(!enabled);

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "10"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(&entries, &["panic_stop_status", "set_panic_stop"]);
    assert!(entries.iter().all(|entry| entry.ok));
    Ok(())
}

#[test]
fn cli_replays_trace_directory_against_real_daemon() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let denied_screenshot = Path::new("/tmp/plasma-pilot-denied-full-resolution.png");
    fs::remove_file(denied_screenshot).ok();

    let trace_dir = workspace_root().join("examples/traces");
    let trace_arg = trace_dir.to_string_lossy().into_owned();
    let report = daemon.cli_value(&["trace", "replay", "--dir", &trace_arg])?;
    assert_eq!(report["type"], "trace_replay_set");
    assert_eq!(report["dir"], trace_arg);
    let traces = report["traces"]
        .as_array()
        .context("trace replay directory report traces are an array")?;
    assert!(
        traces.len() >= 3,
        "expected checked-in trace fixtures under {}, got {traces:?}",
        trace_dir.display()
    );
    assert_eq!(
        report["trace_count"],
        serde_json::Value::from(u64::try_from(traces.len()).expect("trace count fits u64"))
    );
    assert!(
        report["step_count"].as_u64().unwrap_or_default() >= 19,
        "trace replay directory report did not include aggregate steps: {report}"
    );
    assert!(
        traces.iter().any(|trace| {
            trace["file"]
                .as_str()
                .is_some_and(|path| path.ends_with("status-smoke.json"))
                && trace["steps"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().all(|step| step["ok"] == true))
        }),
        "status replay trace did not report all-ok steps: {traces:?}"
    );
    assert!(
        traces.iter().any(|trace| {
            trace["file"]
                .as_str()
                .is_some_and(|path| path.ends_with("journal-tail-smoke.json"))
                && trace["steps"].as_array().is_some_and(|steps| {
                    steps.len() == 3
                        && steps.iter().all(|step| step["ok"] == true)
                        && steps.iter().any(|step| {
                            step["method"] == "journal_tail" && step["response_type"] == "journal"
                        })
                })
        }),
        "journal-tail replay trace did not report compact journal output: {traces:?}"
    );
    assert!(
        traces.iter().any(|trace| {
            trace["file"]
                .as_str()
                .is_some_and(|path| path.ends_with("policy-denials-smoke.json"))
                && trace["steps"].as_array().is_some_and(|steps| {
                    steps
                        .iter()
                        .all(|step| step["response_type"] == "error" && step["ok"] == false)
                })
        }),
        "policy denial replay trace did not report fail-closed steps: {traces:?}"
    );
    assert!(
        traces.iter().any(|trace| {
            trace["file"]
                .as_str()
                .is_some_and(|path| path.ends_with("input-denials-smoke.json"))
                && trace["steps"].as_array().is_some_and(|steps| {
                    steps.len() == 9
                        && steps
                            .iter()
                            .all(|step| step["response_type"] == "error" && step["ok"] == false)
                        && steps.iter().any(|step| step["method"] == "type_text")
                        && steps.iter().any(|step| step["method"] == "click_pointer")
                        && steps
                            .iter()
                            .any(|step| step["method"] == "remote_desktop_session_probe")
                        && steps
                            .iter()
                            .any(|step| step["method"] == "remote_desktop_eis_probe")
                        && steps
                            .iter()
                            .any(|step| step["method"] == "remote_desktop_eis_start")
                })
        }),
        "input denial replay trace did not report keyboard/pointer/RemoteDesktop denials: {traces:?}"
    );
    assert!(
        traces.iter().any(|trace| {
            trace["file"]
                .as_str()
                .is_some_and(|path| path.ends_with("panic-stop-smoke.json"))
                && trace["steps"].as_array().is_some_and(|steps| {
                    steps.iter().any(|step| step["method"] == "set_panic_stop")
                        && steps
                            .iter()
                            .all(|step| step["response_type"] == "panic_stop" && step["ok"] == true)
                })
        }),
        "panic-stop replay trace did not report panic-stop steps: {traces:?}"
    );
    assert!(
        !denied_screenshot.exists(),
        "denied full-resolution screenshot trace wrote {}",
        denied_screenshot.display()
    );
    fs::remove_file(denied_screenshot).ok();
    Ok(())
}

#[test]
fn cli_replay_directory_rejects_empty_trace_set() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;
    let socket = root.join("missing.sock").to_string_lossy().into_owned();
    let trace_arg = root.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["--socket", &socket, "trace", "replay", "--dir", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace replay --dir")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("contains no .json traces"),
        "stderr did not include empty trace dir detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_validates_trace_without_daemon() -> Result<()> {
    let trace_path = workspace_root().join("examples/traces/status-smoke.json");
    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    require_success(&["trace", "validate"], &output)?;

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse trace validation report")?;
    assert_eq!(report["type"], "trace_validation");
    assert_eq!(report["trace_version"], 1);
    assert_eq!(report["step_count"], 12);
    assert_eq!(report["steps"][0]["label"], "health");
    assert_eq!(report["steps"][0]["method"], "health");
    assert_eq!(report["steps"][0]["expect_response_type"], "health");
    assert_eq!(report["steps"][3]["method"], "safety_status");
    assert_eq!(report["steps"][3]["expect_json_count"], 1);
    assert_eq!(report["steps"][4]["method"], "desktop_session_status");
    assert_eq!(report["steps"][5]["method"], "kwin_bridge_status");
    assert_eq!(report["steps"][5]["expect_json_count"], 6);
    assert_eq!(report["steps"][6]["method"], "uinput_status");
    assert_eq!(report["steps"][6]["expect_json_count"], 2);
    assert_eq!(report["steps"][7]["method"], "capture_backend_status");
    assert_eq!(report["steps"][7]["expect_json_count"], 4);
    assert_eq!(report["steps"][8]["method"], "clipboard_backend_status");
    assert_eq!(report["steps"][8]["expect_json_count"], 6);
    assert_eq!(report["steps"][9]["method"], "input_backend_status");
    assert_eq!(report["steps"][9]["expect_json_count"], 3);
    assert_eq!(
        report["steps"][10]["method"],
        "remote_desktop_eis_session_status"
    );
    assert_eq!(report["steps"][11]["method"], "remote_desktop_eis_stop");
    Ok(())
}

#[test]
fn cli_validates_all_checked_in_traces_without_daemon() -> Result<()> {
    let trace_dir = workspace_root().join("examples/traces");
    let mut traces = fs::read_dir(&trace_dir)
        .with_context(|| format!("read trace dir {}", trace_dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .context("read checked-in trace paths")?;
    traces.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    traces.sort();
    assert!(
        traces.len() >= 3,
        "expected checked-in trace fixtures under {}, got {traces:?}",
        trace_dir.display()
    );

    for trace_path in traces {
        let trace_arg = trace_path.to_string_lossy().into_owned();
        let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
            .args(["trace", "validate", "--file", &trace_arg])
            .output()
            .with_context(|| format!("run plasma-pilot-cli trace validate for {trace_arg}"))?;
        require_success(&["trace", "validate"], &output)?;
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("parse trace validation report")?;
        assert_eq!(report["type"], "trace_validation");
        assert_eq!(report["trace_version"], 1);
        assert!(
            report["step_count"].as_u64().unwrap_or_default() > 0,
            "trace {} did not report any steps",
            trace_path.display()
        );
    }
    Ok(())
}

#[test]
fn cli_validates_trace_directory_without_daemon() -> Result<()> {
    let trace_dir = workspace_root().join("examples/traces");
    let trace_arg = trace_dir.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--dir", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate --dir")?;
    require_success(&["trace", "validate", "--dir"], &output)?;

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse trace directory report")?;
    assert_eq!(report["type"], "trace_validation_set");
    assert_eq!(report["dir"], trace_arg);
    let traces = report["traces"]
        .as_array()
        .context("trace directory report traces are an array")?;
    assert!(
        traces.len() >= 3,
        "expected checked-in trace fixtures under {}, got {traces:?}",
        trace_dir.display()
    );
    assert_eq!(
        report["trace_count"],
        serde_json::Value::from(u64::try_from(traces.len()).expect("trace count fits u64"))
    );
    assert!(
        report["step_count"].as_u64().unwrap_or_default() >= 3,
        "trace directory report did not include aggregate steps: {report}"
    );
    for trace in traces {
        assert_eq!(trace["trace_version"], 1);
        assert!(
            trace["step_count"].as_u64().unwrap_or_default() > 0,
            "trace did not report any steps: {trace}"
        );
        assert!(
            trace["file"]
                .as_str()
                .is_some_and(|path| path.ends_with(".json")),
            "trace did not report a json file: {trace}"
        );
    }
    Ok(())
}

#[test]
fn cli_validate_directory_rejects_empty_trace_set() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;
    let trace_arg = root.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--dir", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate --dir")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("contains no .json traces"),
        "stderr did not include empty trace dir detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_validates_journal_tail_trace_expectations() -> Result<()> {
    let trace_path = workspace_root().join("examples/traces/journal-tail-smoke.json");
    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    require_success(&["trace", "validate"], &output)?;

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse trace validation report")?;
    assert_eq!(report["type"], "trace_validation");
    assert_eq!(report["step_count"], 3);
    assert_eq!(report["steps"][0]["method"], "health");
    assert_eq!(report["steps"][1]["method"], "policy_status");
    assert_eq!(report["steps"][2]["method"], "journal_tail");
    assert_eq!(report["steps"][2]["expect_response_type"], "journal");
    assert_eq!(report["steps"][2]["expect_json_count"], 8);
    Ok(())
}

#[test]
fn cli_validates_policy_denial_trace_expectations() -> Result<()> {
    let trace_path = workspace_root().join("examples/traces/policy-denials-smoke.json");
    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    require_success(&["trace", "validate"], &output)?;

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse trace validation report")?;
    assert_eq!(report["type"], "trace_validation");
    assert_eq!(report["step_count"], 5);
    assert_eq!(report["steps"][0]["method"], "screenshot");
    assert_eq!(
        report["steps"][0]["expect_error_contains"],
        "policy prompt required for FullResolutionScreenshot"
    );
    assert_eq!(
        report["steps"][1]["expect_error_contains"],
        "policy prompt required for ClipboardRead"
    );
    assert_eq!(
        report["steps"][2]["expect_error_contains"],
        "policy prompt required for ControlSemantic"
    );
    assert_eq!(report["steps"][3]["method"], "accessibility_set_caret");
    assert_eq!(
        report["steps"][3]["expect_error_contains"],
        "policy prompt required for ControlSemantic"
    );
    assert_eq!(report["steps"][4]["method"], "accessibility_set_selection");
    assert_eq!(
        report["steps"][4]["expect_error_contains"],
        "policy prompt required for ControlSemantic"
    );
    Ok(())
}

#[test]
fn cli_validates_input_denial_trace_expectations() -> Result<()> {
    let trace_path = workspace_root().join("examples/traces/input-denials-smoke.json");
    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    require_success(&["trace", "validate"], &output)?;

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse trace validation report")?;
    assert_eq!(report["type"], "trace_validation");
    assert_eq!(report["step_count"], 9);
    assert_eq!(report["steps"][0]["method"], "type_text");
    assert_eq!(
        report["steps"][0]["expect_error_contains"],
        "policy prompt required for ControlKeyboard"
    );
    assert_eq!(report["steps"][2]["method"], "move_pointer");
    assert_eq!(
        report["steps"][2]["expect_error_contains"],
        "policy prompt required for ControlPointer"
    );
    assert_eq!(report["steps"][5]["method"], "scroll_pointer");
    assert_eq!(
        report["steps"][5]["expect_error_contains"],
        "policy prompt required for ControlPointer"
    );
    assert_eq!(report["steps"][6]["method"], "remote_desktop_session_probe");
    assert_eq!(
        report["steps"][6]["expect_error_contains"],
        "policy prompt required for ControlPointer"
    );
    assert_eq!(report["steps"][7]["method"], "remote_desktop_eis_probe");
    assert_eq!(
        report["steps"][7]["expect_error_contains"],
        "policy prompt required for ControlPointer"
    );
    assert_eq!(report["steps"][8]["method"], "remote_desktop_eis_start");
    assert_eq!(
        report["steps"][8]["expect_error_contains"],
        "policy prompt required for ControlPointer"
    );
    Ok(())
}

#[test]
fn cli_validates_panic_stop_trace_json_expectations() -> Result<()> {
    let trace_path = workspace_root().join("examples/traces/panic-stop-smoke.json");
    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    require_success(&["trace", "validate"], &output)?;

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse trace validation report")?;
    assert_eq!(report["type"], "trace_validation");
    assert_eq!(report["step_count"], 5);
    assert_eq!(report["steps"][0]["method"], "panic_stop_status");
    assert_eq!(report["steps"][0]["expect_json_count"], 1);
    assert_eq!(report["steps"][1]["method"], "set_panic_stop");
    assert_eq!(report["steps"][1]["expect_json_count"], 1);
    Ok(())
}

#[test]
fn cli_validate_rejects_unknown_expected_response_type() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;
    let trace_path = root.join("bad-status-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("intentionally invalid status trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-health".to_string()),
            request: DaemonRequest::Health,
            expect_response_type: Some("bogus".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: Vec::new(),
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-health" method=health"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("expects unknown response type bogus"),
        "stderr did not include validation detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_validate_rejects_blank_and_duplicate_labels() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;

    let blank_label_trace = root.join("blank-label-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("blank label trace".to_string()),
        steps: vec![TraceStep {
            label: Some("   ".to_string()),
            request: DaemonRequest::Health,
            expect_response_type: Some("health".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: Vec::new(),
        }],
    };
    fs::write(
        &blank_label_trace,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = blank_label_trace.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("trace step 0"),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("label must not be empty"),
        "stderr did not include blank label detail: {stderr}"
    );

    let duplicate_label_trace = root.join("duplicate-label-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("duplicate label trace".to_string()),
        steps: vec![
            TraceStep {
                label: Some("duplicate".to_string()),
                request: DaemonRequest::Health,
                expect_response_type: Some("health".to_string()),
                expect_ok: Some(true),
                expect_error_contains: None,
                expect_json: Vec::new(),
            },
            TraceStep {
                label: Some("duplicate".to_string()),
                request: DaemonRequest::PolicyStatus,
                expect_response_type: Some("policy_status".to_string()),
                expect_ok: Some(true),
                expect_error_contains: None,
                expect_json: Vec::new(),
            },
        ],
    };
    fs::write(
        &duplicate_label_trace,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = duplicate_label_trace.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 1 label="duplicate" method=policy_status"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains(r#"duplicates label "duplicate""#),
        "stderr did not include duplicate label detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_validate_rejects_contradictory_error_expectations() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;

    let non_error_response_trace = root.join("non-error-response-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("contradictory response type trace".to_string()),
        steps: vec![TraceStep {
            label: Some("health-error-text".to_string()),
            request: DaemonRequest::Health,
            expect_response_type: Some("health".to_string()),
            expect_ok: None,
            expect_error_contains: Some("policy prompt required".to_string()),
            expect_json: Vec::new(),
        }],
    };
    fs::write(
        &non_error_response_trace,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = non_error_response_trace.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="health-error-text" method=health"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("expects error text but expect_response_type is health"),
        "stderr did not include response-type contradiction: {stderr}"
    );

    let ok_true_trace = root.join("ok-true-error-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("contradictory ok trace".to_string()),
        steps: vec![TraceStep {
            label: Some("ok-error-text".to_string()),
            request: DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: Some(64),
            }),
            expect_response_type: Some("error".to_string()),
            expect_ok: Some(true),
            expect_error_contains: Some("policy prompt required".to_string()),
            expect_json: Vec::new(),
        }],
    };
    fs::write(
        &ok_true_trace,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = ok_true_trace.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="ok-error-text" method=clipboard_get"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("expects error text but expect_ok is true"),
        "stderr did not include ok contradiction: {stderr}"
    );

    let empty_error_trace = root.join("empty-error-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("empty error expectation trace".to_string()),
        steps: vec![TraceStep {
            label: Some("empty-error-text".to_string()),
            request: DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: Some(64),
            }),
            expect_response_type: Some("error".to_string()),
            expect_ok: Some(false),
            expect_error_contains: Some("   ".to_string()),
            expect_json: Vec::new(),
        }],
    };
    fs::write(
        &empty_error_trace,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = empty_error_trace.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="empty-error-text" method=clipboard_get"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("expect_error_contains must not be empty"),
        "stderr did not include empty error detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_validate_rejects_invalid_json_expectation_pointer() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;
    let trace_path = root.join("bad-json-pointer-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("invalid JSON expectation pointer trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-pointer".to_string()),
            request: DaemonRequest::PanicStopStatus,
            expect_response_type: Some("panic_stop".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: vec![TraceJsonExpectation {
                pointer: "data/enabled".to_string(),
                equals: Some(serde_json::json!(false)),
                value_type: None,
                value_types: Vec::new(),
                exists: None,
            }],
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-pointer" method=panic_stop_status"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("JSON expectation pointer must start with '/'"),
        "stderr did not include JSON pointer detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_validate_rejects_invalid_json_expectation_type() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;
    let trace_path = root.join("bad-json-type-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("invalid JSON expectation type trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-type".to_string()),
            request: DaemonRequest::PanicStopStatus,
            expect_response_type: Some("panic_stop".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: vec![TraceJsonExpectation {
                pointer: "/data/enabled".to_string(),
                equals: None,
                value_type: Some("str".to_string()),
                value_types: Vec::new(),
                exists: None,
            }],
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-type" method=panic_stop_status"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("unknown value_type str"),
        "stderr did not include value type detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_validate_rejects_invalid_json_expectation_type_list() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;
    let trace_path = root.join("bad-json-type-list-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("invalid JSON expectation type list trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-type-list".to_string()),
            request: DaemonRequest::PanicStopStatus,
            expect_response_type: Some("panic_stop".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: vec![TraceJsonExpectation {
                pointer: "/data/enabled".to_string(),
                equals: None,
                value_type: None,
                value_types: vec!["boolean".to_string(), "bool".to_string()],
                exists: None,
            }],
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .args(["trace", "validate", "--file", &trace_arg])
        .output()
        .context("run plasma-pilot-cli trace validate")?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-type-list" method=panic_stop_status"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("unknown value_type bool"),
        "stderr did not include value type detail: {stderr}"
    );

    fs::remove_dir_all(&root).ok();
    Ok(())
}

#[test]
fn cli_replay_errors_include_step_label_and_method() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = daemon.root.join("bad-status-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("intentionally mismatched status trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-health".to_string()),
            request: DaemonRequest::Health,
            expect_response_type: Some("policy_status".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: Vec::new(),
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = daemon.cli_output(&["trace", "replay", "--file", &trace_arg])?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-health" method=health"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("expected response type policy_status, got health"),
        "stderr did not include mismatch detail: {stderr}"
    );
    Ok(())
}

#[test]
fn cli_replay_errors_include_step_context_for_error_expectations() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = daemon.root.join("bad-error-expectation-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("intentionally mismatched error expectation trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-clipboard-error".to_string()),
            request: DaemonRequest::ClipboardGet(ClipboardGetRequest {
                max_bytes: Some(64),
            }),
            expect_response_type: Some("error".to_string()),
            expect_ok: Some(false),
            expect_error_contains: Some("full resolution".to_string()),
            expect_json: Vec::new(),
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = daemon.cli_output(&["trace", "replay", "--file", &trace_arg])?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-clipboard-error" method=clipboard_get"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains(r#"expected error containing "full resolution""#),
        "stderr did not include error-expectation mismatch detail: {stderr}"
    );
    Ok(())
}

#[test]
fn cli_replay_errors_include_step_context_for_json_expectations() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = daemon.root.join("bad-json-expectation-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("intentionally mismatched JSON expectation trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-panic-json".to_string()),
            request: DaemonRequest::PanicStopStatus,
            expect_response_type: Some("panic_stop".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: vec![TraceJsonExpectation {
                pointer: "/data/enabled".to_string(),
                equals: Some(serde_json::json!(true)),
                value_type: None,
                value_types: Vec::new(),
                exists: None,
            }],
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = daemon.cli_output(&["trace", "replay", "--file", &trace_arg])?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-panic-json" method=panic_stop_status"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("expected JSON pointer /data/enabled to match expected value"),
        "stderr did not include JSON expectation mismatch detail: {stderr}"
    );
    Ok(())
}

#[test]
fn cli_replay_errors_include_step_context_for_json_type_expectations() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = daemon.root.join("bad-json-type-expectation-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("intentionally mismatched JSON type trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-panic-type".to_string()),
            request: DaemonRequest::PanicStopStatus,
            expect_response_type: Some("panic_stop".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: vec![TraceJsonExpectation {
                pointer: "/data/enabled".to_string(),
                equals: None,
                value_type: Some("string".to_string()),
                value_types: Vec::new(),
                exists: None,
            }],
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = daemon.cli_output(&["trace", "replay", "--file", &trace_arg])?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-panic-type" method=panic_stop_status"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains("expected JSON pointer /data/enabled to have type string, got boolean"),
        "stderr did not include JSON type mismatch detail: {stderr}"
    );
    Ok(())
}

#[test]
fn cli_replay_errors_include_step_context_for_json_type_list_expectations() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let trace_path = daemon
        .root
        .join("bad-json-type-list-expectation-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("intentionally mismatched JSON type-list trace".to_string()),
        steps: vec![TraceStep {
            label: Some("bad-panic-type-list".to_string()),
            request: DaemonRequest::PanicStopStatus,
            expect_response_type: Some("panic_stop".to_string()),
            expect_ok: Some(true),
            expect_error_contains: None,
            expect_json: vec![TraceJsonExpectation {
                pointer: "/data/enabled".to_string(),
                equals: None,
                value_type: None,
                value_types: vec!["string".to_string(), "null".to_string()],
                exists: None,
            }],
        }],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize bad trace")?,
    )
    .context("write bad trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let output = daemon.cli_output(&["trace", "replay", "--file", &trace_arg])?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(r#"trace step 0 label="bad-panic-type-list" method=panic_stop_status"#),
        "stderr did not include trace step context: {stderr}"
    );
    assert!(
        stderr.contains(
            "expected JSON pointer /data/enabled to have one of types string/null, got boolean"
        ),
        "stderr did not include JSON type-list mismatch detail: {stderr}"
    );
    Ok(())
}

#[test]
fn cli_writes_expiring_approval_grant() -> Result<()> {
    let root = unique_temp_dir();
    fs::create_dir_all(&root).context("create integration temp dir")?;
    let approval_file = root.join("approvals.jsonl");

    let output = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-cli"))
        .arg("--socket")
        .arg(root.join("unused.sock"))
        .arg("approve")
        .arg("--approval-file")
        .arg(&approval_file)
        .arg("--safety-class")
        .arg("control-semantic")
        .arg("--method")
        .arg("focus_window")
        .arg("--ttl-ms")
        .arg("60000")
        .output()
        .context("run plasma-pilot-cli approve")?;
    require_success(&["approve"], &output)?;

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse approve report")?;
    assert_eq!(report["method"], "focus_window");
    assert_eq!(report["safety_class"], "control_semantic");
    assert_eq!(
        fs::metadata(&approval_file)?.permissions().mode() & 0o777,
        0o600
    );

    let contents = fs::read_to_string(&approval_file).context("read approval file")?;
    let grant: serde_json::Value =
        serde_json::from_str(contents.trim()).context("parse approval grant")?;
    assert_eq!(grant["method"], "focus_window");
    assert_eq!(grant["safety_class"], "control_semantic");
    assert!(grant["expires_unix_ms"].as_u64().unwrap_or_default() > unix_time_ms()?);

    fs::remove_dir_all(&root).ok();
    Ok(())
}

fn require_success(args: &[&str], output: &Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "plasma-pilot-cli {} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_methods(entries: &[JournalEntry], expected: &[&str]) {
    let methods = entries
        .iter()
        .map(|entry| entry.method.as_str())
        .collect::<Vec<_>>();
    for expected_method in expected {
        assert!(
            methods.contains(expected_method),
            "journal missing method {expected_method}; got {methods:?}"
        );
    }
}

fn daemon_binary() -> Result<PathBuf> {
    let candidate =
        PathBuf::from(env!("CARGO_BIN_EXE_plasma-pilot-cli")).with_file_name("plasma-pilotd");

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("resolve workspace root")?;
    let status = Command::new(env!("CARGO"))
        .arg("build")
        .arg("-p")
        .arg("plasma-pilotd")
        .arg("--quiet")
        .current_dir(workspace)
        .status()
        .context("build plasma-pilotd test binary")?;
    if !status.success() {
        bail!("cargo build -p plasma-pilotd failed with status {status}");
    }
    if candidate.exists() {
        return Ok(candidate);
    }
    bail!(
        "plasma-pilotd binary was not built at {}",
        candidate.display()
    )
}

fn wait_for_socket(socket: &Path) -> Result<()> {
    for _ in 0..50 {
        if socket.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("daemon socket did not appear at {}", socket.display())
}

fn unique_temp_dir() -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "plasma-pilot-cli-integration-{}-{now}",
        std::process::id()
    ))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("CLI crate is under workspace crates directory")
        .to_path_buf()
}

fn unix_time_ms() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before unix epoch")?;
    u64::try_from(duration.as_millis()).context("unix time milliseconds overflowed u64")
}
