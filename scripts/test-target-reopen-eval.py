#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
RUNNER = SCRIPTS / "target-reopen-eval.py"
sys.path.insert(0, str(SCRIPTS))

from cooperative_acceptance import validate_reopen  # noqa: E402


def load_runner():
    spec = importlib.util.spec_from_file_location("target_reopen_eval_runner", RUNNER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class FakeCli:
    def __init__(self) -> None:
        self.original_id = "original-window-private"
        self.replacement_id = "replacement-window-private"
        self.app_id = "firefox"
        self.reopened = False
        self.capture_active = False
        self.portal_ended = False
        self.sticky_stays_bound = False
        self.commands: list[tuple[str, ...]] = []

    def __call__(
        self, _cli: Path, _socket: Path | None, *arguments: str
    ) -> dict[str, Any]:
        self.commands.append(tuple(arguments))
        if arguments == ("windows",):
            window_id = self.replacement_id if self.reopened else self.original_id
            return {
                "type": "windows",
                "data": [
                    {
                        "id": window_id,
                        "app_id": self.app_id,
                        "pid": 4242,
                        "title": "private title",
                    }
                ],
            }
        if arguments == ("capture", "status"):
            if self.portal_ended and self.reopened:
                self.capture_active = False
            return self.status()
        if arguments[:2] == ("capture", "open"):
            self.capture_active = True
            return self.status(opened=True)
        if arguments[:2] == ("capture", "close"):
            self.capture_active = False
            return self.status(client_closed=True)
        raise AssertionError(f"unexpected fake CLI command: {arguments}")

    def status(
        self, *, opened: bool = False, client_closed: bool = False
    ) -> dict[str, Any]:
        active = self.capture_active
        after_reopen = self.reopened and not opened
        sticky = active and (not after_reopen or self.sticky_stays_bound)
        return {
            "type": "capture_session_status",
            "data": {
                "active": active,
                "opening": False,
                "session_id": "capture-private" if active else None,
                "backend": "portal_screencast_pipewire" if active else None,
                "source_type": "window" if active else None,
                "source_id": "opaque-source" if active else None,
                "restore_token_reference": "screencast-target" if active else None,
                "requested_window_id": self.original_id if active else None,
                "latest_revision": None,
                "consent_required": active,
                "occlusion_possible": False,
                "sticky_target_bound": sticky,
                "target_window_id": self.original_id if sticky else None,
                "target_app_id": self.app_id if sticky else None,
                "target_pid": 4242 if sticky else None,
                "target_expires_in_ms": 60_000 if sticky else None,
                "last_end_reason": (
                    "client_closed"
                    if client_closed
                    else "portal_closed" if self.portal_ended and not active else None
                ),
            },
        }


def config(module: Any, root: Path, fake: FakeCli) -> Any:
    socket = root / "seatgeistd.sock"
    socket.write_text("fixture", encoding="ascii")
    return module.TargetReopenConfig(
        window_id=fake.original_id,
        cli=Path("fake-cli"),
        socket=socket,
        output_dir=root / "evidence",
        transition_timeout_ms=1000,
        poll_interval_ms=10,
    )


def reopen(fake: FakeCli) -> str:
    fake.reopened = True
    return ""


def main() -> None:
    module = load_runner()

    with tempfile.TemporaryDirectory(prefix="seatgeist-reopen-eval-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        evidence = module.run_target_reopen_eval(
            config(module, root, fake),
            cli_runner=fake,
            input_reader=lambda _prompt: reopen(fake),
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "passed"
        assert evidence["post_reopen_status"]["capture_active"] is True
        assert evidence["post_reopen_status"]["sticky_target_bound"] is False
        assert evidence["session_cleanup"] == "client_closed"
        assert evidence["explicit_focus_call_count"] == 0
        assert evidence["raw_input_call_count"] == 0
        validate_reopen(evidence)
        assert fake.commands[-1][:2] == ("capture", "close")
        stored = (root / "evidence/evidence.json").read_text(encoding="utf-8")
        assert fake.original_id not in stored
        assert fake.replacement_id not in stored
        assert "capture-private" not in stored

    with tempfile.TemporaryDirectory(prefix="seatgeist-reopen-portal-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.portal_ended = True
        evidence = module.run_target_reopen_eval(
            config(module, root, fake),
            cli_runner=fake,
            input_reader=lambda _prompt: reopen(fake),
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "passed"
        assert evidence["post_reopen_status"]["capture_active"] is False
        assert evidence["session_cleanup"] == "portal_ended"
        assert not any(command[:2] == ("capture", "close") for command in fake.commands)

    with tempfile.TemporaryDirectory(prefix="seatgeist-reopen-bound-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.sticky_stays_bound = True
        evidence = module.run_target_reopen_eval(
            config(module, root, fake),
            cli_runner=fake,
            input_reader=lambda _prompt: reopen(fake),
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "failed"
        assert "silently retained" in evidence["errors"][0]
        assert fake.commands[-1][:2] == ("capture", "close")

    with tempfile.TemporaryDirectory(prefix="seatgeist-reopen-skip-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        evidence = module.run_target_reopen_eval(
            config(module, root, fake),
            cli_runner=fake,
            input_reader=lambda _prompt: "skip",
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "failed"
        assert evidence["session_cleanup"] == "client_closed"

    with tempfile.TemporaryDirectory(prefix="seatgeist-reopen-interrupt-") as temporary:
        root = Path(temporary)
        fake = FakeCli()

        def interrupt(_prompt: str) -> str:
            raise KeyboardInterrupt

        evidence = module.run_target_reopen_eval(
            config(module, root, fake),
            cli_runner=fake,
            input_reader=interrupt,
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "failed"
        assert evidence["session_cleanup"] == "client_closed"
        assert evidence["errors"] == [
            "operator interrupted target close/reopen evaluation"
        ]

    print("test-target-reopen-eval: ok")


if __name__ == "__main__":
    main()
