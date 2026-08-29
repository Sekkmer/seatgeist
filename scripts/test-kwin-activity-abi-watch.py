#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import time
from pathlib import Path


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
            state,
            fake_notify,
            check_only=False,
            boot_id="test-boot",
        )
        assert first["notification_sent"] is True
        second = module.run_check(
            header,
            plugin,
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
            failure_state,
            failing_notify,
            check_only=False,
            boot_id="failure-boot",
        )
        assert failed["notification_failure"] == "failed"
        failure_stored = json.loads(failure_state.read_text(encoding="utf-8"))
        assert failure_stored["notified"] is False
        assert failure_stored["last_notification_failure"] == "failed"

    print("test-kwin-activity-abi-watch: ok")


if __name__ == "__main__":
    main()
