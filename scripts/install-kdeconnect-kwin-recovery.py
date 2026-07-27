#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ASSET_DIR = ROOT / "systemd/user"
DBUS_ASSET = ROOT / "dbus/user/org.kde.kdeconnect.service"
DEFAULT_UNIT_DIR = Path.home() / ".config/systemd/user"
DEFAULT_DBUS_DIR = Path.home() / ".local/share/dbus-1/services"
KDECONNECT_UNIT = "app-org.kde.kdeconnect.daemon@autostart.service"
RECOVERY_UNIT = "kdeconnect-after-kwin.service"
KDECONNECT_DROP_IN = "50-kwin-lifecycle.conf"
KWIN_DROP_IN = "60-kdeconnect-recovery.conf"


class InstallError(RuntimeError):
    pass


def run(
    args: list[str],
    *,
    check: bool = True,
    timeout: float | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        args,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=timeout,
    )
    if check and completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise InstallError(f"{' '.join(args)} failed: {detail or completed.returncode}")
    return completed


def atomic_copy(source: Path, destination: Path, mode: int = 0o644) -> None:
    if not source.is_file():
        raise InstallError(f"install asset is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as temporary:
        temporary_path = Path(temporary.name)
    try:
        shutil.copyfile(source, temporary_path)
        temporary_path.chmod(mode)
        os.replace(temporary_path, destination)
    finally:
        temporary_path.unlink(missing_ok=True)


def installed_paths(unit_dir: Path, dbus_dir: Path) -> dict[str, Path]:
    return {
        "recovery_unit": unit_dir / RECOVERY_UNIT,
        "kdeconnect_drop_in": (
            unit_dir / f"{KDECONNECT_UNIT}.d" / KDECONNECT_DROP_IN
        ),
        "kwin_drop_in": (
            unit_dir / "plasma-kwin_wayland.service.d" / KWIN_DROP_IN
        ),
        "dbus_service": dbus_dir / "org.kde.kdeconnect.service",
    }


def install_assets(paths: dict[str, Path]) -> None:
    atomic_copy(ASSET_DIR / RECOVERY_UNIT, paths["recovery_unit"])
    atomic_copy(
        ASSET_DIR / "kdeconnect-autostart-kwin.conf",
        paths["kdeconnect_drop_in"],
    )
    atomic_copy(
        ASSET_DIR / "kwin-kdeconnect-recovery.conf",
        paths["kwin_drop_in"],
    )
    atomic_copy(DBUS_ASSET, paths["dbus_service"])


def remove_assets(paths: dict[str, Path]) -> None:
    for path in paths.values():
        path.unlink(missing_ok=True)
        try:
            path.parent.rmdir()
        except OSError:
            pass


def reload_managers() -> None:
    run(["systemctl", "--user", "daemon-reload"])
    run(
        [
            "busctl",
            "--user",
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ReloadConfig",
        ]
    )


def kdeconnect_owner_pid() -> int | None:
    completed = run(
        [
            "busctl",
            "--user",
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            "s",
            "org.kde.kdeconnect",
        ],
        check=False,
    )
    if completed.returncode != 0:
        return None
    fields = completed.stdout.split()
    if len(fields) != 2 or fields[0] != "u":
        raise InstallError(f"unexpected D-Bus owner response: {completed.stdout.strip()}")
    return int(fields[1])


def terminate_existing_daemon() -> int | None:
    pid = kdeconnect_owner_pid()
    if pid is None:
        return None
    comm = Path(f"/proc/{pid}/comm")
    if comm.read_text(encoding="utf-8").strip() != "kdeconnectd":
        raise InstallError(f"refusing to signal unexpected D-Bus owner pid {pid}")

    try:
        run(["kquitapp6", "kdeconnectd"], check=False, timeout=2)
    except subprocess.TimeoutExpired:
        pass
    deadline = time.monotonic() + 2
    while Path(f"/proc/{pid}").exists() and time.monotonic() < deadline:
        time.sleep(0.05)
    if Path(f"/proc/{pid}").exists():
        os.kill(pid, signal.SIGTERM)
    deadline = time.monotonic() + 5
    while Path(f"/proc/{pid}").exists() and time.monotonic() < deadline:
        time.sleep(0.05)
    if Path(f"/proc/{pid}").exists():
        raise InstallError(f"kdeconnectd pid {pid} did not exit after SIGTERM")
    return pid


def activate_managed_daemon() -> dict[str, int | None]:
    previous_pid = terminate_existing_daemon()
    run(["systemctl", "--user", "reset-failed", KDECONNECT_UNIT], check=False)
    run(["systemctl", "--user", "start", KDECONNECT_UNIT])

    deadline = time.monotonic() + 10
    current_pid = None
    while time.monotonic() < deadline:
        current_pid = kdeconnect_owner_pid()
        if current_pid is not None:
            break
        time.sleep(0.1)
    if current_pid is None:
        raise InstallError("managed KDE Connect did not acquire its D-Bus name")
    run(["kdeconnect-cli", "--refresh"], check=False, timeout=10)
    return {"previous_pid": previous_pid, "current_pid": current_pid}


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Tie KDE Connect to the Plasma KWin lifecycle so compositor restarts "
            "recreate its RemoteDesktop/EIS session without restarting KWin."
        )
    )
    parser.add_argument("--unit-dir", type=Path, default=DEFAULT_UNIT_DIR)
    parser.add_argument("--dbus-dir", type=Path, default=DEFAULT_DBUS_DIR)
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--no-systemd-management", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()

    paths = installed_paths(args.unit_dir, args.dbus_dir)
    process_report: dict[str, int | None] = {
        "previous_pid": None,
        "current_pid": None,
    }
    if args.remove:
        remove_assets(paths)
        action = "removed"
    else:
        install_assets(paths)
        action = "installed"

    if not args.no_systemd_management:
        reload_managers()
        if not args.remove:
            process_report = activate_managed_daemon()

    print(
        json.dumps(
            {
                "type": "seatgeist_kdeconnect_kwin_recovery_install",
                "version": 1,
                "action": action,
                **{name: str(path) for name, path in paths.items()},
                **process_report,
                "compositor_restarted": False,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
