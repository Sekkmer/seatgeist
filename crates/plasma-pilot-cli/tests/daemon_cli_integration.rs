use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use libplasma_pilot::{
    BackendCapability, CapabilitySet, DaemonRequest, DaemonResponse, HealthStatus, JournalEntry,
    PanicStopStatus, PolicyStatus, ReplayTrace, ToolApprovalLevel, TraceStep, UinputStatus,
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

    let policy = daemon.cli_json(&["policy-status"])?;
    assert_eq!(
        policy,
        DaemonResponse::PolicyStatus(PolicyStatus {
            default_observe: ToolApprovalLevel::Allow,
            default_control: ToolApprovalLevel::Prompt,
            default_clipboard_read: ToolApprovalLevel::Prompt,
            default_clipboard_write: ToolApprovalLevel::Allow,
        })
    );

    let uinput = daemon.cli_json(&["input", "status"])?;
    let DaemonResponse::UinputStatus(UinputStatus {
        path, setup_hint, ..
    }) = uinput
    else {
        bail!("expected uinput status response, got {uinput:?}");
    };
    assert_eq!(path, PathBuf::from("/dev/uinput"));
    assert!(!setup_hint.is_empty());

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "10"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(
        &entries,
        &["health", "capabilities", "policy_status", "uinput_status"],
    );
    assert!(entries.iter().all(|entry| entry.ok));
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
    let trace_path = daemon.root.join("status-trace.json");
    let trace = ReplayTrace {
        version: 1,
        description: Some("status trace".to_string()),
        steps: vec![
            TraceStep {
                label: Some("health".to_string()),
                request: DaemonRequest::Health,
                expect_response_type: Some("health".to_string()),
                expect_ok: Some(true),
            },
            TraceStep {
                label: Some("capabilities".to_string()),
                request: DaemonRequest::Capabilities,
                expect_response_type: Some("capabilities".to_string()),
                expect_ok: Some(true),
            },
            TraceStep {
                label: Some("policy".to_string()),
                request: DaemonRequest::PolicyStatus,
                expect_response_type: Some("policy_status".to_string()),
                expect_ok: Some(true),
            },
        ],
    };
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).context("serialize trace")?,
    )
    .context("write trace file")?;

    let trace_arg = trace_path.to_string_lossy().into_owned();
    let report = daemon.cli_value(&["trace", "replay", "--file", &trace_arg])?;
    assert_eq!(report["type"], "trace_replay");
    assert_eq!(report["trace_version"], 1);
    assert_eq!(
        report["steps"]
            .as_array()
            .context("trace report steps are an array")?
            .len(),
        3
    );
    assert_eq!(report["steps"][0]["method"], "health");
    assert_eq!(report["steps"][2]["response_type"], "policy_status");
    assert_eq!(report["steps"][2]["ok"], true);

    let journal = daemon.cli_json(&["journal", "tail", "--limit", "10"])?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(&entries, &["health", "capabilities", "policy_status"]);
    assert!(entries.iter().all(|entry| entry.ok));
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
    if candidate.exists() {
        return Ok(candidate);
    }

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
