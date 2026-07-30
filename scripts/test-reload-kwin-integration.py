#!/usr/bin/env python3
"""Deterministic tests for the no-compositor-restart KWin reload helper."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts/reload-kwin-integration.py"
ABI = "6.4.3"

FAKE_QDBUS = """#!/usr/bin/env python3
import os
import sys

with open(os.environ["COMMAND_LOG"], "a", encoding="utf-8") as handle:
    handle.write(" ".join(sys.argv[1:]) + "\\n")

args = sys.argv[1:]
if args[-1] == "supportInformation":
    print("KWin version: " + os.environ.get("KWIN_ABI", "6.4.3"))
elif args[-1].endswith(".AvailablePlugins"):
    print("seatgeistactivity\\nseatgeistagentseat")
elif args[-1].endswith(".LoadedPlugins"):
    print("seatgeistactivity\\nseatgeistagentseat")
elif any(value.endswith(".LoadPlugin") for value in args):
    print("true")
elif any(value.endswith(".isScriptLoaded") for value in args):
    print("true")
elif any(value.endswith(".loadScript") for value in args):
    print("9")
"""


def fixture(directory: Path) -> tuple[dict[str, str], Path, Path, Path]:
    bin_dir = directory / "bin"
    bin_dir.mkdir()
    qdbus = bin_dir / "qdbus6"
    qdbus.write_text(FAKE_QDBUS, encoding="utf-8")
    qdbus.chmod(0o755)
    log = directory / "commands.log"
    log.write_text("", encoding="utf-8")
    plugin_root = directory / "plugins"
    plugin_dir = plugin_root / "kwin/plugins"
    plugin_dir.mkdir(parents=True)
    for plugin_id in ("seatgeistactivity", "seatgeistagentseat"):
        (plugin_dir / f"{plugin_id}.so").write_bytes(
            f"org.kde.kwin.PluginFactoryInterface{ABI}".encode()
        )
    data_home = directory / "data"
    bridge = data_home / "kwin/scripts/seatgeist-bridge/contents/code/main.js"
    bridge.parent.mkdir(parents=True)
    bridge.write_text("// test bridge\n", encoding="utf-8")
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{bin_dir}:{env['PATH']}",
            "COMMAND_LOG": str(log),
        }
    )
    return env, log, plugin_root, data_home


with tempfile.TemporaryDirectory(prefix="seatgeist-reload-kwin-") as temporary:
    env, log, plugin_root, data_home = fixture(Path(temporary))
    completed = subprocess.run(
        [
            str(HELPER),
            "agent-seat",
            "--plugin-root",
            str(plugin_root),
            "--data-home",
            str(data_home),
        ],
        check=True,
        text=True,
        capture_output=True,
        env=env,
    )
    result = json.loads(completed.stdout)
    assert result["loaded_after"] is True
    assert result["compositor_restarted"] is False
    commands = log.read_text(encoding="utf-8")
    assert ".UnloadPlugin seatgeistagentseat" in commands
    assert ".LoadPlugin seatgeistagentseat" in commands

with tempfile.TemporaryDirectory(prefix="seatgeist-reload-kwin-") as temporary:
    env, log, plugin_root, data_home = fixture(Path(temporary))
    completed = subprocess.run(
        [
            str(HELPER),
            "bridge",
            "--data-home",
            str(data_home),
        ],
        check=True,
        text=True,
        capture_output=True,
        env=env,
    )
    result = json.loads(completed.stdout)
    assert result["script_id"] == 9
    commands = log.read_text(encoding="utf-8")
    assert ".unloadScript seatgeist-bridge" in commands
    assert ".loadScript" in commands
    assert "/Scripting/Script9 org.kde.kwin.Script.run" in commands

with tempfile.TemporaryDirectory(prefix="seatgeist-reload-kwin-") as temporary:
    env, log, plugin_root, data_home = fixture(Path(temporary))
    env["KWIN_ABI"] = "6.4.4"
    completed = subprocess.run(
        [
            str(HELPER),
            "activity",
            "--plugin-root",
            str(plugin_root),
        ],
        check=False,
        text=True,
        capture_output=True,
        env=env,
    )
    assert completed.returncode != 0
    assert "does not match running KWin" in completed.stderr
    assert ".UnloadPlugin" not in log.read_text(encoding="utf-8")

with tempfile.TemporaryDirectory(prefix="seatgeist-reload-kwin-") as temporary:
    env, log, plugin_root, data_home = fixture(Path(temporary))
    completed = subprocess.run(
        [str(HELPER), "bridge", "--restart-compositor"],
        check=False,
        text=True,
        capture_output=True,
        env=env,
    )
    assert completed.returncode != 0
    assert "refusing compositor restart" in completed.stderr
    assert log.read_text(encoding="utf-8") == ""

print("test-reload-kwin-integration: ok")
