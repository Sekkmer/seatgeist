from __future__ import annotations

import os
import subprocess
import sys
import time
from contextlib import ExitStack
from pathlib import Path
from typing import Any, Mapping

from computer_use_eval import EvalError
from nested_seatgeist_probe import (
    daemon_socket_path,
    run_cli,
    stop_process,
    wait_for_probe,
)


TARGET_TITLE = "Seatgeist Step 12 Firefox"


def prepare_firefox_profile(profile: Path) -> None:
    profile.mkdir(mode=0o700)
    user_js = profile / "user.js"
    user_js.write_text(
        "\n".join(
            (
                'user_pref("browser.aboutwelcome.enabled", false);',
                'user_pref("browser.shell.checkDefaultBrowser", false);',
                'user_pref("browser.startup.firstrunSkipsHomepage", true);',
                'user_pref("browser.tabs.inTitlebar", 0);',
                'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
                'user_pref("toolkit.telemetry.enabled", false);',
                "",
            )
        ),
        encoding="utf-8",
    )
    user_js.chmod(0o600)


def find_fixture_windows(response: Mapping[str, Any]) -> tuple[dict[str, Any], int]:
    windows = response.get("data")
    if response.get("type") != "windows" or not isinstance(windows, list):
        raise EvalError("nested daemon window response is malformed")
    targets = []
    helper_count = 0
    for window in windows:
        if not isinstance(window, dict):
            continue
        app_id = str(window.get("app_id") or "").lower()
        title = str(window.get("title") or "")
        if "firefox" in app_id and TARGET_TITLE in title:
            window_id = window.get("id")
            pid = window.get("pid")
            if isinstance(window_id, str) and window_id and isinstance(pid, int):
                targets.append({"window_id": window_id, "pid": pid})
        if "konsole" in app_id:
            helper_count += 1
    if len(targets) != 1:
        raise EvalError("nested Firefox target is missing or ambiguous")
    if helper_count < 1:
        raise EvalError("nested helper window is missing")
    return targets[0], helper_count


def wait_for_fixture_windows(
    daemon: subprocess.Popen[bytes],
    apps: tuple[subprocess.Popen[bytes], ...],
    cli: Path,
    socket: Path,
    timeout_seconds: float,
) -> tuple[dict[str, Any], int]:
    deadline = time.monotonic() + timeout_seconds
    last_error = "nested fixture windows did not become ready"
    while time.monotonic() < deadline:
        if daemon.poll() is not None:
            raise EvalError(f"nested daemon exited with code {daemon.returncode}")
        exited = [process.returncode for process in apps if process.poll() is not None]
        if exited:
            raise EvalError(f"nested fixture application exited early: {exited}")
        try:
            return find_fixture_windows(run_cli(cli, socket, "windows"))
        except (EvalError, OSError, subprocess.SubprocessError) as err:
            last_error = str(err)
        time.sleep(0.1)
    raise EvalError(f"nested fixture window readiness timed out: {last_error}")


def daemon_command(daemon: Path, socket: Path, state: Path) -> list[str]:
    return [
        str(daemon),
        "--socket",
        str(socket),
        "--journal",
        str(state / "seatgeistd-journal.jsonl"),
        "--capture-restore-file",
        str(state / "capture-restore.json"),
    ]


def firefox_command(firefox: Path, profile: Path, fixture_url: str) -> list[str]:
    return [
        str(firefox),
        "--no-remote",
        "--new-instance",
        "--profile",
        str(profile),
        "--new-window",
        fixture_url,
    ]


def helper_command(repository: Path) -> list[str]:
    return [
        "konsole",
        "--separate",
        "--builtin-profile",
        "--notransparency",
        "--workdir",
        str(repository),
    ]


def run_nested_retained_capture(
    repository: Path,
    runtime: Path,
    state: Path,
    logs: Path,
    *,
    probe_only: bool,
    scenarios: tuple[str, ...] = (),
    timeout_seconds: float = 20.0,
) -> dict[str, Any]:
    daemon_binary = repository / "target/debug/seatgeistd"
    cli = repository / "target/debug/seatgeist-cli"
    firefox = Path("/usr/lib/firefox/firefox")
    fixture_url = (repository / "examples/live-eval/firefox/index.html").as_uri()
    socket = daemon_socket_path(runtime)
    profile = state / "firefox-profile"
    prepare_firefox_profile(profile)
    environment = dict(os.environ)
    environment.update({"MOZ_ENABLE_WAYLAND": "1", "MOZ_DBUS_REMOTE": "0"})

    processes: list[subprocess.Popen[bytes]] = []
    with ExitStack() as stack:

        def launch(
            name: str, command: list[str], env: Mapping[str, str]
        ) -> subprocess.Popen[bytes]:
            log = stack.enter_context((logs / f"{name}.log").open("wb"))
            process = subprocess.Popen(
                command,
                env=dict(env),
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            processes.append(process)
            return process

        daemon = launch(
            "seatgeistd", daemon_command(daemon_binary, socket, state), environment
        )
        try:
            monitors, bridge = wait_for_probe(daemon, cli, socket, timeout_seconds)
            helper = launch("konsole", helper_command(repository), environment)
            firefox_process = launch(
                "firefox", firefox_command(firefox, profile, fixture_url), environment
            )
            target, helper_count = wait_for_fixture_windows(
                daemon,
                (helper, firefox_process),
                cli,
                socket,
                timeout_seconds,
            )
            if probe_only:
                return {
                    "status": "passed",
                    "monitors": monitors,
                    "bridge": bridge,
                    "target_found": True,
                    "target_pid_present": target["pid"] > 0,
                    "helper_window_count": helper_count,
                    "runner_exit_code": None,
                }

            print(
                "Nested Firefox and helper Konsole are ready. Select the Firefox "
                "window in the nested KDE chooser, then follow the scenario prompts."
            )
            runner_command = [
                sys.executable,
                str(repository / "scripts/retained-capture-eval.py"),
                "--window-id",
                target["window_id"],
                "--cli",
                str(cli),
                "--socket",
                str(socket),
                "--output-dir",
                str(state / "retained-capture"),
                "--require-multi-output-nonzero-origin",
            ]
            for scenario in scenarios:
                runner_command.extend(("--scenario", scenario))
            runner = subprocess.run(runner_command, env=environment, check=False)
            return {
                "status": "passed" if runner.returncode == 0 else "failed",
                "monitors": monitors,
                "bridge": bridge,
                "target_found": True,
                "target_pid_present": target["pid"] > 0,
                "helper_window_count": helper_count,
                "selected_scenarios": list(scenarios),
                "runner_exit_code": runner.returncode,
            }
        finally:
            for process in reversed(processes):
                stop_process(process)
