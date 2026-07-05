#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class Check:
    name: str
    ok: bool
    summary: str
    evidence: list[str]
    next_action: str

    def to_json(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "ok": self.ok,
            "summary": self.summary,
            "evidence": self.evidence,
            "next_action": self.next_action,
        }


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def git(args: list[str]) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()


def run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=False)


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def cargo_metadata_urls() -> dict[str, str]:
    urls: dict[str, str] = {}
    for line in read("Cargo.toml").splitlines():
        match = re.match(r'^(repository|homepage)\s*=\s*"([^"]+)"\s*$', line.strip())
        if match:
            urls[match.group(1)] = match.group(2)
    return urls


def valid_public_https_url(url: str) -> bool:
    parsed = urlparse(url)
    if parsed.scheme != "https" or not parsed.netloc:
        return False
    if parsed.hostname in {"example.invalid", "example.com", "localhost"}:
        return False
    return parsed.hostname is not None and not parsed.hostname.endswith(".invalid")


def release_readiness() -> dict[str, Any]:
    result = run(["scripts/release-readiness.py", "--json"])
    if result.returncode != 0:
        return {
            "type": "release_readiness",
            "ready": False,
            "checks": [],
            "error": result.stderr.strip() or result.stdout.strip(),
        }
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as err:
        return {
            "type": "release_readiness",
            "ready": False,
            "checks": [],
            "error": f"invalid release-readiness JSON: {err}",
        }
    return value if isinstance(value, dict) else {"type": "release_readiness", "ready": False, "checks": []}


def readiness_check(report: dict[str, Any], name: str) -> dict[str, Any] | None:
    checks = report.get("checks")
    if not isinstance(checks, list):
        return None
    for check in checks:
        if isinstance(check, dict) and check.get("name") == name:
            return check
    return None


def latest_manifest(report: dict[str, Any]) -> Path | None:
    value = report.get("latest_manifest")
    if not isinstance(value, str) or not value:
        return None
    path = ROOT / value
    return path if path.is_file() else None


def check_public_metadata() -> Check:
    urls = cargo_metadata_urls()
    missing = [name for name in ("repository", "homepage") if name not in urls]
    bad = [name for name, url in urls.items() if not valid_public_https_url(url)]
    ok = not missing and not bad
    evidence = [f"{name}={urls[name]}" for name in sorted(urls)] + [f"missing={name}" for name in missing]
    if bad:
        evidence.append(f"invalid_public_url_fields={','.join(sorted(bad))}")
    return Check(
        "public_metadata",
        ok,
        "Cargo package metadata points at public HTTPS URLs" if ok else "Cargo package metadata still needs real public URLs",
        evidence,
        "Set Cargo.toml repository/homepage to the final public Seatgeist project URLs.",
    )


def check_signed_tag(current_git: str) -> Check:
    tags = git(["tag", "--points-at", "HEAD"]).splitlines()
    release_tags = [tag for tag in tags if re.fullmatch(r"v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?", tag)]
    signed = []
    unsigned = []
    for tag in release_tags:
        result = run(["git", "tag", "-v", tag])
        if result.returncode == 0:
            signed.append(tag)
        else:
            unsigned.append(tag)
    ok = bool(signed)
    evidence = [f"HEAD={current_git}"]
    if signed:
        evidence.extend(f"signed={tag}" for tag in signed)
    if unsigned:
        evidence.extend(f"unsigned={tag}" for tag in unsigned)
    if not release_tags:
        evidence.append("no vX.Y.Z release tag points at HEAD")
    return Check(
        "signed_release_tag",
        ok,
        "signed release tag points at HEAD" if ok else "no signed release tag points at HEAD",
        evidence,
        "Create and verify a signed release tag, for example git tag -s v0.1.0.",
    )


def check_release_artifact_upload_plan(report: dict[str, Any]) -> Check:
    manifest = latest_manifest(report)
    readiness_artifacts = readiness_check(report, "release_artifacts")
    readiness_signatures = readiness_check(report, "release_signatures")
    artifacts_ok = bool(readiness_artifacts and readiness_artifacts.get("ok"))
    signatures_ok = bool(readiness_signatures and readiness_signatures.get("ok"))
    evidence = []
    if manifest is not None:
        evidence.append(rel(manifest))
    for check in (readiness_artifacts, readiness_signatures):
        if isinstance(check, dict):
            evidence.append(f"{check.get('name')}: {check.get('summary')}")
    ok = artifacts_ok and signatures_ok
    return Check(
        "signed_release_artifacts",
        ok,
        "local release artifacts and signatures are present" if ok else "local release artifacts or signatures are missing",
        evidence,
        "Run make verify-release-install, sign-release-artifacts, and verify-release-signatures before upload.",
    )


