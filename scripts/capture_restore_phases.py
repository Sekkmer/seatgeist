from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from capture_restore_eval import (
    EVIDENCE_TYPE,
    EVIDENCE_VERSION,
    normalize_chooser_answer,
    require_daemon_restart,
    require_prepared_evidence,
    require_restore_reference,
    restart_acceptance_complete,
    restore_file_was_replaced,
    same_file_identity,
)
from computer_use_eval import (
    EvalError,
    private_file_identity,
    read_private_json,
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


CliRunner = Callable[..., dict[str, Any]]
InputReader = Callable[[str], str]
MessageWriter = Callable[[str], None]


@dataclass(frozen=True)
class RestoreEvalConfig:
    window_id: str
    cli: Path
    socket: Path | None
    restore_file: Path
    output_dir: Path
    max_edge: int
    timeout_ms: int


def close_opened_session(
    config: RestoreEvalConfig,
    session_id: str | None,
    cli_runner: CliRunner,
) -> tuple[bool, str | None]:
    if session_id is None:
        return False, None
    try:
        response = cli_runner(
            config.cli,
            config.socket,
            "capture",
            "close",
            "--session-id",
            session_id,
        )
        data = response_data(response, "capture_session_status")
        if data.get("active") is False:
            return True, None
        return False, "capture close left the session active"
    except EvalError as err:
        return False, f"capture close failed: {err}"


def request_open(
    config: RestoreEvalConfig, cli_runner: CliRunner
) -> dict[str, Any]:
    return cli_runner(
        config.cli,
        config.socket,
        "capture",
        "open",
        "--requested-window-id",
        config.window_id,
        "--timeout-ms",
        str(max(config.timeout_ms, 120_000)),
    )


def capture_snapshot(
    config: RestoreEvalConfig,
    session_id: str,
    filename: str,
    cli_runner: CliRunner,
) -> dict[str, Any]:
    output = config.output_dir / filename
    response = cli_runner(
        config.cli,
        config.socket,
        "capture",
        "snapshot",
        "--session-id",
        session_id,
        "--output",
        str(output),
        "--max-edge",
        str(config.max_edge),
        "--timeout-ms",
        str(config.timeout_ms),
    )
    return validate_frame(
        response,
        response_type="capture_frame",
        session_id=session_id,
        artifact_root=config.output_dir,
        expected_output=output,
        max_edge=config.max_edge,
    )


def prepare_restore_eval(
    config: RestoreEvalConfig,
    *,
    cli_runner: CliRunner = run_cli,
    message_writer: MessageWriter = print,
) -> dict[str, Any]:
    config.output_dir.mkdir(parents=True, exist_ok=False)
    config.output_dir.chmod(0o700)
    evidence_path = config.output_dir / "evidence.json"
    evidence: dict[str, Any] = {
        "type": EVIDENCE_TYPE,
        "version": EVIDENCE_VERSION,
        "status": "running",
        "acceptance_complete": False,
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "target_window_sha256": hashed_window_id(config.window_id),
        "prepared_socket_identity": socket_identity(config.socket),
        "prepare": None,
        "prepare_session_closed": False,
        "daemon_restart_proven": False,
        "resume": None,
        "resume_session_closed": False,
        "errors": [],
    }
    session_id: str | None = None
    try:
        if evidence["prepared_socket_identity"] is None:
            raise EvalError("daemon socket identity is unavailable")
        require_no_active_capture(
            cli_runner(config.cli, config.socket, "capture", "status")
        )
        message_writer("Select exactly the requested window if the portal chooser appears.")
        opened = request_open(config, cli_runner)
        session_id = session_id_for_cleanup(opened)
        session = validate_open_session(opened, config.window_id)
        validated_id = session.pop("session_id")
        if session_id != validated_id:
            raise EvalError("capture session id changed during open validation")
        require_restore_reference(session)
        if session_id is None:
            raise EvalError("capture open returned no session id")
        frame = capture_snapshot(config, session_id, "prepare.png", cli_runner)
        restore_file = private_file_identity(config.restore_file)
        evidence["prepare"] = {
            "session": session,
            "frame": frame,
            "restore_file_after_open": restore_file,
        }
    except EvalError as err:
        evidence["errors"].append(str(err))
    finally:
        closed, close_error = close_opened_session(config, session_id, cli_runner)
        evidence["prepare_session_closed"] = closed
        if close_error is not None:
            evidence["errors"].append(close_error)
        evidence["status"] = "prepared" if not evidence["errors"] else "failed"
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)
    return evidence


