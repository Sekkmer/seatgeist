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

    fn run_mcp(&self, requests: &[Value]) -> Result<Vec<Value>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_plasma-pilot-mcp"))
            .arg("--stdio")
            .arg("--socket")
            .arg(&self.socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn plasma-pilot-mcp")?;

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
        require_success("plasma-pilot-mcp --stdio", &output)?;
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
                "name": "plasma.health",
                "arguments": {}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "plasma.a11y_text_attributes",
                "arguments": {
                    "node_id": "invalid-atspi-node",
                    "offset": 0
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "plasma.journal_tail",
                "arguments": {
                    "limit": 10
                }
            }
        }),
    ])?;

    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(
        responses[0]["result"]["capabilities"]["tools"]["listChanged"],
        false
    );

    let tools = responses[1]["result"]["tools"]
        .as_array()
        .context("tools/list result is an array")?;
    assert_tool_present(tools, "plasma.health");
    assert_tool_present(tools, "plasma.remote_desktop_session_probe");
    assert_tool_present(tools, "plasma.remote_desktop_eis_probe");
    assert_tool_present(tools, "plasma.a11y_text_attributes");
    assert_tool_present(tools, "plasma.journal_tail");

    let health = &responses[2]["result"];
    assert_eq!(health["isError"], false);
    assert_eq!(health["structuredContent"]["type"], "health");
    assert_eq!(
        health["structuredContent"]["data"]["service"],
        "plasma-pilotd"
    );

    let attributes_error = &responses[3]["result"];
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

    let journal = &responses[4]["result"];
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
    Ok(())
}

fn assert_tool_present(tools: &[Value], expected_name: &str) {
    assert!(
        tools.iter().any(|tool| tool["name"] == expected_name),
        "tools/list missing {expected_name}: {tools:?}"
    );
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
    let candidate =
        PathBuf::from(env!("CARGO_BIN_EXE_plasma-pilot-mcp")).with_file_name("plasma-pilotd");

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
        "plasma-pilot-mcp-integration-{}-{now}",
        std::process::id()
    ))
}
