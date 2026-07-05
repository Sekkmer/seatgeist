#!/usr/bin/env python3
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from dataclasses import dataclass
from typing import Any


PACKAGES = [
    "xdg-desktop-portal",
    "xdg-desktop-portal-kde",
    "plasma-workspace",
    "spectacle",
]

TARGET_NAMES = {
    1: "screen",
    2: "window",
    4: "area",
    8: "active_window",
}


@dataclass
class CommandResult:
    ok: bool
    stdout: str
    stderr: str
    returncode: int | None


def run(command: list[str], timeout: float = 3.0) -> CommandResult:
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except FileNotFoundError:
        return CommandResult(False, "", f"{command[0]} not found", None)
    except subprocess.TimeoutExpired as err:
        stdout = err.stdout.decode("utf-8", "replace") if isinstance(err.stdout, bytes) else err.stdout or ""
        stderr = err.stderr.decode("utf-8", "replace") if isinstance(err.stderr, bytes) else err.stderr or ""
        return CommandResult(False, stdout, stderr or "command timed out", None)
    return CommandResult(
        completed.returncode == 0,
        completed.stdout.strip(),
        completed.stderr.strip(),
        completed.returncode,
    )


def parse_busctl_uint(output: str) -> int | None:
    parts = output.split()
    if len(parts) != 2 or parts[0] != "u":
        return None
    try:
        return int(parts[1], 10)
    except ValueError:
        return None


def get_portal_property(name: str) -> tuple[int | None, CommandResult]:
    result = run(
        [
            "busctl",
            "--user",
            "get-property",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Screenshot",
            name,
        ]
    )
    return parse_busctl_uint(result.stdout), result


def target_names(mask: int | None) -> list[str]:
    if mask is None:
        return []
    return [name for bit, name in TARGET_NAMES.items() if mask & bit]


def pacman_installed_versions() -> dict[str, str | None]:
    if shutil.which("pacman") is None:
        return {package: None for package in PACKAGES}
    result = run(["pacman", "-Q", *PACKAGES], timeout=5.0)
    versions: dict[str, str | None] = {package: None for package in PACKAGES}
    for line in result.stdout.splitlines():
        parts = line.split(maxsplit=1)
        if len(parts) == 2 and parts[0] in versions:
            versions[parts[0]] = parts[1]
    return versions


def pacman_pending_upgrades() -> dict[str, str]:
    if shutil.which("pacman") is None:
        return {}
    result = run(["pacman", "-Qu", *PACKAGES], timeout=5.0)
    upgrades: dict[str, str] = {}
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 4 and parts[0] in PACKAGES and parts[2] == "->":
            upgrades[parts[0]] = parts[3]
    return upgrades


def build_next_steps(
    busctl_available: bool,
    version: int | None,
    target_mask: int | None,
    pending_upgrades: dict[str, str],
) -> list[str]:
    if not busctl_available:
        return [
            "Install or expose busctl/systemd in the user session, then rerun make portal-screenshot-v3-status.",
            "Do not run a system upgrade until the portal D-Bus state can be measured.",
        ]
    if version is None:
        return [
            "Restart or inspect the user xdg-desktop-portal services; the Screenshot version property was not readable.",
            "Check seatgeist-cli capture-backends for the daemon view before trying portal-target captures.",
        ]
    if version >= 3 and target_mask is not None:
        return [
            "Screenshot v3 targets are exported; retry seatgeist-cli screenshot --portal-target active-window from a guarded test.",
            "If capture still fails, inspect the portal consent result and journal entry instead of changing packages first.",
        ]
    if pending_upgrades:
        return [
            "A package upgrade is visible in the local pacman sync DB; update the portal/KDE packages through the normal operator workflow, then restart the user session or xdg-desktop-portal services.",
            "After the restart, rerun make portal-screenshot-v3-status and seatgeist-cli capture-backends before relying on --portal-target.",
        ]
    return [
        "The current user-session portal exports Screenshot v2 or lacks AvailableTargets; Seatgeist must keep failing closed for portal_target requests.",
        "If pacman -Syu later offers newer xdg-desktop-portal or xdg-desktop-portal-kde packages, upgrade through the normal operator workflow and restart the user session before retesting.",
        "If fully updated packages still export v2, this is a KDE portal/backend capability gap; prefer Seatgeist's bounded full-screen/tile path or implement a documented KWin-native fallback.",
    ]


def main() -> None:
    version, version_result = get_portal_property("version")
    target_mask, target_result = get_portal_property("AvailableTargets")
    installed_versions = pacman_installed_versions()
    pending_upgrades = pacman_pending_upgrades()
    target_option_supported = version is not None and version >= 3 and target_mask is not None
    ready = target_option_supported and bool(target_names(target_mask))

    payload: dict[str, Any] = {
        "type": "portal_screenshot_v3_status",
        "ok": ready,
        "portal": {
            "busctl_available": shutil.which("busctl") is not None,
            "screenshot_interface_version": version,
            "available_targets_mask": target_mask,
            "available_targets": target_names(target_mask),
            "target_option_supported": target_option_supported,
            "version_probe_ok": version_result.ok,
            "available_targets_probe_ok": target_result.ok,
            "version_probe_error": version_result.stderr if not version_result.ok else None,
            "available_targets_probe_error": target_result.stderr if not target_result.ok else None,
        },
        "packages": {
            "manager": "pacman" if shutil.which("pacman") else None,
            "installed": installed_versions,
            "pending_upgrades": pending_upgrades,
            "aur_step_available": shutil.which("aur-step") is not None,
            "aur_step_path": shutil.which("aur-step"),
        },
        "notes": [
            "This is a read-only diagnostic. It does not install, upgrade, restart services, request portal consent, or capture pixels.",
            "The xdg-desktop-portal Screenshot target option and AvailableTargets property are version 3 interface features.",
        ],
        "next_steps": build_next_steps(
            shutil.which("busctl") is not None,
            version,
            target_mask,
            pending_upgrades,
        ),
    }
    json.dump(payload, sys.stdout, indent=2, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