def resume_restore_eval(
    config: RestoreEvalConfig,
    *,
    cli_runner: CliRunner = run_cli,
    input_reader: InputReader = input,
    message_writer: MessageWriter = print,
) -> dict[str, Any]:
    evidence_path = config.output_dir / "evidence.json"
    evidence = read_private_json(evidence_path)
    prepared = require_prepared_evidence(evidence, config.window_id)
    if evidence.get("workspace") != workspace_revision():
        raise EvalError("workspace changed between capture-restore prepare and resume")

    current_socket = socket_identity(config.socket)
    require_daemon_restart(evidence.get("prepared_socket_identity"), current_socket)
    restore_before = private_file_identity(config.restore_file)
    if not same_file_identity(prepared["restore_file_after_open"], restore_before):
        raise EvalError("restore-token state changed between prepare and resume")
    require_no_active_capture(
        cli_runner(config.cli, config.socket, "capture", "status")
    )

    evidence["status"] = "running"
    evidence["ended_unix_ms"] = None
    evidence["daemon_restart_proven"] = True
    evidence["resumed_socket_identity"] = current_socket
    evidence["resume"] = None
    evidence["resume_session_closed"] = False
    evidence["errors"] = []
    session_id: str | None = None
    try:
        message_writer(
            "Opening the same requested target. A working restore token should avoid "
            "the source chooser."
        )
        opened = request_open(config, cli_runner)
        session_id = session_id_for_cleanup(opened)
        session = validate_open_session(opened, config.window_id)
        validated_id = session.pop("session_id")
        if session_id != validated_id:
            raise EvalError("capture session id changed during open validation")
        require_restore_reference(session)
        if session_id is None:
            raise EvalError("capture open returned no session id")
        chooser_avoided = normalize_chooser_answer(
            input_reader("Did a portal source chooser appear during this open? [yes/no]: ")
        )
        frame = capture_snapshot(config, session_id, "resume.png", cli_runner)
        restore_after = private_file_identity(config.restore_file)
        replaced = restore_file_was_replaced(restore_before, restore_after)
        evidence["resume"] = {
            "session": session,
            "frame": frame,
            "portal_chooser_avoided": chooser_avoided,
            "restore_file_before_open": restore_before,
            "restore_file_after_open": restore_after,
            "restore_file_replaced": replaced,
        }
        if require_restore_reference(prepared["session"]) != require_restore_reference(
            session
        ):
            raise EvalError("opaque restore reference changed for the same target")
        if not chooser_avoided:
            raise EvalError("portal source chooser reappeared after daemon restart")
        if not replaced:
            raise EvalError("private restore-token state was not atomically rotated")
    except (EvalError, EOFError) as err:
        evidence["errors"].append(str(err))
    finally:
        closed, close_error = close_opened_session(config, session_id, cli_runner)
        evidence["resume_session_closed"] = closed
        if close_error is not None:
            evidence["errors"].append(close_error)
        try:
            evidence["acceptance_complete"] = restart_acceptance_complete(evidence)
        except EvalError as err:
            evidence["errors"].append(str(err))
            evidence["acceptance_complete"] = False
        evidence["status"] = (
            "passed"
            if evidence["acceptance_complete"] and not evidence["errors"]
            else "failed"
        )
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)
    return evidence
