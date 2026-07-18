from __future__ import annotations

import hashlib
import json
import time
from pathlib import Path
from typing import Any

from computer_use_eval import EvalError, private_file_identity
from cooperative_acceptance import SPECS, require


BUNDLE_TYPE = "seatgeist_cooperative_use_acceptance_bundle"
BUNDLE_VERSION = 1
MAX_ARTIFACT_BYTES = 4 * 1024 * 1024


def read_evidence(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    identity = private_file_identity(path)
    require(identity["bytes"] <= MAX_ARTIFACT_BYTES, "evidence artifact is too large")
    try:
        encoded = path.read_bytes()
        value = json.loads(encoded)
    except (OSError, UnicodeError, json.JSONDecodeError) as err:
        raise EvalError(f"read evidence artifact: {err}") from err
    require(isinstance(value, dict), "evidence artifact is not an object")
    return value, {
        "bytes": len(encoded),
        "sha256": hashlib.sha256(encoded).hexdigest(),
    }


def evidence_interval(evidence: dict[str, Any]) -> tuple[int, int]:
    if evidence.get("type") == "seatgeist_computer_use_baseline":
        filters = evidence.get("filters")
        require(isinstance(filters, dict), "cooperative evidence filters are missing")
        start = filters.get("start_unix_ms")
        end = filters.get("end_unix_ms")
    else:
        start = evidence.get("started_unix_ms")
        end = evidence.get("ended_unix_ms")
    require(
        isinstance(start, int)
        and not isinstance(start, bool)
        and isinstance(end, int)
        and not isinstance(end, bool)
        and 0 <= start <= end,
        "evidence timestamps are invalid",
    )
    return start, end


def validate_workspace(value: Any) -> dict[str, Any]:
    require(isinstance(value, dict), "workspace revision is missing")
    git_head = value.get("git_head")
    tree = value.get("tree_sha256")
    dirty = value.get("dirty")
    require(
        isinstance(git_head, str)
        and len(git_head) == 40
        and all(character in "0123456789abcdef" for character in git_head),
        "workspace git_head is invalid",
    )
    require(
        isinstance(tree, str)
        and len(tree) == 64
        and all(character in "0123456789abcdef" for character in tree),
        "workspace tree_sha256 is invalid",
    )
    require(isinstance(dirty, bool), "workspace dirty flag is invalid")
    return {"git_head": git_head, "tree_sha256": tree, "dirty": dirty}


def build_bundle(
    artifacts: dict[str, Path],
    *,
    expected_workspace: dict[str, Any],
    now_unix_ms: int | None = None,
    max_age_ms: int = 24 * 60 * 60 * 1000,
    max_span_ms: int = 24 * 60 * 60 * 1000,
) -> dict[str, Any]:
    require(
        set(artifacts) == {spec.name for spec in SPECS},
        "acceptance artifact set is incomplete",
    )
    expected_workspace = validate_workspace(expected_workspace)
    now = int(time.time() * 1000) if now_unix_ms is None else now_unix_ms
    require(
        max_age_ms > 0 and max_span_ms > 0,
        "evidence age and span limits must be positive",
    )

    summaries: list[dict[str, Any]] = []
    starts: list[int] = []
    ends: list[int] = []
    for spec in SPECS:
        evidence, file_info = read_evidence(artifacts[spec.name])
        require(
            evidence.get("type") == spec.evidence_type,
            f"{spec.name} has the wrong evidence type",
        )
        require(
            evidence.get("version") == spec.version,
            f"{spec.name} has the wrong evidence version",
        )
        require(
            validate_workspace(evidence.get("workspace")) == expected_workspace,
            f"{spec.name} was recorded from a different workspace revision",
        )
        spec.validator(evidence)
        start, end = evidence_interval(evidence)
        require(end <= now + 5 * 60 * 1000, f"{spec.name} timestamp is in the future")
        require(now - end <= max_age_ms, f"{spec.name} evidence is stale")
        starts.append(start)
        ends.append(end)
        summaries.append(
            {
                "name": spec.name,
                "type": spec.evidence_type,
                "version": spec.version,
                "started_unix_ms": start,
                "ended_unix_ms": end,
                **file_info,
            }
        )
    require(
        max(ends) - min(starts) <= max_span_ms,
        "acceptance evidence exceeds the allowed span",
    )
    return {
        "type": BUNDLE_TYPE,
        "version": BUNDLE_VERSION,
        "status": "passed",
        "acceptance_complete": True,
        "generated_unix_ms": now,
        "workspace": expected_workspace,
        "max_age_ms": max_age_ms,
        "max_span_ms": max_span_ms,
        "artifacts": summaries,
    }
