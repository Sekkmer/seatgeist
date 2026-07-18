from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path
from typing import Mapping

from computer_use_eval import EvalError


BRIDGE_ID = "seatgeist-bridge"
SCREENCAST_INTERFACE = "zkde_screencast_unstable_v1"


def validate_bridge_source(source: Path) -> None:
    metadata_path = source / "metadata.json"
    entrypoint = source / "contents/code/main.js"
    if not metadata_path.is_file() or not entrypoint.is_file():
        raise EvalError("nested KWin bridge source is incomplete")
    try:
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as err:
        raise EvalError(f"nested KWin bridge metadata is invalid: {err}") from err
    plugin = metadata.get("KPlugin")
    if not isinstance(plugin, dict) or plugin.get("Id") != BRIDGE_ID:
        raise EvalError("nested KWin bridge metadata has the wrong plugin id")


def install_bridge(source: Path, paths: Mapping[str, Path]) -> Path:
    source = source.expanduser().resolve()
    validate_bridge_source(source)
    destination = paths["data"] / "kwin/scripts" / BRIDGE_ID
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination)
    kwinrc = paths["config"] / "kwinrc"
    kwinrc.write_text(
        "[Plugins]\nseatgeist-bridgeEnabled=true\n",
        encoding="utf-8",
    )
    kwinrc.chmod(0o600)
    return destination


def install_protocol_probe_desktop(paths: Mapping[str, Path]) -> Path:
    applications = paths["data"] / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    desktop = applications / "org.seatgeist.NestedProtocolProbe.desktop"
    desktop.write_text(
        "\n".join(
            (
                "[Desktop Entry]",
                "Type=Application",
                "Name=Seatgeist Nested Protocol Probe",
                "Exec=/usr/bin/wayland-info",
                f"X-KDE-Wayland-Interfaces={SCREENCAST_INTERFACE}",
                "NoDisplay=true",
                "",
            )
        ),
        encoding="utf-8",
    )
    # KService application metadata is non-secret and is conventionally
    # world-readable; the enclosing fixture tree remains mode 0700.
    desktop.chmod(0o644)
    return desktop


def rebuild_service_cache(environment: Mapping[str, str], log_path: Path) -> None:
    cache_environment = dict(environment)
    cache_environment["QT_QPA_PLATFORM"] = "offscreen"
    cache_environment.pop("WAYLAND_DISPLAY", None)
    with log_path.open("wb") as log:
        for key in ("HOME", "XDG_DATA_HOME", "XDG_DATA_DIRS", "XDG_CACHE_HOME"):
            log.write(f"{key}={environment.get(key, '<unset>')}\n".encode("utf-8"))
        log.flush()
        result = subprocess.run(
            ["kbuildsycoca6", "--noincremental"],
            env=cache_environment,
            stdout=log,
            stderr=subprocess.STDOUT,
            check=False,
            timeout=30,
        )
    if result.returncode != 0:
        raise EvalError("failed to build the nested KDE service cache")
