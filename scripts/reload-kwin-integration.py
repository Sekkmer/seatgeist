#!/usr/bin/env python3
"""Reload only a verified Seatgeist KWin integration without restarting KWin."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
from pathlib import Path


KWIN_SERVICE = "org.kde.KWin"
PLUGIN_INTERFACE = "org.kde.KWin.Plugins"
SCRIPTING_INTERFACE = "org.kde.kwin.Scripting"
BRIDGE_ID = "seatgeist-bridge"
PLUGIN_IDS = {
    "activity": "seatgeistactivity",
    "agent-seat": "seatgeistagentseat",
}
PLUGIN_ABI_RE = re.compile(rb"org\.kde\.kwin\.PluginFactoryInterface(\d+\.\d+\.\d+)")
KWIN_VERSION_RE = re.compile(r"^KWin version:\s*(\d+\.\d+\.\d+)\s*$", re.MULTILINE)


def run(command: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, check=check, text=True, capture_output=True)


def qdbus_call(qdbus: str, path: str, method: str, *arguments: str) -> str:
    completed = run([qdbus, KWIN_SERVICE, path, method, *arguments], check=False)
    if completed.returncode != 0:
        detail = completed.stderr.strip() or f"exit status {completed.returncode}"
        raise SystemExit(f"KWin D-Bus call {method} failed: {detail}")
    return completed.stdout.strip()


def values(output: str) -> set[str]:
    return {
        value.strip().strip('"')
        for line in output.splitlines()
        for value in line.replace(",", " ").split()
        if value.strip().strip('"')
    }


def plugin_abi(path: Path) -> str | None:
    try:
        content = path.read_bytes()
    except OSError:
        return None
    match = PLUGIN_ABI_RE.search(content)
    return match.group(1).decode("ascii") if match else None


def running_kwin_abi(qdbus: str) -> str | None:
    support = qdbus_call(qdbus, "/KWin", "supportInformation")
    match = KWIN_VERSION_RE.search(support)
    return match.group(1) if match else None


def reload_script(qdbus: str, data_home: Path, dry_run: bool) -> dict[str, object]:
    installed_main = (
        data_home
        / "kwin"
        / "scripts"
        / BRIDGE_ID
        / "contents"
        / "code"
        / "main.js"
    )
    if not installed_main.is_file():
        raise SystemExit(f"installed KWin bridge entry point is missing: {installed_main}")
    loaded_before = (
        qdbus_call(
            qdbus,
            "/Scripting",
            f"{SCRIPTING_INTERFACE}.isScriptLoaded",
            BRIDGE_ID,
        ).lower()
        == "true"
    )
    if dry_run:
        return {
            "component": "bridge",
            "kind": "script",
            "loaded_before": loaded_before,
            "would_reload": True,
        }
    if loaded_before:
        qdbus_call(
            qdbus,
            "/Scripting",
            f"{SCRIPTING_INTERFACE}.unloadScript",
            BRIDGE_ID,
        )
    script_id_text = qdbus_call(
        qdbus,
        "/Scripting",
        f"{SCRIPTING_INTERFACE}.loadScript",
        str(installed_main),
        BRIDGE_ID,
    )
    try:
        script_id = int(script_id_text)
    except ValueError as error:
        raise SystemExit(f"KWin returned an invalid script id: {script_id_text!r}") from error
    if script_id < 0:
        raise SystemExit(f"KWin refused to load {BRIDGE_ID}: script id {script_id}")
    qdbus_call(qdbus, f"/Scripting/Script{script_id}", "org.kde.kwin.Script.run")
    loaded_after = (
        qdbus_call(
            qdbus,
            "/Scripting",
            f"{SCRIPTING_INTERFACE}.isScriptLoaded",
            BRIDGE_ID,
        ).lower()
        == "true"
    )
    if not loaded_after:
        raise SystemExit("KWin bridge did not report loaded after the script-only reload")
    return {
        "component": "bridge",
        "kind": "script",
        "loaded_before": loaded_before,
        "loaded_after": loaded_after,
        "script_id": script_id,
    }


def reload_binary_plugin(
    qdbus: str,
    component: str,
    plugin_root: Path,
    dry_run: bool,
) -> dict[str, object]:
    plugin_id = PLUGIN_IDS[component]
    plugin = plugin_root / "kwin" / "plugins" / f"{plugin_id}.so"
    if not plugin.is_file():
        raise SystemExit(f"installed KWin plugin is missing: {plugin}")
    embedded_abi = plugin_abi(plugin)
    compositor_abi = running_kwin_abi(qdbus)
    if embedded_abi is None or compositor_abi is None:
        raise SystemExit(
            "could not prove the installed plugin ABI matches the running compositor; "
            "refusing dynamic reload"
        )
    if embedded_abi != compositor_abi:
        raise SystemExit(
            f"plugin ABI {embedded_abi} does not match running KWin {compositor_abi}; "
            "use a normal logout/login after rebuilding instead"
        )
    available = values(
        qdbus_call(qdbus, "/Plugins", f"{PLUGIN_INTERFACE}.AvailablePlugins")
    )
    loaded = values(qdbus_call(qdbus, "/Plugins", f"{PLUGIN_INTERFACE}.LoadedPlugins"))
    if plugin_id not in available:
        raise SystemExit(
            f"KWin does not advertise {plugin_id}; refusing to mutate compositor plugins"
        )
    loaded_before = plugin_id in loaded
    if dry_run:
        return {
            "component": component,
            "kind": "binary_plugin",
            "plugin_id": plugin_id,
            "abi": embedded_abi,
            "loaded_before": loaded_before,
            "would_reload": True,
        }
    if loaded_before:
        qdbus_call(qdbus, "/Plugins", f"{PLUGIN_INTERFACE}.UnloadPlugin", plugin_id)
    load_result = qdbus_call(
        qdbus, "/Plugins", f"{PLUGIN_INTERFACE}.LoadPlugin", plugin_id
    )
    if load_result.lower() not in {"true", "1"}:
        raise SystemExit(f"KWin refused to load {plugin_id}: {load_result!r}")
    loaded_after = plugin_id in values(
        qdbus_call(qdbus, "/Plugins", f"{PLUGIN_INTERFACE}.LoadedPlugins")
    )
    if not loaded_after:
        raise SystemExit(f"KWin did not report {plugin_id} loaded after reload")
    return {
        "component": component,
        "kind": "binary_plugin",
        "plugin_id": plugin_id,
        "abi": embedded_abi,
        "loaded_before": loaded_before,
        "loaded_after": loaded_after,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Safely reload one installed Seatgeist KWin script/plugin in place. "
            "This helper never restarts the compositor."
        )
    )
    parser.add_argument("component", choices=("bridge", "activity", "agent-seat"))
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--restart-compositor",
        action="store_true",
        help="always refused; use a normal logout/login when an ABI change requires it",
    )
    parser.add_argument(
        "--plugin-root",
        type=Path,
        default=Path.home() / ".local" / "lib" / "qt6" / "plugins",
    )
    parser.add_argument(
        "--data-home",
        type=Path,
        default=Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")),
    )
    args = parser.parse_args()
    if args.restart_compositor:
        raise SystemExit(
            "refusing compositor restart: reload one verified Seatgeist component, "
            "or use a normal logout/login for an ABI transition"
        )
    qdbus = shutil.which("qdbus6")
    if qdbus is None:
        raise SystemExit("qdbus6 is required for a live KWin integration reload")
    if args.component == "bridge":
        result = reload_script(qdbus, args.data_home, args.dry_run)
    else:
        result = reload_binary_plugin(
            qdbus, args.component, args.plugin_root, args.dry_run
        )
    result.update(
        {
            "type": "seatgeist_kwin_integration_reload",
            "version": 1,
            "dry_run": args.dry_run,
            "compositor_restarted": False,
        }
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
