#!/usr/bin/env python3
"""Write a compact PlasmaPilot audit summary for Codex Stop hooks."""

from __future__ import annotations

import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


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
                    "method": entry.get("method"),
                    "ok": entry.get("ok"),
                    "response_type": entry.get("response_type"),
                    "safety_class": entry.get("safety_class"),
                }
            )
    return entries[-limit:]


def write_summary(root: Path) -> None:
    status = run(["git", "status", "--short"], root)
    output_dir = root / "target" / "plasma-pilot-hook-audit"
    output_dir.mkdir(parents=True, exist_ok=True)
    summary = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "git_root": str(root),
        "head": run(["git", "rev-parse", "--short", "HEAD"], root),
        "branch": run(["git", "branch", "--show-current"], root),
        "dirty": bool(status),
        "status_short": status.splitlines() if status else [],
        "recent_plasma_pilot_journal": recent_journal_entries(root),
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
