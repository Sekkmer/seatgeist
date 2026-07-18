#!/usr/bin/env python3
from __future__ import annotations

import os
from pathlib import Path

from computer_use_eval import ROOT, unix_time_ms, workspace_revision, write_private_json
from nested_seatgeist_probe import probe_nested_daemon


def main() -> None:
    runtime = Path(os.environ["XDG_RUNTIME_DIR"])
    state = Path(os.environ["XDG_STATE_HOME"])
    logs = runtime.parent / "logs"
    evidence_path = state / "nested-seatgeist-probe.json"
    evidence = {
        "type": "seatgeist_nested_daemon_probe",
        "version": 1,
        "status": "running",
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "monitors": None,
        "bridge": None,
        "errors": [],
    }
    try:
        monitors, bridge = probe_nested_daemon(
            ROOT / "target/debug/seatgeistd",
            ROOT / "target/debug/seatgeist-cli",
            runtime,
            state,
            logs,
        )
        evidence["monitors"] = monitors
        evidence["bridge"] = bridge
        evidence["status"] = "passed"
    except Exception as err:
        evidence["status"] = "failed"
        evidence["errors"].append(str(err))
    finally:
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)
    print(
        f"nested-seatgeist-probe: status={evidence['status']} evidence={evidence_path}"
    )
    if evidence["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
