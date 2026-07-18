from __future__ import annotations

import subprocess
from typing import Any, Mapping

from computer_use_eval import EvalError
from nested_kde_contract import parse_unsigned_property


REMOTE_DESKTOP_SERVICE = "org.freedesktop.impl.portal.desktop.kde"
REMOTE_DESKTOP_OBJECT = "/org/freedesktop/portal/desktop"
REMOTE_DESKTOP_INTERFACE = "org.freedesktop.impl.portal.RemoteDesktop"
KEYBOARD_DEVICE = 1
POINTER_DEVICE = 2
TOUCHSCREEN_DEVICE = 4


def parse_methods(output: bytes) -> set[str]:
    methods: set[str] = set()
    for raw_line in output.decode("utf-8", errors="strict").splitlines():
        fields = raw_line.split()
        if len(fields) >= 2 and fields[0].startswith(".") and fields[1] == "method":
            methods.add(fields[0][1:])
    return methods


def sanitized_capabilities(
    version: int, available_devices: int, methods: set[str]
) -> dict[str, Any]:
    required_methods = {"CreateSession", "SelectDevices", "Start", "ConnectToEIS"}
    missing = sorted(required_methods - methods)
    if version < 2:
        raise EvalError("nested RemoteDesktop portal does not support ConnectToEIS")
    if available_devices & (KEYBOARD_DEVICE | POINTER_DEVICE) != (
        KEYBOARD_DEVICE | POINTER_DEVICE
    ):
        raise EvalError("nested RemoteDesktop portal lacks keyboard or pointer support")
    if missing:
        raise EvalError(
            "nested RemoteDesktop portal is missing required methods: "
            + ", ".join(missing)
        )
    return {
        "version": version,
        "available_device_types": available_devices,
        "keyboard": bool(available_devices & KEYBOARD_DEVICE),
        "pointer": bool(available_devices & POINTER_DEVICE),
        "touchscreen": bool(available_devices & TOUCHSCREEN_DEVICE),
        "connect_to_eis": True,
        "lifecycle_methods": True,
    }


def run_busctl(arguments: list[str], environment: Mapping[str, str]) -> bytes:
    result = subprocess.run(
        ["busctl", "--user", *arguments],
        env=dict(environment),
        capture_output=True,
        check=False,
        timeout=5,
    )
    if result.returncode != 0:
        message = result.stderr.decode("utf-8", errors="replace").strip()
        raise EvalError(message or "nested RemoteDesktop portal probe failed")
    return result.stdout


def probe_remote_desktop(environment: Mapping[str, str]) -> dict[str, Any]:
    def read_property(name: str) -> int:
        return parse_unsigned_property(
            run_busctl(
                [
                    "get-property",
                    REMOTE_DESKTOP_SERVICE,
                    REMOTE_DESKTOP_OBJECT,
                    REMOTE_DESKTOP_INTERFACE,
                    name,
                ],
                environment,
            )
        )

    introspection = run_busctl(
        [
            "introspect",
            REMOTE_DESKTOP_SERVICE,
            REMOTE_DESKTOP_OBJECT,
            REMOTE_DESKTOP_INTERFACE,
        ],
        environment,
    )
    return sanitized_capabilities(
        read_property("version"),
        read_property("AvailableDeviceTypes"),
        parse_methods(introspection),
    )
