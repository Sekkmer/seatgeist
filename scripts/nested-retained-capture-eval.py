#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
from pathlib import Path

from computer_use_eval import ROOT, unix_time_ms, workspace_revision, write_private_json
from nested_retained_capture import run_nested_retained_capture


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run the retained multi-output matrix inside the nested KDE fixture."
    )
    parser.add_argument("--probe-only", action="store_true")
    parser.add_argument(
        "--scenario",
        action="append",
        default=[],
        help="Run only this retained-capture scenario; repeat to preserve order.",
    )
    args = parser.parse_args()
    runtime = Path(os.environ["XDG_RUNTIME_DIR"])
    state = Path(os.environ["XDG_STATE_HOME"])
    logs = runtime.parent / "logs"
    evidence_path = state / "nested-retained-workload.json"
    evidence = {
        "type": "seatgeist_nested_retained_workload",
        "version": 1,
        "status": "running",
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "probe_only": args.probe_only,
        "selected_scenarios": args.scenario,
        "result": None,
        "errors": [],
    }
    try:
        evidence["result"] = run_nested_retained_capture(
            ROOT,
            runtime,
            state,
            logs,
            probe_only=args.probe_only,
            scenarios=tuple(args.scenario),
        )
        evidence["status"] = evidence["result"]["status"]
    except (Exception, KeyboardInterrupt) as err:
        evidence["status"] = "failed"
        evidence["errors"].append(
            "operator interrupted nested retained evaluation"
            if isinstance(err, KeyboardInterrupt)
            else str(err)
        )
    finally:
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)
    print(
        f"nested-retained-capture-eval: status={evidence['status']} "
        f"evidence={evidence_path}"
    )
    if evidence["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
