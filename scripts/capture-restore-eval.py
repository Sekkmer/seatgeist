#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import time
from pathlib import Path

from capture_restore_phases import (
    RestoreEvalConfig,
    prepare_restore_eval,
    resume_restore_eval,
)
from computer_use_eval import EvalError, ROOT, default_capture_restore_path


DEFAULT_CLI = ROOT / "target/debug/seatgeist-cli"
DEFAULT_RUN_ROOT = ROOT / "target/seatgeist-capture-restore-eval"


def default_output_dir() -> Path:
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    return DEFAULT_RUN_ROOT / f"restart-{stamp}-{os.getpid()}"


def add_common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--window-id", required=True, help="Exact KWin target window id.")
    parser.add_argument("--cli", type=Path, default=DEFAULT_CLI)
    parser.add_argument("--socket", type=Path)
    parser.add_argument(
        "--restore-file", type=Path, default=default_capture_restore_path()
    )
    parser.add_argument("--max-edge", type=int, default=1200)
    parser.add_argument("--timeout-ms", type=int, default=8000)


def checked_config(args: argparse.Namespace, output_dir: Path) -> RestoreEvalConfig:
    if not 1 <= args.max_edge <= 2048:
        raise EvalError("--max-edge must be between 1 and 2048")
    if not 1 <= args.timeout_ms <= 30_000:
        raise EvalError("--timeout-ms must be between 1 and 30000")
    return RestoreEvalConfig(
        window_id=args.window_id,
        cli=args.cli,
        socket=args.socket,
        restore_file=args.restore_file,
        output_dir=output_dir,
        max_edge=args.max_edge,
        timeout_ms=args.timeout_ms,
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Run the opt-in two-phase ScreenCast restore-token restart eval. Both "
            "phases capture real window pixels; the operator restarts the daemon."
        )
    )
    commands = parser.add_subparsers(dest="phase", required=True)
    prepare = commands.add_parser("prepare")
    add_common_arguments(prepare)
    prepare.add_argument("--output-dir", type=Path, default=default_output_dir())
    resume = commands.add_parser("resume")
    add_common_arguments(resume)
    resume.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()

    try:
        config = checked_config(args, args.output_dir)
        if args.phase == "prepare":
            evidence = prepare_restore_eval(config)
        else:
            evidence = resume_restore_eval(config)
    except EvalError as err:
        raise SystemExit(f"capture-restore-eval: {err}") from err

    print(
        f"capture-restore-eval: phase={args.phase} status={evidence['status']} "
        f"evidence={args.output_dir / 'evidence.json'}"
    )
    if evidence["status"] == "failed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
