#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from computer_use_eval import EvalError, response_data, run_cli


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CLI = ROOT / "target/debug/seatgeist-cli"
DEFAULT_JOURNAL = Path.home() / ".local/state/seatgeist/journal.jsonl"
DEFAULT_OUTPUT = ROOT / "target/seatgeist-computer-use-baseline/firefox-sticky-live.json"


def require_activity_backend(response: dict[str, Any]) -> None:
    data = response_data(response, "safety_status")
    if data.get("human_input_activity_backend") != "kwin_input_spy_v1":
        raise EvalError("kwin_input_spy_v1 is not registered")
    if data.get("human_input_activity_trusted") is not True:
        raise EvalError("KWin input activity provenance is not trusted")


def session_id_from_open(response: dict[str, Any], window_id: str) -> str:
    data = response_data(response, "capture_session_status")
    session_id = data.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        raise EvalError("capture open returned no session id")
    if data.get("sticky_target_bound") is not True:
        raise EvalError("capture session did not bind a sticky target")
    if data.get("target_window_id") != window_id:
        raise EvalError("capture session bound a different target window")
    return session_id


def require_restored_action(response: dict[str, Any], iteration: int) -> None:
    data = response_data(response, "action")
    if data.get("ok") is not True:
        raise EvalError(f"iteration {iteration} action was not successful")
    message = data.get("message")
    if not isinstance(message, str) or "focus_reacquired=true" not in message:
        raise EvalError(f"iteration {iteration} did not reacquire the sticky target")
    if "focus_restored=true" not in message or "restoration=restored" not in message:
        raise EvalError(f"iteration {iteration} did not restore user focus: {message}")


def approval_ttl_ms(iterations: int, quiet_ms: int) -> int:
    return max(60_000, iterations * (quiet_ms + 3_000) + 30_000)


def grant_method_approval(
    cli: Path,
    socket: Path | None,
    safety_class: str,
    method: str,
    ttl_ms: int,
) -> dict[str, Any]:
    grant = run_cli(
        cli,
        socket,
        "approve",
        "--safety-class",
        safety_class,
        "--method",
        method,
        "--ttl-ms",
        str(ttl_ms),
        "--reason",
        f"cooperative-sticky live acceptance {method}",
    )
    if grant.get("safety_class") != safety_class.replace("-", "_"):
        raise EvalError("cooperative sticky approval has the wrong safety class")
    if grant.get("method") != method:
        raise EvalError("cooperative sticky approval has the wrong method")
    if not isinstance(grant.get("expires_unix_ms"), int):
        raise EvalError("cooperative sticky approval has no expiry")
    return grant


def grant_sticky_approvals(
    cli: Path,
    socket: Path | None,
    iterations: int,
    quiet_ms: int,
) -> tuple[dict[str, Any], dict[str, Any]]:
    ttl_ms = approval_ttl_ms(iterations, quiet_ms)
    return (
        grant_method_approval(
            cli, socket, "control-semantic", "focus_window", ttl_ms
        ),
        grant_method_approval(
            cli, socket, "control-keyboard", "key_combo", ttl_ms
        ),
    )


def analyze(
    journal: Path,
    output: Path,
    start_unix_ms: int,
    end_unix_ms: int,
) -> dict[str, Any]:
    command = [
        str(ROOT / "scripts/computer-use-baseline.py"),
        "--journal",
        str(journal),
        "--scenario",
        "firefox-sticky-live",
        "--client-tool",
        "seatgeist-cli",
        "--start-unix-ms",
        str(start_unix_ms),
        "--end-unix-ms",
        str(end_unix_ms),
        "--output",
        str(output),
    ]
    completed = subprocess.run(command, cwd=ROOT, text=True, check=False)
    if completed.returncode != 0:
        raise EvalError("computer-use baseline analyzer failed")
    return json.loads(output.read_text(encoding="utf-8"))


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run the opt-in 20-action cooperative sticky-target acceptance eval."
    )
    parser.add_argument("--window-id", required=True, help="Exact Firefox KWin window id.")
    parser.add_argument("--socket", type=Path)
    parser.add_argument("--journal", type=Path, default=DEFAULT_JOURNAL)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--cli", type=Path, default=DEFAULT_CLI)
    parser.add_argument("--iterations", type=int, default=20)
    parser.add_argument("--quiet-ms", type=int, default=2000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")
    if args.quiet_ms < 0:
        parser.error("--quiet-ms must not be negative")

    grant_sticky_approvals(args.cli, args.socket, args.iterations, args.quiet_ms)
    start_unix_ms = int(time.time() * 1000)
    session_id: str | None = None
    try:
        require_activity_backend(run_cli(args.cli, args.socket, "safety-status"))
        print("Select the requested Firefox window in the portal chooser.")
        session_id = session_id_from_open(
            run_cli(
                args.cli,
                args.socket,
                "capture",
                "open",
                "--requested-window-id",
                args.window_id,
            ),
            args.window_id,
        )
        for iteration in range(1, args.iterations + 1):
            input(
                f"[{iteration}/{args.iterations}] Keep this terminal as your work window, "
                "do any desired typing or pointer activity, then press Enter: "
            )
            time.sleep(args.quiet_ms / 1000)
            response = run_cli(
                args.cli,
                args.socket,
                "input",
                "key-combo",
                "Shift",
                "--session-id",
                session_id,
            )
            require_restored_action(response, iteration)
            print(f"iteration {iteration}: target reacquired and user focus restored")
    except (EvalError, EOFError, KeyboardInterrupt) as err:
        if isinstance(err, KeyboardInterrupt):
            err = EvalError("operator interrupted cooperative sticky evaluation")
        raise SystemExit(f"cooperative-sticky-eval: {err}") from err
    finally:
        if session_id is not None:
            try:
                run_cli(
                    args.cli,
                    args.socket,
                    "capture",
                    "close",
                    "--session-id",
                    session_id,
                )
            except EvalError as err:
                print(f"warning: failed to close session: {err}", file=sys.stderr)

    end_unix_ms = int(time.time() * 1000)
    report = analyze(args.journal, args.output, start_unix_ms, end_unix_ms)
    budget = report.get("budget", {})
    failed = [name for name, passed in budget.items() if passed is not True]
    if failed:
        raise SystemExit(
            "cooperative-sticky-eval: budget failed: " + ", ".join(sorted(failed))
        )
    print(f"cooperative-sticky-eval: all budgets passed; evidence={args.output}")


if __name__ == "__main__":
    main()
