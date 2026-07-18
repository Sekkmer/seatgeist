#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/cooperative-sticky-eval.py"


def load_module():
    spec = importlib.util.spec_from_file_location("cooperative_sticky_eval", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> None:
    module = load_module()
    safety = {
        "type": "safety_status",
        "data": {
            "human_input_activity_backend": "kwin_input_spy_v1",
            "human_input_activity_trusted": True,
        },
    }
    module.require_activity_backend(safety)
    assert module.approval_ttl_ms(20, 2000) == 130_000

    approval_commands = []

    def approve(_cli, _socket, *arguments):
        approval_commands.append(arguments)
        safety_class = arguments[arguments.index("--safety-class") + 1]
        method = arguments[arguments.index("--method") + 1]
        return {
            "safety_class": safety_class.replace("-", "_"),
            "method": method,
            "expires_unix_ms": 123,
        }

    original_run_cli = module.run_cli
    module.run_cli = approve
    try:
        module.grant_sticky_approvals(Path("cli"), None, 20, 2000)
    finally:
        module.run_cli = original_run_cli
    assert len(approval_commands) == 2
    assert approval_commands[0][:3] == (
        "approve",
        "--safety-class",
        "control-semantic",
    )
    assert "focus_window" in approval_commands[0]
    assert approval_commands[1][:3] == (
        "approve",
        "--safety-class",
        "control-keyboard",
    )
    assert "key_combo" in approval_commands[1]

    session = {
        "type": "capture_session_status",
        "data": {
            "session_id": "capture-1",
            "sticky_target_bound": True,
            "target_window_id": "firefox-1",
        },
    }
    assert module.session_id_from_open(session, "firefox-1") == "capture-1"

    action = {
        "type": "action",
        "data": {
            "ok": True,
            "message": (
                "sent session=capture-1 focus_reacquired=true "
                "focus_restored=true restoration=restored"
            ),
        },
    }
    module.require_restored_action(action, 1)

    try:
        module.require_restored_action(
            {"type": "action", "data": {"ok": True, "message": "focus_restored=false"}},
            2,
        )
    except module.EvalError:
        pass
    else:
        raise AssertionError("unrestored action must fail acceptance")

    print("test-cooperative-sticky-eval: ok")


if __name__ == "__main__":
    main()
