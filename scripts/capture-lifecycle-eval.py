#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from capture_lifecycle_eval import (
    lifecycle_acceptance_complete,
    stale_session_rejection_kind,
    validate_portal_closed_status,
)
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
    validate_frame,
    validate_open_session,
)


DEFAULT_CLI = ROOT / "target/debug/seatgeist-cli"
DEFAULT_RUN_ROOT = ROOT / "target/seatgeist-capture-lifecycle-eval"

CliRunner = Callable[..., dict[str, Any]]
InputReader = Callable[[str], str]
MessageWriter = Callable[[str], None]


@dataclass(frozen=True)
class LifecycleEvalConfig:
    window_id: str
    cli: Path
    socket: Path | None
    output_dir: Path
    max_edge: int
    frame_timeout_ms: int
    revocation_timeout_ms: int
    poll_interval_ms: int


def default_output_dir() -> Path:
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    return DEFAULT_RUN_ROOT / f"portal-revocation-{stamp}-{os.getpid()}"


def redacted_error(error: Exception, window_id: str, session_id: str | None) -> str:
    message = str(error).replace(window_id, "<target-window>")
    if session_id:
        message = message.replace(session_id, "<capture-session>")
    return message


def run_lifecycle_eval(
    config: LifecycleEvalConfig,
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
        "type": "seatgeist_capture_lifecycle_eval",
        "version": 1,
        "status": "running",
        "acceptance_complete": False,
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "target_window_sha256": hashed_window_id(config.window_id),
        "socket_identity": socket_identity(config.socket),
        "portal_open_count": 0,
        "initial_frame": None,
        "initial_frame_captured": False,
        "status_poll_count": 0,
        "ended_status": None,
        "stale_session_rejected": False,
        "stale_session_rejection_kind": None,
        "cleanup_close_called": False,
        "explicit_focus_call_count": 0,
        "raw_input_call_count": 0,
        "errors": [],
    }
    session_id: str | None = None
    portal_ended = False
    try:
        if evidence["socket_identity"] is None:
            raise EvalError("daemon socket identity is unavailable")
        require_no_active_capture(
            cli_runner(config.cli, config.socket, "capture", "status")
        )
        message_writer("Select exactly the requested window in the portal chooser.")
        opened = cli_runner(
            config.cli,
            config.socket,
            "capture",
            "open",
            "--requested-window-id",
            config.window_id,
            "--timeout-ms",
            str(max(config.frame_timeout_ms, 120_000)),
        )
        evidence["portal_open_count"] = 1
        session_id = session_id_for_cleanup(opened)
        session = validate_open_session(opened, config.window_id)
        validated_id = session.pop("session_id")
        if session_id != validated_id or session_id is None:
            raise EvalError("capture open returned an inconsistent session id")

        initial_output = config.output_dir / "initial.png"
        initial = cli_runner(
            config.cli,
            config.socket,
            "capture",
            "snapshot",
            "--session-id",
            session_id,
            "--output",
            str(initial_output),
            "--max-edge",
            str(config.max_edge),
            "--timeout-ms",
            str(config.frame_timeout_ms),
        )
        evidence["initial_frame"] = validate_frame(
            initial,
            response_type="capture_frame",
            session_id=session_id,
            artifact_root=config.output_dir,
            expected_output=initial_output,
            max_edge=config.max_edge,
        )
        evidence["initial_frame_captured"] = True

        answer = input_reader(
            "Stop or revoke this share from KDE's portal/sharing UI, then press Enter "
            "(type skip to abort): "
        )
        if answer.strip().lower() in {"s", "skip"}:
            raise EvalError("operator skipped portal revocation")

        deadline = time.monotonic() + config.revocation_timeout_ms / 1000
        while time.monotonic() < deadline:
            status_response = cli_runner(
                config.cli, config.socket, "capture", "status"
            )
            evidence["status_poll_count"] += 1
            status = response_data(status_response, "capture_session_status")
            if status.get("active") is False:
                evidence["ended_status"] = validate_portal_closed_status(status_response)
                portal_ended = True
                break
            sleeper(config.poll_interval_ms / 1000)
        if not portal_ended:
            raise EvalError("portal revocation did not end the capture session before timeout")

        stale_output = config.output_dir / "stale-session.png"
        try:
            cli_runner(
                config.cli,
                config.socket,
                "capture",
                "snapshot",
                "--session-id",
                session_id,
                "--output",
                str(stale_output),
                "--max-edge",
                str(config.max_edge),
                "--timeout-ms",
                str(config.frame_timeout_ms),
            )
        except EvalError as err:
            rejection_kind = stale_session_rejection_kind(err)
            if rejection_kind is None:
                raise
            evidence["stale_session_rejected"] = True
            evidence["stale_session_rejection_kind"] = rejection_kind
        else:
            raise EvalError("snapshot unexpectedly succeeded after portal closure")
        if stale_output.exists():
            raise EvalError("rejected stale-session snapshot wrote an artifact")
    except (EvalError, EOFError, KeyboardInterrupt) as err:
        if isinstance(err, KeyboardInterrupt):
            err = EvalError("operator interrupted portal-revocation evaluation")
        evidence["errors"].append(redacted_error(err, config.window_id, session_id))
    finally:
        if session_id is not None and not portal_ended:
            evidence["cleanup_close_called"] = True
            try:
                cli_runner(
                    config.cli,
                    config.socket,
                    "capture",
                    "close",
                    "--session-id",
                    session_id,
                )
            except EvalError as err:
                evidence["errors"].append(
                    redacted_error(err, config.window_id, session_id)
                )
        evidence["acceptance_complete"] = lifecycle_acceptance_complete(evidence)
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
            "Run the opt-in portal-revocation lifecycle eval. It opens one real "
            "window capture and requires the operator to revoke that share."
        )
    )
    parser.add_argument("--window-id", required=True)
    parser.add_argument("--cli", type=Path, default=DEFAULT_CLI)
    parser.add_argument("--socket", type=Path)
    parser.add_argument("--output-dir", type=Path, default=default_output_dir())
    parser.add_argument("--max-edge", type=int, default=1200)
    parser.add_argument("--frame-timeout-ms", type=int, default=8000)
    parser.add_argument("--revocation-timeout-ms", type=int, default=15_000)
    parser.add_argument("--poll-interval-ms", type=int, default=100)
    args = parser.parse_args()
    if not 1 <= args.max_edge <= 2048:
        parser.error("--max-edge must be between 1 and 2048")
    if not 1 <= args.frame_timeout_ms <= 30_000:
        parser.error("--frame-timeout-ms must be between 1 and 30000")
    if not 1_000 <= args.revocation_timeout_ms <= 120_000:
        parser.error("--revocation-timeout-ms must be between 1000 and 120000")
    if not 10 <= args.poll_interval_ms <= 1_000:
        parser.error("--poll-interval-ms must be between 10 and 1000")

    evidence = run_lifecycle_eval(
        LifecycleEvalConfig(
            window_id=args.window_id,
            cli=args.cli,
            socket=args.socket,
            output_dir=args.output_dir,
            max_edge=args.max_edge,
            frame_timeout_ms=args.frame_timeout_ms,
            revocation_timeout_ms=args.revocation_timeout_ms,
            poll_interval_ms=args.poll_interval_ms,
        )
    )
    print(
        f"capture-lifecycle-eval: status={evidence['status']} "
        f"evidence={args.output_dir / 'evidence.json'}"
    )
    if evidence["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
