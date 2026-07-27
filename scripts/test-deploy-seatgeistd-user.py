#!/usr/bin/env python3
from __future__ import annotations

import json
import hashlib
import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/deploy-seatgeistd-user.py"


def write_executable(path: Path, content: str) -> None:
    path.write_text(content, encoding="utf-8")
    path.chmod(0o755)


def invoke(arguments: list[str], state: Path, *, check: bool = True) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["FAKE_DEPLOY_STATE"] = str(state)
    return subprocess.run(
        [str(SCRIPT), *arguments],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        check=check,
    )


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="seatgeist-user-daemon-deploy-") as temporary:
        root = Path(temporary)
        state = root / "state"
        state.mkdir()
        cargo = root / "cargo"
        cli = root / "seatgeist-cli"
        systemctl = root / "systemctl"
        release = root / "target/release/seatgeistd"
        installed = root / "home/.local/bin/seatgeistd"
        proc_root = root / "proc"
        running_link = proc_root / "4242/exe"
        release.parent.mkdir(parents=True)
        installed.parent.mkdir(parents=True)
        running_link.parent.mkdir(parents=True)
        release.write_bytes(b"verified release daemon\n")
        installed.write_bytes(b"old daemon\n")
        running_link.symlink_to(installed)

        write_executable(
            cargo,
            """#!/usr/bin/env python3
import os
from pathlib import Path
state = Path(os.environ["FAKE_DEPLOY_STATE"])
with (state / "cargo.log").open("a", encoding="utf-8") as log:
    log.write(" ".join(__import__("sys").argv[1:]) + "\\n")
""",
        )
        write_executable(
            systemctl,
            """#!/usr/bin/env python3
import os
import sys
from pathlib import Path
state = Path(os.environ["FAKE_DEPLOY_STATE"])
args = sys.argv[1:]
with (state / "systemctl.log").open("a", encoding="utf-8") as log:
    log.write(" ".join(args) + "\\n")
if "show" in args:
    print("4242")
""",
        )
        write_executable(
            cli,
            """#!/usr/bin/env python3
import json
import os
import sys
from pathlib import Path
state = Path(os.environ["FAKE_DEPLOY_STATE"])
args = sys.argv[1:]
command = args[args.index("--socket") + 2:]
def count(name):
    path = state / name
    value = int(path.read_text() if path.exists() else "0") + 1
    path.write_text(str(value))
    return value
if command == ["capture", "status"]:
    call = count("capture.count")
    active = (state / "capture.active").exists()
    trigger = state / "capture.active-on-call"
    if trigger.exists() and call == int(trigger.read_text()):
        active = True
    print(json.dumps({"type":"capture_session_status","data":{"active":active}}))
elif command == ["input", "remote-desktop-eis-session-status"]:
    active = (state / "eis.active").exists()
    print(json.dumps({"type":"remote_desktop_eis_session_status","data":{"active":active,"runtime_connected":active}}))
elif command == ["doctor"]:
    call = count("doctor.count")
    if call < 3:
        print("not ready", file=sys.stderr)
        raise SystemExit(1)
    digest = (state / "health.sha256").read_text().strip()
    print(json.dumps({"type":"health","data":{"status":"ok","protocol_version":"1","run_id":"00000000-0000-0000-0000-000000000001","binary_sha256":digest,"config_fingerprint":"test-config"}}))
elif command == ["kwin-bridge-status"]:
    call = count("bridge.count")
    ready = call >= 3 and not (state / "bridge.never").exists()
    print(json.dumps({"type":"kwin_bridge_status","data":{"dbus_service_registered":True,"active_window_update_seen":ready,"window_list_update_seen":ready,"window_count":7 if ready else 0}}))
else:
    print(f"unexpected command: {command}", file=sys.stderr)
    raise SystemExit(2)
""",
        )

        common = [
            "--root",
            str(root),
            "--cargo",
            str(cargo),
            "--cli",
            str(cli),
            "--systemctl",
            str(systemctl),
            "--socket",
            str(root / "daemon.sock"),
            "--release-binary",
            str(release),
            "--install-path",
            str(installed),
            "--proc-root",
            str(proc_root),
            "--timeout-ms",
            "1000",
            "--poll-ms",
            "1",
        ]

        (state / "health.sha256").write_text(
            hashlib.sha256(release.read_bytes()).hexdigest(),
            encoding="ascii",
        )
        completed = invoke(common, state)
        report = json.loads(completed.stdout)
        assert report["ok"] is True
        assert report["daemon_readiness_attempts"] == 3
        assert report["bridge_readiness_attempts"] == 3
        assert report["window_count"] == 7
        assert report["run_id"] == "00000000-0000-0000-0000-000000000001"
        assert installed.read_bytes() == release.read_bytes()
        assert installed.stat().st_mode & 0o777 == 0o755
        cargo_calls = (state / "cargo.log").read_text(encoding="utf-8").splitlines()
        assert cargo_calls == ["build -p seatgeist-cli", "build --release -p seatgeistd"]
        systemctl_calls = (state / "systemctl.log").read_text(encoding="utf-8")
        assert "--user restart seatgeistd.service" in systemctl_calls
        assert "--user show --property MainPID --value seatgeistd.service" in systemctl_calls
        assert int((state / "capture.count").read_text()) == 3

        restart_count = systemctl_calls.count("--user restart")
        (state / "capture.active").touch()
        release.write_bytes(b"must not install while active\n")
        refused = invoke([*common, "--skip-build"], state, check=False)
        assert refused.returncode != 0
        assert "active retained session(s): capture" in refused.stderr
        assert installed.read_bytes() != release.read_bytes()
        assert (state / "systemctl.log").read_text().count("--user restart") == restart_count
        (state / "capture.active").unlink()

        (state / "eis.active").touch()
        eis_refused = invoke([*common, "--skip-build"], state, check=False)
        assert eis_refused.returncode != 0
        assert "active retained session(s): remote-desktop-eis" in eis_refused.stderr
        assert (state / "systemctl.log").read_text().count("--user restart") == restart_count
        (state / "eis.active").unlink()

        for counter in ("capture.count", "doctor.count", "bridge.count"):
            (state / counter).write_text("0", encoding="ascii")
        (state / "capture.active-on-call").write_text("2", encoding="ascii")
        between = invoke([*common, "--skip-build"], state, check=False)
        assert between.returncode != 0
        assert "refusing daemon restart" in between.stderr
        assert (state / "systemctl.log").read_text().count("--user restart") == restart_count
        (state / "capture.active-on-call").unlink()

        bad_running = root / "bad-running-daemon"
        bad_running.write_bytes(b"different process image\n")
        running_link.unlink()
        running_link.symlink_to(bad_running)
        for counter in ("capture.count", "doctor.count", "bridge.count"):
            (state / counter).write_text("0", encoding="ascii")
        (state / "health.sha256").write_text(
            hashlib.sha256(release.read_bytes()).hexdigest(),
            encoding="ascii",
        )
        mismatch = invoke([*common, "--skip-build"], state, check=False)
        assert mismatch.returncode != 0
        assert "daemon hash mismatch" in mismatch.stderr

    print("test-deploy-seatgeistd-user: ok")


if __name__ == "__main__":
    main()
