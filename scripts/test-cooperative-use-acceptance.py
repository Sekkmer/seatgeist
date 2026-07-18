#!/usr/bin/env python3
from __future__ import annotations

import copy
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

from computer_use_eval import EvalError, ROOT, workspace_revision, write_private_json
from cooperative_acceptance import REQUIRED_BUDGETS, RETAINED_SCENARIOS, SPECS
from cooperative_acceptance_bundle import build_bundle


NOW = 1_000_000
START = 900_000
END = 901_000
WORKSPACE = {
    "git_head": "a" * 40,
    "tree_sha256": "b" * 64,
    "dirty": True,
}


def common(evidence_type: str, version: int) -> dict[str, Any]:
    return {
        "type": evidence_type,
        "version": version,
        "status": "passed",
        "acceptance_complete": True,
        "workspace": copy.deepcopy(WORKSPACE),
        "started_unix_ms": START,
        "ended_unix_ms": END,
        "errors": [],
    }


def retained(*, multi_output: bool) -> dict[str, Any]:
    evidence = common("seatgeist_retained_capture_eval", 1)
    evidence.update(
        {
            "all_scenarios_selected": True,
            "selected_scenarios": sorted(RETAINED_SCENARIOS),
            "scenarios": [
                {
                    "name": name,
                    "visual_verdict": "pass",
                    "frame": {"fresh_frame": True},
                }
                for name in sorted(RETAINED_SCENARIOS)
            ],
            "layout_requirement": (
                "multi_output_nonzero_origin" if multi_output else "none"
            ),
            "monitor_layout": {
                "monitor_count": 2 if multi_output else 1,
                "has_negative_logical_origin": False,
                "has_nonzero_logical_origin": multi_output,
            },
            "portal_open_count": 1,
            "explicit_focus_call_count": 0,
            "session_closed": True,
        }
    )
    return evidence


def fixtures() -> dict[str, dict[str, Any]]:
    restore = common("seatgeist_capture_restore_restart_eval", 1)
    restore.update(
        {
            "daemon_restart_proven": True,
            "prepare_session_closed": True,
            "resume_session_closed": True,
            "resume": {
                "portal_chooser_avoided": True,
                "restore_file_replaced": True,
            },
        }
    )
    lifecycle = common("seatgeist_capture_lifecycle_eval", 1)
    lifecycle.update(
        {
            "ended_status": {"last_end_reason": "portal_closed"},
            "stale_session_rejected": True,
            "cleanup_close_called": False,
            "explicit_focus_call_count": 0,
            "raw_input_call_count": 0,
        }
    )
    reopen = common("seatgeist_target_reopen_eval", 1)
    reopen.update(
        {
            "replacement": {"same_app": True},
            "post_reopen_status": {"sticky_target_bound": False},
            "session_cleanup": "client_closed",
            "explicit_focus_call_count": 0,
            "raw_input_call_count": 0,
        }
    )

    def background(scenario: str) -> dict[str, Any]:
        evidence = common("seatgeist_background_semantic_eval", 2)
        evidence.update(
            {
                "scenario": scenario,
                "non_target_focus_before": True,
                "non_target_focus_after": True,
                "user_window_changed_during_action": True,
                "semantic_action_succeeded": True,
                "visual_change_confirmed": True,
                "operator_target_never_focused_confirmed": True,
                "journal_match_count": 1,
                "explicit_focus_call_count": 0,
                "raw_input_call_count": 0,
            }
        )
        return evidence

    cooperative = {
        "type": "seatgeist_computer_use_baseline",
        "version": 2,
        "scenario": "firefox-sticky-live",
        "workspace": copy.deepcopy(WORKSPACE),
        "filters": {"start_unix_ms": START, "end_unix_ms": END},
        "budget": {name: True for name in REQUIRED_BUDGETS},
        "metrics": {
            "sticky_raw_action_count": 20,
            "model_focus_poll_request_count": 0,
            "portal_open_request_count": 1,
            "focus_restore_success_count": 20,
            "input_activity_check_count": 20,
            "reliability_failure_count": 0,
        },
    }
    return {
        "retained_capture": retained(multi_output=False),
        "retained_capture_multi_output": retained(multi_output=True),
        "capture_restore_restart": restore,
        "capture_revocation": lifecycle,
        "target_reopen": reopen,
        "background_semantic_firefox": background("firefox"),
        "background_semantic_kde": background("kde"),
        "cooperative_sticky": cooperative,
    }


