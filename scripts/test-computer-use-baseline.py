#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "computer-use-baseline.py"


def entry(
    sequence: int,
    method: str,
    *,
    ok: bool = True,
    safety_class: str = "observe",
    summary: str = "ok",
    pid: int = 4242,
    guard_present: bool = False,
    before: str | None = None,
    after: str | None = None,
    action_id: str | None = None,
    session_id: str | None = None,
) -> dict[str, object]:
    value: dict[str, object] = {
        "sequence": sequence,
        "unix_time_ms": 1_000 + sequence * 10,
        "method": method,
        "client": {"tool": "seatgeist-mcp", "pid": pid, "process_name": "seatgeist-mcp"},
        "safety_class": safety_class,
        "guard_present": guard_present,
        "ok": ok,
        "summary": summary,
    }
    if before is not None:
        value["active_window_before"] = {
            "id": before,
            "title": "sensitive title before",
        }
    if after is not None:
        value["active_window_after"] = {
            "id": after,
            "title": "sensitive title after",
        }
    if action_id is not None:
        fields = {"session_id": session_id} if session_id is not None else {}
        value["control"] = {
            "action_id": action_id,
            "requested_target": {"kind": "fixture", "fields": fields},
        }
    return value


def run_script(journal: Path, *arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(SCRIPT), "--journal", str(journal), *arguments],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="seatgeist-computer-use-baseline-") as temporary:
        journal = Path(temporary) / "journal.jsonl"
        entries = [
            entry(1, "computer_use_readiness", safety_class="policy"),
            entry(2, "active_window"),
            entry(
                3,
                "focus_window",
                ok=False,
                safety_class="control_semantic",
                guard_present=True,
                summary="error kind=FocusGuard: sensitive window details must not escape",
                before="window-a",
                after="window-a",
            ),
            entry(
                4,
                "focus_window",
                safety_class="control_semantic",
                guard_present=True,
                before="window-a",
                after="window-b",
            ),
            entry(
                5,
                "click_pointer",
                safety_class="control_pointer",
                guard_present=False,
                before="window-b",
                after="window-b",
                action_id="00000000-0000-0000-0000-000000000005",
                session_id="capture-1",
            ),
            entry(
                6,
                "observe",
                ok=False,
                summary="error kind=ConsentCancelled: sensitive portal text",
            ),
            entry(7, "health", safety_class="policy", pid=9999),
        ]
        for offset, method in enumerate(
            [
                "interaction_focus",
                "interaction_focus_verify",
                "interaction_input_activity",
                "interaction_restore_focus",
                "interaction_restore_verify",
            ]
        ):
            internal = entry(
                20 + offset,
                method,
                safety_class="control_semantic",
                action_id="00000000-0000-0000-0000-000000000005",
            )
            internal.pop("client")
            internal["unix_time_ms"] = 1051 + offset
            entries.append(internal)
        journal.write_text("".join(json.dumps(value) + "\n" for value in entries), encoding="utf-8")

        completed = run_script(
            journal,
            "--scenario",
            "sticky-firefox-baseline",
            "--client-pid",
            "4242",
            "--start-unix-ms",
            "1010",
            "--end-unix-ms",
            "1060",
        )
        report = json.loads(completed.stdout)
        assert report["type"] == "seatgeist_computer_use_baseline"
        assert report["version"] == 2
        assert report["scenario"] == "sticky-firefox-baseline"
        assert report["filters"]["client_tool"] == "seatgeist-mcp"
        assert report["filters"]["client_pid"] == 4242

        metrics = report["metrics"]
        assert metrics["request_count"] == 6
        assert metrics["success_count"] == 4
        assert metrics["failure_count"] == 2
        assert metrics["failure_rate"] == 0.333333
        assert metrics["reliability_failure_count"] == 1
        assert metrics["reliability_failure_rate"] == 0.2
        assert metrics["preflight_request_count"] == 1
        assert metrics["observation_request_count"] == 2
        assert metrics["active_window_check_count"] == 1
        assert metrics["focus_request_count"] == 2
        assert metrics["focus_failure_count"] == 1
        assert metrics["focus_context_change_count"] == 1
        assert metrics["model_focus_poll_request_count"] == 3
        assert metrics["sticky_raw_action_count"] == 1
        assert metrics["sticky_success_without_focus_verification_count"] == 0
        assert metrics["sticky_success_without_input_activity_check_count"] == 0
        assert metrics["internal_action_step_count"] == 5
        assert metrics["automatic_focus_request_count"] == 1
        assert metrics["automatic_focus_verify_failure_count"] == 0
        assert metrics["input_activity_check_count"] == 1
        assert metrics["input_activity_conflict_count"] == 0
        assert metrics["focus_restore_request_count"] == 1
        assert metrics["focus_restore_success_count"] == 1
        assert metrics["control_request_count"] == 3
        assert metrics["guarded_control_count"] == 2
        assert metrics["unguarded_control_count"] == 1
        assert metrics["failure_kinds"] == [
            {"kind": "ConsentCancelled", "count": 1},
            {"kind": "FocusGuard", "count": 1},
        ]
        assert metrics["failure_categories"] == [
            {"category": "focus_conflict", "count": 1},
            {"category": "user_consent_cancelled", "count": 1},
        ]
        assert report["budget"] == {
            "automatic_focus_verification_failures_zero": True,
            "model_focus_polling_zero": False,
            "reliability_below_5_percent": False,
            "repeated_portal_prompts_zero": True,
            "sticky_budget_applicable": True,
            "successful_sticky_actions_all_activity_checked": True,
            "successful_sticky_actions_all_verified": True,
        }

        encoded = completed.stdout
        assert "sensitive title" not in encoded
        assert "sensitive portal text" not in encoded
        assert "sensitive window details" not in encoded

        no_match = run_script(
            journal,
            "--scenario",
            "empty-selection",
            "--client-pid",
            "12345",
            check=False,
        )
        assert no_match.returncode != 0
        assert "no journal entries matched" in no_match.stderr

        invalid = Path(temporary) / "invalid.jsonl"
        invalid.write_text("not-json\n", encoding="utf-8")
        invalid_result = run_script(invalid, "--scenario", "invalid-journal", check=False)
        assert invalid_result.returncode != 0
        assert "line 1 is invalid JSON" in invalid_result.stderr

        owner_journal = Path(temporary) / "owner-conflict.jsonl"
        owner_journal.write_text(
            json.dumps(
                entry(
                    1,
                    "capture_snapshot",
                    ok=False,
                    summary="error kind=SessionOwnerMismatch: private owner details",
                )
            )
            + "\n",
            encoding="utf-8",
        )
        owner_report = json.loads(
            run_script(owner_journal, "--scenario", "parallel-owner-conflict").stdout
        )
        assert owner_report["metrics"]["reliability_failure_count"] == 0
        assert owner_report["metrics"]["failure_categories"] == [
            {"category": "parallel_owner_conflict", "count": 1}
        ]
        assert "private owner details" not in json.dumps(owner_report)

    print("test-computer-use-baseline: ok")


if __name__ == "__main__":
    main()