def check_live_eval_evidence(report: dict[str, Any]) -> Check:
    check = readiness_check(report, "live_eval_evidence")
    ok = bool(check and check.get("ok"))
    evidence = []
    if isinstance(check, dict):
        values = check.get("evidence")
        if isinstance(values, list):
            evidence = [str(item) for item in values]
    return Check(
        "live_eval_evidence",
        ok,
        "release-blocking live KDE eval evidence is complete"
        if ok
        else "release-blocking live KDE eval evidence is incomplete",
        evidence,
        "Run SEATGEIST_RELEASE_LIVE_EVALS_APPROVED=1 make release-live-evals on the target KDE workstation.",
    )


def check_name_collision(report: dict[str, Any]) -> Check:
    check = readiness_check(report, "name_collision_report")
    ok = bool(check and check.get("ok"))
    evidence = []
    if isinstance(check, dict):
        values = check.get("evidence")
        if isinstance(values, list):
            evidence = [str(item) for item in values]
    return Check(
        "name_collision_report",
        ok,
        "repeatable exact-name registry evidence is clean" if ok else "repeatable exact-name registry evidence is missing, stale, or not clean",
        evidence,
        "Run make check-public-name and review formal trademark/domain checks separately before publishing.",
    )


def check_local_codex_install() -> Check:
    result = run(["scripts/check-local-codex-install.py", "--json"])
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        return Check(
            "local_codex_install",
            False,
            "local Codex Seatgeist plugin install preflight failed",
            [detail] if detail else ["scripts/check-local-codex-install.py --json failed"],
            "Run make check-local-codex-install and repair the reported Codex config, plugin cache, or launcher path.",
        )
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as err:
        return Check(
            "local_codex_install",
            False,
            "local Codex install preflight returned invalid JSON",
            [str(err)],
            "Run make check-local-codex-install and inspect its text output.",
        )
    if not isinstance(report, dict):
        return Check(
            "local_codex_install",
            False,
            "local Codex install preflight returned the wrong JSON shape",
            [],
            "Run make check-local-codex-install and inspect its text output.",
        )
    checks = report.get("checks")
    evidence = [
        f"codex_home={report.get('codex_home')}",
        f"blockers={report.get('blocker_count')}",
        f"warnings={report.get('warning_count')}",
    ]
    if isinstance(checks, list):
        for item in checks:
            if not isinstance(item, dict):
                continue
            name = item.get("name")
            ok = item.get("ok")
            summary = item.get("summary")
            if name in {
                "marketplace_source",
                "plugin_enabled",
                "installed_plugin_cache",
                "binary_seatgeist-mcp",
                "binary_seatgeist-cli",
                "binary_seatgeistd",
            }:
                evidence.append(f"{name}: {'ok' if ok else 'not ok'}: {summary}")
    ok = bool(report.get("ok"))
    return Check(
        "local_codex_install",
        ok,
        "local Codex Seatgeist plugin install is usable"
        if ok
        else "local Codex Seatgeist plugin install has blockers",
        evidence,
        "Run make check-local-codex-install and repair the reported Codex config, plugin cache, or launcher path.",
    )


def build_report() -> dict[str, Any]:
    current_git = git(["rev-parse", "--short=12", "HEAD"])
    readiness = release_readiness()
    checks = [
        check_public_metadata(),
        check_name_collision(readiness),
        check_local_codex_install(),
        check_live_eval_evidence(readiness),
        check_signed_tag(current_git),
        check_release_artifact_upload_plan(readiness),
    ]
    blockers = [check for check in checks if not check.ok]
    return {
        "type": "release_external_preflight",
        "ready": not blockers,
        "blocker_count": len(blockers),
        "git": current_git,
        "latest_manifest": readiness.get("latest_manifest"),
        "checks": [check.to_json() for check in checks],
    }


def print_text(report: dict[str, Any]) -> None:
    status = "ready" if report["ready"] else f"not ready ({report['blocker_count']} blockers)"
    print(f"release-external-preflight: {status}")
    print(f"git: {report['git']}")
    if report.get("latest_manifest"):
        print(f"latest manifest: {report['latest_manifest']}")
    for check in report["checks"]:
        mark = "ok" if check["ok"] else "blocker"
        print(f"- {mark}: {check['name']}: {check['summary']}")
        for item in check["evidence"][:6]:
            print(f"  {item}")
        if len(check["evidence"]) > 6:
            print(f"  ... {len(check['evidence']) - 6} more")
        if not check["ok"]:
            print(f"  next: {check['next_action']}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Preflight external Seatgeist release prerequisites.")
    parser.add_argument("--json", action="store_true", help="Print machine-readable JSON.")
    parser.add_argument("--strict", action="store_true", help="Exit non-zero when any external blocker remains.")
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
