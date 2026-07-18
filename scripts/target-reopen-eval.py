#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from computer_use_eval import (
    EvalError,
    ROOT,
    response_data,
    run_cli,
    socket_identity,
    unix_time_ms,
    workspace_revision,
    write_private_json,
)
from retained_capture_eval import (
    hashed_window_id,
    require_no_active_capture,
    session_id_for_cleanup,
    validate_open_session,
)
from target_reopen_eval import (
    find_replacement,
    resolve_original_target,
    sanitized_replacement,
    validate_unbound_capture_status,
)


DEFAULT_CLI = ROOT / "target/debug/seatgeist-cli"
DEFAULT_RUN_ROOT = ROOT / "target/seatgeist-target-reopen-eval"

CliRunner = Callable[..., dict[str, Any]]
InputReader = Callable[[str], str]
MessageWriter = Callable[[str], None]


@dataclass(frozen=True)
class TargetReopenConfig:
    window_id: str
    cli: Path
    socket: Path | None
    output_dir: Path
    transition_timeout_ms: int
    poll_interval_ms: int


def default_output_dir() -> Path:
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    return DEFAULT_RUN_ROOT / f"target-reopen-{stamp}-{os.getpid()}"


def redact_error(
    error: Exception,
    window_id: str,
    session_id: str | None,
    replacement_id: str | None,
) -> str:
    message = str(error).replace(window_id, "<original-window>")
    if session_id:
        message = message.replace(session_id, "<capture-session>")
    if replacement_id:
        message = message.replace(replacement_id, "<replacement-window>")
    return message


def run_target_reopen_eval(
    config: TargetReopenConfig,
    *,
    cli_runner: CliRunner = run_cli,
    input_reader: InputReader = input,
    message_writer: MessageWriter = print,
    sleeper: Callable[[float], None] = time.sleep,
) -> dict[str, Any]:
    config.output_dir.mkdir(parents=True, exist_ok=False)
    config.output_dir.chmod(0o700)
    evidence_path = config.output_dir / "evidence.json"
    evidence: dict[str, Any] = {
        "type": "seatgeist_target_reopen_eval",
        "version": 1,
        "status": "running",
        "acceptance_complete": False,
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "original_window_sha256": hashed_window_id(config.window_id),
        "socket_identity": socket_identity(config.socket),
        "portal_open_count": 0,
        "replacement": None,
        "post_reopen_status": None,
        "session_cleanup": None,
        "explicit_focus_call_count": 0,
        "raw_input_call_count": 0,
        "errors": [],
    }
    session_id: str | None = None
    replacement_id: str | None = None
    capture_active = False
    cleanup_ok = False
    try:
        if evidence["socket_identity"] is None:
            raise EvalError("daemon socket identity is unavailable")
        original = resolve_original_target(
            cli_runner(config.cli, config.socket, "windows"), config.window_id
        )
        require_no_active_capture(
            cli_runner(config.cli, config.socket, "capture", "status")
        )
        message_writer("Select exactly the requested original window in the portal chooser.")
        opened = cli_runner(
            config.cli,
            config.socket,
            "capture",
            "open",
            "--requested-window-id",
            config.window_id,
            "--timeout-ms",
            "120000",
        )
        evidence["portal_open_count"] = 1
        session_id = session_id_for_cleanup(opened)
        session = validate_open_session(opened, config.window_id)
        validated_id = session.pop("session_id")
        if session_id != validated_id or session_id is None:
            raise EvalError("capture open returned an inconsistent session id")
        capture_active = True

        answer = input_reader(
            "Close the original target window, reopen the same application as a new "
            "window, then press Enter (type skip to abort): "
        )
        if answer.strip().lower() in {"s", "skip"}:
            raise EvalError("operator skipped target close/reopen")

        replacement = None
        deadline = time.monotonic() + config.transition_timeout_ms / 1000
        while time.monotonic() < deadline:
            replacement = find_replacement(
                cli_runner(config.cli, config.socket, "windows"), original
            )
            if replacement is not None:
                break
            sleeper(config.poll_interval_ms / 1000)
        if replacement is None:
            raise EvalError("no distinct same-application replacement window appeared")
        replacement_id = replacement["window_id"]
        evidence["replacement"] = sanitized_replacement(original, replacement)

        status = validate_unbound_capture_status(
            cli_runner(config.cli, config.socket, "capture", "status"), session_id
        )
        evidence["post_reopen_status"] = status
        capture_active = status["capture_active"]
    except (EvalError, EOFError, KeyboardInterrupt) as err:
        if isinstance(err, KeyboardInterrupt):
            err = EvalError("operator interrupted target close/reopen evaluation")
        evidence["errors"].append(
            redact_error(err, config.window_id, session_id, replacement_id)
        )
    finally:
        if session_id is not None and capture_active:
            try:
                closed = cli_runner(
                    config.cli,
                    config.socket,
                    "capture",
                    "close",
                    "--session-id",
                    session_id,
                )
                closed_data = response_data(closed, "capture_session_status")
                cleanup_ok = (
                    closed_data.get("active") is False
                    and closed_data.get("last_end_reason") == "client_closed"
                )
                evidence["session_cleanup"] = "client_closed" if cleanup_ok else "failed"
            except EvalError as err:
                evidence["errors"].append(
                    redact_error(err, config.window_id, session_id, replacement_id)
                )
                evidence["session_cleanup"] = "failed"
        elif session_id is not None:
            cleanup_ok = True
            evidence["session_cleanup"] = "portal_ended"

        post = evidence.get("post_reopen_status")
        evidence["acceptance_complete"] = bool(
            evidence.get("replacement")
            and isinstance(post, dict)
            and post.get("sticky_target_bound") is False
            and cleanup_ok
            and evidence["explicit_focus_call_count"] == 0
            and evidence["raw_input_call_count"] == 0
        )
        evidence["status"] = (
            "passed"
            if evidence["acceptance_complete"] and not evidence["errors"]
            else "failed"
        )
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)
    return evidence


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Run the opt-in close/reopen target-identity eval. It opens a real "
            "window capture but sends no focus or raw input."
        )
    )
    parser.add_argument("--window-id", required=True)
    parser.add_argument("--cli", type=Path, default=DEFAULT_CLI)
    parser.add_argument("--socket", type=Path)
    parser.add_argument("--output-dir", type=Path, default=default_output_dir())
    parser.add_argument("--transition-timeout-ms", type=int, default=30_000)
    parser.add_argument("--poll-interval-ms", type=int, default=250)
    args = parser.parse_args()
    if not 1_000 <= args.transition_timeout_ms <= 120_000:
        parser.error("--transition-timeout-ms must be between 1000 and 120000")
    if not 10 <= args.poll_interval_ms <= 1_000:
        parser.error("--poll-interval-ms must be between 10 and 1000")

    evidence = run_target_reopen_eval(
        TargetReopenConfig(
            window_id=args.window_id,
            cli=args.cli,
            socket=args.socket,
            output_dir=args.output_dir,
            transition_timeout_ms=args.transition_timeout_ms,
            poll_interval_ms=args.poll_interval_ms,
        )
    )
    print(
        f"target-reopen-eval: status={evidence['status']} "
        f"evidence={args.output_dir / 'evidence.json'}"
    )
    if evidence["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
