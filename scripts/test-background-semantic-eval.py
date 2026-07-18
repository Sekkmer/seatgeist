#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
RUNNER = SCRIPTS / "background-semantic-eval.py"
sys.path.insert(0, str(SCRIPTS))

from cooperative_acceptance import validate_background  # noqa: E402


def load_runner():
    spec = importlib.util.spec_from_file_location("background_semantic_eval_runner", RUNNER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class FakeCli:
    def __init__(self) -> None:
        self.target_id = "target-window-private"
        self.user_id = "user-window-private"
        self.target_app_id = "firefox"
        self.target_pid = 4242
        self.active_id = self.user_id
        self.other_user_id = "other-user-window-private"
        self.journal_before_id = self.user_id
        self.journal_after_id = self.user_id
        self.switch_user_after_action = False
        self.focus_target_after_action = False
        self.include_journal = True
        self.commands: list[tuple[str, ...]] = []

    def __call__(
        self, _cli: Path, _socket: Path | None, *arguments: str
    ) -> dict[str, Any]:
        self.commands.append(tuple(arguments))
        if arguments == ("windows",):
            return {
                "type": "windows",
                "data": [
                    {
                        "id": self.target_id,
                        "app_id": self.target_app_id,
                        "pid": self.target_pid,
                        "title": "private target title",
                    },
                    {
                        "id": self.user_id,
                        "app_id": "org.kde.konsole",
                        "pid": 5252,
                        "title": "private user title",
                    },
                ],
            }
        if arguments == ("active-window",):
            return {
                "type": "active_window",
                "data": {
                    "id": self.active_id,
                    "app_id": "org.kde.konsole",
                    "title": "private user title",
                },
            }
        if arguments and arguments[0] == "approve":
            return {
                "approval_file": "private-approval-path",
                "safety_class": "control_semantic",
                "method": "click_button",
                "expires_unix_ms": int(time.time() * 1000) + 60_000,
            }
        if arguments[:2] == ("semantic", "click-button"):
            assert arguments[arguments.index("--target-window-id") + 1] == self.target_id
            assert arguments[arguments.index("--target-app-id") + 1] == self.target_app_id
            assert arguments[arguments.index("--target-pid") + 1] == str(self.target_pid)
            if self.focus_target_after_action:
                self.active_id = self.target_id
                self.journal_after_id = self.target_id
            elif self.switch_user_after_action:
                self.active_id = self.other_user_id
                self.journal_after_id = self.other_user_id
            return {
                "type": "action",
                "data": {
                    "id": "action-private",
                    "ok": True,
                    "observation": None,
                    "message": "clicked",
                },
            }
        if arguments[:2] == ("journal", "tail"):
            entries = []
            if self.include_journal:
                entries.append(
                    {
                        "sequence": 9,
                        "unix_time_ms": int(time.time() * 1000) + 1_000,
                        "method": "click_button",
                        "ok": True,
                        "summary": "semantic action",
                        "active_window_before": {"id": self.journal_before_id},
                        "active_window_after": {"id": self.journal_after_id},
                        "control": {
                            "requested_target": {
                                "kind": "semantic_button",
                                "fields": {
                                    "target_window_id": self.target_id,
                                    "target_app_id": self.target_app_id,
                                },
                            }
                        },
                    }
                )
            return {"type": "journal", "data": entries}
        raise AssertionError(f"unexpected fake CLI command: {arguments}")


def config(module: Any, root: Path, fake: FakeCli, scenario: str = "firefox") -> Any:
    socket = root / "seatgeistd.sock"
    socket.write_text("fixture", encoding="ascii")
    return module.BackgroundEvalConfig(
        scenario=scenario,
        target_window_id=fake.target_id,
        user_window_id=fake.user_id,
        button_name="private button name",
        app_filter=None,
        cli=Path("fake-cli"),
        socket=socket,
        approval_file=root / "approvals.jsonl",
        output_dir=root / "evidence",
        approval_ttl_ms=60_000,
    )


def run(module: Any, cfg: Any, fake: FakeCli, verdict: str = "yes") -> dict[str, Any]:
    return module.run_background_eval(
        cfg,
        cli_runner=fake,
        input_reader=lambda _prompt: verdict,
        message_writer=lambda _message: None,
    )


def main() -> None:
    module = load_runner()

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-eval-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        cfg = config(module, root, fake)
        evidence = run(module, cfg, fake)
        assert evidence["status"] == "passed"
        assert evidence["acceptance_complete"] is True
        assert evidence["non_target_focus_before"] is True
        assert evidence["non_target_focus_after"] is True
        assert evidence["user_window_changed_during_action"] is False
        assert evidence["semantic_action_succeeded"] is True
        assert evidence["operator_target_never_focused_confirmed"] is True
        assert evidence["journal_match_count"] == 1
        assert evidence["explicit_focus_call_count"] == 0
        assert evidence["raw_input_call_count"] == 0
        assert evidence["daemon_request_count"] == 5
        validate_background("firefox")(evidence)
        assert all(command[0] not in {"focus", "input"} for command in fake.commands)
        evidence_path = cfg.output_dir / "evidence.json"
        stored = evidence_path.read_text(encoding="utf-8")
        assert fake.target_id not in stored
        assert fake.user_id not in stored
        assert cfg.button_name not in stored
        assert "action-private" not in stored
        assert evidence_path.stat().st_mode & 0o777 == 0o600

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-focus-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.focus_target_after_action = True
        evidence = run(module, config(module, root, fake), fake)
        assert evidence["status"] == "failed"
        assert evidence["non_target_focus_after"] is False
        assert "background semantic target became active" in evidence["errors"][0]

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-pre-focus-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.active_id = fake.target_id
        evidence = run(module, config(module, root, fake), fake)
        assert evidence["status"] == "failed"
        assert evidence["semantic_action_succeeded"] is False
        assert "background semantic target became active" in evidence["errors"][0]

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-user-switch-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.switch_user_after_action = True
        evidence = run(module, config(module, root, fake), fake)
        assert evidence["status"] == "passed"
        assert evidence["non_target_focus_before"] is True
        assert evidence["non_target_focus_after"] is True
        assert evidence["user_window_changed_during_action"] is True
        validate_background("firefox")(evidence)

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-stale-reference-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.active_id = fake.other_user_id
        fake.journal_before_id = fake.other_user_id
        fake.journal_after_id = fake.other_user_id
        evidence = run(module, config(module, root, fake), fake)
        assert evidence["status"] == "passed"
        assert evidence["user_window_changed_during_action"] is False
        validate_background("firefox")(evidence)

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-journal-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.include_journal = False
        evidence = run(module, config(module, root, fake), fake)
        assert evidence["status"] == "failed"
        assert "journal entry with non-target focus is missing" in evidence["errors"][0]

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-journal-focus-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.journal_after_id = fake.target_id
        evidence = run(module, config(module, root, fake), fake)
        assert evidence["status"] == "failed"
        assert "non-target focus is missing" in evidence["errors"][0]

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-family-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.target_app_id = "org.kde.kcalc"
        evidence = run(module, config(module, root, fake, scenario="firefox"), fake)
        assert evidence["status"] == "failed"
        assert fake.commands == [("windows",)]

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-kde-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        fake.target_app_id = "org.kde.kcalc"
        evidence = run(module, config(module, root, fake, scenario="kde"), fake)
        assert evidence["status"] == "passed"
        assert evidence["scenario"] == "kde"
        validate_background("kde")(evidence)

    with tempfile.TemporaryDirectory(prefix="seatgeist-background-verdict-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        evidence = run(module, config(module, root, fake), fake, verdict="no")
        assert evidence["status"] == "failed"
        assert evidence["visual_change_confirmed"] is False

    with tempfile.TemporaryDirectory(
        prefix="seatgeist-background-interrupt-"
    ) as temporary:
        root = Path(temporary)
        fake = FakeCli()

        def interrupt(_prompt: str) -> str:
            raise KeyboardInterrupt

        evidence = module.run_background_eval(
            config(module, root, fake),
            cli_runner=fake,
            input_reader=interrupt,
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "failed"
        assert evidence["errors"] == [
            "operator_verdict: operator interrupted background semantic evaluation"
        ]

    print("test-background-semantic-eval: ok")


if __name__ == "__main__":
    main()
