from __future__ import annotations

import hashlib
import math
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from computer_use_eval import EvalError, private_png_info, response_data


@dataclass(frozen=True)
class CaptureScenario:
    name: str
    instruction: str


SCENARIOS = (
    CaptureScenario(
        "focused_visible",
        "Keep the approved target focused and fully visible with visibly changing content.",
    ),
    CaptureScenario(
        "unfocused_visible",
        "Focus another window while leaving the approved target fully visible.",
    ),
    CaptureScenario(
        "partially_occluded",
        "Cover part of the approved target with another window while its content keeps changing.",
    ),
    CaptureScenario(
        "fully_occluded",
        "Cover the approved target completely while its content keeps changing.",
    ),
    CaptureScenario(
        "minimized",
        "Minimize the approved target while its content keeps changing.",
    ),
    CaptureScenario(
        "popup_or_dialog",
        "Restore the target and open one browser menu, context menu, popup, or dialog over it.",
    ),
    CaptureScenario(
        "moved_resized",
        "Move and resize the approved target, keeping visibly changing content inside it.",
    ),
    CaptureScenario(
        "monitor_or_scale_change",
        "Move the target to another monitor or scale domain; use skip only if unavailable.",
    ),
)

SCENARIO_BY_NAME = {scenario.name: scenario for scenario in SCENARIOS}


def hashed_window_id(window_id: str) -> str:
    if not window_id.strip():
        raise EvalError("window id must be non-empty")
    return hashlib.sha256(window_id.encode("utf-8")).hexdigest()


def require_no_active_capture(response: dict[str, Any]) -> None:
    data = response_data(response, "capture_session_status")
    if data.get("active") is True or data.get("opening") is True:
        raise EvalError("another capture session is active or opening; it was left untouched")


def session_id_for_cleanup(response: dict[str, Any]) -> str | None:
    data = response_data(response, "capture_session_status")
    session_id = data.get("session_id")
    if data.get("active") is True and isinstance(session_id, str) and session_id:
        return session_id
    return None


def validate_open_session(
    response: dict[str, Any], window_id: str
) -> dict[str, Any]:
    data = response_data(response, "capture_session_status")
    session_id = data.get("session_id")
    if data.get("active") is not True or not isinstance(session_id, str) or not session_id:
        raise EvalError("capture open returned no active session id")
    if data.get("backend") != "portal_screencast_pipewire":
        raise EvalError("capture session did not select the retained PipeWire backend")
    if data.get("source_type") != "window":
        raise EvalError("portal returned a non-window source")
    if data.get("requested_window_id") != window_id:
        raise EvalError("capture session reports a different requested window")
    if data.get("sticky_target_bound") is not True:
        raise EvalError("capture session did not bind the requested sticky target")
    if data.get("target_window_id") != window_id:
        raise EvalError("capture session bound a different sticky target")
    if data.get("occlusion_possible") is not False:
        raise EvalError("retained window stream unexpectedly reports occlusion risk")
    if data.get("last_end_reason") is not None:
        raise EvalError("new capture session unexpectedly reports an earlier end reason")
    reference = data.get("restore_token_reference")
    if reference is not None and (not isinstance(reference, str) or not reference):
        raise EvalError("restore token reference is malformed")
    return {
        "session_id": session_id,
        "backend": data["backend"],
        "source_type": data["source_type"],
        "source_id_present": isinstance(data.get("source_id"), str)
        and bool(data.get("source_id")),
        "restore_token_reference": reference,
        "sticky_target_bound": True,
        "occlusion_possible": False,
    }


