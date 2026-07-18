from __future__ import annotations

from typing import Any

from computer_use_eval import EvalError, response_data


def validate_portal_closed_status(response: dict[str, Any]) -> dict[str, Any]:
    data = response_data(response, "capture_session_status")
    if data.get("active") is not False or data.get("opening") is not False:
        raise EvalError("capture session is still active or opening")
    if data.get("session_id") is not None:
        raise EvalError("ended capture status still exposes a session id")
    if data.get("last_end_reason") != "portal_closed":
        raise EvalError("capture status did not attribute the end to portal closure")
    if data.get("sticky_target_bound") is True or data.get("target_window_id") is not None:
        raise EvalError("portal-closed capture still exposes a sticky target binding")
    return {
        "active": False,
        "opening": False,
        "last_end_reason": "portal_closed",
        "sticky_target_bound": False,
    }


def lifecycle_acceptance_complete(evidence: dict[str, Any]) -> bool:
    ended = evidence.get("ended_status")
    return all(
        (
            isinstance(ended, dict),
            ended.get("last_end_reason") == "portal_closed"
            if isinstance(ended, dict)
            else False,
            evidence.get("initial_frame_captured") is True,
            evidence.get("stale_session_rejected") is True,
            evidence.get("cleanup_close_called") is False,
            evidence.get("explicit_focus_call_count") == 0,
            evidence.get("raw_input_call_count") == 0,
        )
    )


def stale_session_rejection_kind(error: Exception) -> str | None:
    message = str(error).lower()
    if "session owner mismatch" in message or "sessionownermismatch" in message:
        return "session_owner_mismatch"
    if "no active capture session" in message or "session ended" in message:
        return "session_ended"
    return None
