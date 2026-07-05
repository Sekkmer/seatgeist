#!/usr/bin/env python3
"""Validate repo-shipped systemd, udev, and polkit install assets."""

from __future__ import annotations

import configparser
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def fail(message: str) -> None:
    raise SystemExit(f"install asset validation failed: {message}")


def read_systemd_unit(path: Path) -> configparser.ConfigParser:
    if not path.is_file():
        fail(f"{path} is missing")
    parser = configparser.ConfigParser(interpolation=None)
    parser.optionxform = str
    try:
        with path.open(encoding="utf-8") as handle:
            parser.read_file(handle)
    except configparser.Error as err:
        fail(f"{path} is not a valid systemd-style unit: {err}")
    return parser


def require_unit_value(
    parser: configparser.ConfigParser, path: Path, section: str, key: str, expected: str
) -> None:
    if not parser.has_section(section):
        fail(f"{path} is missing [{section}]")
    actual = parser.get(section, key, fallback=None)
    if actual != expected:
        fail(f"{path} [{section}] {key} expected {expected!r}, got {actual!r}")


def validate_systemd(root: Path) -> None:
    service_path = root / "systemd" / "plasma-pilotd.service"
    socket_path = root / "systemd" / "plasma-pilotd.socket"
    service = read_systemd_unit(service_path)
    socket = read_systemd_unit(socket_path)

    require_unit_value(
        service,
        service_path,
        "Service",
        "ExecStart",
        "%h/.cargo/bin/plasma-pilotd --socket %t/plasma-pilot/plasma-pilotd.sock",
    )
    require_unit_value(service, service_path, "Service", "Restart", "on-failure")
    require_unit_value(service, service_path, "Service", "NoNewPrivileges", "true")
    require_unit_value(service, service_path, "Install", "WantedBy", "default.target")
    if service.has_option("Service", "User") or service.has_option("Service", "Group"):
        fail(f"{service_path} must remain user-scoped and not set User/Group")

    require_unit_value(
        socket,
        socket_path,
        "Socket",
        "ListenStream",
        "%t/plasma-pilot/plasma-pilotd.sock",
    )
    require_unit_value(socket, socket_path, "Socket", "SocketMode", "0600")
    require_unit_value(socket, socket_path, "Socket", "DirectoryMode", "0700")
    require_unit_value(socket, socket_path, "Install", "WantedBy", "sockets.target")


def validate_udev(root: Path) -> None:
    path = root / "udev" / "99-plasma-pilot-uinput.rules"
    if not path.is_file():
        fail(f"{path} is missing")
    rules = [
        line.strip()
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    expected = 'KERNEL=="uinput", GROUP="uinput", MODE="0660", OPTIONS+="static_node=uinput"'
    if rules != [expected]:
        fail(f"{path} must contain exactly the narrow uinput rule")


def child_text(element: ET.Element, tag: str) -> str | None:
    child = element.find(tag)
    if child is None or child.text is None:
        return None
    return child.text.strip()


def validate_polkit(root: Path) -> None:
    path = root / "polkit" / "org.plasmapilot.policy"
    if not path.is_file():
        fail(f"{path} is missing")
    try:
        tree = ET.parse(path)
    except ET.ParseError as err:
        fail(f"{path} is not valid XML: {err}")

    root_element = tree.getroot()
    if root_element.tag != "policyconfig":
        fail(f"{path} root element must be policyconfig")
    if child_text(root_element, "vendor") != "PlasmaPilot":
        fail(f"{path} vendor must be PlasmaPilot")

    actions = root_element.findall("action")
    if len(actions) != 1:
        fail(f"{path} must define exactly one placeholder action")
    action = actions[0]
    if action.get("id") != "org.plasmapilot.control-input":
        fail(f"{path} action id must be org.plasmapilot.control-input")
    if not child_text(action, "description") or not child_text(action, "message"):
        fail(f"{path} action must include description and message")
    defaults = action.find("defaults")
    if defaults is None:
        fail(f"{path} action must include defaults")
    expected_defaults = {
        "allow_any": "no",
        "allow_inactive": "no",
        "allow_active": "auth_admin_keep",
    }
    for tag, expected in expected_defaults.items():
        if child_text(defaults, tag) != expected:
            fail(f"{path} defaults {tag} expected {expected!r}")


def main() -> None:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    if not root.is_dir():
        fail(f"{root} is not a repository directory")
    validate_systemd(root)
    validate_udev(root)
    validate_polkit(root)
    print(f"install asset validation passed: {root}")


if __name__ == "__main__":
    main()
