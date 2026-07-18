#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "desktop/org.seatgeist.daemon.desktop.in"


class InstallError(RuntimeError):
    pass


def render(executable: Path) -> str:
    if not executable.is_absolute():
        raise InstallError("Seatgeist daemon path must be absolute")
    value = str(executable)
    if any(character in value for character in ("\n", "\r", "\0")):
        raise InstallError("Seatgeist daemon path contains an invalid character")
    template = TEMPLATE.read_text(encoding="utf-8")
    marker = "@SEATGEISTD_EXECUTABLE@"
    if template.count(marker) != 1:
        raise InstallError("KDE authorization template has an invalid executable marker")
    return template.replace(marker, value)


def atomic_write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        dir=path.parent,
        delete=False,
    ) as temporary:
        temporary.write(content)
        temporary.flush()
        os.fsync(temporary.fileno())
        temporary_path = Path(temporary.name)
    try:
        temporary_path.chmod(0o644)
        os.replace(temporary_path, path)
    finally:
        temporary_path.unlink(missing_ok=True)


def install(executable: Path, applications_dir: Path, kbuildsycoca: str) -> dict[str, object]:
    executable = executable.expanduser().resolve(strict=False)
    destination = applications_dir.expanduser() / "org.seatgeist.daemon.desktop"
    atomic_write(destination, render(executable))
    completed = subprocess.run(
        [kbuildsycoca, "--noincremental"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or str(completed.returncode)
        raise InstallError(f"KDE service cache refresh failed: {detail}")
    return {
        "type": "seatgeist_kwin_screenshot_authorization",
        "version": 1,
        "ok": True,
        "desktop_file": str(destination),
        "executable": str(executable),
        "restricted_interface": "org.kde.KWin.ScreenShot2",
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Authorize the installed Seatgeist daemon for KWin's restricted exact-window "
            "screenshot interface and refresh KDE's service cache."
        )
    )
    parser.add_argument(
        "--daemon-path",
        type=Path,
        default=Path.home() / ".local/bin/seatgeistd",
    )
    parser.add_argument(
        "--applications-dir",
        type=Path,
        default=Path.home() / ".local/share/applications",
    )
    parser.add_argument("--kbuildsycoca", default="kbuildsycoca6")
    args = parser.parse_args()
    try:
        report = install(args.daemon_path, args.applications_dir, args.kbuildsycoca)
    except (InstallError, OSError) as error:
        raise SystemExit(f"install-kwin-screenshot-authorization: {error}") from error
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
