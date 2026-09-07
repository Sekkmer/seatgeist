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
DEFAULT_ARTIFACT = ROOT / "target/kwin-seatgeist-activity/seatgeistactivity.so"
DEFAULT_PLUGIN_ROOT = Path.home() / ".local/lib/qt6/plugins"
DEFAULT_DROP_IN = (
    Path.home()
    / ".config/systemd/user/plasma-kwin_wayland.service.d/50-seatgeist-activity.conf"
)
DEFAULT_WATCHER_SOURCE = ROOT / "scripts/kwin-activity-abi-watch.py"
DEFAULT_WATCHER = Path.home() / ".local/libexec/seatgeist/kwin-activity-abi-watch"
DEFAULT_UNIT_SOURCE_DIR = ROOT / "systemd/user"
DEFAULT_UNIT_DIR = Path.home() / ".config/systemd/user"
DEFAULT_STATE = Path.home() / ".local/state/seatgeist/kwin-activity-abi.json"
SERVICE_NAME = "seatgeist-kwin-activity-abi.service"
PATH_NAME = "seatgeist-kwin-activity-abi.path"
TIMER_NAME = "seatgeist-kwin-activity-abi.timer"


def render_drop_in(plugin_root: Path) -> str:
    path = str(plugin_root.resolve())
    if any(character in path for character in ['"', "\n", "\r"]):
        raise ValueError("plugin root contains unsupported characters")
    return f'[Service]\nEnvironment="QT_PLUGIN_PATH={path}"\n'


def render_service(template: str, watcher: Path) -> str:
    path = str(watcher.resolve())
    if any(character in path for character in ['"', "\n", "\r"]):
        raise ValueError("watcher path contains unsupported characters")
    return template.replace("@WATCHER@", f'"{path}"')


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


def daemon_reload() -> None:
    completed = subprocess.run(
        ["systemctl", "--user", "daemon-reload"], check=False, timeout=10
    )
    if completed.returncode != 0:
        raise RuntimeError("systemctl --user daemon-reload failed")


def run_systemctl(*arguments: str) -> None:
    completed = subprocess.run(
        ["systemctl", "--user", *arguments],
        check=False,
        timeout=10,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"systemctl --user {' '.join(arguments)} failed")


