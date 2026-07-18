#!/usr/bin/env python3
"""Deterministic tests for the KWin bridge installer and live refresh."""

from __future__ import annotations

import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "scripts" / "install-kwin-bridge.py"


FAKE_COMMAND = """#!/usr/bin/env python3
import os
import sys

with open(os.environ["COMMAND_LOG"], "a", encoding="utf-8") as handle:
    handle.write(os.path.basename(sys.argv[0]) + " " + " ".join(sys.argv[1:]) + "\\n")

args = sys.argv[1:]
if os.path.basename(sys.argv[0]) == "kpackagetool6" and "--list" in args:
    if os.environ.get("PACKAGE_INSTALLED") == "1":
        print("seatgeist-bridge")
elif os.path.basename(sys.argv[0]) == "qdbus6":
    if os.environ.get("KWIN_ONLINE") != "1":
        raise SystemExit(1)
    if any(value.endswith(".isScriptLoaded") for value in args):
        print(os.environ.get("SCRIPT_LOADED", "false"))
    elif any(value.endswith(".unloadScript") for value in args):
        print("true")
    elif any(value.endswith(".loadScript") for value in args):
        print("7")
"""


def prepare_case(directory: Path) -> tuple[dict[str, str], Path]:
    bin_dir = directory / "bin"
    bin_dir.mkdir()
    for name in ("kpackagetool6", "kwriteconfig6", "qdbus6"):
        command = bin_dir / name
        command.write_text(FAKE_COMMAND, encoding="utf-8")
        command.chmod(0o755)

    data_home = directory / "data"
    installed_main = (
        data_home
        / "kwin"
        / "scripts"
        / "seatgeist-bridge"
        / "contents"
        / "code"
        / "main.js"
    )
    installed_main.parent.mkdir(parents=True)
    installed_main.write_text("// installed test bridge\n", encoding="utf-8")

    log = directory / "commands.log"
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{bin_dir}:{env['PATH']}",
            "COMMAND_LOG": str(log),
            "XDG_DATA_HOME": str(data_home),
        }
    )
    return env, log


def run_case(**overrides: str) -> tuple[str, list[str]]:
    with tempfile.TemporaryDirectory(prefix="seatgeist-kwin-installer-") as temporary:
        env, log = prepare_case(Path(temporary))
        env.update(overrides)
        result = subprocess.run(
            [str(INSTALLER), "--root", str(ROOT)],
            check=True,
            text=True,
            capture_output=True,
            env=env,
        )
        return result.stdout, log.read_text(encoding="utf-8").splitlines()


stdout, commands = run_case(
    PACKAGE_INSTALLED="1", KWIN_ONLINE="1", SCRIPT_LOADED="true"
)
assert any(" --type=KWin/Script -u " in f" {command} " for command in commands)
assert any(".unloadScript seatgeist-bridge" in command for command in commands)
assert any(".loadScript" in command and "seatgeist-bridge" in command for command in commands)
assert any("/Scripting/Script7 org.kde.kwin.Script.run" in command for command in commands)
assert "refreshed in the live compositor" in stdout

stdout, commands = run_case(
    PACKAGE_INSTALLED="0", KWIN_ONLINE="1", SCRIPT_LOADED="false"
)
assert any(" --type=KWin/Script -i " in f" {command} " for command in commands)
assert not any(".unloadScript" in command for command in commands)
assert any(".loadScript" in command for command in commands)

stdout, commands = run_case(PACKAGE_INSTALLED="1", KWIN_ONLINE="0")
assert "loading is deferred to the next KDE session" in stdout
assert not any(".unloadScript" in command for command in commands)
assert not any(".loadScript" in command for command in commands)

print("test-install-kwin-bridge: ok")
