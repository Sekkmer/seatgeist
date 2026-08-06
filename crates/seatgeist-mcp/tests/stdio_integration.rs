use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

struct DaemonFixture {
    child: Child,
    socket: PathBuf,
    root: PathBuf,
}

impl DaemonFixture {
    fn start() -> Result<Self> {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).context("create integration temp dir")?;
        let socket = root.join("seatgeistd.sock");
        let journal = root.join("journal.jsonl");
        let panic_stop = root.join("panic-stop");
        let child = Command::new(daemon_binary()?)
            .arg("--disable-kwin-bridge")
            .arg("--socket")
            .arg(&socket)
            .arg("--journal")
            .arg(&journal)
            .arg("--panic-stop-file")
            .arg(&panic_stop)
            .env("HOME", &root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn seatgeistd")?;
        wait_for_socket(&socket)?;
        Ok(Self {
            child,
            socket,
            root,
        })
    }

    fn start_with_config(config_contents: &str) -> Result<Self> {
        Self::start_with_config_and_env(config_contents, &[])
    }

    fn start_with_config_and_env(
        config_contents: &str,
        env_overrides: &[(&str, &str)],
    ) -> Result<Self> {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).context("create integration temp dir")?;
        let socket = root.join("configured.sock");
        let journal = root.join("configured-journal.jsonl");
        let panic_stop = root.join("configured-panic-stop");
        let config = root.join("config.toml");
        let empty_bin = root.join("empty-bin");
        fs::create_dir_all(&empty_bin).context("create empty PATH fixture")?;
        fs::write(
            &config,
            config_contents
                .replace("__ROOT__", &root.display().to_string())
                .replace("__EMPTY_BIN__", &empty_bin.display().to_string())
                .replace("__SOCKET__", &socket.display().to_string())
                .replace("__JOURNAL__", &journal.display().to_string())
                .replace("__PANIC_STOP__", &panic_stop.display().to_string()),
        )
        .context("write daemon config fixture")?;

        let mut command = Command::new(daemon_binary()?);
        command
            .arg("--disable-kwin-bridge")
            .arg("--config")
            .arg(&config)
            .env("HOME", &root)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, value) in env_overrides {
            command.env(
                key,
                value
                    .replace("__ROOT__", &root.display().to_string())
                    .replace("__EMPTY_BIN__", &empty_bin.display().to_string()),
            );
        }
        let child = command.spawn().context("spawn configured seatgeistd")?;
        wait_for_socket(&socket)?;
        Ok(Self {
            child,
            socket,
            root,
        })
    }

    fn run_mcp(&self, requests: &[Value]) -> Result<Vec<Value>> {
        self.run_mcp_with_profile(None, requests)
    }

    fn run_mcp_with_profile(
        &self,
        profile: Option<&str>,
        requests: &[Value],
    ) -> Result<Vec<Value>> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_seatgeist-mcp"));
        command.arg("--stdio").arg("--socket").arg(&self.socket);
        if let Some(profile) = profile {
            command.arg("--tool-profile").arg(profile);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn seatgeist-mcp")?;

        {
            let stdin = child.stdin.as_mut().context("open MCP stdin")?;
            for request in requests {
                serde_json::to_writer(&mut *stdin, request).context("write MCP request")?;
                stdin
                    .write_all(b"\n")
                    .context("write MCP request newline")?;
            }
        }

        let output = child.wait_with_output().context("wait for MCP output")?;
        require_success("seatgeist-mcp --stdio", &output)?;
        parse_json_lines(&output.stdout)
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
fn mcp_core_caches_readiness_until_another_tool_call() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let responses = daemon.run_mcp_with_profile(
        Some("core"),
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": "seatgeist.computer_status", "arguments": {}}
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "seatgeist.computer_status", "arguments": {}}
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "seatgeist.panic_stop", "arguments": {}}
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {"name": "seatgeist.computer_status", "arguments": {}}
            }),
        ],
    )?;
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["result"], responses[1]["result"]);
    assert_eq!(
        responses[2]["result"]["structuredContent"]["type"],
        "panic_stop"
    );

    let journal = fs::read_to_string(daemon.root.join("journal.jsonl"))
        .context("read readiness cache journal")?;
    let readiness_calls = journal
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["method"] == "computer_use_readiness")
        .count();
    assert_eq!(readiness_calls, 2, "second consecutive status is cached");
    Ok(())
}

