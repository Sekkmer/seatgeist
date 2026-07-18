from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable

from computer_use_eval import EvalError


RETAINED_SCENARIOS = frozenset(
    {
        "focused_visible",
        "unfocused_visible",
        "partially_occluded",
        "fully_occluded",
        "minimized",
        "popup_or_dialog",
        "moved_resized",
        "monitor_or_scale_change",
    }
)
REQUIRED_BUDGETS = frozenset(
    {
        "sticky_budget_applicable",
        "reliability_below_5_percent",
        "model_focus_polling_zero",
        "automatic_focus_verification_failures_zero",
        "successful_sticky_actions_all_verified",
        "successful_sticky_actions_all_activity_checked",
        "repeated_portal_prompts_zero",
    }
)


@dataclass(frozen=True)
class ArtifactSpec:
    name: str
    evidence_type: str
    version: int
    validator: Callable[[dict[str, Any]], None]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise EvalError(message)


def require_common(evidence: dict[str, Any]) -> None:
    require(evidence.get("status") == "passed", "evidence status is not passed")
    require(
        evidence.get("acceptance_complete") is True,
        "evidence acceptance_complete is not true",
    )
    require(evidence.get("errors") == [], "evidence contains errors")


def validate_retained(evidence: dict[str, Any]) -> None:
    require_common(evidence)
    require(evidence.get("all_scenarios_selected") is True, "not all capture scenarios ran")
    selected = evidence.get("selected_scenarios")
    require(
        isinstance(selected, list) and set(selected) == RETAINED_SCENARIOS,
        "retained capture scenario set is incomplete",
    )
    scenarios = evidence.get("scenarios")
    require(
        isinstance(scenarios, list) and len(scenarios) == len(RETAINED_SCENARIOS),
        "retained capture scenario results are incomplete",
    )
    scenario_names = {
        scenario.get("name")
        for scenario in scenarios
        if isinstance(scenario, dict)
    }
    require(
        scenario_names == RETAINED_SCENARIOS,
        "retained capture scenario results have the wrong names",
    )
    for scenario in scenarios:
        require(isinstance(scenario, dict), "retained capture scenario is malformed")
        frame = scenario.get("frame")
        require(
            scenario.get("visual_verdict") == "pass"
            and isinstance(frame, dict)
            and frame.get("fresh_frame") is True,
            "retained capture scenario did not pass with a fresh frame",
        )
    require(evidence.get("portal_open_count") == 1, "retained capture opened the portal more than once")
    require(evidence.get("explicit_focus_call_count") == 0, "retained capture used explicit focus")
    require(evidence.get("session_closed") is True, "retained capture session was not closed")


def validate_multi_output(evidence: dict[str, Any]) -> None:
    validate_retained(evidence)
    layout = evidence.get("monitor_layout")
    require(
        evidence.get("layout_requirement") == "multi_output_nonzero_origin",
        "multi-output retained capture used the wrong layout requirement",
    )
    require(
        isinstance(layout, dict)
        and isinstance(layout.get("monitor_count"), int)
        and layout["monitor_count"] >= 2
        and layout.get("has_nonzero_logical_origin") is True,
        "multi-output retained capture has no non-zero monitor origin",
    )


def validate_restore(evidence: dict[str, Any]) -> None:
    require_common(evidence)
    resume = evidence.get("resume")
    require(evidence.get("daemon_restart_proven") is True, "daemon restart is not proven")
    require(
        evidence.get("prepare_session_closed") is True
        and evidence.get("resume_session_closed") is True,
        "capture-restore sessions were not closed",
    )
    require(
        isinstance(resume, dict)
        and resume.get("portal_chooser_avoided") is True
        and resume.get("restore_file_replaced") is True,
        "capture restore did not avoid the chooser and rotate private state",
    )


def validate_lifecycle(evidence: dict[str, Any]) -> None:
    require_common(evidence)
    ended = evidence.get("ended_status")
    require(
        isinstance(ended, dict) and ended.get("last_end_reason") == "portal_closed",
        "portal revocation was not attributed to portal_closed",
    )
    require(evidence.get("stale_session_rejected") is True, "stale capture session was not rejected")
    require(evidence.get("cleanup_close_called") is False, "revoked session required client cleanup")
    require(evidence.get("explicit_focus_call_count") == 0, "lifecycle eval used explicit focus")
    require(evidence.get("raw_input_call_count") == 0, "lifecycle eval used raw input")


