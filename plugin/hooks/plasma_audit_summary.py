#!/usr/bin/env python3
"""Write a compact PlasmaPilot audit summary for Codex Stop hooks."""

from __future__ import annotations

import json
import subprocess
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

CONTROL_SAFETY_CLASSES = {
    "control_pointer",
    "control_keyboard",
    "control_semantic",
    "destructive_action",
    "secret_field",
}
MAX_AUDIT_EXAMPLES = 5


def run(args: list[str], cwd: Path) -> str | None:
    try:
        result = subprocess.run(
            args,
            cwd=cwd,
            check=False,
            capture_output=True,
            text=True,
            timeout=3,
        )
    except Exception:
        return None
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def git_root(cwd: Path) -> Path:
    root = run(["git", "rev-parse", "--show-toplevel"], cwd)
    if root:
        return Path(root)
    return cwd


def recent_journal_entries(root: Path, limit: int = 20) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for path in sorted((root / "target").glob("plasma-pilot*-journal.jsonl")):
        try:
            lines = path.read_text(encoding="utf-8").splitlines()[-limit:]
        except OSError:
            continue
        for line in lines:
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue
            entries.append(
                {
                    "journal": str(path.relative_to(root)),
                    "sequence": entry.get("sequence"),
                    "unix_time_ms": entry.get("unix_time_ms"),
                    "method": entry.get("method"),
                    "ok": entry.get("ok"),
                    "safety_class": entry.get("safety_class"),
                    "guard_present": entry.get("guard_present", False),
                    "active_window_before": entry.get("active_window_before"),
                    "active_window_after": entry.get("active_window_after"),
                    "summary": entry.get("summary"),
                }
            )
    return entries[-limit:]


def is_control_entry(entry: dict[str, Any]) -> bool:
    return entry.get("safety_class") in CONTROL_SAFETY_CLASSES


def compact_window(window: Any) -> dict[str, Any] | None:
    if not isinstance(window, dict):
        return None
    compact = {
        "id": window.get("id"),
        "app_id": window.get("app_id"),
        "title": window.get("title"),
        "monitor_id": window.get("monitor_id"),
    }
    return {key: value for key, value in compact.items() if value not in (None, "")}


def compact_example(entry: dict[str, Any]) -> dict[str, Any]:
    example = {
        "journal": entry.get("journal"),
        "sequence": entry.get("sequence"),
        "method": entry.get("method"),
        "ok": entry.get("ok"),
        "safety_class": entry.get("safety_class"),
        "guard_present": entry.get("guard_present", False),
        "summary": entry.get("summary"),
    }
    before = compact_window(entry.get("active_window_before"))
    after = compact_window(entry.get("active_window_after"))
    if before:
        example["active_window_before"] = before
    if after:
        example["active_window_after"] = after
    return {key: value for key, value in example.items() if value is not None}


def latest_window_context(entries: list[dict[str, Any]]) -> dict[str, Any] | None:
    for entry in reversed(entries):
        for key in ("active_window_after", "active_window_before"):
            window = compact_window(entry.get(key))
            if window:
                return window
    return None


def summarize_journal(entries: list[dict[str, Any]]) -> dict[str, Any]:
    methods = Counter(
        str(entry.get("method"))
        for entry in entries
        if isinstance(entry.get("method"), str) and entry.get("method")
    )
    safety_classes = Counter(
        str(entry.get("safety_class"))
        for entry in entries
        if isinstance(entry.get("safety_class"), str) and entry.get("safety_class")
    )
    failures = [entry for entry in entries if entry.get("ok") is False]
    controls = [entry for entry in entries if is_control_entry(entry)]
    unguarded_controls = [
        entry for entry in controls if not bool(entry.get("guard_present", False))
    ]

    audit = {
        "entry_count": len(entries),
        "ok_count": sum(1 for entry in entries if entry.get("ok") is True),
        "failure_count": len(failures),
        "control_count": len(controls),
        "unguarded_control_count": len(unguarded_controls),
        "methods": dict(sorted(methods.items())),
        "safety_classes": dict(sorted(safety_classes.items())),
        "recent_failures": [
            compact_example(entry) for entry in failures[-MAX_AUDIT_EXAMPLES:]
        ],
        "unguarded_control_examples": [
            compact_example(entry) for entry in unguarded_controls[-MAX_AUDIT_EXAMPLES:]
        ],
        "last_active_window": latest_window_context(entries),
    }
    return audit


def write_summary(root: Path) -> None:
    status = run(["git", "status", "--short"], root)
    recent_entries = recent_journal_entries(root)
    output_dir = root / "target" / "plasma-pilot-hook-audit"
    output_dir.mkdir(parents=True, exist_ok=True)
    summary = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "git_root": str(root),
        "head": run(["git", "rev-parse", "--short", "HEAD"], root),
        "branch": run(["git", "branch", "--show-current"], root),
        "dirty": bool(status),
        "status_short": status.splitlines() if status else [],
        "plasma_pilot_audit": summarize_journal(recent_entries),
        "recent_plasma_pilot_journal": recent_entries,
    }
    (output_dir / "latest.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> int:
    try:
        write_summary(git_root(Path.cwd()))
    except Exception:
        return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
