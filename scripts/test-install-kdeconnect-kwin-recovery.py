#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/install-kdeconnect-kwin-recovery.py"


def load_module():
    spec = importlib.util.spec_from_file_location(
        "install_kdeconnect_kwin_recovery", SCRIPT
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> None:
    module = load_module()
    with tempfile.TemporaryDirectory(prefix="seatgeist-kdeconnect-recovery-") as temporary:
        root = Path(temporary)
        unit_dir = root / "units"
        dbus_dir = root / "dbus"
        common = [
            str(SCRIPT),
            "--unit-dir",
            str(unit_dir),
            "--dbus-dir",
            str(dbus_dir),
            "--no-systemd-management",
        ]
        completed = subprocess.run(
            common,
            text=True,
            check=True,
            stdout=subprocess.PIPE,
        )
        report = json.loads(completed.stdout)
        assert report["action"] == "installed"
        assert report["compositor_restarted"] is False

        paths = module.installed_paths(unit_dir, dbus_dir)
        expected = {
            "recovery_unit": module.ASSET_DIR / module.RECOVERY_UNIT,
            "kdeconnect_drop_in": (
                module.ASSET_DIR / "kdeconnect-autostart-kwin.conf"
            ),
            "kwin_drop_in": (
                module.ASSET_DIR / "kwin-kdeconnect-recovery.conf"
            ),
            "dbus_service": module.DBUS_ASSET,
        }
        for name, path in paths.items():
            assert path.read_bytes() == expected[name].read_bytes()
            assert path.stat().st_mode & 0o777 == 0o644

        subprocess.run(
            [*common, "--remove"],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        assert all(not path.exists() for path in paths.values())
    print("test-install-kdeconnect-kwin-recovery: ok")


if __name__ == "__main__":
    main()