def validate_reopen(evidence: dict[str, Any]) -> None:
    require_common(evidence)
    post = evidence.get("post_reopen_status")
    require(bool(evidence.get("replacement")), "replacement window identity was not observed")
    require(
        isinstance(post, dict) and post.get("sticky_target_bound") is False,
        "reopened target silently retained sticky authority",
    )
    require(
        evidence.get("session_cleanup") in {"client_closed", "portal_ended"},
        "target-reopen session cleanup failed",
    )
    require(evidence.get("explicit_focus_call_count") == 0, "target-reopen eval used explicit focus")
    require(evidence.get("raw_input_call_count") == 0, "target-reopen eval used raw input")


def validate_background(expected_scenario: str) -> Callable[[dict[str, Any]], None]:
    def validate(evidence: dict[str, Any]) -> None:
        require_common(evidence)
        require(evidence.get("scenario") == expected_scenario, "background semantic scenario is wrong")
        require(
            evidence.get("non_target_focus_before") is True
            and evidence.get("non_target_focus_after") is True,
            "background semantic target received focus",
        )
        require(evidence.get("semantic_action_succeeded") is True, "background semantic action failed")
        require(
            evidence.get("visual_change_confirmed") is True
            and evidence.get("operator_target_never_focused_confirmed") is True,
            "background semantic effect was not confirmed",
        )
        require(evidence.get("journal_match_count") == 1, "background semantic journal correlation is wrong")
        require(evidence.get("explicit_focus_call_count") == 0, "background semantic eval used focus")
        require(evidence.get("raw_input_call_count") == 0, "background semantic eval used raw input")

    return validate


def validate_cooperative(evidence: dict[str, Any]) -> None:
    require(evidence.get("scenario") == "firefox-sticky-live", "cooperative scenario is wrong")
    budget = evidence.get("budget")
    metrics = evidence.get("metrics")
    require(isinstance(budget, dict), "cooperative budget is missing")
    require(
        REQUIRED_BUDGETS.issubset(budget) and all(budget[name] is True for name in REQUIRED_BUDGETS),
        "one or more cooperative-use budgets failed",
    )
    require(isinstance(metrics, dict), "cooperative metrics are missing")
    sticky = metrics.get("sticky_raw_action_count")
    require(isinstance(sticky, int) and sticky >= 20, "fewer than 20 sticky actions were measured")
    require(metrics.get("model_focus_poll_request_count") == 0, "model focus polling was measured")
    require(metrics.get("portal_open_request_count") == 1, "cooperative eval did not use one portal open")
    require(
        metrics.get("focus_restore_success_count") == sticky,
        "not every sticky action restored user focus",
    )
    require(
        metrics.get("input_activity_check_count") == sticky,
        "not every sticky action had a final activity check",
    )
    require(metrics.get("reliability_failure_count") == 0, "cooperative eval has reliability failures")


SPECS = (
    ArtifactSpec("retained_capture", "seatgeist_retained_capture_eval", 1, validate_retained),
    ArtifactSpec(
        "retained_capture_multi_output",
        "seatgeist_retained_capture_eval",
        1,
        validate_multi_output,
    ),
    ArtifactSpec(
        "capture_restore_restart",
        "seatgeist_capture_restore_restart_eval",
        1,
        validate_restore,
    ),
    ArtifactSpec("capture_revocation", "seatgeist_capture_lifecycle_eval", 1, validate_lifecycle),
    ArtifactSpec("target_reopen", "seatgeist_target_reopen_eval", 1, validate_reopen),
    ArtifactSpec(
        "background_semantic_firefox",
        "seatgeist_background_semantic_eval",
        2,
        validate_background("firefox"),
    ),
    ArtifactSpec(
        "background_semantic_kde",
        "seatgeist_background_semantic_eval",
        2,
        validate_background("kde"),
    ),
    ArtifactSpec(
        "cooperative_sticky",
        "seatgeist_computer_use_baseline",
        2,
        validate_cooperative,
    ),
)
