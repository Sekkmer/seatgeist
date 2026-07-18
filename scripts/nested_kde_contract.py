from __future__ import annotations

import math
import os
import re
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence

from computer_use_eval import EvalError


SOCKET_PATTERN = re.compile(r"[A-Za-z0-9_.-]{1,80}")
SAFE_BASE_KEYS = {
    "DBUS_SESSION_BUS_ADDRESS",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LOGNAME",
    "PATH",
    "QT_PLUGIN_PATH",
    "QT_LOGGING_RULES",
    "QT_WAYLAND_RECONNECT",
    "SHELL",
    "SEATGEIST_NESTED_KDE_PRIVATE_BUS",
    "TERM",
    "TZ",
    "USER",
    "XDG_DATA_DIRS",
}


@dataclass(frozen=True)
class NestedKdeConfig:
    root: Path
    socket_name: str
    output_count: int = 2
    width: int = 1280
    height: int = 720
    visible: bool = False
    host_wayland_display: str | None = None
    host_pipewire_runtime: Path | None = None
    startup_timeout_seconds: float = 15.0
    require_screencast_portal: bool = True
    require_seatgeist_bridge: bool = True


@dataclass(frozen=True)
class NestedKdeResult:
    layout: dict[str, Any]
    portal: dict[str, int] | None
    bridge_loaded: bool
    screencast_protocol_visible: bool
    command_exit_code: int


def validate_config(config: NestedKdeConfig) -> None:
    if not SOCKET_PATTERN.fullmatch(config.socket_name):
        raise EvalError("nested Wayland socket name is invalid")
    if config.output_count < 2 or config.output_count > 8:
        raise EvalError("nested fixture requires between two and eight outputs")
    if config.width < 320 or config.height < 240:
        raise EvalError("nested output dimensions are too small")
    if config.startup_timeout_seconds <= 0:
        raise EvalError("nested fixture startup timeout must be positive")
    if config.visible and not config.host_wayland_display:
        raise EvalError("visible nested mode requires the host Wayland display")


def absolute_wayland_display(display: str, runtime_dir: Path) -> str:
    if not display:
        raise EvalError("host Wayland display is empty")
    path = Path(display)
    if path.is_absolute():
        return str(path)
    if "/" in display:
        raise EvalError("relative host Wayland display must be a socket basename")
    return str(runtime_dir.expanduser().resolve() / display)


def normalized_payload(arguments: Sequence[str]) -> tuple[str, ...]:
    payload = tuple(arguments)
    if payload[:1] == ("--",):
        payload = payload[1:]
    return payload


def fixture_paths(config: NestedKdeConfig) -> dict[str, Path]:
    root = config.root.expanduser().resolve()
    return {
        "root": root,
        "home": root / "home",
        "runtime": root / "runtime",
        "config": root / "config",
        "cache": root / "cache",
        "data": root / "data",
        "state": root / "state",
        "logs": root / "logs",
    }


def validate_socket_path(config: NestedKdeConfig, paths: Mapping[str, Path]) -> None:
    socket_path = paths["runtime"] / config.socket_name
    if len(os.fsencode(socket_path)) + 1 > 108:
        raise EvalError("nested Wayland socket path exceeds the Unix socket limit")


def prepare_fixture_directories(config: NestedKdeConfig) -> dict[str, Path]:
    validate_config(config)
    paths = fixture_paths(config)
    validate_socket_path(config, paths)
    paths["root"].mkdir(parents=True, exist_ok=False)
    paths["root"].chmod(0o700)
    for name, path in paths.items():
        if name != "root":
            path.mkdir(mode=0o700)
    return paths


def require_prepared_fixture_directories(
    config: NestedKdeConfig,
) -> dict[str, Path]:
    validate_config(config)
    paths = fixture_paths(config)
    validate_socket_path(config, paths)
    for path in paths.values():
        if not path.is_dir():
            raise EvalError("prepared nested fixture directory is missing")
        if stat.S_IMODE(path.stat().st_mode) & 0o077:
            raise EvalError("prepared nested fixture directory is not private")
    return paths


