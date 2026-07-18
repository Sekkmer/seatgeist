#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
import time
from collections import Counter
from pathlib import Path
from typing import Any, Iterable

from computer_use_eval import workspace_revision, write_private_json


ROOT = Path(__file__).resolve().parents[1]
ERROR_KIND_RE = re.compile(r"^error kind=([A-Za-z][A-Za-z0-9_]*):")
MAX_SCENARIO_CHARS = 96

PREFLIGHT_METHODS = frozenset(
    {
        "health",
        "capabilities",
        "policy_status",
        "safety_status",
        "desktop_session_status",
        "computer_use_readiness",
        "panic_stop_status",
        "kwin_bridge_status",
        "uinput_status",
        "input_backend_status",
        "remote_desktop_eis_session_status",
        "capture_backend_status",
        "pointer_calibration",
        "clipboard_backend_status",
        "accessibility_quality_status",
    }
)
OBSERVATION_METHODS = frozenset(
    {
        "list_monitors",
        "list_windows",
        "active_window",
        "observe",
        "screenshot",
        "screenshot_tile",
        "wait_for_change",
        "focused_accessibility_tree",
        "accessibility_find",
        "accessibility_text_attributes",
    }
)
CAPTURE_METHODS = frozenset({"screenshot", "screenshot_tile", "wait_for_change"})
STICKY_RAW_METHODS = frozenset(
    {"type_text", "key_combo", "move_pointer", "click_pointer", "drag_pointer", "scroll_pointer"}
)
EXPECTED_OUTCOME_KINDS = frozenset(
    {
        "policypromptrequired",
        "policydenied",
        "appdenied",
        "humaninputpause",
        "panicstop",
        "targetlost",
        "targetmismatch",
        "consentcancelled",
        "sessionownermismatch",
    }
)


class BaselineError(RuntimeError):
    pass


def git_head() -> str | None:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short=12", "HEAD"],
            cwd=ROOT,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def validate_scenario(value: str) -> str:
    scenario = value.strip()
    if not scenario:
        raise BaselineError("scenario must not be empty")
    if len(scenario) > MAX_SCENARIO_CHARS:
        raise BaselineError(f"scenario must be at most {MAX_SCENARIO_CHARS} characters")
    if any(not character.isprintable() for character in scenario):
        raise BaselineError("scenario must contain only printable characters")
    return scenario


def read_journal(path: Path) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as err:
        raise BaselineError(f"read journal {path}: {err}") from err

    for line_number, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as err:
            raise BaselineError(f"journal line {line_number} is invalid JSON: {err}") from err
        if not isinstance(value, dict):
            raise BaselineError(f"journal line {line_number} is not an object")
        entries.append(value)
    return entries


def optional_int(value: Any) -> int | None:
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def client_field(entry: dict[str, Any], field: str) -> Any:
    client = entry.get("client")
    return client.get(field) if isinstance(client, dict) else None


def entry_matches(
    entry: dict[str, Any],
    *,
    client_tool: str | None,
    client_pid: int | None,
    start_unix_ms: int | None,
    end_unix_ms: int | None,
) -> bool:
    if client_tool is not None and client_field(entry, "tool") != client_tool:
        return False
    if client_pid is not None and optional_int(client_field(entry, "pid")) != client_pid:
        return False
    timestamp = optional_int(entry.get("unix_time_ms"))
    if start_unix_ms is not None and (timestamp is None or timestamp < start_unix_ms):
        return False
    if end_unix_ms is not None and (timestamp is None or timestamp > end_unix_ms):
        return False
    return True


def is_control(entry: dict[str, Any]) -> bool:
    safety_class = entry.get("safety_class")
    return safety_class in {
        "control_pointer",
        "control_keyboard",
        "control_semantic",
        "destructive_action",
        "secret_field",
    }


def window_id(entry: dict[str, Any], key: str) -> str | None:
    context = entry.get(key)
    if not isinstance(context, dict):
        return None
    value = context.get("id")
    return value if isinstance(value, str) and value else None


