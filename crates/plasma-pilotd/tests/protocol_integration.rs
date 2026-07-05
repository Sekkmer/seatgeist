use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use libplasma_pilot::{
    CapabilitySet, DaemonRequest, DaemonResponse, DesktopSessionStatus, HealthStatus, JournalEntry,
    JournalTailRequest, PanicStopStatus, PolicyStatus, SafetyStatus, SetPanicStopRequest,
    ToolApprovalLevel, UinputStatus,
};

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
        let child = Command::new(env!("CARGO_BIN_EXE_plasma-pilotd"))
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

        let child = Command::new(env!("CARGO_BIN_EXE_plasma-pilotd"))
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

    fn request(&self, request: &DaemonRequest) -> Result<DaemonResponse> {
        let mut stream = UnixStream::connect(&self.socket).context("connect to daemon socket")?;
        let line = serde_json::to_string(request).context("serialize daemon request")?;
        stream.write_all(line.as_bytes()).context("write request")?;
        stream.write_all(b"\n").context("write request newline")?;
        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader
            .read_line(&mut response)
            .context("read daemon response")?;
        serde_json::from_str(&response).context("parse daemon response")
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
fn daemon_loads_socket_and_policy_from_config_file() -> Result<()> {
    let daemon = DaemonFixture::start_with_config(
        r#"
[daemon]
socket = "__SOCKET__"
journal = "__JOURNAL__"
panic_stop_file = "__PANIC_STOP__"

[journal]
include_artifact_metadata = true

[policy]
default_observe = "allow"
default_control = "deny"
destructive_actions = "deny"
secret_fields = "prompt"
default_clipboard_read = "allow"
default_clipboard_write = "prompt"
full_resolution_screenshot = "deny"

[safety]
require_focus_guard = false
human_input_quiet_ms = 2500
control_rate_limit_per_minute = 42
preview_max_edge = 1024
tile_max_edge = 2048
"#,
    )?;

    let policy = daemon.request(&DaemonRequest::PolicyStatus)?;
    assert_eq!(
        policy,
        DaemonResponse::PolicyStatus(PolicyStatus {
            default_observe: ToolApprovalLevel::Allow,
            default_control: ToolApprovalLevel::Deny,
            default_destructive_actions: ToolApprovalLevel::Deny,
            default_secret_fields: ToolApprovalLevel::Prompt,
            default_full_resolution_screenshot: ToolApprovalLevel::Deny,
            default_clipboard_read: ToolApprovalLevel::Allow,
            default_clipboard_write: ToolApprovalLevel::Prompt,
        })
    );

    let safety = daemon.request(&DaemonRequest::SafetyStatus)?;
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
fn daemon_serves_core_protocol_and_journal() -> Result<()> {
    let daemon = DaemonFixture::start()?;

    let health = daemon.request(&DaemonRequest::Health)?;
    assert_eq!(
        health,
        DaemonResponse::Health(HealthStatus {
            service: "plasma-pilotd".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            status: "ok".to_string(),
        })
    );

    let policy = daemon.request(&DaemonRequest::PolicyStatus)?;
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

    let capabilities = daemon.request(&DaemonRequest::Capabilities)?;
    let DaemonResponse::Capabilities(CapabilitySet { capabilities }) = capabilities else {
        bail!("expected capabilities response, got {capabilities:?}");
    };
    assert!(capabilities.contains(&libplasma_pilot::BackendCapability::DaemonHealth));
    assert!(capabilities.contains(&libplasma_pilot::BackendCapability::DaemonPolicyStatus));
    assert!(capabilities.contains(&libplasma_pilot::BackendCapability::DaemonSafetyStatus));
    assert!(capabilities.contains(&libplasma_pilot::BackendCapability::DaemonDesktopSessionStatus));
    assert!(capabilities.contains(&libplasma_pilot::BackendCapability::DaemonComputerUseReadiness));

    let safety = daemon.request(&DaemonRequest::SafetyStatus)?;
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

    let desktop_session = daemon.request(&DaemonRequest::DesktopSessionStatus)?;
    let DaemonResponse::DesktopSessionStatus(DesktopSessionStatus { setup_hint, .. }) =
        desktop_session
    else {
        bail!("expected desktop session status response, got {desktop_session:?}");
    };
    assert!(!setup_hint.is_empty());

    let readiness = daemon.request(&DaemonRequest::ComputerUseReadiness)?;
    let DaemonResponse::ComputerUseReadiness(status) = readiness else {
        bail!("expected readiness response, got {readiness:?}");
    };
    assert_eq!(
        status.dbus_session_bus_present && status.runtime_dir_present,
        status.desktop_session_ready
    );
    assert!(status.focus_guard_required);

    let panic_stop = daemon.request(&DaemonRequest::PanicStopStatus)?;
    let DaemonResponse::PanicStop(PanicStopStatus { enabled, path }) = panic_stop else {
        bail!("expected panic-stop response, got {panic_stop:?}");
    };
    assert!(!enabled);
    assert!(path.starts_with(&daemon.root));

    let uinput = daemon.request(&DaemonRequest::UinputStatus)?;
    let DaemonResponse::UinputStatus(UinputStatus {
        path, setup_hint, ..
    }) = uinput
    else {
        bail!("expected uinput status response, got {uinput:?}");
    };
    assert_eq!(path, plasma_pilot_uinput::uinput_path());
    assert!(!setup_hint.is_empty());

    let input_backends = daemon.request(&DaemonRequest::InputBackendStatus)?;
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

    let capture_backends = daemon.request(&DaemonRequest::CaptureBackendStatus)?;
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

    let panic_stop = daemon.request(&DaemonRequest::SetPanicStop(SetPanicStopRequest {
        enabled: true,
    }))?;
    let DaemonResponse::PanicStop(PanicStopStatus { enabled, path }) = panic_stop else {
        bail!("expected panic-stop enable response, got {panic_stop:?}");
    };
    assert!(enabled);
    assert!(path.exists());

    let panic_stop = daemon.request(&DaemonRequest::SetPanicStop(SetPanicStopRequest {
        enabled: false,
    }))?;
    let DaemonResponse::PanicStop(PanicStopStatus { enabled, path }) = panic_stop else {
        bail!("expected panic-stop disable response, got {panic_stop:?}");
    };
    assert!(!enabled);
    assert!(!path.exists());

    let journal = daemon.request(&DaemonRequest::JournalTail(JournalTailRequest {
        limit: 12,
        method_filter: None,
        ok: None,
    }))?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(
        &entries,
        &[
            "health",
            "policy_status",
            "capabilities",
            "safety_status",
            "desktop_session_status",
            "computer_use_readiness",
            "panic_stop_status",
            "uinput_status",
            "input_backend_status",
            "capture_backend_status",
            "set_panic_stop",
        ],
    );
    assert!(entries.iter().all(|entry| entry.ok));

    let journal = daemon.request(&DaemonRequest::JournalTail(JournalTailRequest {
        limit: 10,
        method_filter: None,
        ok: None,
    }))?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected second journal response, got {journal:?}");
    };
    assert_methods(&entries, &["journal_tail"]);

    let journal = daemon.request(&DaemonRequest::JournalTail(JournalTailRequest {
        limit: 10,
        method_filter: Some("set_panic_stop".to_string()),
        ok: Some(true),
    }))?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected filtered journal response, got {journal:?}");
    };
    assert!(!entries.is_empty());
    assert!(entries.iter().all(|entry| entry.method == "set_panic_stop"));
    assert!(entries.iter().all(|entry| entry.ok));
    Ok(())
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
        "plasma-pilotd-integration-{}-{now}",
        std::process::id()
    ))
}
