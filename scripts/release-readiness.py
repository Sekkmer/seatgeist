#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
RELEASE_DIR = ROOT / "target" / "seatgeist-release"
GUI_EVAL_DIR = ROOT / "target" / "seatgeist-gui-eval"


@dataclass(frozen=True)
class Check:
    name: str
    ok: bool
    summary: str
    evidence: list[str]

    def to_json(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "ok": self.ok,
            "summary": self.summary,
            "evidence": self.evidence,
        }


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def git(args: list[str]) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()


def latest_manifest() -> Path | None:
    manifests = sorted(
        RELEASE_DIR.glob("seatgeist-*.manifest.json"),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    return manifests[0] if manifests else None


def load_manifest(path: Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None
    return value if isinstance(value, dict) else None


def artifact_paths(manifest_path: Path | None, manifest: dict[str, Any] | None) -> list[Path]:
    if manifest_path is None or manifest is None:
        return []
    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, dict):
        return []
    paths = []
    for key in ("bundle", "bundle_sha256", "source", "source_sha256"):
        value = artifacts.get(key)
        if isinstance(value, str) and "/" not in value:
            paths.append(manifest_path.parent / value)
    paths.append(manifest_path)
    return paths


def check_public_metadata() -> Check:
    cargo = read("Cargo.toml")
    placeholders = [
        line.strip()
        for line in cargo.splitlines()
        if "example.invalid" in line or "TODO" in line or "placeholder" in line.lower()
    ]
    ok = not placeholders
    summary = "workspace package metadata has public URLs" if ok else "workspace package metadata still has placeholder URLs"
    return Check("public_metadata", ok, summary, placeholders)


def check_release_checklist() -> Check:
    checklist = read("docs/release-checklist.md")
    open_items = [
        line.strip()
        for line in checklist.splitlines()
        if line.startswith("- [ ]") or line.startswith("- [~]")
    ]
    ok = not open_items
    summary = "release checklist has no open blocking evidence" if ok else f"release checklist has {len(open_items)} open or partial items"
    return Check("release_checklist", ok, summary, open_items)


def check_release_artifacts(
    manifest_path: Path | None, manifest: dict[str, Any] | None, current_git: str
) -> Check:
    paths = artifact_paths(manifest_path, manifest)
    missing = [rel(path) for path in paths if not path.is_file()]
    if manifest_path is None:
        return Check(
            "release_artifacts",
            False,
            "no release manifest found; run make verify-release-artifacts",
            [],
        )
    if manifest is None:
        return Check("release_artifacts", False, "latest release manifest is not valid JSON", [rel(manifest_path)])
    manifest_git = manifest.get("git")
    if manifest_git != current_git:
        return Check(
            "release_artifacts",
            False,
            "latest release manifest is stale for the current commit",
            [f"manifest git: {manifest_git}", f"current git: {current_git}", rel(manifest_path)],
        )
    if missing:
        return Check("release_artifacts", False, "release manifest references missing artifacts", missing)
    return Check(
        "release_artifacts",
        True,
        "latest release artifacts are present",
        [rel(path) for path in paths],
    )


def check_release_signatures(manifest_path: Path | None, manifest: dict[str, Any] | None) -> Check:
    artifacts = artifact_paths(manifest_path, manifest)
    if not artifacts:
        return Check(
            "release_signatures",
            False,
            "no artifact list available for signature checks",
            [],
        )
    signatures = [path.with_name(path.name + ".asc") for path in artifacts]
    signature_manifest = manifest_path.with_suffix("").with_suffix(".signatures.sha256") if manifest_path else None
    if signature_manifest is not None:
        signatures.extend([signature_manifest, signature_manifest.with_name(signature_manifest.name + ".asc")])
    missing = [rel(path) for path in signatures if not path.is_file()]
    if missing:
        return Check("release_signatures", False, "release signatures are missing", missing)
    return Check("release_signatures", True, "release signatures are present", [rel(path) for path in signatures])


def newest_matching(prefix: str) -> Path | None:
    matches = sorted(
        GUI_EVAL_DIR.glob(f"{prefix}-*"),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    return matches[0] if matches else None


def check_live_eval_evidence() -> Check:
    required = {
        "KWrite/Kate input": ROOT / "target" / "seatgeist-gui-input-smoke",
        "KCalc visual input": ROOT / "target" / "seatgeist-gui-calculator-smoke",
        "Firefox localhost button": ROOT / "target" / "seatgeist-gui-browser-smoke",
        "portal Screenshot": newest_matching("portal-screenshot"),
        "RemoteDesktop probe": newest_matching("remote-desktop-probe"),
        "retained RemoteDesktop EIS session": newest_matching("remote-desktop-eis-session"),
    }
    missing = [name for name, path in required.items() if path is None or not path.exists()]
    evidence = [f"{name}: {rel(path)}" for name, path in required.items() if path is not None and path.exists()]
    if missing:
        return Check(
            "live_eval_evidence",
            False,
            "opt-in live KDE eval evidence is incomplete",
            [f"missing: {name}" for name in missing] + evidence,
        )
    return Check("live_eval_evidence", True, "opt-in live KDE eval evidence exists", evidence)


def check_git_state() -> Check:
    head = git(["rev-parse", "--short=12", "HEAD"])
    dirty = git(["status", "--short"])
    if dirty:
        evidence = [line for line in dirty.splitlines() if line]
        return Check("git_state", False, f"worktree has uncommitted changes at {head}", evidence)
    return Check("git_state", True, f"worktree is clean at {head}", [head])


def build_report() -> dict[str, Any]:
    manifest_path = latest_manifest()
    manifest = load_manifest(manifest_path)
    current_git = git(["rev-parse", "--short=12", "HEAD"])
    checks = [
        check_git_state(),
        check_public_metadata(),
        check_release_checklist(),
        check_release_artifacts(manifest_path, manifest, current_git),
        check_release_signatures(manifest_path, manifest),
        check_live_eval_evidence(),
    ]
    blockers = [check for check in checks if not check.ok]
    return {
        "type": "release_readiness",
        "ready": not blockers,
        "blocker_count": len(blockers),
        "latest_manifest": rel(manifest_path) if manifest_path else None,
        "checks": [check.to_json() for check in checks],
    }


def print_text(report: dict[str, Any]) -> None:
    status = "ready" if report["ready"] else f"not ready ({report['blocker_count']} blockers)"
    print(f"release-readiness: {status}")
    if report["latest_manifest"]:
        print(f"latest manifest: {report['latest_manifest']}")
    for check in report["checks"]:
        mark = "ok" if check["ok"] else "blocker"
        print(f"- {mark}: {check['name']}: {check['summary']}")
        for item in check["evidence"][:8]:
            print(f"  {item}")
        if len(check["evidence"]) > 8:
            print(f"  ... {len(check['evidence']) - 8} more")


def main() -> None:
    parser = argparse.ArgumentParser(description="Audit Seatgeist public release readiness evidence.")
    parser.add_argument("--json", action="store_true", help="Print machine-readable JSON.")
    parser.add_argument("--strict", action="store_true", help="Exit non-zero when any release blocker remains.")
    args = parser.parse_args()

    report = build_report()
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_text(report)
    if args.strict and not report["ready"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
