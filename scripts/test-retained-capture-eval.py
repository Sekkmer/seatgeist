#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
RUNNER = SCRIPTS / "retained-capture-eval.py"
sys.path.insert(0, str(SCRIPTS))

import retained_capture_eval as contract  # noqa: E402
from cooperative_acceptance import validate_retained  # noqa: E402


def load_runner():
    spec = importlib.util.spec_from_file_location("retained_capture_eval_runner", RUNNER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class FakeCli:
    def __init__(
        self,
        *,
        active: bool = False,
        stale_wait: bool = False,
        wrong_backend: bool = False,
        multi_output: bool = False,
    ) -> None:
        self.active = active
        self.stale_wait = stale_wait
        self.wrong_backend = wrong_backend
        self.multi_output = multi_output
        self.sequence = 0
        self.commands: list[tuple[str, ...]] = []
        self.latest_path: Path | None = None

    def __call__(
        self, _cli: Path, _socket: Path | None, *arguments: str
    ) -> dict[str, Any]:
        self.commands.append(tuple(arguments))
        if arguments == ("monitors",):
            monitors = [
                {
                    "id": "private-monitor",
                    "name": "private-name",
                    "physical_width": 2560,
                    "physical_height": 1440,
                    "logical_width": 2560,
                    "logical_height": 1440,
                    "scale_factor": 1.0,
                    "logical_origin_x": 0,
                    "logical_origin_y": 0,
                    "transform": None,
                }
            ]
            if self.multi_output:
                monitors.append(
                    {
                        "id": "private-monitor-two",
                        "name": "private-name-two",
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "logical_width": 1920,
                        "logical_height": 1080,
                        "scale_factor": 1.0,
                        "logical_origin_x": 2560,
                        "logical_origin_y": 0,
                        "transform": None,
                    }
                )
            return {
                "type": "monitors",
                "data": monitors,
            }
        if arguments == ("capture", "status"):
            return self.status()
        if arguments[:2] == ("capture", "open"):
            self.active = True
            return self.status(opened=True)
        if arguments[:2] == ("capture", "snapshot"):
            return self.frame_response(arguments, wait=False)
        if arguments[:2] == ("capture", "wait"):
            return self.frame_response(arguments, wait=True)
        if arguments[:2] == ("capture", "close"):
            self.active = False
            return self.status()
        raise AssertionError(f"unexpected fake CLI command: {arguments}")

    def status(self, *, opened: bool = False) -> dict[str, Any]:
        data: dict[str, Any] = {
            "active": self.active,
            "opening": False,
            "session_id": "capture-test" if self.active else None,
            "backend": (
                "unexpected_backend"
                if self.active and self.wrong_backend
                else "portal_screencast_pipewire" if self.active else None
            ),
            "source_type": "window" if self.active else None,
            "source_id": "opaque-source" if self.active else None,
            "restore_token_reference": "screencast-test" if self.active else None,
            "requested_window_id": "window-test" if self.active else None,
            "latest_revision": None,
            "consent_required": self.active,
            "occlusion_possible": False,
            "sticky_target_bound": self.active,
            "target_window_id": "window-test" if self.active else None,
            "target_app_id": "org.mozilla.firefox" if self.active else None,
            "target_pid": 4242 if self.active else None,
            "target_expires_in_ms": 60_000 if self.active else None,
        }
        return {"type": "capture_session_status", "data": data}

    def frame_response(
        self, arguments: tuple[str, ...], *, wait: bool
    ) -> dict[str, Any]:
        if wait and self.stale_wait:
            assert self.latest_path is not None
            output = self.latest_path
            revision = f"revision-{self.sequence}"
            changed = False
        else:
            self.sequence += 1
            output = Path(arguments[arguments.index("--output") + 1])
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(b"\x89PNG\r\n\x1a\nfixture")
            output.chmod(0o600)
            self.latest_path = output
            revision = f"revision-{self.sequence}"
            changed = True
        frame = {
            "session_id": "capture-test",
            "screenshot": {
                "path": str(output),
                "backend": "portal_screencast_pipewire",
                "source_width": 1920,
                "source_height": 1080,
                "output_width": 1200,
                "output_height": 675,
            },
            "revision": revision,
            "sequence": max(self.sequence, 1),
            "complete": True,
            "damage_present": changed,
        }
        if wait:
            return {
                "type": "capture_wait",
                "data": {
                    "frame": frame,
                    "changed": changed,
                    "timed_out": not changed,
                    "timeout_ms": 8000,
                    "elapsed_ms": 5 if changed else 8000,
                },
            }
        return {"type": "capture_frame", "data": frame}


def answers_for(scenario_count: int) -> Any:
    answers = iter(value for _ in range(scenario_count) for value in ("", "yes"))
    return lambda _prompt: next(answers)


def config(
    module: Any,
    root: Path,
    scenarios: tuple[Any, ...],
    *,
    require_multi_output_nonzero_origin: bool = False,
) -> Any:
    socket = root / "seatgeistd.sock"
    socket.write_text("fixture", encoding="ascii")
    return module.EvalConfig(
        window_id="window-test",
        cli=Path("fake-cli"),
        socket=socket,
        output_dir=root / "evidence",
        scenarios=scenarios,
        max_edge=1200,
        timeout_ms=8000,
        require_multi_output_nonzero_origin=require_multi_output_nonzero_origin,
    )


def main() -> None:
    module = load_runner()
    assert contract.normalize_visual_verdict("yes") == "pass"
    assert contract.normalize_visual_verdict("skip") == "skip"
    assert len(contract.selected_scenarios(None)) == 8
    assert module.absolute_output_dir(Path("relative-evidence")) == (
        Path.cwd() / "relative-evidence"
    ).resolve()

    with tempfile.TemporaryDirectory(prefix="seatgeist-retained-eval-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        evidence = module.run_capture_eval(
            config(module, root, contract.SCENARIOS),
            cli_runner=fake,
            input_reader=answers_for(len(contract.SCENARIOS)),
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "passed"
        assert evidence["acceptance_complete"] is True
        assert evidence["portal_open_count"] == 1
        assert evidence["explicit_focus_call_count"] == 0
        assert evidence["session_closed"] is True
        assert len(evidence["scenarios"]) == 8
        assert all(item["frame"]["fresh_frame"] for item in evidence["scenarios"])
        validate_retained(evidence)
        assert fake.commands[0] == ("monitors",)
        assert fake.commands[1] == ("capture", "status")
        assert fake.commands[-1][:2] == ("capture", "close")
        stored = json.loads((root / "evidence/evidence.json").read_text(encoding="utf-8"))
        assert stored["target_window_sha256"] == contract.hashed_window_id("window-test")
        assert "window-test" not in json.dumps(stored)
        assert (root / "evidence/evidence.json").stat().st_mode & 0o777 == 0o600

    with tempfile.TemporaryDirectory(prefix="seatgeist-retained-stale-") as temporary:
        root = Path(temporary)
        fake = FakeCli(stale_wait=True)
        one = contract.selected_scenarios(["focused_visible"])
        evidence = module.run_capture_eval(
            config(module, root, one),
            cli_runner=fake,
            input_reader=answers_for(1),
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "incomplete"
        assert evidence["scenarios"][0]["frame"]["fresh_frame"] is False
        assert evidence["session_closed"] is True

    with tempfile.TemporaryDirectory(prefix="seatgeist-retained-busy-") as temporary:
        root = Path(temporary)
        fake = FakeCli(active=True)
        evidence = module.run_capture_eval(
            config(module, root, contract.selected_scenarios(["focused_visible"])),
            cli_runner=fake,
            input_reader=lambda _prompt: "",
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "failed"
        assert "left untouched" in evidence["errors"][0]
        assert fake.commands == [("monitors",), ("capture", "status")]

    with tempfile.TemporaryDirectory(prefix="seatgeist-retained-invalid-") as temporary:
        root = Path(temporary)
        fake = FakeCli(wrong_backend=True)
        evidence = module.run_capture_eval(
            config(module, root, contract.selected_scenarios(["focused_visible"])),
            cli_runner=fake,
            input_reader=lambda _prompt: "",
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "failed"
        assert "retained PipeWire backend" in evidence["errors"][0]
        assert evidence["session_closed"] is True
        assert fake.commands[-1][:2] == ("capture", "close")

    with tempfile.TemporaryDirectory(prefix="seatgeist-retained-interrupt-") as temporary:
        root = Path(temporary)
        fake = FakeCli()

        def interrupt(_prompt: str) -> str:
            raise KeyboardInterrupt

        evidence = module.run_capture_eval(
            config(module, root, contract.selected_scenarios(["focused_visible"])),
            cli_runner=fake,
            input_reader=interrupt,
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "failed"
        assert evidence["session_closed"] is True
        assert evidence["errors"] == [
            "operator interrupted retained-capture evaluation"
        ]
        assert fake.commands[-1][:2] == ("capture", "close")

    with tempfile.TemporaryDirectory(prefix="seatgeist-retained-negative-") as temporary:
        root = Path(temporary)
        fake = FakeCli(multi_output=True)
        evidence = module.run_capture_eval(
            config(
                module,
                root,
                contract.selected_scenarios(["monitor_or_scale_change"]),
                require_multi_output_nonzero_origin=True,
            ),
            cli_runner=fake,
            input_reader=answers_for(1),
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "partial"
        assert evidence["monitor_layout"]["monitor_count"] == 2
        assert evidence["monitor_layout"]["has_nonzero_logical_origin"] is True
        assert evidence["layout_requirement"] == "multi_output_nonzero_origin"
        assert "private-monitor" not in json.dumps(evidence)
        assert "private-name" not in json.dumps(evidence)

    with tempfile.TemporaryDirectory(prefix="seatgeist-retained-nonnegative-") as temporary:
        root = Path(temporary)
        fake = FakeCli()
        evidence = module.run_capture_eval(
            config(
                module,
                root,
                contract.selected_scenarios(["monitor_or_scale_change"]),
                require_multi_output_nonzero_origin=True,
            ),
            cli_runner=fake,
            input_reader=answers_for(1),
            message_writer=lambda _message: None,
        )
        assert evidence["status"] == "failed"
        assert "multiple outputs with a non-zero logical origin" in evidence["errors"][0]
        assert fake.commands == [("monitors",)]

    print("test-retained-capture-eval: ok")


if __name__ == "__main__":
    main()
