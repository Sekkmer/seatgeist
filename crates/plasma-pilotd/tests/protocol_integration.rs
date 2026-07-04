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
    CapabilitySet, DaemonRequest, DaemonResponse, HealthStatus, JournalEntry, JournalTailRequest,
    PolicyStatus, ToolApprovalLevel,
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
        let child = Command::new(env!("CARGO_BIN_EXE_plasma-pilotd"))
            .arg("--socket")
            .arg(&socket)
            .arg("--journal")
            .arg(&journal)
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

    let journal = daemon.request(&DaemonRequest::JournalTail(JournalTailRequest {
        limit: 10,
    }))?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected journal response, got {journal:?}");
    };
    assert_methods(&entries, &["health", "policy_status", "capabilities"]);
    assert!(entries.iter().all(|entry| entry.ok));

    let journal = daemon.request(&DaemonRequest::JournalTail(JournalTailRequest {
        limit: 10,
    }))?;
    let DaemonResponse::Journal(entries) = journal else {
        bail!("expected second journal response, got {journal:?}");
    };
    assert_methods(&entries, &["journal_tail"]);
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
