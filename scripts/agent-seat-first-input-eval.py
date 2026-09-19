#!/usr/bin/env python3
"""Test the built plugin on a private bus/headless KWin, never the live desktop."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import selectors
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from computer_use_eval import ROOT
from nested_kde_assets import install_bridge, install_protocol_probe_desktop, rebuild_service_cache
from nested_kde_contract import NestedKdeConfig, fixture_paths, isolated_environment, prepare_fixture_directories
from nested_kde_fixture import run_fixture
from nested_seatgeist_probe import run_cli, stop_process, wait_for_probe


class McpClient:
    """A real independent stdio server, with its own trusted Unix peer PID."""
    def __init__(self, socket: Path):
        self.process = subprocess.Popen([
            str(ROOT / "target/debug/seatgeist-mcp"), "--stdio", "--socket", str(socket),
        ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
           text=True, start_new_session=True)
        self.sequence = 0

    def call(self, name: str, arguments: dict, error: str | None = None) -> dict:
        self.sequence += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.sequence,
            "method": "tools/call", "params": {"name": name, "arguments": arguments}}) + "\n")
        self.process.stdin.flush()
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout, selectors.EVENT_READ)
            if not selector.select(15):
                raise RuntimeError(f"MCP {self.process.pid} did not reply to {name}")
        response = json.loads(self.process.stdout.readline())
        result = response.get("result", {})
        failed = "error" in response or result.get("isError", False)
        if error:
            if not failed or error not in json.dumps(response):
                raise RuntimeError(f"expected {error}: {response}")
        elif failed:
            raise RuntimeError(f"MCP action failed: {response}")
        return result.get("structuredContent", {}).get("data", {})

    def open(self, window: dict) -> str:
        return self.call("seatgeist.window_session", {"operation": "open",
            "requested_window_id": window["id"]})["session_id"]

    def close(self, session: str, error: str | None = None) -> None:
        self.call("seatgeist.window_session", {"operation": "close", "session_id": session}, error)

    def key(self, session: str, key: str, error: str | None = None) -> None:
        self.call("seatgeist.key_combo", {"session_id": session, "combo": key}, error)


def exercise(root: Path) -> None:
    paths = fixture_paths(NestedKdeConfig(root=root, socket_name="w"))
    socket = paths["runtime"] / "d"
    cli = ROOT / "target/debug/seatgeist-cli"
    processes = []
    with (paths["logs"] / "daemon.log").open("wb") as daemon_log, (paths["logs"] / "app.log").open("wb") as app_log:
        try:
            daemon_command = [
                str(ROOT / "target/debug/seatgeistd"), "--socket", str(socket),
                "--journal", str(paths["state"] / "journal.jsonl"),
                "--capture-restore-file", str(paths["state"] / "restore.json"),
                "--input-backend", "kwin_agent_seat", "--allow-control",
            ]
            daemon = subprocess.Popen(daemon_command, stdout=daemon_log, stderr=subprocess.STDOUT, start_new_session=True)
            processes.append(daemon)
            wait_for_probe(daemon, cli, socket, 15)
            app = subprocess.Popen([str(ROOT / "target/kwin-seatgeist-agent-seat/focus_fixture")],
                                   stdout=app_log, stderr=subprocess.STDOUT, start_new_session=True)
            processes.append(app)
            deadline = time.monotonic() + 15
            target = None
            while time.monotonic() < deadline:
                windows = run_cli(cli, socket, "windows")["data"]
                target = next((w for w in windows if w["title"] == "Seatgeist first-input target"), None)
                active = run_cli(cli, socket, "active-window")["data"]
                if target and active and active["title"] == "Seatgeist physical-focus sentinel":
                    break
                time.sleep(0.05)
            else:
                raise RuntimeError("fixture windows/focus did not become ready")
            original_focus = active["id"]
            second = next(w for w in windows if w["title"] == "Seatgeist second-input target")
            status = run_cli(cli, socket, "capture", "open", "--requested-window-id", target["id"])
            session = status["data"]["session_id"]
            try:
                # No warm-up key, click, retry, or arbitrary seat startup delay.
                run_cli(cli, socket, "input", "key-combo", "7", "--session-id", session)
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    content = (paths["logs"] / "app.log").read_text()
                    if "target=7\n" in content:
                        break
                    time.sleep(0.02)
                else:
                    raise RuntimeError("first key was not delivered to the target")
                if "sentinel=" in content:
                    raise RuntimeError("input leaked into the physical-focus sentinel")
                if run_cli(cli, socket, "active-window")["data"]["id"] != original_focus:
                    raise RuntimeError("agent input stole physical workspace focus")
            finally:
                run_cli(cli, socket, "capture", "close", "--session-id", session)

            # Reuse the lane across two windows of one client, then switch back.
            for window, key, expected in [(second, "8", "second=8\n"), (target, "9", "target=79\n")]:
                session = run_cli(cli, socket, "capture", "open", "--requested-window-id", window["id"])["data"]["session_id"]
                try:
                    run_cli(cli, socket, "input", "key-combo", key, "--session-id", session)
                    deadline = time.monotonic() + 3
                    while time.monotonic() < deadline:
                        content = (paths["logs"] / "app.log").read_text()
                        if expected in content:
                            break
                        time.sleep(0.02)
                    else:
                        raise RuntimeError("key failed after same-client target switch")
                    if "sentinel=" in content or run_cli(cli, socket, "active-window")["data"]["id"] != original_focus:
                        raise RuntimeError("target switch affected the physical-focus sentinel")
                finally:
                    run_cli(cli, socket, "capture", "close", "--session-id", session)
            # Concurrent independent MCP owners, including the same-client case
            # in this Qt fixture. Neither may use/close the other's
            # session, claim its target, or disturb the physical seat.
            clients = [McpClient(socket), McpClient(socket)]
            processes.extend(client.process for client in clients)
            first_session = clients[0].open(target)
            second_session = clients[1].open(second)
            clients[1].call("seatgeist.window_session", {"operation": "open",
                "requested_window_id": target["id"]}, "agent_target_in_use")
            clients[1].close(first_session, "owner mismatch")
            clients[1].key(first_session, "0", "owner mismatch")

            def parallel_keys(left: str, right: str) -> None:
                with ThreadPoolExecutor(max_workers=2) as pool:
                    futures = [pool.submit(clients[0].key, first_session, left),
                               pool.submit(clients[1].key, second_session, right)]
                    for future in futures:
                        future.result(timeout=15)

            def check_text(left: str, right: str) -> str:
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    content = (paths["logs"] / "app.log").read_text()
                    if f"target={left}\n" in content and f"second={right}\n" in content:
                        break
                    time.sleep(0.02)
                else:
                    raise RuntimeError(f"concurrent input was lost/misrouted: {content}")
                if "sentinel=" in content or run_cli(cli, socket, "active-window")["data"]["id"] != original_focus:
                    raise RuntimeError("concurrent owners affected the physical-focus sentinel")
                return content

            parallel_keys("1", "2")
            content = check_text("791", "82")
            seat_globals = {line.split("=", 1)[1] for line in content.splitlines() if line.startswith("seat-global=")}
            # Repeated owner churn must reuse released seats, not evict the
            # remaining active owner or keep announcing new wl_seat globals.
            for _ in range(5):
                clients[0].close(first_session)
                first_session = clients[0].open(target)
                clients[0].key(first_session, "Shift")
            parallel_keys("3", "4")
            check_text("7913", "824")
            clients[0].close(first_session)
            clients[1].close(second_session)

            # Keep KWin, GUI clients, and both MCP processes alive. Only the
            # replaceable Rust daemon is restarted; retained sessions must reopen.
            stop_process(daemon)
            daemon = subprocess.Popen(daemon_command, stdout=daemon_log, stderr=subprocess.STDOUT, start_new_session=True)
            processes.append(daemon)
            wait_for_probe(daemon, cli, socket, 15)
            first_session = clients[0].open(target)
            second_session = clients[1].open(second)
            parallel_keys("5", "6")
            content = check_text("79135", "8246")
            after = {line.split("=", 1)[1] for line in content.splitlines() if line.startswith("seat-global=")}
            removed = {line.split("=", 1)[1] for line in content.splitlines() if line.startswith("global-removed=")}
            if after != seat_globals or removed.intersection(seat_globals):
                raise RuntimeError("daemon restart/owner churn changed the native seat globals")
            # Modifiers belong to a lane, not the application or the next
            # owner that reuses its native seat after release.
            parallel_keys("Shift+a", "b")
            check_text("79135A", "8246b")
            clients[0].close(first_session)
            first_session = clients[0].open(target)
            clients[0].key(first_session, "a")
            check_text("79135Aa", "8246b")
            clients[0].close(first_session)
            clients[1].close(second_session)
            print(json.dumps({"first_key": "passed", "same_client_target_switch": "passed",
                              "independent_mcp_owners": "passed", "owner_isolation": "passed",
                              "modifier_isolation_and_recycling": "passed",
                              "daemon_restart": "passed", "seat_globals_preserved": True,
                              "physical_focus_preserved": True}), flush=True)
        finally:
            for process in reversed(processes):
                stop_process(process)


def main() -> None:
    if len(sys.argv) == 4 and sys.argv[1] == "--exercise":
        if sys.argv[3].startswith("firefox"):
            from agent_seat_firefox_eval import exercise_firefox
            exercise_firefox(Path(sys.argv[2]), McpClient, sys.argv[3] == "firefox-shared-client")
            return
        exercise(Path(sys.argv[2]))
        return
    if len(sys.argv) == 4 and sys.argv[1] == "--private":
        root = Path(sys.argv[2])
        result = run_fixture(NestedKdeConfig(root=root, socket_name="w", require_screencast_portal=False),
                             [sys.executable, __file__, "--exercise", str(root), sys.argv[3]], prepared=True)
        raise SystemExit(result.command_exit_code)

    # A fresh private tree keeps sockets, app state, policy grants and journals
    # separate from the operator's compositor and daemon.
    if sys.argv[1:] not in ([], ["--firefox"], ["--firefox-shared-client"]):
        raise SystemExit("usage: agent-seat-first-input-eval.py [--firefox | --firefox-shared-client]")
    scenario = sys.argv[1][2:] if sys.argv[1:] else "qt"
    parent = Path(tempfile.mkdtemp(prefix="seatgeist-first-input-"))
    root = parent / "fixture"
    config = NestedKdeConfig(root=root, socket_name="w", require_screencast_portal=False)
    paths = prepare_fixture_directories(config)
    install_bridge(ROOT / "kwin/seatgeist-bridge", paths)
    (paths["config"] / "kwinrc").write_text("[Plugins]\nseatgeist-bridgeEnabled=true\nseatgeistagentseatEnabled=true\n")
    plugin_root = root / "plugins"
    destination = plugin_root / "kwin/plugins"
    destination.mkdir(parents=True)
    shutil.copy2(ROOT / "target/kwin-seatgeist-agent-seat/seatgeistagentseat.so", destination)
    environment = isolated_environment(config, paths, os.environ)
    environment["QT_PLUGIN_PATH"] = str(plugin_root)
    install_protocol_probe_desktop(paths)
    (paths["data"] / "applications/org.seatgeist.FirstInputDaemon.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Private Seatgeist input fixture\n"
        f"Exec={(ROOT / 'target/debug/seatgeistd').resolve()}\nNoDisplay=true\n"
        "X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2\n")
    rebuild_service_cache(environment, paths["logs"] / "kservice-cache.log")
    print(f"Isolated first-input evidence: {root}", flush=True)
    result = subprocess.run(["dbus-run-session", "--", sys.executable, __file__, "--private", str(root), scenario], env=environment)
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
