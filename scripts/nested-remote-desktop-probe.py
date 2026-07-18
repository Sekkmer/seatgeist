#!/usr/bin/env python3
from __future__ import annotations

import os
from pathlib import Path

from computer_use_eval import unix_time_ms, workspace_revision, write_private_json
from nested_remote_desktop_probe import probe_remote_desktop


def main() -> None:
    state = Path(os.environ["XDG_STATE_HOME"])
    runtime = Path(os.environ["XDG_RUNTIME_DIR"])
    display = os.environ.get("WAYLAND_DISPLAY", "")
    evidence_path = state / "nested-remote-desktop-probe.json"
    evidence = {
        "type": "seatgeist_nested_remote_desktop_probe",
        "version": 1,
        "status": "running",
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "isolation": {
            "private_session_bus": os.environ.get(
                "SEATGEIST_NESTED_KDE_PRIVATE_BUS"
            )
            == "1",
            "private_wayland_socket": bool(display)
            and (runtime / display).exists(),
        },
        "remote_desktop": None,
        "input_sent": False,
        "session_created": False,
        "errors": [],
    }
    try:
        if not all(evidence["isolation"].values()):
            raise RuntimeError("nested RemoteDesktop probe is outside fixture isolation")
        evidence["remote_desktop"] = probe_remote_desktop(os.environ)
        evidence["status"] = "passed"
    except Exception as err:
        evidence["status"] = "failed"
        evidence["errors"].append(str(err))
    finally:
        evidence["ended_unix_ms"] = unix_time_ms()
        write_private_json(evidence_path, evidence)
    print(
        "nested-remote-desktop-probe: "
        f"status={evidence['status']} evidence={evidence_path}"
    )
    if evidence["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
