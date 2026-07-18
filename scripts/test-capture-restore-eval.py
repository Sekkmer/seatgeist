#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
RUNNER = SCRIPTS / "capture-restore-eval.py"
sys.path.insert(0, str(SCRIPTS))

from computer_use_eval import EvalError  # noqa: E402
import capture_restore_eval as contract  # noqa: E402
from cooperative_acceptance import validate_restore  # noqa: E402


def load_runner():
    spec = importlib.util.spec_from_file_location("capture_restore_eval_runner", RUNNER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class FakeCli:
    def __init__(self, restore_file: Path, window_id: str) -> None:
        self.restore_file = restore_file
        self.window_id = window_id
        self.active = False
        self.sequence = 0
        self.open_count = 0
        self.rotate_restore = True
        self.wrong_backend = False
        self.commands: list[tuple[str, ...]] = []

    def __call__(
        self, _cli: Path, _socket: Path | None, *arguments: str
    ) -> dict[str, Any]:
        self.commands.append(tuple(arguments))
        if arguments == ("capture", "status"):
            return self.status()
        if arguments[:2] == ("capture", "open"):
            self.active = True
            self.open_count += 1
            if self.rotate_restore:
                self.replace_restore_file()
            return self.status()
        if arguments[:2] == ("capture", "snapshot"):
            return self.snapshot(arguments)
        if arguments[:2] == ("capture", "close"):
            self.active = False
            return self.status()
        raise AssertionError(f"unexpected fake CLI command: {arguments}")

    def replace_restore_file(self) -> None:
        self.restore_file.parent.mkdir(parents=True, exist_ok=True)
        self.restore_file.parent.chmod(0o700)
        temporary = self.restore_file.with_name(f"token-{self.open_count}.tmp")
        temporary.write_text(f"rotated-{self.open_count}\n", encoding="ascii")
        temporary.chmod(0o600)
        os.replace(temporary, self.restore_file)

    def status(self) -> dict[str, Any]:
        data: dict[str, Any] = {
            "active": self.active,
            "opening": False,
            "session_id": f"capture-{self.open_count}" if self.active else None,
            "backend": (
                "unexpected_backend"
                if self.active and self.wrong_backend
                else "portal_screencast_pipewire" if self.active else None
            ),
            "source_type": "window" if self.active else None,
            "source_id": "opaque-source" if self.active else None,
            "restore_token_reference": "screencast-target" if self.active else None,
            "requested_window_id": self.window_id if self.active else None,
            "latest_revision": None,
            "consent_required": self.active,
            "occlusion_possible": False,
            "sticky_target_bound": self.active,
            "target_window_id": self.window_id if self.active else None,
            "target_app_id": "org.mozilla.firefox" if self.active else None,
            "target_pid": 4242 if self.active else None,
            "target_expires_in_ms": 60_000 if self.active else None,
        }
        return {"type": "capture_session_status", "data": data}

    def snapshot(self, arguments: tuple[str, ...]) -> dict[str, Any]:
        self.sequence += 1
        output = Path(arguments[arguments.index("--output") + 1])
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(b"\x89PNG\r\n\x1a\nfixture")
        output.chmod(0o600)
        return {
            "type": "capture_frame",
            "data": {
                "session_id": f"capture-{self.open_count}",
                "screenshot": {
                    "path": str(output),
                    "backend": "portal_screencast_pipewire",
                    "source_width": 1920,
                    "source_height": 1080,
                    "output_width": 1200,
                    "output_height": 675,
                },
                "revision": f"revision-{self.sequence}",
                "sequence": self.sequence,
                "complete": True,
                "damage_present": True,
            },
        }


def config(module: Any, root: Path, window_id: str) -> Any:
    socket = root / "seatgeistd.sock"
    socket.write_text("daemon-one", encoding="ascii")
    return module.RestoreEvalConfig(
        window_id=window_id,
        cli=Path("fake-cli"),
        socket=socket,
        restore_file=root / "state/capture-restore.json",
        output_dir=root / "evidence",
        max_edge=1200,
        timeout_ms=8000,
    )


def replace_socket(path: Path) -> None:
    old = path.with_name("seatgeistd-old.sock")
    path.rename(old)
    path.write_text("daemon-two", encoding="ascii")


def main() -> None:
    module = load_runner()
    window_id = "window-private-test"
    assert contract.same_file_identity({}, {}) is False
    try:
        contract.require_daemon_restart({}, {"device": 1, "inode": 2})
    except EvalError as err:
        assert "malformed" in str(err)
    else:
        raise AssertionError("malformed socket identity must fail closed")

    with tempfile.TemporaryDirectory(prefix="seatgeist-restore-eval-") as temporary:
        root = Path(temporary)
        cfg = config(module, root, window_id)
        fake = FakeCli(cfg.restore_file, window_id)
        prepared = module.prepare_restore_eval(
            cfg, cli_runner=fake, message_writer=lambda _message: None
        )
        assert prepared["status"] == "prepared"
        assert prepared["prepare_session_closed"] is True
        assert fake.open_count == 1
        evidence_path = cfg.output_dir / "evidence.json"
        stored = json.loads(evidence_path.read_text(encoding="utf-8"))
        assert window_id not in json.dumps(stored)
        assert "capture-1" not in json.dumps(stored)
        assert evidence_path.stat().st_mode & 0o777 == 0o600

        try:
            module.resume_restore_eval(
                cfg,
                cli_runner=fake,
                input_reader=lambda _prompt: "no",
                message_writer=lambda _message: None,
            )
        except EvalError as err:
            assert "restart is not proven" in str(err)
        else:
            raise AssertionError("resume without restart must fail")
        assert json.loads(evidence_path.read_text(encoding="utf-8"))["status"] == "prepared"

        replace_socket(cfg.socket)
        resumed = module.resume_restore_eval(
            cfg,
            cli_runner=fake,
            input_reader=lambda _prompt: "no",
            message_writer=lambda _message: None,
        )
        assert resumed["status"] == "passed"
        assert resumed["acceptance_complete"] is True
        assert resumed["daemon_restart_proven"] is True
        assert resumed["resume"]["portal_chooser_avoided"] is True
        assert resumed["resume"]["restore_file_replaced"] is True
        assert resumed["resume_session_closed"] is True
        assert fake.open_count == 2
        validate_restore(resumed)
        assert all(command[0] == "capture" for command in fake.commands)
        stored_text = evidence_path.read_text(encoding="utf-8")
        assert window_id not in stored_text
        assert "capture-1" not in stored_text
        assert "capture-2" not in stored_text

    with tempfile.TemporaryDirectory(prefix="seatgeist-restore-prompt-") as temporary:
        root = Path(temporary)
        cfg = config(module, root, window_id)
        fake = FakeCli(cfg.restore_file, window_id)
        assert (
            module.prepare_restore_eval(
                cfg, cli_runner=fake, message_writer=lambda _message: None
            )["status"]
            == "prepared"
        )
        replace_socket(cfg.socket)
        resumed = module.resume_restore_eval(
            cfg,
            cli_runner=fake,
            input_reader=lambda _prompt: "yes",
            message_writer=lambda _message: None,
        )
        assert resumed["status"] == "failed"
        assert "chooser reappeared" in resumed["errors"][0]
        assert resumed["resume_session_closed"] is True

    with tempfile.TemporaryDirectory(prefix="seatgeist-restore-stale-") as temporary:
        root = Path(temporary)
        cfg = config(module, root, window_id)
        fake = FakeCli(cfg.restore_file, window_id)
        assert (
            module.prepare_restore_eval(
                cfg, cli_runner=fake, message_writer=lambda _message: None
            )["status"]
            == "prepared"
        )
        replace_socket(cfg.socket)
        fake.rotate_restore = False
        resumed = module.resume_restore_eval(
            cfg,
            cli_runner=fake,
            input_reader=lambda _prompt: "no",
            message_writer=lambda _message: None,
        )
        assert resumed["status"] == "failed"
        assert "not atomically rotated" in resumed["errors"][0]
        assert resumed["resume_session_closed"] is True

    with tempfile.TemporaryDirectory(prefix="seatgeist-restore-invalid-") as temporary:
        root = Path(temporary)
        cfg = config(module, root, window_id)
        fake = FakeCli(cfg.restore_file, window_id)
        fake.wrong_backend = True
        prepared = module.prepare_restore_eval(
            cfg, cli_runner=fake, message_writer=lambda _message: None
        )
        assert prepared["status"] == "failed"
        assert prepared["prepare_session_closed"] is True
        assert fake.commands[-1][:2] == ("capture", "close")

    print("test-capture-restore-eval: ok")


if __name__ == "__main__":
    main()
