#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import subprocess
import tempfile
from pathlib import Path
from unittest import mock


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
        timer = unit_dir / module.TIMER_NAME
        assert timer.read_bytes() == module.DEFAULT_UNIT_SOURCE_DIR.joinpath(
            module.TIMER_NAME
        ).read_bytes()

        service_text = service.read_text(encoding="utf-8")
        path_text = path_unit.read_text(encoding="utf-8")
        timer_text = timer.read_text(encoding="utf-8")
        assert "[Install]" not in service_text
        assert "plasma-core.target" not in service_text
        assert "plasma-workspace.target" not in service_text
        assert "WantedBy=graphical-session.target" in path_text
        assert f"Unit={module.SERVICE_NAME}" in path_text
        assert "WantedBy=graphical-session.target" in timer_text
        assert f"Unit={module.SERVICE_NAME}" in timer_text
        assert " --runtime" in service_text
        assert "TimeoutStartSec=18s" in service_text
        assert "NoNewPrivileges=yes" in service_text

        # Reinstalling the same artifacts is harmless and yields the same
        # exact user-local state.
        subprocess.run(
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
            check=True,
            stdout=subprocess.DEVNULL,
        )
        assert installed.read_bytes() == b"plugin fixture"

        with mock.patch.object(module.subprocess, "run") as systemctl_run:
            systemctl_run.return_value.returncode = 0
            module.manage_units("install")
            commands = [call.args[0] for call in systemctl_run.call_args_list]
        assert commands == [
            [
                "systemctl",
                "--user",
                "disable",
                module.SERVICE_NAME,
            ],
            [
                "systemctl",
                "--user",
                "enable",
                module.PATH_NAME,
                module.TIMER_NAME,
            ],
        ]
        assert all("--now" not in command for command in commands)

        with mock.patch.object(module.subprocess, "run") as systemctl_run:
            systemctl_run.return_value.returncode = 0
            module.manage_units("install", activate_watchers=True)
            commands = [call.args[0] for call in systemctl_run.call_args_list]
        assert commands[-1] == ["systemctl", "--user", "start", module.PATH_NAME, module.TIMER_NAME]
        assert not any("restart" in command or "plasma-kwin_wayland.service" in command or "seatgeistd.service" in command for command in commands)

        # Diagnostic updates must not replace plugin binaries or KWin config,
        # even if the requested artifact is absent.
        drop_in.write_text("operator configuration\n")
        installed.write_bytes(b"operator plugin")
        subprocess.run([
            str(SCRIPT), "--watcher-only", "--artifact", str(root / "missing.so"),
            "--plugin-root", str(plugin_root), "--drop-in", str(drop_in),
            "--watcher", str(watcher), "--unit-dir", str(unit_dir),
            "--no-daemon-reload", "--no-systemd-management",
        ], check=True, stdout=subprocess.DEVNULL)
        assert drop_in.read_text() == "operator configuration\n"
        assert installed.read_bytes() == b"operator plugin"
        assert watcher.read_bytes() == module.DEFAULT_WATCHER_SOURCE.read_bytes()

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
        assert not timer.exists()

        # Removing an already absent install is also harmless.
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
    print("test-install-kwin-activity-user: ok")


if __name__ == "__main__":
    main()