#[test]
fn mcp_stdio_talks_to_real_daemon_and_reports_tool_errors() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let responses = daemon.run_mcp(&[
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.health",
                "arguments": {}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.computer_use_readiness",
                "arguments": {}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.a11y_text_attributes",
                "arguments": {
                    "node_id": "invalid-atspi-node",
                    "offset": 0
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.journal_tail",
                "arguments": {
                    "limit": 10
                }
            }
        }),
    ])?;

    assert_eq!(responses.len(), 6);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(
        responses[0]["result"]["capabilities"]["tools"]["listChanged"],
        false
    );

    let tools = responses[1]["result"]["tools"]
        .as_array()
        .context("tools/list result is an array")?;
    assert_tool_present(tools, "seatgeist.health");
    assert_tool_present(tools, "seatgeist.remote_desktop_session_probe");
    assert_tool_present(tools, "seatgeist.remote_desktop_eis_probe");
    assert_tool_present(tools, "seatgeist.computer_use_readiness");
    assert_tool_present(tools, "seatgeist.capture_open");
    assert_tool_present(tools, "seatgeist.a11y_text_attributes");
    assert_tool_present(tools, "seatgeist.journal_tail");

    let health = &responses[2]["result"];
    assert_eq!(health["isError"], false);
    assert_eq!(health["structuredContent"]["type"], "health");
    assert_eq!(health["structuredContent"]["data"]["service"], "seatgeistd");

    let readiness = &responses[3]["result"];
    assert_eq!(readiness["isError"], false);
    assert_eq!(
        readiness["structuredContent"]["type"],
        "computer_use_readiness"
    );
    assert!(readiness["structuredContent"]["data"]["ready_for_observe"].is_boolean());
    assert!(
        readiness["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("readiness observe=")
    );

    let attributes_error = &responses[4]["result"];
    assert_eq!(attributes_error["isError"], true);
    assert_eq!(attributes_error["structuredContent"]["type"], "error");
    assert!(
        attributes_error["structuredContent"]["data"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid AT-SPI node id")
    );
    assert!(
        attributes_error["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid AT-SPI node id")
    );

    let journal = &responses[5]["result"];
    assert_eq!(journal["isError"], false);
    assert_eq!(journal["structuredContent"]["type"], "journal");
    let entries = journal["structuredContent"]["data"]
        .as_array()
        .context("journal data is an array")?;
    let Some(entry) = entries
        .iter()
        .find(|entry| entry["method"] == "accessibility_text_attributes")
    else {
        bail!("missing accessibility_text_attributes journal entry: {entries:?}");
    };
    assert_eq!(entry["ok"], false);
    assert_eq!(entry["safety_class"], "observe");
    assert_eq!(entry["client"]["tool"], "seatgeist-mcp");
    Ok(())
}

#[test]
fn mcp_post_action_options_are_validated_before_control_side_effects() -> Result<()> {
    let daemon = DaemonFixture::start_with_config(
        r#"
[daemon]
socket = "__SOCKET__"
journal = "__JOURNAL__"
panic_stop_file = "__PANIC_STOP__"

[policy]
default_control = "allow"

[safety]
require_focus_guard = false
"#,
    )?;
    let responses = daemon.run_mcp(&[
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.type_text",
                "arguments": {
                    "text": "must-not-be-typed-or-journaled",
                    "settle_timeout_ms": 0
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.type_text",
                "arguments": {
                    "text": "must-also-not-be-typed",
                    "include_image": true
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.journal_tail",
                "arguments": {
                    "limit": 10,
                    "method": "type_text"
                }
            }
        }),
    ])?;

    let validation = &responses[0]["result"];
    assert_eq!(validation["isError"], true);
    assert_eq!(
        validation["structuredContent"]["data"]["kind"],
        "validation"
    );
    assert!(
        validation["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("settle_timeout_ms")
    );

    let image_validation = &responses[1]["error"];
    assert_eq!(image_validation["code"], -32602);
    assert!(
        image_validation["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires capture_session_id")
    );

    let entries = responses[2]["result"]["structuredContent"]["data"]
        .as_array()
        .context("journal data is an array")?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["method"], "type_text");
    assert_eq!(entries[0]["ok"], false);
    assert_eq!(entries[0]["client"]["tool"], "seatgeist-mcp");
    let journal_json = serde_json::to_string(entries).context("serialize journal assertion")?;
    assert!(!journal_json.contains("must-not-be-typed-or-journaled"));
    assert!(!journal_json.contains("must-also-not-be-typed"));
    Ok(())
}

