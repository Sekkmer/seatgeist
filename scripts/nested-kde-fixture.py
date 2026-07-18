#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from computer_use_eval import (
    ROOT,
    EvalError,
    unix_time_ms,
    workspace_revision,
    write_private_json,
)
from nested_kde_contract import (
    NestedKdeConfig,
    absolute_wayland_display,
    fixture_paths,
    isolated_environment,
    normalized_payload,
    prepare_fixture_directories,
)
from nested_kde_fixture import run_fixture
from nested_kde_assets import (
    install_bridge,
    install_protocol_probe_desktop,
    rebuild_service_cache,
)


DEFAULT_ROOT = ROOT / "target/seatgeist-nested-kde"
BUS_MARKER = "SEATGEIST_NESTED_KDE_PRIVATE_BUS"
HOST_RUNTIME_MARKER = "SEATGEIST_NESTED_KDE_HOST_RUNTIME"
HOST_WAYLAND_MARKER = "SEATGEIST_NESTED_KDE_HOST_WAYLAND"


def default_run_root() -> Path:
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    return DEFAULT_ROOT / f"fixture-{stamp}-{os.getpid()}"


def inside_private_bus() -> bool:
    return os.environ.get(BUS_MARKER) == "1"


def reexec_in_private_bus(arguments: list[str], root: Path) -> int:
    base = dict(os.environ)
    host_runtime = base.get("XDG_RUNTIME_DIR", "/run/user/1000")
    host_wayland = base.get("WAYLAND_DISPLAY", "wayland-0")
    bootstrap = NestedKdeConfig(root=root, socket_name="w")
    paths = prepare_fixture_directories(bootstrap)
    install_bridge(ROOT / "kwin/seatgeist-bridge", paths)
    environment = isolated_environment(bootstrap, paths, base)
    install_protocol_probe_desktop(paths)
    rebuild_service_cache(environment, paths["logs"] / "kservice-cache.log")
    environment[BUS_MARKER] = "1"
    environment[HOST_RUNTIME_MARKER] = host_runtime
    environment[HOST_WAYLAND_MARKER] = host_wayland
    environment["PIPEWIRE_RUNTIME_DIR"] = host_runtime
    return subprocess.run(
        [
            "dbus-run-session",
            "--",
            sys.executable,
            __file__,
            "--root",
            str(root.expanduser().resolve()),
            *arguments,
        ],
        env=environment,
        check=False,
    ).returncode


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Launch an isolated two-output KWin fixture and verify real KScreen "
            "non-zero-origin metadata. Visible mode opens nested output windows."
        )
    )
    parser.add_argument("--root", type=Path, default=default_run_root())
    parser.add_argument("--output-count", type=int, default=2)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--visible", action="store_true")
    parser.add_argument(
        "--operator-present",
        action="store_true",
        help="Required with --visible; confirms that nested output windows may open.",
    )
    parser.add_argument(
        "--host-wayland-display",
        default=os.environ.get(HOST_WAYLAND_MARKER, os.environ.get("WAYLAND_DISPLAY")),
    )
    parser.add_argument(
        "--host-pipewire-runtime",
        type=Path,
        default=Path(
            os.environ.get(
                HOST_RUNTIME_MARKER,
                os.environ.get("XDG_RUNTIME_DIR", "/run/user/1000"),
            )
        ),
    )
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.visible and not args.operator_present:
        parser.error("--visible requires --operator-present")

    if not inside_private_bus():
        raise SystemExit(reexec_in_private_bus(sys.argv[1:], args.root))

    root = args.root.expanduser().resolve()
    # The isolated runtime directory already provides uniqueness. Keep the
    # socket basename tiny because sockaddr_un is limited to 108 bytes.
    socket_name = "w"
    evidence_path = root / "evidence.json"
    evidence = {
        "type": "seatgeist_nested_kde_fixture_probe",
        "version": 1,
        "status": "running",
        "workspace": workspace_revision(),
        "started_unix_ms": unix_time_ms(),
        "ended_unix_ms": None,
        "mode": "visible" if args.visible else "headless",
        "layout": None,
        "portal": None,
        "bridge_loaded": False,
        "screencast_protocol_visible": False,
        "command_exit_code": None,
        "errors": [],
    }
    try:
        host_wayland_display = None
        if args.visible:
            host_wayland_display = absolute_wayland_display(
                args.host_wayland_display or "",
                Path(
                    os.environ.get(
                        HOST_RUNTIME_MARKER,
                        os.environ.get("XDG_RUNTIME_DIR", "/run/user/1000"),
                    )
                ),
            )
        result = run_fixture(
            NestedKdeConfig(
                root=root,
                socket_name=socket_name,
                output_count=args.output_count,
                width=args.width,
                height=args.height,
                visible=args.visible,
                host_wayland_display=host_wayland_display,
                host_pipewire_runtime=args.host_pipewire_runtime,
            ),
            normalized_payload(args.command),
            prepared=True,
        )
        evidence["layout"] = result.layout
        evidence["portal"] = result.portal
        evidence["bridge_loaded"] = result.bridge_loaded
        evidence["screencast_protocol_visible"] = result.screencast_protocol_visible
        evidence["command_exit_code"] = result.command_exit_code
        evidence["status"] = "passed" if result.command_exit_code == 0 else "failed"
    except (EvalError, OSError, subprocess.SubprocessError) as err:
        evidence["status"] = "failed"
        evidence["errors"].append(str(err))
    finally:
        evidence["ended_unix_ms"] = unix_time_ms()
        if root.exists():
            write_private_json(evidence_path, evidence)

    print(
        json.dumps(
            {
                "status": evidence["status"],
                "mode": evidence["mode"],
                "layout": evidence["layout"],
                "portal": evidence["portal"],
                "bridge_loaded": evidence["bridge_loaded"],
                "screencast_protocol_visible": evidence["screencast_protocol_visible"],
                "evidence": str(evidence_path),
                "errors": evidence["errors"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    if evidence["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
