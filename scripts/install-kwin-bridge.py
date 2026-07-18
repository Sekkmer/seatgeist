#!/usr/bin/env python3
"""Install, enable, and safely refresh the Seatgeist KWin script."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path


PLUGIN_ID = "seatgeist-bridge"
KWIN_SERVICE = "org.kde.KWin"
SCRIPTING_PATH = "/Scripting"
SCRIPTING_INTERFACE = "org.kde.kwin.Scripting"


def run(command: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, check=check, text=True, capture_output=True)


def data_home() -> Path:
    configured = os.environ.get("XDG_DATA_HOME")
    if configured:
        return Path(configured).expanduser()
    return Path.home() / ".local" / "share"


def install_package(root: Path) -> None:
    package = root / "kwin" / PLUGIN_ID
    if not package.is_dir():
        raise SystemExit(f"KWin bridge package is missing: {package}")

    listed = run(
        ["kpackagetool6", "--type=KWin/Script", "--list"]
    ).stdout.splitlines()
    operation = "-u" if any(PLUGIN_ID in line for line in listed) else "-i"
    result = run(
        ["kpackagetool6", "--type=KWin/Script", operation, str(package)]
    )
    if result.stdout.strip():
        print(result.stdout.strip())

    run(
        [
            "kwriteconfig6",
            "--file",
            "kwinrc",
            "--group",
            "Plugins",
            "--key",
            f"{PLUGIN_ID}Enabled",
            "true",
        ]
    )


def refresh_running_script() -> None:
    qdbus = shutil.which("qdbus6")
    if qdbus is None:
        print("KWin bridge enabled; qdbus6 is unavailable, so loading is deferred to the next KDE session")
        return

    loaded = run(
        [
            qdbus,
            KWIN_SERVICE,
            SCRIPTING_PATH,
            f"{SCRIPTING_INTERFACE}.isScriptLoaded",
            PLUGIN_ID,
        ],
        check=False,
    )
    if loaded.returncode != 0:
        print("KWin bridge enabled; no live KWin scripting service, so loading is deferred to the next KDE session")
        return

    if loaded.stdout.strip().lower() == "true":
        run(
            [
                qdbus,
                KWIN_SERVICE,
                SCRIPTING_PATH,
                f"{SCRIPTING_INTERFACE}.unloadScript",
                PLUGIN_ID,
            ]
        )

    installed_main = data_home() / "kwin" / "scripts" / PLUGIN_ID / "contents" / "code" / "main.js"
    if not installed_main.is_file():
        raise SystemExit(f"installed KWin bridge entry point is missing: {installed_main}")

    loaded_script = run(
        [
            qdbus,
            KWIN_SERVICE,
            SCRIPTING_PATH,
            f"{SCRIPTING_INTERFACE}.loadScript",
            str(installed_main),
            PLUGIN_ID,
        ]
    ).stdout.strip()
    try:
        script_id = int(loaded_script)
    except ValueError as error:
        raise SystemExit(f"KWin returned an invalid script id: {loaded_script!r}") from error
    if script_id < 0:
        raise SystemExit(f"KWin refused to load {PLUGIN_ID}: script id {script_id}")

    run(
        [
            qdbus,
            KWIN_SERVICE,
            f"/Scripting/Script{script_id}",
            "org.kde.kwin.Script.run",
        ]
    )
    print(f"KWin bridge refreshed in the live compositor (script id {script_id})")


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    if len(sys.argv) > 1:
        if len(sys.argv) != 3 or sys.argv[1] != "--root":
            raise SystemExit("usage: install-kwin-bridge.py [--root REPOSITORY]")
        root = Path(sys.argv[2]).resolve()
    install_package(root)
    refresh_running_script()


if __name__ == "__main__":
    main()