def isolated_environment(
    config: NestedKdeConfig,
    paths: Mapping[str, Path],
    base: Mapping[str, str],
) -> dict[str, str]:
    environment = {
        key: value
        for key, value in base.items()
        if key in SAFE_BASE_KEYS or key.startswith("LC_")
    }
    environment.update(
        {
            "HOME": str(paths["home"]),
            "XDG_RUNTIME_DIR": str(paths["runtime"]),
            "XDG_CONFIG_HOME": str(paths["config"]),
            "XDG_CACHE_HOME": str(paths["cache"]),
            "XDG_DATA_HOME": str(paths["data"]),
            "XDG_STATE_HOME": str(paths["state"]),
            "XDG_CURRENT_DESKTOP": "KDE",
            "XDG_SESSION_DESKTOP": "KDE",
            "XDG_SESSION_TYPE": "wayland",
            "XDG_CONFIG_DIRS": "/etc/xdg",
            "XDG_MENU_PREFIX": "plasma-",
            "KDE_FULL_SESSION": "true",
            "KDE_SESSION_VERSION": "6",
            "QT_QPA_PLATFORM": "wayland",
            "NO_AT_BRIDGE": "1",
            "QT_ACCESSIBILITY": "0",
            "QT_NO_XDG_DESKTOP_PORTAL": "1",
            "PLASMA_INTEGRATION_USE_PORTAL": "0",
            "GTK_A11Y": "none",
            "GDK_BACKEND": "wayland",
            "WAYLAND_DISPLAY": config.socket_name,
        }
    )
    if config.host_pipewire_runtime is not None:
        environment["PIPEWIRE_RUNTIME_DIR"] = str(
            config.host_pipewire_runtime.expanduser().resolve()
        )
    return environment


def kwin_command(config: NestedKdeConfig) -> list[str]:
    validate_config(config)
    command = ["kwin_wayland"]
    if config.visible:
        command.extend(["--wayland-display", str(config.host_wayland_display)])
    else:
        command.append("--virtual")
    command.extend(
        [
            "--width",
            str(config.width),
            "--height",
            str(config.height),
            "--output-count",
            str(config.output_count),
            "--socket",
            config.socket_name,
            "--no-lockscreen",
            "--no-global-shortcuts",
            "--no-kactivities",
        ]
    )
    return command


def sanitized_output_layout(document: Mapping[str, Any]) -> dict[str, Any]:
    raw_outputs = document.get("outputs")
    if not isinstance(raw_outputs, list):
        raise EvalError("kscreen output document is malformed")
    outputs: list[dict[str, Any]] = []
    for raw in raw_outputs:
        if not isinstance(raw, dict) or raw.get("enabled") is not True:
            continue
        position = raw.get("pos")
        scale = raw.get("scale")
        if not isinstance(position, dict):
            raise EvalError("enabled nested output has no position")
        x = position.get("x")
        y = position.get("y")
        if (
            not isinstance(x, int)
            or not isinstance(y, int)
            or not isinstance(scale, (int, float))
            or isinstance(scale, bool)
            or not math.isfinite(scale)
            or scale <= 0
        ):
            raise EvalError("enabled nested output has invalid geometry or scale")
        outputs.append({"logical_origin_x": x, "logical_origin_y": y, "scale": scale})
    outputs.sort(
        key=lambda output: (output["logical_origin_x"], output["logical_origin_y"])
    )
    return {
        "monitor_count": len(outputs),
        "has_nonzero_logical_origin": any(
            output["logical_origin_x"] != 0 or output["logical_origin_y"] != 0
            for output in outputs
        ),
        "outputs": outputs,
    }


def require_multi_output_layout(layout: Mapping[str, Any], expected: int) -> None:
    if layout.get("monitor_count") != expected:
        raise EvalError("nested KWin reported an unexpected output count")
    if layout.get("has_nonzero_logical_origin") is not True:
        raise EvalError("nested KWin reported no non-zero logical output origin")


def parse_unsigned_property(output: bytes) -> int:
    fields = output.decode("utf-8", errors="strict").strip().split()
    if len(fields) != 2 or fields[0] != "u" or not fields[1].isdigit():
        raise EvalError("portal returned a malformed unsigned property")
    return int(fields[1])


def portal_capabilities(version: int, sources: int, cursors: int) -> dict[str, int]:
    if version < 1:
        raise EvalError("KDE ScreenCast portal reported an invalid version")
    if sources & 2 == 0:
        raise EvalError("KDE ScreenCast portal has no window source capability")
    return {
        "version": version,
        "available_source_types": sources,
        "available_cursor_modes": cursors,
    }
