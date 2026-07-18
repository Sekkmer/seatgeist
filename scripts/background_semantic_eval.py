from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from computer_use_eval import EvalError, response_data
from retained_capture_eval import hashed_window_id


SCENARIOS = ("firefox", "kde")


@dataclass(frozen=True)
class BackgroundTarget:
    window_id: str
    app_id: str
    pid: int | None
    user_window_id: str


def resolve_background_target(
    response: dict[str, Any],
    *,
    target_window_id: str,
    user_window_id: str,
    scenario: str,
) -> BackgroundTarget:
    if scenario not in SCENARIOS:
        raise EvalError("background semantic scenario must be firefox or kde")
    if response.get("type") == "error":
        response_data(response, "windows")
    if response.get("type") != "windows":
        raise EvalError("expected windows response")
    data = response.get("data")
    windows = data.get("windows") if isinstance(data, dict) else data
    if not isinstance(windows, list):
        raise EvalError("window list response has no windows")
    if target_window_id == user_window_id:
        raise EvalError("target and user work windows must be different")

    target = next(
        (window for window in windows if window.get("id") == target_window_id), None
    )
    user = next((window for window in windows if window.get("id") == user_window_id), None)
    if not isinstance(target, dict) or not isinstance(user, dict):
        raise EvalError("target or user work window is missing from the KWin window list")
    app_id = target.get("app_id")
    if not isinstance(app_id, str) or not app_id:
        raise EvalError("target window has no app id for exact correlation")
    normalized_app = app_id.lower()
    if scenario == "firefox" and "firefox" not in normalized_app:
        raise EvalError("firefox scenario target is not a Firefox window")
    if scenario == "kde" and not normalized_app.startswith("org.kde."):
        raise EvalError("kde scenario target app id does not start with org.kde.")
    pid = target.get("pid")
    if pid is not None and (not isinstance(pid, int) or pid < 1):
        raise EvalError("target window pid is malformed")
    return BackgroundTarget(target_window_id, app_id, pid, user_window_id)


def active_window_id(response: dict[str, Any]) -> str:
    data = response_data(response, "active_window")
    window_id = data.get("id")
    if not isinstance(window_id, str) or not window_id:
        raise EvalError("KWin reports no active window id")
    return window_id


def require_non_target_focus(response: dict[str, Any], target: BackgroundTarget) -> str:
    current = active_window_id(response)
    if current == target.window_id:
        raise EvalError("the background semantic target became active")
    return current


def click_button_arguments(
    target: BackgroundTarget, button_name: str, app_filter: str | None
) -> tuple[str, ...]:
    if not button_name.strip():
        raise EvalError("button name must be non-empty")
    arguments = [
        "semantic",
        "click-button",
        "--name",
        button_name,
        "--target-window-id",
        target.window_id,
        "--target-app-id",
        target.app_id,
    ]
    if target.pid is not None:
        arguments.extend(("--target-pid", str(target.pid)))
    if app_filter:
        arguments.extend(("--app", app_filter))
    return tuple(arguments)


def validate_action(response: dict[str, Any]) -> None:
    data = response_data(response, "action")
    if data.get("ok") is not True:
        raise EvalError("semantic action did not report success")


def validate_approval(response: dict[str, Any]) -> None:
    if response.get("safety_class") != "control_semantic":
        raise EvalError("approval did not use control_semantic policy")
    if response.get("method") != "click_button":
        raise EvalError("approval did not scope itself to click_button")


def validate_journal(
    response: dict[str, Any], target: BackgroundTarget, start_unix_ms: int
) -> int:
    if response.get("type") != "journal" or not isinstance(response.get("data"), list):
        raise EvalError("journal response is malformed")
    matches = []
    for entry in response["data"]:
        if not isinstance(entry, dict):
            continue
        control = entry.get("control")
        requested = control.get("requested_target") if isinstance(control, dict) else None
        fields = requested.get("fields") if isinstance(requested, dict) else None
        active_before = entry.get("active_window_before")
        active_after = entry.get("active_window_after")
        if (
            entry.get("method") == "click_button"
            and entry.get("ok") is True
            and isinstance(entry.get("unix_time_ms"), int)
            and entry["unix_time_ms"] >= start_unix_ms
            and isinstance(fields, dict)
            and fields.get("target_window_id") == target.window_id
            and fields.get("target_app_id") == target.app_id
            and isinstance(active_before, dict)
            and isinstance(active_before.get("id"), str)
            and bool(active_before.get("id"))
            and active_before.get("id") != target.window_id
            and isinstance(active_after, dict)
            and isinstance(active_after.get("id"), str)
            and bool(active_after.get("id"))
            and active_after.get("id") != target.window_id
        ):
            matches.append(entry)
    if not matches:
        raise EvalError(
            "successful target-guarded click_button journal entry with non-target focus is missing"
        )
    return len(matches)


def normalize_visual_verdict(value: str) -> bool:
    normalized = value.strip().lower()
    if normalized in {"y", "yes", "pass"}:
        return True
    if normalized in {"n", "no", "fail"}:
        return False
    raise EvalError("visual verdict must be yes or no")


def sanitized_target(target: BackgroundTarget) -> dict[str, Any]:
    return {
        "target_window_sha256": hashed_window_id(target.window_id),
        "user_window_sha256": hashed_window_id(target.user_window_id),
        "target_app_id": target.app_id,
        "target_pid_present": target.pid is not None,
    }
