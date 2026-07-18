#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/install-kwin-activity-user.py"


def load_module():
    spec = importlib.util.spec_from_file_location("install_kwin_activity_user", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> None:
    module = load_module()
    with tempfile.TemporaryDirectory(prefix="seatgeist-kwin-user-install-") as temporary:
        root = Path(temporary)
        artifact = root / "activity.so"
        artifact.write_bytes(b"plugin fixture")
        plugin_root = root / "plugins"
        drop_in = root / "drop-in.conf"
        watcher = root / "libexec/abi-watch"
        unit_dir = root / "units"
        state = root / "state.json"
        completed = subprocess.run(
            [
                str(SCRIPT),
                "--artifact",
                str(artifact),
                "--plugin-root",
                str(plugin_root),
                "--drop-in",
                str(drop_in),
                "--watcher",
                str(watcher),
                "--unit-dir",
                str(unit_dir),
                "--state",
                str(state),
                "--no-daemon-reload",
                "--no-systemd-management",
            ],
            text=True,
            check=True,
            stdout=subprocess.PIPE,
        )
        assert '"action": "installed"' in completed.stdout
        installed = plugin_root / "kwin/plugins/seatgeistactivity.so"
        assert installed.read_bytes() == b"plugin fixture"
        assert installed.stat().st_mode & 0o777 == 0o755
        assert drop_in.read_text(encoding="utf-8") == module.render_drop_in(plugin_root)
        assert drop_in.stat().st_mode & 0o777 == 0o644
        assert watcher.read_bytes() == module.DEFAULT_WATCHER_SOURCE.read_bytes()
        assert watcher.stat().st_mode & 0o777 == 0o755
        service = unit_dir / module.SERVICE_NAME
        path_unit = unit_dir / module.PATH_NAME
        service_template = module.DEFAULT_UNIT_SOURCE_DIR.joinpath(
            f"{module.SERVICE_NAME}.in"
        ).read_text(encoding="utf-8")
        assert service.read_text(encoding="utf-8") == module.render_service(
            service_template, watcher
        )
        assert path_unit.read_bytes() == module.DEFAULT_UNIT_SOURCE_DIR.joinpath(
            module.PATH_NAME
        ).read_bytes()

        subprocess.run(
            [
                str(SCRIPT),
                "--plugin-root",
                str(plugin_root),
                "--drop-in",
                str(drop_in),
                "--watcher",
                str(watcher),
                "--unit-dir",
                str(unit_dir),
                "--state",
                str(state),
                "--remove",
                "--no-daemon-reload",
                "--no-systemd-management",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        assert not installed.exists()
        assert not drop_in.exists()
        assert not watcher.exists()
        assert not service.exists()
        assert not path_unit.exists()
    print("test-install-kwin-activity-user: ok")


if __name__ == "__main__":
    main()
