from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOCKET = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")) / "seatgeist/seatgeistd.sock"


class DeploymentError(RuntimeError):
    pass


@dataclass(frozen=True)
class Config:
    root: Path
    cargo: str
    cli: Path
    systemctl: str
    socket: Path
    release_binary: Path
    install_path: Path
    service: str
    proc_root: Path
    timeout_ms: int
    poll_ms: int
    skip_build: bool


def run(
    args: list[str],
    *,
    cwd: Path,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        args,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        check=False,
    )
    if check and completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or f"exit {completed.returncode}"
        raise DeploymentError(f"command failed: {args[0]}: {detail}")
    return completed


def cli_json(config: Config, command: list[str]) -> dict[str, Any]:
    completed = run(
        [str(config.cli), "--socket", str(config.socket), *command],
        cwd=config.root,
    )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise DeploymentError(f"Seatgeist CLI returned invalid JSON for {' '.join(command)}") from error
    if not isinstance(value, dict):
        raise DeploymentError(f"Seatgeist CLI returned a non-object for {' '.join(command)}")
    return value


def response_active(response: dict[str, Any], expected_type: str) -> bool:
    if response.get("type") != expected_type:
        raise DeploymentError(
            f"expected {expected_type} response, got {response.get('type')!r}"
        )
    data = response.get("data")
    if not isinstance(data, dict) or not isinstance(data.get("active"), bool):
        raise DeploymentError(f"{expected_type} response is missing boolean data.active")
    return data["active"]


def ensure_retained_sessions_idle(config: Config, stage: str) -> None:
    capture = cli_json(config, ["capture", "status"])
    eis = cli_json(config, ["input", "remote-desktop-eis-session-status"])
    active = []
    if response_active(capture, "capture_session_status"):
        active.append("capture")
    if response_active(eis, "remote_desktop_eis_session_status"):
        active.append("remote-desktop-eis")
    if active:
        raise DeploymentError(
            f"refusing {stage}: active retained session(s): {', '.join(active)}"
        )


