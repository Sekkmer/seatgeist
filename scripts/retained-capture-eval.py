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
    CaptureScenario,
    capture_acceptance_complete,
    hashed_window_id,
    normalize_visual_verdict,
    require_no_active_capture,
    session_id_for_cleanup,
    sanitized_monitor_layout,
    selected_scenarios,
    validate_frame,
    validate_open_session,
)


DEFAULT_CLI = ROOT / "target/debug/seatgeist-cli"
DEFAULT_RUN_ROOT = ROOT / "target/seatgeist-retained-capture-eval"

CliRunner = Callable[..., dict[str, Any]]
InputReader = Callable[[str], str]
MessageWriter = Callable[[str], None]


@dataclass(frozen=True)
class EvalConfig:
    window_id: str
    cli: Path
    socket: Path | None
    output_dir: Path
    scenarios: tuple[CaptureScenario, ...]
    max_edge: int
    timeout_ms: int
    require_multi_output_nonzero_origin: bool


def default_output_dir() -> Path:
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    return DEFAULT_RUN_ROOT / f"capture-{stamp}-{os.getpid()}"


def absolute_output_dir(path: Path) -> Path:
    return path.expanduser().resolve()


def run_capture_eval(
    config: EvalConfig,
    *,
    cli_runner: CliRunner = run_cli,
    input_reader: InputReader = input,
    message_writer: MessageWriter = print,
) -> dict[str, Any]:
    config.output_dir.mkdir(parents=True, exist_ok=False)
    config.output_dir.chmod(0o700)
    evidence_path = config.output_dir / "evidence.json"
    evidence: dict[str, Any] = {
        "type": "seatgeist_retained_capture_eval",
        "version": 1,
        "status": "running",
        "acceptance_complete": False,
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "target_window_sha256": hashed_window_id(config.window_id),
        "socket_identity": socket_identity(config.socket),
        "selected_scenarios": [scenario.name for scenario in config.scenarios],
        "all_scenarios_selected": config.scenarios
        == selected_scenarios(None),
        "portal_open_count": 0,
        "explicit_focus_call_count": 0,
        "session": None,
        "monitor_layout": None,
        "layout_requirement": (
            "multi_output_nonzero_origin"
            if config.require_multi_output_nonzero_origin
            else "none"
        ),
        "initial_frame": None,
        "scenarios": [],
        "session_closed": False,
        "errors": [],
    }
    session_id: str | None = None
    fatal_error: str | None = None
    try:
        evidence["monitor_layout"] = sanitized_monitor_layout(
            cli_runner(config.cli, config.socket, "monitors"),
            require_multi_output_nonzero_origin=(
                config.require_multi_output_nonzero_origin
            ),
        )
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
            str(max(config.timeout_ms, 120_000)),
        )
        evidence["portal_open_count"] = 1
        session_id = session_id_for_cleanup(opened)
        session = validate_open_session(opened, config.window_id)
        validated_session_id = session.pop("session_id")
        if session_id != validated_session_id:
            raise EvalError("capture session id changed during open validation")
        evidence["session"] = session

        initial_output = config.output_dir / "initial.png"
        initial_response = cli_runner(
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
            str(config.timeout_ms),
        )
        initial_frame = validate_frame(
            initial_response,
            response_type="capture_frame",
            session_id=session_id,
            artifact_root=config.output_dir,
            expected_output=initial_output,
            max_edge=config.max_edge,
        )
        evidence["initial_frame"] = initial_frame
        previous_revision = initial_frame["revision"]

        for index, scenario in enumerate(config.scenarios, start=1):
            ready = input_reader(
                f"[{index}/{len(config.scenarios)}] {scenario.instruction} "
                "Press Enter to sample, or type skip: "
            )
            if ready.strip().lower() in {"s", "skip"}:
                evidence["scenarios"].append(
                    {
                        "name": scenario.name,
                        "visual_verdict": "skip",
                        "frame": None,
                    }
                )
                continue

            output = config.output_dir / f"{index:02d}-{scenario.name}.png"
            response = cli_runner(
                config.cli,
                config.socket,
                "capture",
                "wait",
                "--session-id",
                session_id,
                "--after-revision",
                previous_revision,
                "--output",
                str(output),
                "--max-edge",
                str(config.max_edge),
                "--timeout-ms",
                str(config.timeout_ms),
            )
            frame = validate_frame(
                response,
                response_type="capture_wait",
                session_id=session_id,
                artifact_root=config.output_dir,
                expected_output=output,
                max_edge=config.max_edge,
                after_revision=previous_revision,
            )
            previous_revision = frame["revision"]
            message_writer(f"Scenario artifact: {frame['artifact']['path']}")
            verdict = normalize_visual_verdict(
                input_reader(
                    "Does this image show only the approved target and the expected UI "
                    "for this scenario? [yes/no/skip]: "
                )
            )
            evidence["scenarios"].append(
                {"name": scenario.name, "visual_verdict": verdict, "frame": frame}
            )
    except (EvalError, EOFError, KeyboardInterrupt) as err:
        if isinstance(err, KeyboardInterrupt):
            err = EvalError("operator interrupted retained-capture evaluation")
        fatal_error = str(err)
        evidence["errors"].append(fatal_error)
    finally:
        if session_id is not None:
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
                evidence["session_closed"] = closed_data.get("active") is False
                if not evidence["session_closed"]:
                    evidence["errors"].append("capture close left the session active")
            except EvalError as err:
                evidence["errors"].append(f"capture close failed: {err}")

        evidence["acceptance_complete"] = capture_acceptance_complete(
            evidence["scenarios"], config.scenarios
        )
        if fatal_error is not None or evidence["errors"]:
            evidence["status"] = "failed"
        elif evidence["acceptance_complete"]:
            evidence["status"] = "passed"
        elif any(
            scenario.get("visual_verdict") in {"fail", "skip"}
            or not isinstance(scenario.get("frame"), dict)
            or scenario["frame"].get("fresh_frame") is not True
            for scenario in evidence["scenarios"]
        ):
            evidence["status"] = "incomplete"
        else:
            evidence["status"] = "partial"
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)

    return evidence


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Run the opt-in retained-window capture scenario matrix. This opens a portal "
            "chooser and writes bounded PNG evidence."
        )
    )
    parser.add_argument("--window-id", required=True, help="Exact KWin target window id.")
    parser.add_argument("--cli", type=Path, default=DEFAULT_CLI)
    parser.add_argument("--socket", type=Path)
    parser.add_argument("--output-dir", type=Path, default=default_output_dir())
    parser.add_argument(
        "--scenario",
        action="append",
        help="Run only this named scenario; repeat to preserve a specific order.",
    )
    parser.add_argument("--max-edge", type=int, default=1200)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument(
        "--require-multi-output-nonzero-origin", action="store_true"
    )
    args = parser.parse_args()
    if not 1 <= args.max_edge <= 2048:
        parser.error("--max-edge must be between 1 and 2048")
    if not 1 <= args.timeout_ms <= 30_000:
        parser.error("--timeout-ms must be between 1 and 30000")
    try:
        scenarios = selected_scenarios(args.scenario)
        output_dir = absolute_output_dir(args.output_dir)
        evidence = run_capture_eval(
            EvalConfig(
                window_id=args.window_id,
                cli=args.cli,
                socket=args.socket,
                output_dir=output_dir,
                scenarios=scenarios,
                max_edge=args.max_edge,
                timeout_ms=args.timeout_ms,
                require_multi_output_nonzero_origin=(
                    args.require_multi_output_nonzero_origin
                ),
            )
        )
    except EvalError as err:
        raise SystemExit(f"retained-capture-eval: {err}") from err
    print(
        f"retained-capture-eval: status={evidence['status']} "
        f"evidence={output_dir / 'evidence.json'}"
    )
    if evidence["status"] in {"failed", "incomplete"}:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
