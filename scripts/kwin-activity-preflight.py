#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARTIFACT = ROOT / "target/kwin-seatgeist-activity/seatgeistactivity.so"
PLUGIN_ID = "seatgeistactivity"
ABI_RE = re.compile(rb"org\.kde\.kwin\.PluginFactoryInterface(\d+\.\d+\.\d+)")
LIBKWIN_RE = re.compile(r"(?:^|/)libkwin\.so\.(\d+\.\d+\.\d+)(?: \(deleted\))?$")
KWIN_SUPPORT_VERSION_RE = re.compile(
    r"^KWin version:\s*(\d+\.\d+\.\d+)\s*$", re.MULTILINE
)


def command_lines(arguments: list[str]) -> list[str]:
    try:
        completed = subprocess.run(
            arguments,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        return []
    if completed.returncode != 0:
        return []
    return [line.strip() for line in completed.stdout.splitlines() if line.strip()]


def plugin_abi(path: Path) -> str | None:
    try:
        content = path.read_bytes()
    except OSError:
        return None
    match = ABI_RE.search(content)
    return match.group(1).decode("ascii") if match else None


def libkwin_abi_from_maps(content: str) -> str | None:
    versions = {
        match.group(1)
        for line in content.splitlines()
        if (match := LIBKWIN_RE.search(line.split(maxsplit=5)[-1] if line.split(maxsplit=5) else ""))
    }
    return sorted(versions)[-1] if versions else None


def kwin_abi_from_support_information(content: str) -> str | None:
    match = KWIN_SUPPORT_VERSION_RE.search(content)
    return match.group(1) if match else None


def running_kwin_abi_from_dbus() -> str | None:
    lines = command_lines(["qdbus6", "org.kde.KWin", "/KWin", "supportInformation"])
    return kwin_abi_from_support_information("\n".join(lines))


def running_kwin() -> tuple[int | None, str | None, bool]:
    candidates: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry / "comm").read_text(encoding="utf-8").strip() == "kwin_wayland":
                candidates.append(int(entry.name))
        except OSError:
            continue
    if not candidates:
        return None, None, False
    pid = min(candidates)
    try:
        maps = Path(f"/proc/{pid}/maps").read_text(encoding="utf-8")
    except OSError:
        return pid, running_kwin_abi_from_dbus(), False
    maps_abi = libkwin_abi_from_maps(maps)
    return (
        pid,
        maps_abi or running_kwin_abi_from_dbus(),
        "libkwin" in maps and "(deleted)" in maps,
    )


def process_environment(pid: int | None, key: str) -> str | None:
    if pid is None:
        return None
    try:
        entries = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
    except OSError:
        return None
    prefix = key.encode("utf-8") + b"="
    for entry in entries:
        if entry.startswith(prefix):
            return entry[len(prefix) :].decode("utf-8", errors="replace")
    return None


def current_libkwin_abi() -> str | None:
    path = Path("/usr/lib/libkwin.so.6")
    try:
        resolved = path.resolve(strict=True)
    except OSError:
        return None
    match = LIBKWIN_RE.search(str(resolved))
    return match.group(1) if match else None


def qt_plugin_root() -> Path | None:
    lines = command_lines(["qmake6", "-query", "QT_INSTALL_PLUGINS"])
    return Path(lines[0]) if lines else None


def dbus_plugins(property_name: str) -> list[str]:
    return command_lines(
        [
            "qdbus6",
            "org.kde.KWin",
            "/Plugins",
            "org.freedesktop.DBus.Properties.Get",
            "org.kde.KWin.Plugins",
            property_name,
        ]
    )


def sha256(path: Path) -> str | None:
    try:
        content = path.read_bytes()
    except OSError:
        return None
    return hashlib.sha256(content).hexdigest()


def build_report(artifact: Path) -> dict[str, Any]:
    build_abi = plugin_abi(artifact)
    running_pid, running_abi, running_deleted = running_kwin()
    installed_abi = current_libkwin_abi()
    plugin_root = qt_plugin_root()
    system_install_path = (
        plugin_root / "kwin/plugins/seatgeistactivity.so" if plugin_root else None
    )
    user_plugin_root = Path.home() / ".local/lib/qt6/plugins"
    user_install_path = user_plugin_root / "kwin/plugins/seatgeistactivity.so"
    candidates = [user_install_path]
    if system_install_path is not None:
        candidates.append(system_install_path)
    artifact_hash = sha256(artifact)
    matching_install = next(
        (path for path in candidates if artifact_hash is not None and sha256(path) == artifact_hash),
        None,
    )
    install_path = matching_install or user_install_path
    installed_hash = sha256(install_path)
    installed = matching_install is not None
    available = dbus_plugins("AvailablePlugins")
    loaded = dbus_plugins("LoadedPlugins")
    abi_matches_running = build_abi is not None and build_abi == running_abi
    restart_required = running_pid is not None and not abi_matches_running
    running_qt_plugin_path = process_environment(running_pid, "QT_PLUGIN_PATH")
    user_path_active = running_qt_plugin_path is not None and str(user_plugin_root) in (
        part for part in running_qt_plugin_path.split(":") if part
    )
    drop_in = (
        Path.home()
        / ".config/systemd/user/plasma-kwin_wayland.service.d/50-seatgeist-activity.conf"
    )
    user_path_configured = drop_in.is_file()

    next_actions: list[str] = []
    if not artifact.exists():
        next_actions.append("run make check-kwin-activity-plugin")
    elif not installed:
        next_actions.append("install the built plugin into the reported KWin plugin path")
    if restart_required:
        next_actions.append("restart the normal Plasma session to load the installed KWin ABI")
    elif installed and PLUGIN_ID not in available:
        next_actions.append("refresh KWin plugin discovery after installation")
    elif installed and PLUGIN_ID not in loaded:
        next_actions.append("load seatgeistactivity through org.kde.KWin.Plugins.LoadPlugin")
    if PLUGIN_ID in loaded:
        next_actions.append("verify seatgeist.safety_status reports kwin_input_spy_v1 trusted")

    return {
        "type": "seatgeist_kwin_activity_preflight",
        "version": 1,
        "artifact": {
            "path": str(artifact),
            "exists": artifact.exists(),
            "sha256": artifact_hash,
            "plugin_factory_abi": build_abi,
        },
        "installed": {
            "path": str(install_path) if install_path else None,
            "matches_artifact": installed,
            "sha256": installed_hash,
            "libkwin_abi": installed_abi,
            "user_plugin_path_configured": user_path_configured,
        },
        "running": {
            "pid": running_pid,
            "libkwin_abi": running_abi,
            "deleted_binary_or_library": running_deleted,
            "abi_matches_plugin": abi_matches_running,
            "qt_plugin_path": running_qt_plugin_path,
            "user_plugin_path_active": user_path_active,
        },
        "kwin_plugins": {
            "available": PLUGIN_ID in available,
            "loaded": PLUGIN_ID in loaded,
        },
        "restart_required": restart_required,
        "ready": installed and abi_matches_running and PLUGIN_ID in loaded,
        "next_actions": next_actions,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Report whether the Seatgeist KWin activity plugin can be safely loaded."
    )
    parser.add_argument("--artifact", type=Path, default=DEFAULT_ARTIFACT)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    report = build_report(args.artifact.resolve())
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
        print(f"kwin-activity-preflight: wrote {args.output}")
    else:
        print(encoded, end="")


if __name__ == "__main__":
    main()