def error_kind(entry: dict[str, Any]) -> str:
    summary = entry.get("summary")
    if not isinstance(summary, str):
        return "Unclassified"
    matched = ERROR_KIND_RE.match(summary)
    return matched.group(1) if matched else "Unclassified"


def normalized_kind(entry: dict[str, Any]) -> str:
    return re.sub(r"[^a-z0-9]", "", error_kind(entry).lower())


def failure_category(entry: dict[str, Any]) -> str:
    kind = normalized_kind(entry)
    if kind in {"policypromptrequired", "policydenied", "appdenied"}:
        return "policy_or_approval"
    if kind == "consentcancelled":
        return "user_consent_cancelled"
    if kind in {"targetlost", "targetmismatch"}:
        return "target_lost_or_ambiguous"
    if kind == "sessionownermismatch":
        return "parallel_owner_conflict"
    if kind in {"humaninputpause", "panicstop"}:
        return "user_activity_or_stop"
    if kind in {"focusleaseconflict", "focusguard"}:
        return "focus_conflict"
    if kind in {"accessibilityunavailable", "accessibilityweaktree"}:
        return "semantic_capability_missing"
    if kind in {"portalunavailable", "backendunavailable"}:
        return "backend_unavailable"
    if kind == "backendfailed":
        return "backend_failure"
    if kind == "ratelimited":
        return "rate_limit"
    if kind == "validation":
        return "validation"
    return "implementation_defect_or_unclassified"


def control_action_id(entry: dict[str, Any]) -> str | None:
    control = entry.get("control")
    if not isinstance(control, dict):
        return None
    value = control.get("action_id")
    return value if isinstance(value, str) and value else None


def requested_target_field(entry: dict[str, Any], field: str) -> str | None:
    control = entry.get("control")
    target = control.get("requested_target") if isinstance(control, dict) else None
    fields = target.get("fields") if isinstance(target, dict) else None
    value = fields.get(field) if isinstance(fields, dict) else None
    return value if isinstance(value, str) and value else None


