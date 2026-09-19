"""Real Firefox regression inside the private headless KWin fixture only."""
import json
import os
import subprocess
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from computer_use_eval import ROOT
from nested_kde_contract import NestedKdeConfig, fixture_paths
from nested_seatgeist_probe import run_cli, stop_process, wait_for_probe


def exercise_firefox(root: Path, mcp_client, shared_client: bool = False) -> None:
    paths = fixture_paths(NestedKdeConfig(root=root, socket_name="w"))
    socket = paths["runtime"] / "d"
    cli = ROOT / "target/debug/seatgeist-cli"
    received = {"one": [], "two": []}
    events = []

    class Page(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            name = "two" if self.path == "/two" else "one"
            body = '''<!doctype html><meta charset="utf-8"><title>Seatgeist Firefox binding fixture NAME</title>
                <input autofocus id="field" style="width:90vw;height:40vh;font-size:48px">
                <script>
                field.oninput=()=>fetch('/value/NAME',{method:'POST',body:field.value});
                for (const name of ['keydown', 'keyup', 'focus', 'blur'])
                  window.addEventListener(name, e=>fetch('/event',{method:'POST',body:JSON.stringify({page:'NAME',type:e.type,key:e.key,target:e.target.id,active:document.activeElement.id})}),true);
                window.addEventListener('load',()=>{field.focus();fetch('/event',{method:'POST',body:JSON.stringify({page:'NAME',type:'ready',active:document.activeElement.id})});OPEN_SECOND});
                </script>'''.replace("NAME", name).replace("OPEN_SECOND", "window.open('/two','second','width=900,height=600');" if shared_client and name == "one" else "").encode()
            self.send_response(200)
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"])).decode()
            (received[self.path.rsplit("/", 1)[-1]] if self.path.startswith('/value/') else events).append(body)
            self.send_response(204)
            self.end_headers()

    http = ThreadingHTTPServer(("127.0.0.1", 0), Page)
    worker = threading.Thread(target=http.serve_forever, daemon=True)
    worker.start()
    processes = []
    prefs = {
        "browser.shell.checkDefaultBrowser": False,
        "browser.startup.homepage_override.mstone": "ignore",
        "browser.aboutwelcome.enabled": False,
        "browser.aboutwelcome.experimentsGate.enabled": False,
        "browser.preonboarding.enabled": False,
        "dom.disable_open_during_load": False,
        "datareporting.policy.dataSubmissionEnabled": False,
        "toolkit.telemetry.reportingpolicy.firstRun": False,
        "app.normandy.enabled": False,
        "browser.newtabpage.enabled": False,
    }
    with (paths["logs"] / "daemon.log").open("wb") as daemon_log, (paths["logs"] / "firefox-wayland.log").open("wb") as browser_log, (paths["logs"] / "app.log").open("wb") as app_log:
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
            for name in ["one"] if shared_client else ["one", "two"]:
                profile = paths["state"] / f"firefox-profile-{name}"
                profile.mkdir(mode=0o700)
                (profile / "user.js").write_text("\n".join(f"user_pref({json.dumps(k)}, {json.dumps(v)});" for k, v in prefs.items()))
                browser = subprocess.Popen([
                    "firefox", "--no-remote", "--profile", str(profile),
                    f"http://127.0.0.1:{http.server_port}/{name}",
                ], env={**os.environ, "MOZ_ENABLE_WAYLAND": "1", "WAYLAND_DEBUG": "client"},
                   stdout=browser_log, stderr=subprocess.STDOUT, start_new_session=True)
                processes.append(browser)
            deadline = time.monotonic() + 25
            while time.monotonic() < deadline:
                windows = run_cli(cli, socket, "windows")["data"]
                targets = [next((w for w in windows if f"Seatgeist Firefox binding fixture {name}" in w["title"]), None) for name in ["one", "two"]]
                if all(targets) and all(any(f'"page":"{name}","type":"ready","active":"field"' in event for event in events) for name in ["one", "two"]):
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError("private Firefox did not become ready")
            sentinel = subprocess.Popen([str(ROOT / "target/kwin-seatgeist-agent-seat/focus_fixture")],
                stdout=app_log, stderr=subprocess.STDOUT, start_new_session=True)
            processes.append(sentinel)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                active = run_cli(cli, socket, "active-window")["data"]
                if active and active["title"] == "Seatgeist physical-focus sentinel":
                    break
                time.sleep(0.05)
            else:
                raise RuntimeError("private sentinel did not acquire initial focus")
            clients = [mcp_client(socket), mcp_client(socket)]
            processes.extend(client.process for client in clients)
            sessions = [client.open(target) for client, target in zip(clients, targets)]
            try:
                if shared_client:
                    for client, session in zip(clients, sessions):
                        client.key(session, "7", "Firefox shared-process window focus is not seat-isolated")
                    if any(received.values()):
                        raise RuntimeError(f"refused shared-client action still delivered text: {received}")
                    if run_cli(cli, socket, "active-window")["data"]["id"] != active["id"]:
                        raise RuntimeError("shared-client refusal changed physical focus")
                    print(json.dumps({"firefox_shared_client": "refused_without_input", "physical_focus_preserved": True}), flush=True)
                    return
                for left, right, expected in [("7", "8", {"one": "7", "two": "8"}), ("9", "0", {"one": "79", "two": "80"}), ("Shift+a", "b", {"one": "79A", "two": "80b"}), ("5", "6", {"one": "79A5", "two": "80b6"})]:
                    if left == "5":
                        # Keep KWin, both Firefox profiles and the actual MCP
                        # processes alive across a daemon-only update.
                        for client, session in zip(clients, sessions):
                            client.close(session)
                        sessions = []
                        stop_process(daemon)
                        processes.remove(daemon)
                        daemon = subprocess.Popen(daemon_command, stdout=daemon_log, stderr=subprocess.STDOUT, start_new_session=True)
                        processes.append(daemon)
                        wait_for_probe(daemon, cli, socket, 15)
                        sessions = [client.open(target) for client, target in zip(clients, targets)]
                    with ThreadPoolExecutor(max_workers=2) as pool:
                        futures = [pool.submit(client.key, session, key) for client, session, key in zip(clients, sessions, [left, right])]
                        for future in futures:
                            future.result(timeout=15)
                    deadline = time.monotonic() + 3
                    while time.monotonic() < deadline and any(not received[name] or received[name][-1] != value for name, value in expected.items()):
                        time.sleep(0.02)
                    if any(not received[name] or received[name][-1] != value for name, value in expected.items()):
                        raise RuntimeError(f"Firefox input was lost/misrouted: expected={expected}, values={received}, events={events}")
                    if run_cli(cli, socket, "active-window")["data"]["id"] != active["id"]:
                        raise RuntimeError("Firefox input changed physical focus")
                print(json.dumps({"firefox_concurrent_mcp": "passed", "daemon_restart": "passed", "shared_client": shared_client, "physical_focus_preserved": True, "values": received}), flush=True)
            finally:
                for client, session in zip(clients, sessions):
                    client.close(session)
        finally:
            for process in reversed(processes):
                stop_process(process)
            http.shutdown()
            http.server_close()
            worker.join(timeout=2)
