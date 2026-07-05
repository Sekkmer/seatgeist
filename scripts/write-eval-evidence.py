#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


def git(args: list[str]) -> str | None:
    try:
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def artifact_entries(run_dir: Path) -> list[dict[str, Any]]:
    entries = []
    for path in sorted(run_dir.iterdir()):
        if not path.is_file() or path.name == "evidence.json":
            continue
        try:
            stat = path.stat()
        except OSError:
            continue
        entries.append(
            {
                "path": rel(path),
                "bytes": stat.st_size,
            }
        )
    return entries


def main() -> None:
    parser = argparse.ArgumentParser(description="Write Seatgeist GUI eval evidence metadata.")
    parser.add_argument("--run-dir", required=True, help="Eval artifact directory.")
    parser.add_argument("--case", required=True, help="Eval case name.")
    parser.add_argument("--kind", required=True, help="Eval kind, such as safe-gui, local-input, visual, browser, or portal.")
    parser.add_argument("--status", default="passed", choices=["passed", "skipped"], help="Eval outcome status.")
    args = parser.parse_args()

    run_dir = Path(args.run_dir)
    if not run_dir.is_absolute():
        run_dir = ROOT / run_dir
    if not run_dir.is_dir():
        raise SystemExit(f"eval evidence run dir is missing: {run_dir}")

    evidence = {
        "type": "seatgeist_eval_evidence",
        "case": args.case,
        "kind": args.kind,
        "status": args.status,
        "unix_time_ms": int(time.time() * 1000),
        "git": git(["rev-parse", "--short=12", "HEAD"]),
        "run_dir": rel(run_dir),
        "hostname": os.uname().nodename,
        "artifacts": artifact_entries(run_dir),
    }
    (run_dir / "evidence.json").write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"write-eval-evidence: {args.status} {args.case} {rel(run_dir / 'evidence.json')}")


if __name__ == "__main__":
    main()
