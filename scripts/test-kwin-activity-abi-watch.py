#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/kwin-activity-abi-watch.py"


def load_module():
    spec = importlib.util.spec_from_file_location("kwin_activity_abi_watch", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def write_fixtures(root: Path, header_abi: str, plugin_abi: str) -> tuple[Path, Path]:
    header = root / "config-kwin.h"
    plugin = root / "seatgeistactivity.so"
    header.write_text(
        f'#define KWIN_PLUGIN_VERSION_STRING "{header_abi}"\n', encoding="ascii"
    )
    plugin.write_bytes(
        f"binary org.kde.kwin.PluginFactoryInterface{plugin_abi} fixture".encode(
            "ascii"
        )
    )
    return header, plugin


def check_runtime_regressions(module, root: Path) -> None:
    header, plugin = write_fixtures(root, "6.8.0", "6.8.0")
    agent = root / "agent.so"
    agent.write_bytes(b"org.kde.kwin.PluginFactoryInterface6.8.0")
    report = module.inspect_abis(header, plugin, agent)
    support = {"type": "s", "data": ["KWin version: 6.8.0\n"]}
    loaded = {"type": "as", "data": ["seatgeistactivity", "seatgeistagentseat"]}
    owner = {"type": "b", "data": [True]}
    safety = {"type": "safety_status", "data": {
        "human_input_activity_trusted": True,
        "human_input_activity_backend": "kwin_input_spy_v2",
    }}
    inputs = {"type": "input_backend_status", "data": {
        "configured_backend": "kwin_agent_seat",
        "implemented_available_backend": "kwin_agent_seat",
    }}

    def inspect(responses):
        with mock.patch.object(module, "probe_json", side_effect=responses) as probe:
            result = module.inspect_runtime(report, Path("/fixture/cli"))
            commands = [call.args[0] for call in probe.call_args_list]
        assert all("--auto-start=no" in command for command in commands if command[0] == "/usr/bin/busctl")
        assert not any("LoadPlugin" in command or "restart" in command for command in commands)
        return result, commands

    ready, commands = inspect([support, loaded, owner, safety, inputs])
    assert ready.status == "ready"
    assert ready.activity_trusted is True
    assert len(commands) == 5

    # Rebuilt on-disk plugins do not imply they are loaded in this session.
    missing, commands = inspect([support, {"type": "as", "data": []}])
    assert missing.status == "plugins_not_loaded"
    assert len(commands) == 2  # no CLI call / daemon activation
    stale, commands = inspect([{"type": "s", "data": ["KWin version: 6.7.0\n"]}])
    assert stale.status == "session_restart_required"
    assert len(commands) == 1
    absent, commands = inspect([support, loaded, {"type": "b", "data": [False]}])
    assert absent.status == "daemon_unavailable"
    assert len(commands) == 3

    untrusted = {"type": "safety_status", "data": {
        "human_input_activity_trusted": False,
        "human_input_activity_backend": None,
    }}
    assert inspect([support, loaded, owner, untrusted, inputs])[0].status == "activity_unregistered"
    unavailable = {"type": "input_backend_status", "data": {
        "configured_backend": "kwin_agent_seat", "implemented_available_backend": None,
    }}
    assert inspect([support, loaded, owner, safety, unavailable])[0].status == "agent_seat_unavailable"
    generic_unavailable = {"type": "input_backend_status", "data": {
        "configured_backend": "portal", "implemented_available_backend": None,
    }}
    assert inspect([support, loaded, owner, safety, generic_unavailable])[0].status == "input_backend_unavailable"

    # Fail closed on schema drift, wrong types and timeouts. Never report ready.
    for responses in [
        [module.ProbeError("timeout")],
        [{"type": "s", "data": ["no version"]}],
        [support, {"type": "as", "data": "seatgeistactivity"}],
        [support, loaded, {"type": "b", "data": [1]}],
        [support, loaded, owner, {"type": "unexpected", "data": {}}],
        [support, loaded, owner, {"type": "safety_status", "data": {}}, inputs],
        [support, loaded, owner, {"type": "safety_status", "data": {
            "human_input_activity_trusted": True, "human_input_activity_backend": [],
        }}, inputs],
        [support, loaded, owner, safety, {"type": "input_backend_status", "data": {
            "configured_backend": "", "implemented_available_backend": "",
        }}],
    ]:
        assert inspect(responses)[0].status == "unknown"
    with mock.patch.object(module.subprocess, "run", side_effect=subprocess.TimeoutExpired("probe", 2)) as run:
        try:
            module.probe_json(["probe"])
            assert False, "timeout must not become a success"
        except module.ProbeError:
            pass
        assert run.call_args.kwargs["timeout"] == 2.0
    for invalid_json in ["garbage", "[]", "null"]:
        with mock.patch.object(module.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, invalid_json)):
            try:
                module.probe_json(["probe"])
                assert False, "invalid JSON must fail closed"
            except module.ProbeError:
                pass

    state = root / "runtime-state.json"
    state.write_text('{"sentinel":true}')
    with mock.patch.object(module, "inspect_runtime", return_value=missing), mock.patch.object(module, "send_notification") as notify:
        result = module.run_check(header, plugin, agent, state, Path("/fake/notify"), check_only=True, runtime_check=True)
        assert result["status"] == "current"
        assert result["runtime_ready"] is False
        assert state.read_text() == '{"sentinel":true}'
        notify.assert_not_called()

    with mock.patch.object(module, "inspect_runtime", return_value=missing), mock.patch.object(module, "send_notification", return_value=(True, None)) as notify:
        first = module.run_check(header, plugin, agent, state, Path("/fake/notify"), check_only=False, runtime_check=True, boot_id="runtime-boot")
        second = module.run_check(header, plugin, agent, state, Path("/fake/notify"), check_only=False, runtime_check=True, boot_id="runtime-boot")
        assert first["notification_sent"] is True
        assert second["notification_suppressed"] is True
        assert notify.call_count == 1
        assert "but session status" in module.notification_text(report, missing)[1]
    stored = json.loads(state.read_text())
    assert stored["report"]["runtime"]["status"] == "plugins_not_loaded"
    assert stored["report"]["runtime_ready"] is False
    with mock.patch.object(module, "inspect_runtime", return_value=ready):
        recovered = module.run_check(header, plugin, agent, state, Path("/fake/notify"), check_only=False, runtime_check=True, boot_id="runtime-boot")
    assert recovered["runtime_ready"] is True
    assert json.loads(state.read_text())["notified"] is False
    # Missing runtime evidence must not reuse a previous healthy snapshot.
    with mock.patch.object(module, "inspect_runtime", return_value=module.RuntimeReport("unknown")), mock.patch.object(module, "send_notification", return_value=(False, "timeout")):
        unknown = module.run_check(header, plugin, agent, state, Path("/fake/notify"), check_only=False, runtime_check=True, boot_id="runtime-boot")
    assert unknown["runtime_ready"] is False
    assert json.loads(state.read_text())["report"]["runtime_ready"] is False

    state.write_bytes(b"\xff")
    assert module.read_state(state) == {}
    for value in ["nan", "inf", "0", "4"]:
        result = subprocess.run([str(SCRIPT), "--check-only", "--notification-timeout-seconds", value], capture_output=True)
        assert result.returncode == 2


