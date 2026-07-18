from __future__ import annotations

from typing import Any

from computer_use_eval import EvalError
from retained_capture_eval import hashed_window_id


EVIDENCE_TYPE = "seatgeist_capture_restore_restart_eval"
EVIDENCE_VERSION = 1


def has_integer_identity(identity: dict[str, Any], fields: tuple[str, ...]) -> bool:
    return all(
        isinstance(identity.get(field), int) and identity[field] >= 0
        for field in fields
    )


def require_restore_reference(session: dict[str, Any]) -> str:
    reference = session.get("restore_token_reference")
    if not isinstance(reference, str) or not reference:
        raise EvalError("requested-window session returned no opaque restore reference")
    return reference


def same_file_identity(left: dict[str, Any], right: dict[str, Any]) -> bool:
    fields = ("device", "inode", "bytes", "mtime_ns")
    if not has_integer_identity(left, fields) or not has_integer_identity(right, fields):
        return False
    return all(
        left.get(field) == right.get(field) for field in fields
    )


def require_daemon_restart(
    prepared: dict[str, Any] | None, current: dict[str, Any] | None
) -> None:
    if prepared is None or current is None:
        raise EvalError("daemon socket identity is unavailable")
    fields = ("device", "inode")
    if not has_integer_identity(prepared, fields) or not has_integer_identity(
        current, fields
    ):
        raise EvalError("daemon socket identity is malformed")
    if all(
        prepared.get(field) == current.get(field) for field in fields
    ):
        raise EvalError("daemon socket identity did not change; restart is not proven")


def require_prepared_evidence(
    evidence: dict[str, Any], window_id: str
) -> dict[str, Any]:
    if evidence.get("type") != EVIDENCE_TYPE:
        raise EvalError("resume state has the wrong evidence type")
    if evidence.get("version") != EVIDENCE_VERSION:
        raise EvalError("resume state has an unsupported version")
    if evidence.get("status") != "prepared":
        raise EvalError("resume state is not in the prepared phase")
    if evidence.get("target_window_sha256") != hashed_window_id(window_id):
        raise EvalError("resume target does not match the prepared target")
    prepared = evidence.get("prepare")
    if not isinstance(prepared, dict):
        raise EvalError("resume state has no prepare evidence")
    session = prepared.get("session")
    frame = prepared.get("frame")
    restore_file = prepared.get("restore_file_after_open")
    if not isinstance(session, dict) or not isinstance(frame, dict):
        raise EvalError("resume state has incomplete prepare evidence")
    if not isinstance(restore_file, dict):
        raise EvalError("resume state has no restore-file identity")
    require_restore_reference(session)
    if frame.get("fresh_frame") is not True:
        raise EvalError("prepared capture did not contain a fresh frame")
    if evidence.get("prepare_session_closed") is not True:
        raise EvalError("prepared capture session was not closed")
    return prepared


def normalize_chooser_answer(value: str) -> bool:
    normalized = value.strip().lower()
    if normalized in {"n", "no"}:
        return True
    if normalized in {"y", "yes"}:
        return False
    raise EvalError("portal chooser answer must be yes or no")


def restore_file_was_replaced(
    before: dict[str, Any], after: dict[str, Any]
) -> bool:
    fields = ("device", "inode")
    if not has_integer_identity(before, fields) or not has_integer_identity(
        after, fields
    ):
        return False
    return any(
        before.get(field) != after.get(field) for field in fields
    )


def restart_acceptance_complete(evidence: dict[str, Any]) -> bool:
    prepare = evidence.get("prepare")
    resume = evidence.get("resume")
    if not isinstance(prepare, dict) or not isinstance(resume, dict):
        return False
    prepared_session = prepare.get("session")
    resumed_session = resume.get("session")
    resumed_frame = resume.get("frame")
    if (
        not isinstance(prepared_session, dict)
        or not isinstance(resumed_session, dict)
        or not isinstance(resumed_frame, dict)
    ):
        return False
    return all(
        (
            evidence.get("daemon_restart_proven") is True,
            evidence.get("prepare_session_closed") is True,
            evidence.get("resume_session_closed") is True,
            resume.get("portal_chooser_avoided") is True,
            resume.get("restore_file_replaced") is True,
            resumed_frame.get("fresh_frame") is True,
            require_restore_reference(prepared_session)
            == require_restore_reference(resumed_session),
        )
    )
