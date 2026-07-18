#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
RUNNER = SCRIPTS / "capture-lifecycle-eval.py"
sys.path.insert(0, str(SCRIPTS))

from computer_use_eval import EvalError  # noqa: E402
from cooperative_acceptance import validate_lifecycle  # noqa: E402


def load_runner():
    spec = importlib.util.spec_from_file_location("capture_lifecycle_eval_runner", RUNNER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class FakeCli:
    def __init__(self, window_id: str) -> None:
        self.window_id = window_id
        self.active = False
        self.revoked = False
        self.end_reason = "portal_closed"
        self.wrong_backend = False
        self.stale_rejection = "no active capture session"
        self.commands: list[tuple[str, ...]] = []

    def __call__(
        self, _cli: Path, _socket: Path | None, *arguments: str
    ) -> dict[str, Any]:
        self.commands.append(tuple(arguments))
        if arguments == ("capture", "status"):
            if self.revoked:
                self.active = False
            return self.status()
        if arguments[:2] == ("capture", "open"):
            self.active = True
            self.revoked = False
            return self.status()
        if arguments[:2] == ("capture", "snapshot"):
            if not self.active:
                raise EvalError(self.stale_rejection)
            output = Path(arguments[arguments.index("--output") + 1])
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(b"\x89PNG\r\n\x1a\nfixture")
            output.chmod(0o600)
            return {
                "type": "capture_frame",
                "data": {
                    "session_id": "capture-private",
                    "screenshot": {
                        "path": str(output),
                        "backend": "portal_screencast_pipewire",
                        "source_width": 1920,
                        "source_height": 1080,
                        "output_width": 1200,
                        "output_height": 675,
                    },
                    "revision": "revision-1",
                    "sequence": 1,
                    "complete": True,
                    "damage_present": True,
                },
            }
        if arguments[:2] == ("capture", "close"):
            self.active = False
            self.end_reason = "client_closed"
            return self.status()
        raise AssertionError(f"unexpected fake CLI command: {arguments}")

    def status(self) -> dict[str, Any]:
        active = self.active
        return {
            "type": "capture_session_status",
            "data": {
                "active": active,
                "opening": False,
                "session_id": "capture-private" if active else None,
                "backend": (
                    "unexpected_backend"
                    if active and self.wrong_backend
                    else "portal_screencast_pipewire" if active else None
                ),
                "source_type": "window" if active else None,
                "source_id": "opaque-source" if active else None,
                "restore_token_reference": "screencast-target" if active else None,
                "requested_window_id": self.window_id if active else None,
                "latest_revision": None,
                "consent_required": active,
                "occlusion_possible": False,
                "sticky_target_bound": active,
                "target_window_id": self.window_id if active else None,
                "target_app_id": "firefox" if active else None,
                "target_pid": 4242 if active else None,
                "target_expires_in_ms": 60_000 if active else None,
                "last_end_reason": (
                    self.end_reason
                    if self.revoked
                    or (
                        not active
                        and self.commands[-1][:2] == ("capture", "close")
                    )
                    else None
                ),
            },
        }


def config(module: Any, root: Path, window_id: str) -> Any:
    socket = root / "seatgeistd.sock"
    socket.write_text("fixture", encoding="ascii")
    return module.LifecycleEvalConfig(
        window_id=window_id,
        cli=Path("fake-cli"),
        socket=socket,
        output_dir=root / "evidence",
        max_edge=1200,
        frame_timeout_ms=8000,
        revocation_timeout_ms=1000,
        poll_interval_ms=10,
    )


def main() -> None:
    module = load_runner()
    window_id = "window-private"

    with tempfile.TemporaryDirectory(prefix="seatgeist-lifecycle-eval-") as temporary:
        root = Path(temporary)
        fake = FakeCli(window_id)
        evidence = module.run_lifecycle_eval(
            config(module, root, window_id),
            cli_runner=fake,
            input_reader=lambda _prompt: setattr(fake, "revoked", True) or "",
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "passed"
        assert evidence["acceptance_complete"] is True
        assert evidence["ended_status"]["last_end_reason"] == "portal_closed"
        assert evidence["stale_session_rejected"] is True
        assert evidence["stale_session_rejection_kind"] == "session_ended"
        assert evidence["cleanup_close_called"] is False
        validate_lifecycle(evidence)
        assert not any(command[:2] == ("capture", "close") for command in fake.commands)
        stored = (root / "evidence/evidence.json").read_text(encoding="utf-8")
        assert window_id not in stored
        assert "capture-private" not in stored
        assert (root / "evidence/evidence.json").stat().st_mode & 0o777 == 0o600

    with tempfile.TemporaryDirectory(
        prefix="seatgeist-lifecycle-owner-gate-"
    ) as temporary:
        root = Path(temporary)
        fake = FakeCli(window_id)
        fake.stale_rejection = (
            "daemon returned SessionOwnerMismatch error: session owner mismatch"
        )
        evidence = module.run_lifecycle_eval(
            config(module, root, window_id),
            cli_runner=fake,
            input_reader=lambda _prompt: setattr(fake, "revoked", True) or "",
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "passed"
        assert evidence["stale_session_rejected"] is True
        assert evidence["stale_session_rejection_kind"] == "session_owner_mismatch"

    with tempfile.TemporaryDirectory(prefix="seatgeist-lifecycle-skip-") as temporary:
        root = Path(temporary)
        fake = FakeCli(window_id)
        evidence = module.run_lifecycle_eval(
            config(module, root, window_id),
            cli_runner=fake,
            input_reader=lambda _prompt: "skip",
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "failed"
        assert evidence["cleanup_close_called"] is True
        assert fake.commands[-1][:2] == ("capture", "close")

    with tempfile.TemporaryDirectory(
        prefix="seatgeist-lifecycle-interrupt-"
    ) as temporary:
        root = Path(temporary)
        fake = FakeCli(window_id)

        def interrupt(_prompt: str) -> str:
            raise KeyboardInterrupt

        evidence = module.run_lifecycle_eval(
            config(module, root, window_id),
            cli_runner=fake,
            input_reader=interrupt,
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "failed"
        assert evidence["cleanup_close_called"] is True
        assert evidence["errors"] == [
            "operator interrupted portal-revocation evaluation"
        ]
        assert fake.commands[-1][:2] == ("capture", "close")

    with tempfile.TemporaryDirectory(prefix="seatgeist-lifecycle-reason-") as temporary:
        root = Path(temporary)
        fake = FakeCli(window_id)
        fake.end_reason = "client_closed"
        evidence = module.run_lifecycle_eval(
            config(module, root, window_id),
            cli_runner=fake,
            input_reader=lambda _prompt: setattr(fake, "revoked", True) or "",
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "failed"
        assert "attribute the end to portal closure" in evidence["errors"][0]

    with tempfile.TemporaryDirectory(prefix="seatgeist-lifecycle-invalid-") as temporary:
        root = Path(temporary)
        fake = FakeCli(window_id)
        fake.wrong_backend = True
        evidence = module.run_lifecycle_eval(
            config(module, root, window_id),
            cli_runner=fake,
            input_reader=lambda _prompt: "",
            message_writer=lambda _message: None,
            sleeper=lambda _seconds: None,
        )
        assert evidence["status"] == "failed"
        assert evidence["cleanup_close_called"] is True
        assert fake.commands[-1][:2] == ("capture", "close")

    print("test-capture-lifecycle-eval: ok")


if __name__ == "__main__":
    main()