def validate_frame(
    response: dict[str, Any],
    *,
    response_type: str,
    session_id: str,
    artifact_root: Path,
    expected_output: Path,
    max_edge: int,
    after_revision: str | None = None,
) -> dict[str, Any]:
    data = response_data(response, response_type)
    if response_type == "capture_wait":
        frame = data.get("frame")
        if not isinstance(frame, dict):
            raise EvalError("capture wait returned no frame")
        changed = data.get("changed") is True
        timed_out = data.get("timed_out") is True
        elapsed_ms = data.get("elapsed_ms")
    else:
        frame = data
        changed = True
        timed_out = False
        elapsed_ms = 0

    if frame.get("session_id") != session_id:
        raise EvalError("capture frame belongs to a different session")
    revision = frame.get("revision")
    sequence = frame.get("sequence")
    screenshot = frame.get("screenshot")
    if not isinstance(revision, str) or not revision:
        raise EvalError("capture frame has no revision")
    if not isinstance(sequence, int) or sequence < 1:
        raise EvalError("capture frame has no positive sequence")
    if frame.get("complete") is not True:
        raise EvalError("capture frame is incomplete")
    if not isinstance(screenshot, dict):
        raise EvalError("capture frame has no screenshot metadata")
    if screenshot.get("backend") != "portal_screencast_pipewire":
        raise EvalError("capture frame came from a different backend")

    dimensions = {}
    for field in ("source_width", "source_height", "output_width", "output_height"):
        value = screenshot.get(field)
        if not isinstance(value, int) or value < 1:
            raise EvalError(f"capture screenshot has invalid {field}")
        dimensions[field] = value
    if max(dimensions["output_width"], dimensions["output_height"]) > max_edge:
        raise EvalError("capture output exceeds the requested bound")

    raw_path = screenshot.get("path")
    if not isinstance(raw_path, str) or not raw_path:
        raise EvalError("capture screenshot has no artifact path")
    artifact_path = Path(raw_path).resolve()
    try:
        artifact_path.relative_to(artifact_root.resolve())
    except ValueError as err:
        raise EvalError("capture artifact escaped the eval directory") from err

    fresh_frame = changed and (after_revision is None or revision != after_revision)
    if fresh_frame and artifact_path != expected_output.resolve():
        raise EvalError("fresh capture frame was not written to the requested artifact")
    artifact = private_png_info(artifact_path, max_edge)
    return {
        "revision": revision,
        "sequence": sequence,
        "changed": changed,
        "timed_out": timed_out,
        "fresh_frame": fresh_frame,
        "elapsed_ms": elapsed_ms if isinstance(elapsed_ms, int) else None,
        "damage_present": frame.get("damage_present") is True,
        "dimensions": dimensions,
        "artifact": artifact,
    }


def normalize_visual_verdict(value: str) -> str:
    normalized = value.strip().lower()
    if normalized in {"y", "yes", "pass"}:
        return "pass"
    if normalized in {"n", "no", "fail"}:
        return "fail"
    if normalized in {"s", "skip"}:
        return "skip"
    raise EvalError("visual verdict must be yes, no, or skip")


def selected_scenarios(names: list[str] | None) -> tuple[CaptureScenario, ...]:
    if not names:
        return SCENARIOS
    unknown = sorted(set(names) - SCENARIO_BY_NAME.keys())
    if unknown:
        raise EvalError("unknown capture scenarios: " + ", ".join(unknown))
    if len(set(names)) != len(names):
        raise EvalError("capture scenarios must not be repeated")
    return tuple(SCENARIO_BY_NAME[name] for name in names)


def sanitized_monitor_layout(
    response: dict[str, Any], *, require_multi_output_nonzero_origin: bool
) -> dict[str, Any]:
    if response.get("type") == "error":
        response_data(response, "monitors")
    if response.get("type") != "monitors" or not isinstance(response.get("data"), list):
        raise EvalError("monitor layout response is malformed")
    monitors = response["data"]
    if not monitors:
        raise EvalError("monitor layout is empty")
    sanitized = []
    for monitor in monitors:
        if not isinstance(monitor, dict):
            raise EvalError("monitor layout contains a malformed entry")
        x = monitor.get("logical_origin_x")
        y = monitor.get("logical_origin_y")
        width = monitor.get("logical_width")
        height = monitor.get("logical_height")
        scale = monitor.get("scale_factor")
        if (
            not isinstance(x, int)
            or not isinstance(y, int)
            or not isinstance(width, int)
            or width < 1
            or not isinstance(height, int)
            or height < 1
            or not isinstance(scale, (int, float))
            or not math.isfinite(scale)
            or scale <= 0
        ):
            raise EvalError("monitor layout contains invalid geometry or scale")
        sanitized.append(
            {
                "logical_origin_x": x,
                "logical_origin_y": y,
                "logical_width": width,
                "logical_height": height,
                "scale_factor": float(scale),
            }
        )
    has_negative = any(
        monitor["logical_origin_x"] < 0 or monitor["logical_origin_y"] < 0
        for monitor in sanitized
    )
    has_nonzero = any(
        monitor["logical_origin_x"] != 0 or monitor["logical_origin_y"] != 0
        for monitor in sanitized
    )
    if require_multi_output_nonzero_origin and (
        len(sanitized) < 2 or not has_nonzero
    ):
        raise EvalError(
            "monitor layout does not contain multiple outputs with a non-zero logical origin"
        )
    return {
        "monitor_count": len(sanitized),
        "has_negative_logical_origin": has_negative,
        "has_nonzero_logical_origin": has_nonzero,
        "monitors": sanitized,
    }


def capture_acceptance_complete(
    scenarios: list[dict[str, Any]], selected: tuple[CaptureScenario, ...]
) -> bool:
    if selected != SCENARIOS or len(scenarios) != len(SCENARIOS):
        return False
    return all(
        scenario.get("visual_verdict") == "pass"
        and scenario.get("frame", {}).get("fresh_frame") is True
        for scenario in scenarios
    )