def main() -> None:
    module = load_module()
    with tempfile.TemporaryDirectory(prefix="seatgeist-kwin-abi-") as temporary:
        root = Path(temporary)
        header, plugin = write_fixtures(root, "6.7.2", "6.7.2")
        report = module.inspect_abis(header, plugin)
        assert report.status == "current"
        assert report.required_abi == "6.7.2"

        header, plugin = write_fixtures(root, "6.8.0", "6.7.2")
        report = module.inspect_abis(header, plugin)
        assert report.status == "rebuild_required"

        header, plugin = write_fixtures(root, "6.8.0", "6.8.0")
        agent_seat = root / "seatgeistagentseat.so"
        agent_seat.write_bytes(
            b"binary org.kde.kwin.PluginFactoryInterface6.7.2 fixture"
        )
        report = module.inspect_abis(header, plugin, agent_seat)
        assert report.status == "rebuild_required"
        assert report.plugin_abi == "6.8.0"
        assert report.plugins[1].name == "agent-seat"
        assert report.plugins[1].status == "rebuild_required"
        assert "agent-seat ABI 6.7.2" in module.notification_text(report)[1]
        agent_seat.unlink()
        report = module.inspect_abis(header, plugin, agent_seat)
        assert report.status == "current"
        assert report.plugins[1].status == "not_installed"

        plugin.unlink()
        assert module.inspect_abis(header, plugin).status == "missing_plugin"
        plugin.write_bytes(b"not a KWin plugin")
        assert module.inspect_abis(header, plugin).status == "invalid_plugin"

        header.unlink()
        assert module.inspect_abis(header, plugin).status == "missing_header"

        header, plugin = write_fixtures(root, "6.8.0", "6.7.2")
        state = root / "state.json"
        fake_notify = root / "notify-send"
        fake_notify.write_text("#!/bin/sh\nexit 0\n", encoding="ascii")
        fake_notify.chmod(0o755)
        first = module.run_check(
            header,
            plugin,
            None,
            state,
            fake_notify,
            check_only=False,
            boot_id="test-boot",
        )
        assert first["notification_sent"] is True
        second = module.run_check(
            header,
            plugin,
            None,
            state,
            fake_notify,
            check_only=False,
            boot_id="test-boot",
        )
        assert second["notification_sent"] is False
        assert second["notification_suppressed"] is True
        assert state.stat().st_mode & 0o777 == 0o600
        stored = json.loads(state.read_text(encoding="utf-8"))
        assert stored["fingerprint"]["status"] == "rebuild_required"
        assert stored["last_notification_failure"] is None

        # A package upgrade changes the watched KWin header while the graphical
        # session is still running. The next path-triggered check must notice
        # the mismatch before logout or reboot.
        header, plugin = write_fixtures(root, "6.8.0", "6.8.0")
        upgrade_state = root / "upgrade-state.json"
        current = module.run_check(
            header,
            plugin,
            None,
            upgrade_state,
            fake_notify,
            check_only=False,
            boot_id="upgrade-boot",
        )
        assert current["status"] == "current"
        header.write_text(
            '#define KWIN_PLUGIN_VERSION_STRING "6.9.0"\n', encoding="ascii"
        )
        upgraded = module.run_check(
            header,
            plugin,
            None,
            upgrade_state,
            fake_notify,
            check_only=False,
            boot_id="upgrade-boot",
        )
        assert upgraded["status"] == "rebuild_required"
        assert upgraded["notification_sent"] is True

        # notify-send's expiry flag does not bound the process itself. A hung
        # notification service must be terminated promptly and recorded in
        # coherent durable state so the periodic checker can retry.
        hanging_notify = root / "hanging-notify-send"
        hanging_notify.write_text("#!/bin/sh\nexec sleep 10\n", encoding="ascii")
        hanging_notify.chmod(0o755)
        timeout_state = root / "timeout-state.json"
        started = time.monotonic()
        timed_out = module.run_check(
            header,
            plugin,
            None,
            timeout_state,
            hanging_notify,
            check_only=False,
            boot_id="timeout-boot",
            notification_timeout_seconds=0.1,
        )
        elapsed = time.monotonic() - started
        assert elapsed < 1.0, elapsed
        assert timed_out["notification_sent"] is False
        assert timed_out["notification_failure"] == "timeout"
        timeout_stored = json.loads(timeout_state.read_text(encoding="utf-8"))
        assert timeout_stored["fingerprint"]["status"] == "rebuild_required"
        assert timeout_stored["notified"] is False
        assert timeout_stored["last_notification_failure"] == "timeout"
        assert isinstance(timeout_stored["checked_at_unix_ms"], int)

        failing_notify = root / "failing-notify-send"
        failing_notify.write_text("#!/bin/sh\nexit 7\n", encoding="ascii")
        failing_notify.chmod(0o755)
        failure_state = root / "failure-state.json"
        failed = module.run_check(
            header,
            plugin,
            None,
            failure_state,
            failing_notify,
            check_only=False,
            boot_id="failure-boot",
        )
        assert failed["notification_failure"] == "failed"
        failure_stored = json.loads(failure_state.read_text(encoding="utf-8"))
        assert failure_stored["notified"] is False
        assert failure_stored["last_notification_failure"] == "failed"

        check_runtime_regressions(module, root)

    print("test-kwin-activity-abi-watch: ok")


if __name__ == "__main__":
    main()