def manage_units(action: str, *, activate_watchers: bool = False) -> None:
    if action == "install":
        # The service is path/timer-triggered only. In particular, never leave
        # the checker enabled directly in a graphical boot target.
        run_systemctl("disable", SERVICE_NAME)
        # Plugin installation is still a next-login operation. Starting only
        # the diagnostic triggers is a separate, explicit opt-in.
        run_systemctl("enable", PATH_NAME, TIMER_NAME)
        if activate_watchers:
            run_systemctl("start", PATH_NAME, TIMER_NAME)
    elif action == "remove":
        run_systemctl("disable", SERVICE_NAME, PATH_NAME, TIMER_NAME)
    else:
        raise ValueError(f"unsupported unit management action: {action}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Install the Seatgeist KWin binary plugin for the next Plasma session "
            "without restarting the active compositor."
        )
    )
    parser.add_argument("--artifact", type=Path, default=DEFAULT_ARTIFACT)
    parser.add_argument("--plugin-root", type=Path, default=DEFAULT_PLUGIN_ROOT)
    parser.add_argument("--drop-in", type=Path, default=DEFAULT_DROP_IN)
    parser.add_argument("--watcher-source", type=Path, default=DEFAULT_WATCHER_SOURCE)
    parser.add_argument("--watcher", type=Path, default=DEFAULT_WATCHER)
    parser.add_argument(
        "--unit-source-dir", type=Path, default=DEFAULT_UNIT_SOURCE_DIR
    )
    parser.add_argument("--unit-dir", type=Path, default=DEFAULT_UNIT_DIR)
    parser.add_argument("--state", type=Path, default=DEFAULT_STATE)
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--watcher-only", action="store_true", help="Update only diagnostic script/units; leave plugins and KWin configuration unchanged")
    parser.add_argument("--activate-watchers", action="store_true", help="Start only diagnostic path/timer units now, without restarting KWin or Seatgeist")
    parser.add_argument("--no-daemon-reload", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument(
        "--no-systemd-management", action="store_true", help=argparse.SUPPRESS
    )
    args = parser.parse_args()
    if args.remove and (args.watcher_only or args.activate_watchers):
        parser.error("--remove cannot be combined with --watcher-only or --activate-watchers")
    if args.activate_watchers and (args.no_systemd_management or args.no_daemon_reload):
        parser.error("--activate-watchers requires systemd management and daemon reload")

    plugin = args.plugin_root / "kwin/plugins/seatgeistactivity.so"
    experimental_focus_plugins = (
        args.plugin_root / "kwin/plugins/seatgeistfocus.so",
        args.plugin_root / "kwin/plugins/seatgeistfocuscontroller.so",
        args.plugin_root / "kwin/plugins/seatgeistfocusbridge.so",
        args.plugin_root / "kwin/plugins/seatgeistfocuslease.so",
    )
    service = args.unit_dir / SERVICE_NAME
    path_unit = args.unit_dir / PATH_NAME
    timer = args.unit_dir / TIMER_NAME
    if args.remove:
        if not args.no_systemd_management:
            manage_units("remove")
        plugin.unlink(missing_ok=True)
        for experimental_focus_plugin in experimental_focus_plugins:
            experimental_focus_plugin.unlink(missing_ok=True)
        args.drop_in.unlink(missing_ok=True)
        args.watcher.unlink(missing_ok=True)
        service.unlink(missing_ok=True)
        path_unit.unlink(missing_ok=True)
        timer.unlink(missing_ok=True)
        args.state.unlink(missing_ok=True)
        action = "removed"
    else:
        if not args.watcher_only and not args.artifact.is_file():
            raise SystemExit("activity plugin artifact is missing; run make check-kwin-activity-plugin")
        service_template = args.unit_source_dir.joinpath(
            f"{SERVICE_NAME}.in"
        ).read_text(encoding="utf-8")
        path_source = args.unit_source_dir / PATH_NAME
        timer_source = args.unit_source_dir / TIMER_NAME
        if (
            not args.watcher_source.is_file()
            or not path_source.is_file()
            or not timer_source.is_file()
        ):
            raise SystemExit("activity ABI watcher install assets are missing")
        # Render/validate all unit paths before touching the installed plugin.
        service_text = render_service(service_template, args.watcher)
        if not args.watcher_only:
            drop_in_text = render_drop_in(args.plugin_root)
            atomic_copy(args.artifact, plugin, 0o755)
            for experimental_focus_plugin in experimental_focus_plugins:
                experimental_focus_plugin.unlink(missing_ok=True)
            atomic_text(drop_in_text, args.drop_in, 0o644)
        atomic_copy(args.watcher_source, args.watcher, 0o755)
        atomic_text(service_text, service, 0o644)
        atomic_copy(path_source, path_unit, 0o644)
        atomic_copy(timer_source, timer, 0o644)
        action = "installed"
    if not args.no_daemon_reload:
        daemon_reload()
    if not args.remove and not args.no_systemd_management:
        manage_units("install", activate_watchers=args.activate_watchers)

    print(
        json.dumps(
            {
                "type": "seatgeist_kwin_activity_user_install",
                "version": 4,
                "action": action,
                "plugin": str(plugin),
                "drop_in": str(args.drop_in),
                "abi_watcher": str(args.watcher),
                "abi_service": str(service),
                "abi_path": str(path_unit),
                "abi_timer": str(timer),
                "compositor_restarted": False,
                "plugin_updated": action == "installed" and not args.watcher_only,
                "watchers_activated": args.activate_watchers,
                "next_step": (
                    "inspect the ABI watcher with --check-only --runtime; follow its recovery guidance"
                    if args.watcher_only else
                    "restart the normal Plasma session, then run the ABI watcher with --check-only --runtime"
                ),
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
