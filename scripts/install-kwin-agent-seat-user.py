#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARTIFACT = (
    ROOT / "target/kwin-seatgeist-agent-seat/seatgeistagentseat.so"
)
DEFAULT_PLUGIN_ROOT = Path.home() / ".local/lib/qt6/plugins"
DEFAULT_DROP_IN = (
    Path.home()
    / ".config/systemd/user/plasma-kwin_wayland.service.d/51-seatgeist-agent-seat.conf"
)


def atomic_copy(source: Path, destination: Path, mode: int) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as temporary:
        temporary_path = Path(temporary.name)
    try:
        shutil.copyfile(source, temporary_path)
        temporary_path.chmod(mode)
        os.replace(temporary_path, destination)
    finally:
        temporary_path.unlink(missing_ok=True)


def atomic_text(content: str, destination: Path, mode: int) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=destination.parent, delete=False
    ) as temporary:
        temporary.write(content)
        temporary_path = Path(temporary.name)
    try:
        temporary_path.chmod(mode)
        os.replace(temporary_path, destination)
    finally:
        temporary_path.unlink(missing_ok=True)


def set_enabled(enabled: bool) -> None:
    completed = subprocess.run(
        [
            "kwriteconfig6",
            "--file",
            "kwinrc",
            "--group",
            "Plugins",
            "--key",
            "seatgeistagentseatEnabled",
            "true" if enabled else "false",
        ],
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError("kwriteconfig6 could not update the KWin plugin setting")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Install the experimental Seatgeist independent KWin agent-seat plugin "
            "for the next Plasma session."
        )
    )
    parser.add_argument("--artifact", type=Path, default=DEFAULT_ARTIFACT)
    parser.add_argument("--plugin-root", type=Path, default=DEFAULT_PLUGIN_ROOT)
    parser.add_argument("--drop-in", type=Path, default=DEFAULT_DROP_IN)
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--no-daemon-reload", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--no-config-update", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()

    plugin = args.plugin_root / "kwin/plugins/seatgeistagentseat.so"
    if args.remove:
        plugin.unlink(missing_ok=True)
        args.drop_in.unlink(missing_ok=True)
        if not args.no_config_update:
            set_enabled(False)
        action = "removed"
    else:
        if not args.artifact.is_file():
            raise SystemExit(
                "agent-seat plugin artifact is missing; "
                "run make check-kwin-agent-seat-plugin"
            )
        plugin_root = str(args.plugin_root.resolve())
        if any(character in plugin_root for character in ['"', "\n", "\r"]):
            raise SystemExit("plugin root contains unsupported characters")
        atomic_copy(args.artifact, plugin, 0o755)
        atomic_text(
            f'[Service]\nEnvironment="QT_PLUGIN_PATH={plugin_root}"\n',
            args.drop_in,
            0o644,
        )
        if not args.no_config_update:
            set_enabled(True)
        action = "installed"

    if not args.no_daemon_reload:
        completed = subprocess.run(
            ["systemctl", "--user", "daemon-reload"],
            check=False,
        )
        if completed.returncode != 0:
            raise RuntimeError("systemctl --user daemon-reload failed")

    print(
        json.dumps(
            {
                "type": "seatgeist_kwin_agent_seat_user_install",
                "version": 1,
                "action": action,
                "plugin": str(plugin),
                "drop_in": str(args.drop_in),
                "compositor_restarted": False,
                "next_step": (
                    "restart the normal Plasma session, configure "
                    '[backends] input = "kwin_agent_seat", and check '
                    "seatgeist.input_backend_status"
                ),
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
