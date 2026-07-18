#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from background_semantic_eval import (
    SCENARIOS,
    click_button_arguments,
    normalize_visual_verdict,
    require_non_target_focus,
    resolve_background_target,
    sanitized_target,
    validate_action,
    validate_approval,
    validate_journal,
)
from computer_use_eval import (
    EvalError,
    ROOT,
    default_approval_file_path,
    run_cli,
    socket_identity,
    unix_time_ms,
    workspace_revision,
    write_private_json,
)


DEFAULT_CLI = ROOT / "target/debug/seatgeist-cli"
DEFAULT_RUN_ROOT = ROOT / "target/seatgeist-background-semantic-eval"

CliRunner = Callable[..., dict[str, Any]]
InputReader = Callable[[str], str]
MessageWriter = Callable[[str], None]


@dataclass(frozen=True)
class BackgroundEvalConfig:
    scenario: str
    target_window_id: str
    user_window_id: str
    button_name: str
    app_filter: str | None
    cli: Path
    socket: Path | None
    approval_file: Path
    output_dir: Path
    approval_ttl_ms: int


def default_output_dir() -> Path:
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    return DEFAULT_RUN_ROOT / f"background-{stamp}-{os.getpid()}"


def redact_error(error: Exception, config: BackgroundEvalConfig) -> str:
    message = str(error)
    for secret, replacement in (
        (config.target_window_id, "<target-window>"),
        (config.user_window_id, "<user-window>"),
        (config.button_name, "<button-name>"),
    ):
        if secret:
            message = message.replace(secret, replacement)
    return message


def run_background_eval(
    config: BackgroundEvalConfig,
    *,
    cli_runner: CliRunner = run_cli,
    input_reader: InputReader = input,
    message_writer: MessageWriter = print,
) -> dict[str, Any]:
    config.output_dir.mkdir(parents=True, exist_ok=False)
    config.output_dir.chmod(0o700)
    evidence_path = config.output_dir / "evidence.json"
    start_unix_ms = unix_time_ms()
    evidence: dict[str, Any] = {
        "type": "seatgeist_background_semantic_eval",
        "version": 2,
        "status": "running",
        "acceptance_complete": False,
        "workspace": workspace_revision(),
        "scenario": config.scenario,
        "started_unix_ms": start_unix_ms,
        "ended_unix_ms": None,
        "socket_identity": socket_identity(config.socket),
        "target": None,
        "non_target_focus_before": False,
        "non_target_focus_after": False,
        "user_window_changed_during_action": False,
        "approval": None,
        "semantic_action_succeeded": False,
        "visual_change_confirmed": False,
        "operator_target_never_focused_confirmed": False,
        "journal_match_count": 0,
        "explicit_focus_call_count": 0,
        "raw_input_call_count": 0,
        "daemon_request_count": 0,
        "errors": [],
    }
    stage = "preflight"
    try:
        if evidence["socket_identity"] is None:
            raise EvalError("daemon socket identity is unavailable")
        windows = cli_runner(config.cli, config.socket, "windows")
        evidence["daemon_request_count"] += 1
        target = resolve_background_target(
            windows,
            target_window_id=config.target_window_id,
            user_window_id=config.user_window_id,
            scenario=config.scenario,
        )
        evidence["target"] = sanitized_target(target)

        active_before = require_non_target_focus(
            cli_runner(config.cli, config.socket, "active-window"), target
        )
        evidence["daemon_request_count"] += 1
        evidence["non_target_focus_before"] = True

        stage = "approval"
        approval = cli_runner(
            config.cli,
            config.socket,
            "approve",
            "--approval-file",
            str(config.approval_file),
            "--safety-class",
            "control-semantic",
            "--method",
            "click_button",
            "--ttl-ms",
            str(config.approval_ttl_ms),
            "--reason",
            f"background-semantic-eval {config.scenario}",
        )
        validate_approval(approval)
        evidence["approval"] = {
            "safety_class": "control_semantic",
            "method": "click_button",
            "expires_unix_ms": approval.get("expires_unix_ms"),
        }

        stage = "semantic_action"
        action = cli_runner(
            config.cli,
            config.socket,
            *click_button_arguments(target, config.button_name, config.app_filter),
        )
        evidence["daemon_request_count"] += 1
        validate_action(action)
        evidence["semantic_action_succeeded"] = True

        stage = "focus_verification"
        active_after = require_non_target_focus(
            cli_runner(config.cli, config.socket, "active-window"), target
        )
        evidence["daemon_request_count"] += 1
        evidence["non_target_focus_after"] = True
        evidence["user_window_changed_during_action"] = (
            active_after != active_before
        )

        stage = "operator_verdict"
        message_writer(
            "The semantic action completed without making the background target active."
        )
        operator_confirmed = normalize_visual_verdict(
            input_reader(
                "Did the background target perform the intended safe action without the "
                "target receiving focus? [yes/no]: "
            )
        )
        evidence["visual_change_confirmed"] = operator_confirmed
        evidence["operator_target_never_focused_confirmed"] = operator_confirmed
        if not operator_confirmed:
            raise EvalError(
                "operator did not confirm the background change and non-target focus"
            )

        stage = "journal_verification"
        journal = cli_runner(
            config.cli,
            config.socket,
            "journal",
            "tail",
            "--limit",
            "50",
            "--method",
            "click_button",
            "--ok",
            "true",
        )
        evidence["daemon_request_count"] += 1
        evidence["journal_match_count"] = validate_journal(
            journal, target, start_unix_ms
        )
        evidence["acceptance_complete"] = True
        evidence["status"] = "passed"
    except (EvalError, EOFError, KeyboardInterrupt) as err:
        if isinstance(err, KeyboardInterrupt):
            err = EvalError("operator interrupted background semantic evaluation")
        evidence["errors"].append(f"{stage}: {redact_error(err, config)}")
        evidence["status"] = "failed"
    finally:
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)
    return evidence


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Run one opt-in target-guarded background click_button eval. This performs "
            "a real semantic action but makes no focus, raw-input, or screenshot call."
        )
    )
    parser.add_argument("--scenario", choices=SCENARIOS, required=True)
    parser.add_argument("--target-window-id", required=True)
    parser.add_argument("--user-window-id", required=True)
    parser.add_argument("--button-name", required=True)
    parser.add_argument("--app-filter")
    parser.add_argument("--cli", type=Path, default=DEFAULT_CLI)
    parser.add_argument("--socket", type=Path)
    parser.add_argument(
        "--approval-file", type=Path, default=default_approval_file_path()
    )
    parser.add_argument("--approval-ttl-ms", type=int, default=60_000)
    parser.add_argument("--output-dir", type=Path, default=default_output_dir())
    args = parser.parse_args()
    if not 10_000 <= args.approval_ttl_ms <= 300_000:
        parser.error("--approval-ttl-ms must be between 10000 and 300000")

    evidence = run_background_eval(
        BackgroundEvalConfig(
            scenario=args.scenario,
            target_window_id=args.target_window_id,
            user_window_id=args.user_window_id,
            button_name=args.button_name,
            app_filter=args.app_filter,
            cli=args.cli,
            socket=args.socket,
            approval_file=args.approval_file,
            output_dir=args.output_dir,
            approval_ttl_ms=args.approval_ttl_ms,
        )
    )
    print(
        f"background-semantic-eval: status={evidence['status']} "
        f"evidence={args.output_dir / 'evidence.json'}"
    )
    if evidence["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