def count_failure_categories(entries: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    counts = Counter(failure_category(entry) for entry in entries if entry.get("ok") is not True)
    return [{"category": category, "count": counts[category]} for category in sorted(counts)]


def ratio(numerator: int, denominator: int) -> float:
    return round(numerator / denominator, 6) if denominator else 0.0


def count_methods(entries: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    requests: Counter[str] = Counter()
    failures: Counter[str] = Counter()
    for entry in entries:
        method = entry.get("method")
        if not isinstance(method, str) or not method:
            method = "unknown"
        requests[method] += 1
        if entry.get("ok") is not True:
            failures[method] += 1
    return [
        {
            "method": method,
            "requests": requests[method],
            "failures": failures[method],
        }
        for method in sorted(requests)
    ]


def count_failure_kinds(entries: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    counts = Counter(error_kind(entry) for entry in entries if entry.get("ok") is not True)
    return [{"kind": kind, "count": counts[kind]} for kind in sorted(counts)]


def build_report(
    entries: list[dict[str, Any]],
    *,
    scenario: str,
    client_tool: str | None,
    client_pid: int | None,
    start_unix_ms: int | None,
    end_unix_ms: int | None,
    generated_unix_ms: int | None = None,
    git: str | None = None,
    workspace: dict[str, Any] | None = None,
) -> dict[str, Any]:
    scenario = validate_scenario(scenario)
    selected = [
        entry
        for entry in entries
        if entry_matches(
            entry,
            client_tool=client_tool,
            client_pid=client_pid,
            start_unix_ms=start_unix_ms,
            end_unix_ms=end_unix_ms,
        )
    ]
    if not selected:
        raise BaselineError("no journal entries matched the requested task filters")

    action_ids = {action_id for entry in selected if (action_id := control_action_id(entry))}
    internal = [
        entry
        for entry in entries
        if client_field(entry, "tool") is None
        and control_action_id(entry) in action_ids
        and entry_matches(
            entry,
            client_tool=None,
            client_pid=None,
            start_unix_ms=start_unix_ms,
            end_unix_ms=end_unix_ms,
        )
    ]

    requests = len(selected)
    failures = sum(entry.get("ok") is not True for entry in selected)
    control_entries = [entry for entry in selected if is_control(entry)]
    control_failures = sum(entry.get("ok") is not True for entry in control_entries)
    guarded_control = sum(entry.get("guard_present") is True for entry in control_entries)
    preflight = sum(entry.get("method") in PREFLIGHT_METHODS for entry in selected)
    observations = sum(entry.get("method") in OBSERVATION_METHODS for entry in selected)
    captures = sum(entry.get("method") in CAPTURE_METHODS for entry in selected)
    focus_entries = [entry for entry in selected if entry.get("method") == "focus_window"]
    focus_failures = sum(entry.get("ok") is not True for entry in focus_entries)
    active_window_checks = sum(entry.get("method") == "active_window" for entry in selected)
    focus_context_changes = sum(
        before is not None and after is not None and before != after
        for entry in selected
        for before, after in [(window_id(entry, "active_window_before"), window_id(entry, "active_window_after"))]
    )
    timestamps = sorted(
        timestamp
        for entry in selected
        if (timestamp := optional_int(entry.get("unix_time_ms"))) is not None
    )
    duration_ms = timestamps[-1] - timestamps[0] if len(timestamps) >= 2 else 0
    sticky_raw = [
        entry
        for entry in selected
        if entry.get("method") in STICKY_RAW_METHODS
        and requested_target_field(entry, "session_id") is not None
    ]
    verified_action_ids = {
        action_id
        for entry in internal
        if entry.get("method") == "interaction_focus_verify"
        and entry.get("ok") is True
        and (action_id := control_action_id(entry))
    }
    successful_sticky_ids = {
        action_id
        for entry in sticky_raw
        if entry.get("ok") is True and (action_id := control_action_id(entry))
    }
    activity_checked_action_ids = {
        action_id
        for entry in internal
        if entry.get("method") == "interaction_input_activity"
        and entry.get("ok") is True
        and (action_id := control_action_id(entry))
    }
    reliability_failures = [
        entry
        for entry in selected
        if entry.get("ok") is not True and normalized_kind(entry) not in EXPECTED_OUTCOME_KINDS
    ]
    reliability_denominator = requests - sum(
        entry.get("ok") is not True and normalized_kind(entry) in EXPECTED_OUTCOME_KINDS
        for entry in selected
    )
    active_focus_polling = active_window_checks + len(focus_entries)
    focus_verify_failures = sum(
        entry.get("method") == "interaction_focus_verify" and entry.get("ok") is not True
        for entry in internal
    )
    portal_open_count = sum(entry.get("method") == "window_capture_open" for entry in selected)
    unverified_successful_sticky = len(successful_sticky_ids - verified_action_ids)
    unchecked_activity_successful_sticky = len(
        successful_sticky_ids - activity_checked_action_ids
    )
    sticky_budget_applicable = bool(sticky_raw)

    return {
        "type": "seatgeist_computer_use_baseline",
        "version": 2,
        "scenario": scenario,
        "generated_unix_ms": generated_unix_ms if generated_unix_ms is not None else int(time.time() * 1000),
        "git": git if git is not None else git_head(),
        "workspace": workspace if workspace is not None else workspace_revision(),
        "filters": {
            "client_tool": client_tool,
            "client_pid": client_pid,
            "start_unix_ms": start_unix_ms,
            "end_unix_ms": end_unix_ms,
        },
        "metrics": {
            "request_count": requests,
            "success_count": requests - failures,
            "failure_count": failures,
            "failure_rate": ratio(failures, requests),
            "reliability_failure_count": len(reliability_failures),
            "reliability_failure_rate": ratio(len(reliability_failures), reliability_denominator),
            "duration_ms": duration_ms,
            "preflight_request_count": preflight,
            "observation_request_count": observations,
            "capture_request_count": captures,
            "active_window_check_count": active_window_checks,
            "focus_request_count": len(focus_entries),
            "focus_failure_count": focus_failures,
            "focus_context_change_count": focus_context_changes,
            "model_focus_poll_request_count": active_focus_polling,
            "portal_open_request_count": portal_open_count,
            "sticky_raw_action_count": len(sticky_raw),
            "sticky_success_without_focus_verification_count": unverified_successful_sticky,
            "sticky_success_without_input_activity_check_count": unchecked_activity_successful_sticky,
            "internal_action_step_count": len(internal),
            "automatic_focus_request_count": sum(
                entry.get("method") == "interaction_focus" for entry in internal
            ),
            "automatic_focus_verify_failure_count": focus_verify_failures,
            "input_activity_check_count": sum(
                entry.get("method") == "interaction_input_activity" for entry in internal
            ),
            "input_activity_conflict_count": sum(
                entry.get("method") == "interaction_input_activity"
                and entry.get("ok") is not True
                for entry in internal
            ),
            "focus_restore_request_count": sum(
                entry.get("method") == "interaction_restore_focus" for entry in internal
            ),
            "focus_restore_success_count": sum(
                entry.get("method") == "interaction_restore_verify" and entry.get("ok") is True
                for entry in internal
            ),
            "control_request_count": len(control_entries),
            "control_failure_count": control_failures,
            "guarded_control_count": guarded_control,
            "unguarded_control_count": len(control_entries) - guarded_control,
            "methods": count_methods(selected),
            "failure_kinds": count_failure_kinds(selected),
            "failure_categories": count_failure_categories(selected),
        },
        "budget": {
            "sticky_budget_applicable": sticky_budget_applicable,
            "reliability_below_5_percent": ratio(
                len(reliability_failures), reliability_denominator
            )
            < 0.05,
            "model_focus_polling_zero": active_focus_polling == 0,
            "automatic_focus_verification_failures_zero": (
                focus_verify_failures == 0 if sticky_budget_applicable else None
            ),
            "successful_sticky_actions_all_verified": (
                unverified_successful_sticky == 0 if sticky_budget_applicable else None
            ),
            "successful_sticky_actions_all_activity_checked": (
                unchecked_activity_successful_sticky == 0
                if sticky_budget_applicable
                else None
            ),
            "repeated_portal_prompts_zero": portal_open_count <= 1,
        },
    }


def non_negative_int(value: str) -> int:
    parsed = int(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be greater than or equal to zero")
    return parsed


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("must be greater than zero")
    return parsed


def write_report(report: dict[str, Any], output: Path | None) -> None:
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if output is None:
        print(encoded, end="")
        return
    write_private_json(output, report)
    print(f"computer-use-baseline: wrote {output}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Summarize a bounded Seatgeist journal task without copying UI content into evidence."
    )
    parser.add_argument("--journal", type=Path, required=True, help="Seatgeist JSONL journal to analyze.")
    parser.add_argument("--scenario", required=True, help="Short non-sensitive task/scenario label.")
    parser.add_argument(
        "--client-tool",
        default="seatgeist-mcp",
        help="Require this journal client tool. Pass an empty string to include all tools.",
    )
    parser.add_argument("--client-pid", type=positive_int, help="Optional client process id filter.")
    parser.add_argument("--start-unix-ms", type=non_negative_int, help="Inclusive task start timestamp.")
    parser.add_argument("--end-unix-ms", type=non_negative_int, help="Inclusive task end timestamp.")
    parser.add_argument("--output", type=Path, help="Optional JSON evidence output path.")
    args = parser.parse_args()

    if args.start_unix_ms is not None and args.end_unix_ms is not None and args.start_unix_ms > args.end_unix_ms:
        parser.error("--start-unix-ms must be less than or equal to --end-unix-ms")

    try:
        report = build_report(
            read_journal(args.journal),
            scenario=args.scenario,
            client_tool=args.client_tool or None,
            client_pid=args.client_pid,
            start_unix_ms=args.start_unix_ms,
            end_unix_ms=args.end_unix_ms,
        )
        write_report(report, args.output)
    except BaselineError as err:
        raise SystemExit(f"computer-use-baseline: {err}") from err


if __name__ == "__main__":
    main()
