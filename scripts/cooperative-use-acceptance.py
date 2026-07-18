#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

from computer_use_eval import EvalError, ROOT, workspace_revision, write_private_json
from cooperative_acceptance_bundle import build_bundle


DEFAULT_OUTPUT = ROOT / "target/seatgeist-cooperative-acceptance/bundle.json"


def positive_hours(value: str) -> float:
    parsed = float(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("must be greater than zero")
    return parsed


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Verify one private, same-workspace evidence bundle for the complete "
            "Seatgeist cooperative-use acceptance gate without controlling the desktop."
        )
    )
    parser.add_argument("--retained-capture", type=Path, required=True)
    parser.add_argument("--retained-capture-multi-output", type=Path, required=True)
    parser.add_argument("--capture-restore-restart", type=Path, required=True)
    parser.add_argument("--capture-revocation", type=Path, required=True)
    parser.add_argument("--target-reopen", type=Path, required=True)
    parser.add_argument("--background-semantic-firefox", type=Path, required=True)
    parser.add_argument("--background-semantic-kde", type=Path, required=True)
    parser.add_argument("--cooperative-sticky", type=Path, required=True)
    parser.add_argument("--max-age-hours", type=positive_hours, default=24.0)
    parser.add_argument("--max-span-hours", type=positive_hours, default=24.0)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    artifacts = {
        "retained_capture": args.retained_capture,
        "retained_capture_multi_output": args.retained_capture_multi_output,
        "capture_restore_restart": args.capture_restore_restart,
        "capture_revocation": args.capture_revocation,
        "target_reopen": args.target_reopen,
        "background_semantic_firefox": args.background_semantic_firefox,
        "background_semantic_kde": args.background_semantic_kde,
        "cooperative_sticky": args.cooperative_sticky,
    }
    try:
        bundle = build_bundle(
            artifacts,
            expected_workspace=workspace_revision(),
            max_age_ms=int(args.max_age_hours * 60 * 60 * 1000),
            max_span_ms=int(args.max_span_hours * 60 * 60 * 1000),
        )
        write_private_json(args.output, bundle)
    except EvalError as err:
        raise SystemExit(f"cooperative-use-acceptance: {err}") from err
    print(
        "cooperative-use-acceptance: all scenarios and budgets passed; "
        f"bundle={args.output}"
    )


if __name__ == "__main__":
    main()