def write_fixtures(root: Path, values: dict[str, dict[str, Any]]) -> dict[str, Path]:
    paths: dict[str, Path] = {}
    for name, value in values.items():
        path = root / f"{name}.json"
        write_private_json(path, value)
        paths[name] = path
    return paths


def expect_error(action: Any, text: str) -> None:
    try:
        action()
    except EvalError as err:
        assert text in str(err), err
    else:
        raise AssertionError(f"expected acceptance failure containing {text!r}")


def run_cli(paths: dict[str, Path], output: Path) -> subprocess.CompletedProcess[str]:
    flags = {
        "retained_capture": "--retained-capture",
        "retained_capture_multi_output": "--retained-capture-multi-output",
        "capture_restore_restart": "--capture-restore-restart",
        "capture_revocation": "--capture-revocation",
        "target_reopen": "--target-reopen",
        "background_semantic_firefox": "--background-semantic-firefox",
        "background_semantic_kde": "--background-semantic-kde",
        "cooperative_sticky": "--cooperative-sticky",
    }
    command = [str(ROOT / "scripts/cooperative-use-acceptance.py")]
    for name, flag in flags.items():
        command.extend([flag, str(paths[name])])
    command.extend(
        [
            "--max-age-hours",
            "1",
            "--max-span-hours",
            "1",
            "--output",
            str(output),
        ]
    )
    return subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def main() -> None:
    assert {spec.name for spec in SPECS} == set(fixtures())
    with tempfile.TemporaryDirectory(prefix="seatgeist-acceptance-bundle-") as temporary:
        root = Path(temporary)
        values = fixtures()
        paths = write_fixtures(root, values)
        bundle = build_bundle(
            paths,
            expected_workspace=WORKSPACE,
            now_unix_ms=NOW,
            max_age_ms=200_000,
            max_span_ms=200_000,
        )
        assert bundle["status"] == "passed"
        assert bundle["acceptance_complete"] is True
        assert len(bundle["artifacts"]) == len(SPECS)
        encoded = str(bundle)
        assert str(root) not in encoded
        assert "target_window_sha256" not in encoded

        mismatched = fixtures()
        mismatched["background_semantic_kde"]["workspace"]["tree_sha256"] = "c" * 64
        mismatch_paths = write_fixtures(root / "mismatch", mismatched)
        expect_error(
            lambda: build_bundle(
                mismatch_paths,
                expected_workspace=WORKSPACE,
                now_unix_ms=NOW,
                max_age_ms=200_000,
                max_span_ms=200_000,
            ),
            "different workspace revision",
        )

        failed = fixtures()
        failed["cooperative_sticky"]["budget"]["model_focus_polling_zero"] = False
        failed_paths = write_fixtures(root / "failed", failed)
        expect_error(
            lambda: build_bundle(
                failed_paths,
                expected_workspace=WORKSPACE,
                now_unix_ms=NOW,
                max_age_ms=200_000,
                max_span_ms=200_000,
            ),
            "budgets failed",
        )

        stale_paths = write_fixtures(root / "stale", fixtures())
        expect_error(
            lambda: build_bundle(
                stale_paths,
                expected_workspace=WORKSPACE,
                now_unix_ms=NOW + 1_000_000,
                max_age_ms=200_000,
                max_span_ms=200_000,
            ),
            "stale",
        )
        incomplete = dict(paths)
        incomplete.pop("capture_revocation")
        expect_error(
            lambda: build_bundle(
                incomplete,
                expected_workspace=WORKSPACE,
                now_unix_ms=NOW,
                max_age_ms=200_000,
                max_span_ms=200_000,
            ),
            "artifact set is incomplete",
        )

        missing_paths = write_fixtures(root / "missing", fixtures())
        missing_paths["capture_revocation"].unlink()
        expect_error(
            lambda: build_bundle(
                missing_paths,
                expected_workspace=WORKSPACE,
                now_unix_ms=NOW,
                max_age_ms=200_000,
                max_span_ms=200_000,
            ),
            "missing",
        )

        producer_mismatch = fixtures()
        producer_mismatch["retained_capture"]["scenarios"][0]["name"] = "wrong"
        mismatch_paths = write_fixtures(root / "producer-mismatch", producer_mismatch)
        expect_error(
            lambda: build_bundle(
                mismatch_paths,
                expected_workspace=WORKSPACE,
                now_unix_ms=NOW,
                max_age_ms=200_000,
                max_span_ms=200_000,
            ),
            "wrong names",
        )

        cli_values = fixtures()
        cli_workspace = workspace_revision()
        cli_end = int(time.time() * 1000)
        cli_start = cli_end - 5_000
        for value in cli_values.values():
            value["workspace"] = copy.deepcopy(cli_workspace)
            if value["type"] == "seatgeist_computer_use_baseline":
                value["filters"]["start_unix_ms"] = cli_start
                value["filters"]["end_unix_ms"] = cli_end
            else:
                value["started_unix_ms"] = cli_start
                value["ended_unix_ms"] = cli_end
        cli_paths = write_fixtures(root / "cli", cli_values)
        cli_output = root / "cli-bundle.json"
        completed = run_cli(cli_paths, cli_output)
        assert completed.returncode == 0, completed.stderr
        assert "all scenarios and budgets passed" in completed.stdout
        assert cli_output.stat().st_mode & 0o777 == 0o600
        cli_bundle = json.loads(cli_output.read_text(encoding="utf-8"))
        assert cli_bundle["workspace"] == cli_workspace
        assert cli_bundle["acceptance_complete"] is True

        make_output = root / "make-bundle.json"
        make_environment = {
            **os.environ,
            "RETAINED_CAPTURE_EVIDENCE": str(cli_paths["retained_capture"]),
            "MULTI_OUTPUT_EVIDENCE": str(
                cli_paths["retained_capture_multi_output"]
            ),
            "CAPTURE_RESTORE_EVIDENCE": str(cli_paths["capture_restore_restart"]),
            "CAPTURE_REVOCATION_EVIDENCE": str(cli_paths["capture_revocation"]),
            "TARGET_REOPEN_EVIDENCE": str(cli_paths["target_reopen"]),
            "BACKGROUND_FIREFOX_EVIDENCE": str(
                cli_paths["background_semantic_firefox"]
            ),
            "BACKGROUND_KDE_EVIDENCE": str(
                cli_paths["background_semantic_kde"]
            ),
            "COOPERATIVE_STICKY_EVIDENCE": str(cli_paths["cooperative_sticky"]),
            "ACCEPTANCE_MAX_AGE_HOURS": "1",
            "ACCEPTANCE_MAX_SPAN_HOURS": "1",
            "ACCEPTANCE_OUTPUT": str(make_output),
        }
        make_completed = subprocess.run(
            ["make", "--no-print-directory", "verify-cooperative-use-acceptance"],
            cwd=ROOT,
            env=make_environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        assert make_completed.returncode == 0, make_completed.stderr
        assert make_output.stat().st_mode & 0o777 == 0o600
        make_bundle = json.loads(make_output.read_text(encoding="utf-8"))
        assert make_bundle["workspace"] == cli_bundle["workspace"]
        assert make_bundle["artifacts"] == cli_bundle["artifacts"]
        assert make_bundle["acceptance_complete"] is True

    print("test-cooperative-use-acceptance: ok")


if __name__ == "__main__":
    main()
