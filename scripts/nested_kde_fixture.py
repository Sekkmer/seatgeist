from __future__ import annotations

import json
import os
import signal
import subprocess
import time
from typing import Any, Mapping, Sequence

from computer_use_eval import EvalError
from nested_kde_contract import (
    NestedKdeConfig,
    NestedKdeResult,
    isolated_environment,
    kwin_command,
    parse_unsigned_property,
    portal_capabilities,
    prepare_fixture_directories,
    require_multi_output_layout,
    require_prepared_fixture_directories,
    sanitized_output_layout,
)


def has_screencast_protocol(output: bytes) -> bool:
    return b"interface: 'zkde_screencast_unstable_v1'" in output


def wait_for_screencast_protocol(
    process: subprocess.Popen[bytes],
    environment: Mapping[str, str],
    timeout_seconds: float,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error = "authorized screencast protocol probe did not become ready"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise EvalError(
                f"nested KWin exited during protocol startup with code {process.returncode}"
            )
        result = subprocess.run(
            ["wayland-info"],
            env=dict(environment),
            capture_output=True,
            check=False,
            timeout=3,
        )
        if result.returncode == 0 and has_screencast_protocol(result.stdout):
            return
        if result.stderr:
            last_error = result.stderr.decode("utf-8", errors="replace").strip()
        time.sleep(0.1)
    raise EvalError(f"KWin screencast protocol readiness timed out: {last_error}")


def read_portal_property(environment: Mapping[str, str], property_name: str) -> int:
    result = subprocess.run(
        [
            "busctl",
            "--user",
            "get-property",
            "org.freedesktop.impl.portal.desktop.kde",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.impl.portal.ScreenCast",
            property_name,
        ],
        env=dict(environment),
        capture_output=True,
        check=False,
        timeout=3,
    )
    if result.returncode != 0:
        message = result.stderr.decode("utf-8", errors="replace").strip()
        raise EvalError(message or f"portal property {property_name} is unavailable")
    return parse_unsigned_property(result.stdout)


def wait_for_screencast_portal(
    process: subprocess.Popen[bytes],
    environment: Mapping[str, str],
    timeout_seconds: float,
) -> dict[str, int]:
    deadline = time.monotonic() + timeout_seconds
    last_error = "KDE ScreenCast portal did not become ready"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise EvalError(
                f"nested KWin exited during portal startup with code {process.returncode}"
            )
        try:
            return portal_capabilities(
                read_portal_property(environment, "version"),
                read_portal_property(environment, "AvailableSourceTypes"),
                read_portal_property(environment, "AvailableCursorModes"),
            )
        except (EvalError, subprocess.SubprocessError) as err:
            last_error = str(err)
        time.sleep(0.1)
    raise EvalError(f"KDE ScreenCast portal readiness timed out: {last_error}")


def wait_for_bridge(
    process: subprocess.Popen[bytes],
    environment: Mapping[str, str],
    timeout_seconds: float,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error = "Seatgeist KWin bridge did not become ready"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise EvalError(
                f"nested KWin exited during bridge startup with code {process.returncode}"
            )
        result = subprocess.run(
            [
                "qdbus6",
                "org.kde.KWin",
                "/Scripting",
                "org.kde.kwin.Scripting.isScriptLoaded",
                "seatgeist-bridge",
            ],
            env=dict(environment),
            capture_output=True,
            check=False,
            timeout=3,
        )
        if result.returncode == 0 and result.stdout.strip().lower() == b"true":
            return
        if result.stderr:
            last_error = result.stderr.decode("utf-8", errors="replace").strip()
        time.sleep(0.1)
    raise EvalError(f"Seatgeist KWin bridge readiness timed out: {last_error}")


def wait_for_layout(
    process: subprocess.Popen[bytes],
    environment: Mapping[str, str],
    timeout_seconds: float,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    last_error = "KWin did not become ready"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise EvalError(
                f"nested KWin exited during startup with code {process.returncode}"
            )
        result = subprocess.run(
            ["kscreen-doctor", "-j"],
            env=dict(environment),
            capture_output=True,
            check=False,
            timeout=3,
        )
        if result.returncode == 0:
            try:
                return sanitized_output_layout(json.loads(result.stdout))
            except (EvalError, json.JSONDecodeError) as err:
                last_error = str(err)
        elif result.stderr:
            last_error = result.stderr.decode("utf-8", errors="replace").strip()
        time.sleep(0.1)
    raise EvalError(f"nested KWin readiness timed out: {last_error}")


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def run_fixture(
    config: NestedKdeConfig,
    command: Sequence[str] = (),
    *,
    prepared: bool = False,
) -> NestedKdeResult:
    paths = (
        require_prepared_fixture_directories(config)
        if prepared
        else prepare_fixture_directories(config)
    )
    environment = isolated_environment(config, paths, os.environ)
    log_path = paths["logs"] / "kwin.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            kwin_command(config),
            env=environment,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            layout = wait_for_layout(
                process, environment, config.startup_timeout_seconds
            )
            require_multi_output_layout(layout, config.output_count)
            wait_for_screencast_protocol(
                process, environment, config.startup_timeout_seconds
            )
            portal = None
            if config.require_screencast_portal:
                portal = wait_for_screencast_portal(
                    process, environment, config.startup_timeout_seconds
                )
            bridge_loaded = False
            if config.require_seatgeist_bridge:
                wait_for_bridge(process, environment, config.startup_timeout_seconds)
                bridge_loaded = True
            exit_code = 0
            if command:
                exit_code = subprocess.run(
                    list(command), env=environment, check=False
                ).returncode
            return NestedKdeResult(layout, portal, bridge_loaded, True, exit_code)
        finally:
            stop_process(process)