#[test]
fn mcp_core_profile_exposes_only_the_bounded_facade() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let responses = daemon.run_mcp_with_profile(
        Some("core"),
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
                "params": {}
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "seatgeist.computer_status",
                    "arguments": {}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "seatgeist.type_text",
                    "arguments": {"text": "must-not-run"}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "seatgeist.window_session",
                    "arguments": {"operation": "status"}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "seatgeist.snapshot",
                    "arguments": {}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {
                    "name": "seatgeist.wait",
                    "arguments": {}
                }
            }),
        ],
    )?;

    let tools = responses[0]["result"]["tools"]
        .as_array()
        .context("core tools/list is an array")?;
    assert!(tools.len() <= 8);
    assert_tool_present(tools, "seatgeist.computer_status");
    assert_tool_present(tools, "seatgeist.window_session");
    assert_tool_present(tools, "seatgeist.snapshot");
    assert_tool_present(tools, "seatgeist.act");
    assert_tool_present(tools, "seatgeist.wait");
    assert_tool_present(tools, "seatgeist.panic_stop");
    for name in ["seatgeist.snapshot", "seatgeist.wait"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .context("core retained capture tool exists")?;
        assert_eq!(tool["inputSchema"]["required"], json!(["session_id"]));
    }
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "seatgeist.type_text")
    );

    assert_eq!(
        responses[1]["result"]["structuredContent"]["type"],
        "computer_use_readiness"
    );
    assert_eq!(responses[2]["error"]["code"], -32602);
    assert!(
        responses[2]["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not available in the Core tool profile")
    );
    assert_eq!(
        responses[3]["result"]["structuredContent"]["type"],
        "capture_session_status"
    );
    assert_eq!(
        responses[3]["result"]["structuredContent"]["data"]["active"],
        false
    );
    for response in &responses[4..=5] {
        assert_eq!(response["error"]["code"], -32602);
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("argument 'session_id' is required")
        );
    }
    Ok(())
}

#[test]
fn mcp_stdio_reports_configured_denial_kinds() -> Result<()> {
    let focus_guard = DaemonFixture::start_with_config(
        r#"
[daemon]
socket = "__SOCKET__"
journal = "__JOURNAL__"
panic_stop_file = "__PANIC_STOP__"

[policy]
default_control = "allow"

[safety]
require_focus_guard = true
"#,
    )?;
    assert_mcp_error_kind(
        &focus_guard,
        "seatgeist.type_text",
        json!({
            "text": "blocked-before-input"
        }),
        "focus_guard",
        "focus guard is required",
    )?;

    let human_pause = DaemonFixture::start_with_config(
        r#"
[daemon]
socket = "__SOCKET__"
journal = "__JOURNAL__"
panic_stop_file = "__PANIC_STOP__"

[policy]
default_control = "allow"

[safety]
require_focus_guard = false
pause_on_human_input = true
human_input_activity_file = "__ROOT__/human-input-active"
human_input_quiet_ms = 60000
"#,
    )?;
    fs::write(human_pause.root.join("human-input-active"), "activity")
        .context("write fresh human input activity signal")?;
    assert_mcp_error_kind(
        &human_pause,
        "seatgeist.type_text",
        json!({
            "text": "blocked-before-input"
        }),
        "human_input_pause",
        "human input activity is fresh",
    )?;

    let app_policy = DaemonFixture::start_with_config(
        r#"
[daemon]
socket = "__SOCKET__"
journal = "__JOURNAL__"
panic_stop_file = "__PANIC_STOP__"

[policy]
default_control = "allow"

[apps]
allow = ["org.kde.kate"]

[safety]
require_focus_guard = false
"#,
    )?;
    assert_mcp_error_kind(
        &app_policy,
        "seatgeist.type_text",
        json!({
            "text": "blocked-before-input"
        }),
        "app_denied",
        "app policy could not read active window",
    )?;

    let portal_unavailable = DaemonFixture::start_with_config_and_env(
        r#"
[daemon]
socket = "__SOCKET__"
journal = "__JOURNAL__"
panic_stop_file = "__PANIC_STOP__"

[policy]
default_control = "allow"

[safety]
require_focus_guard = false
"#,
        &[("PATH", "__EMPTY_BIN__")],
    )?;
    assert_mcp_error_kind(
        &portal_unavailable,
        "seatgeist.remote_desktop_session_probe",
        json!({
            "keyboard": true,
            "pointer": true,
            "touchscreen": false,
            "timeout_ms": 1000
        }),
        "portal_unavailable",
        "xdg-desktop-portal RemoteDesktop is not available",
    )?;

    let accessibility = DaemonFixture::start()?;
    assert_mcp_error_kind(
        &accessibility,
        "seatgeist.a11y_text_attributes",
        json!({
            "node_id": "invalid-atspi-node",
            "offset": 0
        }),
        "validation",
        "invalid AT-SPI node id",
    )?;

    Ok(())
}

