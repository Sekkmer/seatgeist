from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from computer_use_eval import EvalError, response_data
from retained_capture_eval import hashed_window_id


@dataclass(frozen=True)
class OriginalTarget:
    window_id: str
    app_id: str
    pid: int | None


def window_list(response: dict[str, Any]) -> list[dict[str, Any]]:
    if response.get("type") == "error":
        response_data(response, "windows")
    if response.get("type") != "windows" or not isinstance(response.get("data"), list):
        raise EvalError("window list response is malformed")
    if not all(isinstance(window, dict) for window in response["data"]):
        raise EvalError("window list contains a malformed entry")
    return response["data"]


def resolve_original_target(
    response: dict[str, Any], window_id: str
) -> OriginalTarget:
    target = next(
        (window for window in window_list(response) if window.get("id") == window_id),
        None,
    )
    if target is None:
        raise EvalError("requested original target is not in the KWin window list")
    app_id = target.get("app_id")
    if not isinstance(app_id, str) or not app_id:
        raise EvalError("original target has no app id")
    pid = target.get("pid")
    if pid is not None and (not isinstance(pid, int) or pid < 1):
        raise EvalError("original target pid is malformed")
    return OriginalTarget(window_id=window_id, app_id=app_id, pid=pid)


def find_replacement(
    response: dict[str, Any], original: OriginalTarget
) -> dict[str, Any] | None:
    windows = window_list(response)
    if any(window.get("id") == original.window_id for window in windows):
        return None
    replacement = next(
        (
            window
            for window in windows
            if window.get("app_id") == original.app_id
            and isinstance(window.get("id"), str)
            and window["id"] != original.window_id
        ),
        None,
    )
    if replacement is None:
        return None
    pid = replacement.get("pid")
    if pid is not None and (not isinstance(pid, int) or pid < 1):
        raise EvalError("replacement target pid is malformed")
    return {
        "window_id": replacement["id"],
        "pid": pid,
    }


def validate_unbound_capture_status(
    response: dict[str, Any], session_id: str
) -> dict[str, Any]:
    data = response_data(response, "capture_session_status")
    if data.get("sticky_target_bound") is True:
        raise EvalError("capture status silently retained the closed target binding")
    if data.get("target_window_id") is not None:
        raise EvalError("capture status rebound to a replacement window")
    active = data.get("active") is True
    if active:
        if data.get("session_id") != session_id:
            raise EvalError("active capture session identity changed after target reopen")
        if data.get("last_end_reason") is not None:
            raise EvalError("active capture unexpectedly reports an end reason")
    else:
        if data.get("session_id") is not None:
            raise EvalError("inactive capture still exposes a session id")
        if data.get("last_end_reason") not in {
            "portal_closed",
            "portal_monitor_failed",
        }:
            raise EvalError("inactive capture has no portal lifecycle end reason")
    return {
        "capture_active": active,
        "sticky_target_bound": False,
        "last_end_reason": data.get("last_end_reason"),
    }


def sanitized_replacement(
    original: OriginalTarget, replacement: dict[str, Any]
) -> dict[str, Any]:
    return {
        "original_window_sha256": hashed_window_id(original.window_id),
        "replacement_window_sha256": hashed_window_id(replacement["window_id"]),
        "app_id": original.app_id,
        "original_pid_present": original.pid is not None,
        "replacement_pid_present": replacement.get("pid") is not None,
        "pid_changed": (
            original.pid != replacement.get("pid")
            if original.pid is not None and replacement.get("pid") is not None
            else None
        ),
    }