def build_artifacts(config: Config) -> None:
    if config.skip_build:
        return
    run([config.cargo, "build", "-p", "seatgeist-cli"], cwd=config.root)
    build_env = os.environ.copy()
    build_env["SEATGEIST_BUILD_UNIX_MS"] = str(time.time_ns() // 1_000_000)
    git = run(
        ["git", "rev-parse", "--verify", "HEAD"],
        cwd=config.root,
        check=False,
    )
    if git.returncode == 0 and git.stdout.strip():
        build_env["SEATGEIST_GIT_SHA"] = git.stdout.strip()
    run(
        [config.cargo, "build", "--release", "-p", "seatgeistd"],
        cwd=config.root,
        env=build_env,
    )


def atomic_install(source: Path, destination: Path) -> None:
    if not source.is_file():
        raise DeploymentError(f"release daemon is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as temporary:
        temporary_path = Path(temporary.name)
    try:
        shutil.copyfile(source, temporary_path)
        temporary_path.chmod(0o755)
        os.replace(temporary_path, destination)
    finally:
        temporary_path.unlink(missing_ok=True)


def restart_service(config: Config) -> None:
    run(
        [config.systemctl, "--user", "restart", config.service],
        cwd=config.root,
    )


def wait_until(config: Config, probe, description: str) -> tuple[Any, int]:
    deadline = time.monotonic() + config.timeout_ms / 1000
    attempts = 0
    last_error = "not attempted"
    while True:
        attempts += 1
        try:
            value = probe()
            if value is not None:
                return value, attempts
            last_error = "probe returned not ready"
        except (DeploymentError, OSError) as error:
            last_error = str(error)
        if time.monotonic() >= deadline:
            raise DeploymentError(
                f"timed out waiting for {description} after {attempts} attempts: {last_error}"
            )
        time.sleep(config.poll_ms / 1000)


def wait_for_daemon(config: Config) -> tuple[dict[str, Any], int]:
    def probe() -> dict[str, Any] | None:
        response = cli_json(config, ["doctor"])
        return response if response.get("type") == "health" else None

    return wait_until(config, probe, "daemon request readiness")


def bridge_ready(response: dict[str, Any]) -> bool:
    if response.get("type") != "kwin_bridge_status":
        return False
    data = response.get("data")
    return bool(
        isinstance(data, dict)
        and data.get("dbus_service_registered") is True
        and data.get("active_window_update_seen") is True
        and data.get("window_list_update_seen") is True
        and isinstance(data.get("window_count"), int)
        and not isinstance(data.get("window_count"), bool)
        and data["window_count"] > 0
    )


def wait_for_bridge(config: Config) -> tuple[dict[str, Any], int]:
    def probe() -> dict[str, Any] | None:
        response = cli_json(config, ["kwin-bridge-status"])
        return response if bridge_ready(response) else None

    return wait_until(config, probe, "KWin bridge heartbeat")


def service_pid(config: Config) -> int:
    completed = run(
        [
            config.systemctl,
            "--user",
            "show",
            "--property",
            "MainPID",
            "--value",
            config.service,
        ],
        cwd=config.root,
    )
    try:
        pid = int(completed.stdout.strip())
    except ValueError as error:
        raise DeploymentError("systemd returned an invalid MainPID") from error
    if pid <= 0:
        raise DeploymentError("systemd reports that the daemon has no running MainPID")
    return pid


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise DeploymentError(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def verify_hashes(config: Config, pid: int) -> str:
    running_exe = config.proc_root / str(pid) / "exe"
    hashes = {
        "release": sha256(config.release_binary),
        "installed": sha256(config.install_path),
        "running": sha256(running_exe),
    }
    if len(set(hashes.values())) != 1:
        raise DeploymentError(
            "daemon hash mismatch: "
            + ", ".join(f"{name}={value}" for name, value in hashes.items())
        )
    return hashes["release"]


def verify_health_provenance(health: dict[str, Any], digest: str) -> dict[str, Any]:
    data = health.get("data")
    if not isinstance(data, dict):
        raise DeploymentError("health response is missing provenance data")
    if data.get("binary_sha256") != digest:
        raise DeploymentError(
            "health binary fingerprint does not match deployed daemon: "
            f"reported={data.get('binary_sha256')!r}, expected={digest}"
        )
    if data.get("protocol_version") != "1":
        raise DeploymentError(
            f"health reported unsupported protocol version {data.get('protocol_version')!r}"
        )
    for field in ("run_id", "config_fingerprint"):
        if not isinstance(data.get(field), str) or not data[field]:
            raise DeploymentError(f"health response is missing {field}")
    return data


def deploy(config: Config) -> dict[str, Any]:
    build_artifacts(config)
    if not config.cli.is_file():
        raise DeploymentError(f"Seatgeist CLI is missing: {config.cli}")
    ensure_retained_sessions_idle(config, "daemon deployment")
    atomic_install(config.release_binary, config.install_path)
    ensure_retained_sessions_idle(config, "daemon restart")
    restart_service(config)
    health, daemon_attempts = wait_for_daemon(config)
    bridge, bridge_attempts = wait_for_bridge(config)
    pid = service_pid(config)
    digest = verify_hashes(config, pid)
    health_data = verify_health_provenance(health, digest)
    ensure_retained_sessions_idle(config, "post-deployment verification")
    return {
        "type": "seatgeist_user_daemon_deployment",
        "version": 1,
        "ok": True,
        "service": config.service,
        "pid": pid,
        "sha256": digest,
        "run_id": health_data["run_id"],
        "git_sha": health_data.get("git_sha"),
        "config_fingerprint": health_data["config_fingerprint"],
        "daemon_readiness_attempts": daemon_attempts,
        "bridge_readiness_attempts": bridge_attempts,
        "window_count": bridge["data"]["window_count"],
        "capture_session_active": False,
        "eis_session_active": False,
    }


def parse_args(argv: list[str] | None = None) -> Config:
    parser = argparse.ArgumentParser(
        description=(
            "Build, install, restart, and verify the user Seatgeist daemon while "
            "failing closed around retained capture and EIS sessions."
        )
    )
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--cli", type=Path)
    parser.add_argument("--systemctl", default="systemctl")
    parser.add_argument("--socket", type=Path, default=DEFAULT_SOCKET)
    parser.add_argument("--release-binary", type=Path)
    parser.add_argument("--install-path", type=Path, default=Path.home() / ".local/bin/seatgeistd")
    parser.add_argument("--service", default="seatgeistd.service")
    parser.add_argument("--timeout-ms", type=int, default=10_000)
    parser.add_argument("--poll-ms", type=int, default=100)
    parser.add_argument("--proc-root", type=Path, default=Path("/proc"), help=argparse.SUPPRESS)
    parser.add_argument("--skip-build", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if args.timeout_ms <= 0 or args.timeout_ms > 60_000:
        parser.error("--timeout-ms must be between 1 and 60000")
    if args.poll_ms <= 0 or args.poll_ms > args.timeout_ms:
        parser.error("--poll-ms must be positive and no greater than --timeout-ms")
    root = args.root.resolve()
    return Config(
        root=root,
        cargo=args.cargo,
        cli=(args.cli or root / "target/debug/seatgeist-cli").resolve(),
        systemctl=args.systemctl,
        socket=args.socket,
        release_binary=(args.release_binary or root / "target/release/seatgeistd").resolve(),
        install_path=args.install_path.resolve(),
        service=args.service,
        proc_root=args.proc_root.resolve(),
        timeout_ms=args.timeout_ms,
        poll_ms=args.poll_ms,
        skip_build=args.skip_build,
    )


def main(argv: list[str] | None = None) -> None:
    config = parse_args(argv)
    try:
        report = deploy(config)
    except DeploymentError as error:
        raise SystemExit(f"deploy-seatgeistd-user: {error}") from error
    print(json.dumps(report, indent=2, sort_keys=True))
