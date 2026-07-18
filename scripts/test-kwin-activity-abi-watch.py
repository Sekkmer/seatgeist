#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
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

    print("test-kwin-activity-abi-watch: ok")


if __name__ == "__main__":
    main()