#[test]
fn mcp_stdio_raw_input_fails_closed_and_is_journaled() -> Result<()> {
    let daemon = DaemonFixture::start()?;
    let denied_text = "blocked-through-mcp";
    let responses = daemon.run_mcp(&[
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.type_text",
                "arguments": {
                    "text": denied_text
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "seatgeist.journal_tail",
                "arguments": {
                    "limit": 10,
                    "method": "type_text",
                    "ok": false
                }
            }
        }),
    ])?;

    assert_eq!(responses.len(), 3);

    let denied = &responses[1]["result"];
    assert_eq!(denied["isError"], true);
    assert_eq!(denied["structuredContent"]["type"], "error");
    assert_eq!(
        denied["structuredContent"]["data"]["kind"],
        "policy_prompt_required"
    );

    let journal = &responses[2]["result"];
    assert_eq!(journal["isError"], false);
    assert_eq!(journal["structuredContent"]["type"], "journal");
    let entries = journal["structuredContent"]["data"]
        .as_array()
        .context("journal data is an array")?;
    let Some(entry) = entries.iter().find(|entry| entry["method"] == "type_text") else {
        bail!("missing type_text journal entry: {entries:?}");
    };
    assert_eq!(entry["ok"], false);
    assert_eq!(entry["safety_class"], "control_keyboard");
    assert_eq!(entry["client"]["tool"], "seatgeist-mcp");
    assert!(
        entry["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("policy_prompt_required")
    );

    let journal_json = serde_json::to_string(journal).context("serialize journal response")?;
    assert!(
        !journal_json.contains(denied_text),
        "MCP journal response must not echo denied typed text: {journal_json}"
    );
    Ok(())
}

fn assert_tool_present(tools: &[Value], expected_name: &str) {
    assert!(
        tools.iter().any(|tool| tool["name"] == expected_name),
        "tools/list missing {expected_name}: {tools:?}"
    );
}

fn assert_mcp_error_kind(
    daemon: &DaemonFixture,
    tool_name: &str,
    arguments: Value,
    expected_kind: &str,
    expected_message: &str,
) -> Result<()> {
    let responses = daemon.run_mcp(&[
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        }),
    ])?;

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[1]["id"], 2);
    let result = &responses[1]["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["type"], "error");
    assert_eq!(
        result["structuredContent"]["data"]["kind"], expected_kind,
        "unexpected MCP error kind for {tool_name}: {result:?}"
    );
    assert!(
        result["structuredContent"]["data"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains(expected_message),
        "unexpected MCP error message for {tool_name}: {result:?}"
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains(expected_message),
        "unexpected MCP compact error text for {tool_name}: {result:?}"
    );
    Ok(())
}

fn parse_json_lines(stdout: &[u8]) -> Result<Vec<Value>> {
    String::from_utf8_lossy(stdout)
        .lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("parse MCP JSON response line {}", index + 1))
        })
        .collect()
}

fn require_success(label: &str, output: &Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "{label} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn daemon_binary() -> Result<PathBuf> {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_seatgeist-mcp")).with_file_name("seatgeistd");

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("resolve workspace root")?;
    let status = Command::new(env!("CARGO"))
        .arg("build")
        .arg("-p")
        .arg("seatgeistd")
        .arg("--quiet")
        .current_dir(workspace)
        .status()
        .context("build seatgeistd test binary")?;
    if !status.success() {
        bail!("cargo build -p seatgeistd failed with status {status}");
    }
    if candidate.exists() {
        return Ok(candidate);
    }
    bail!("seatgeistd binary was not built at {}", candidate.display())
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
        "seatgeist-mcp-integration-{}-{now}",
        std::process::id()
    ))
}
