from __future__ import annotations

import json
import os
import signal
import subprocess
import time
from pathlib import Path
from typing import Any, Mapping

from computer_use_eval import EvalError


def run_cli(cli: Path, socket: Path, *arguments: str) -> dict[str, Any]:
    result = subprocess.run(
        [str(cli), "--socket", str(socket), *arguments],
        capture_output=True,
        check=False,
        timeout=5,
    )
    if result.returncode != 0:
        message = result.stderr.decode("utf-8", errors="replace").strip()
        raise EvalError(message or f"nested CLI command failed: {' '.join(arguments)}")
    try:
        response = json.loads(result.stdout)
    except json.JSONDecodeError as err:
        raise EvalError("nested CLI returned malformed JSON") from err
    if not isinstance(response, dict) or response.get("type") == "error":
        raise EvalError("nested CLI returned an error response")
    return response


def sanitized_monitors(response: Mapping[str, Any]) -> dict[str, Any]:
    monitors = response.get("data")
    if response.get("type") != "monitors" or not isinstance(monitors, list):
        raise EvalError("nested daemon monitor response is malformed")
    outputs = []
    for monitor in monitors:
        if not isinstance(monitor, dict):
            raise EvalError("nested daemon returned a malformed monitor")
        x = monitor.get("logical_origin_x")
        y = monitor.get("logical_origin_y")
        if not isinstance(x, int) or not isinstance(y, int):
            raise EvalError("nested daemon returned invalid monitor origins")
        outputs.append({"logical_origin_x": x, "logical_origin_y": y})
    outputs.sort(
        key=lambda output: (output["logical_origin_x"], output["logical_origin_y"])
    )
    result = {
        "monitor_count": len(outputs),
        "has_nonzero_logical_origin": any(
            output["logical_origin_x"] != 0 or output["logical_origin_y"] != 0
            for output in outputs
        ),
        "outputs": outputs,
    }
    if result["monitor_count"] != 2 or not result["has_nonzero_logical_origin"]:
        raise EvalError("nested daemon did not preserve the two-output topology")
    return result


def sanitized_bridge(response: Mapping[str, Any]) -> dict[str, Any]:
    data = response.get("data")
    if response.get("type") != "kwin_bridge_status" or not isinstance(data, dict):
        raise EvalError("nested daemon bridge response is malformed")
    required = (
        "dbus_service_registered",
        "active_window_update_seen",
        "window_list_update_seen",
        "package_installed",
        "script_enabled",
    )
    if any(data.get(field) is not True for field in required):
        raise EvalError("nested daemon did not receive a complete KWin bridge snapshot")
    return {field: True for field in required}


def daemon_socket_path(runtime_dir: Path) -> Path:
    path = runtime_dir / "d"
    if len(os.fsencode(path)) + 1 > 108:
        raise EvalError("nested daemon socket path exceeds the Unix socket limit")
    return path


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def wait_for_probe(
    process: subprocess.Popen[bytes],
    cli: Path,
    socket: Path,
    timeout_seconds: float,
) -> tuple[dict[str, Any], dict[str, Any]]:
    deadline = time.monotonic() + timeout_seconds
    last_error = "nested daemon did not become ready"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise EvalError(f"nested daemon exited with code {process.returncode}")
        try:
            monitors = sanitized_monitors(run_cli(cli, socket, "monitors"))
            bridge = sanitized_bridge(run_cli(cli, socket, "kwin-bridge-status"))
            return monitors, bridge
        except (EvalError, OSError, subprocess.SubprocessError) as err:
            last_error = str(err)
        time.sleep(0.1)
    raise EvalError(f"nested daemon readiness timed out: {last_error}")


def probe_nested_daemon(
    daemon: Path,
    cli: Path,
    runtime_dir: Path,
    state_dir: Path,
    log_dir: Path,
    timeout_seconds: float = 10.0,
) -> tuple[dict[str, Any], dict[str, Any]]:
    socket = daemon_socket_path(runtime_dir)
    journal = state_dir / "seatgeistd-journal.jsonl"
    log_path = log_dir / "seatgeistd.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            [
                str(daemon),
                "--socket",
                str(socket),
                "--journal",
                str(journal),
                "--capture-restore-file",
                str(state_dir / "capture-restore.json"),
            ],
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            return wait_for_probe(process, cli, socket, timeout_seconds)
        finally:
            stop_process(process)
