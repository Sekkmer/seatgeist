#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_RELEASE_DIR = ROOT / "target" / "seatgeist-release"
REQUIRED_READINESS_CHECKS = {
    "git_state",
    "public_metadata",
    "name_collision_report",
    "release_checklist",
    "release_artifacts",
    "release_signatures",
    "live_eval_evidence",
}
REQUIRED_PORTAL_PACKAGES = {
    "xdg-desktop-portal",
    "xdg-desktop-portal-kde",
    "plasma-workspace",
    "spectacle",
}


def fail(message: str) -> None:
    print(f"verify-release-evidence: {message}", file=sys.stderr)
    raise SystemExit(1)


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def load_json(path: Path) -> dict[str, Any]:
    if not path.is_file():
        fail(f"missing evidence file: {path}")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        fail(f"{path} is not valid JSON: {err}")
    if not isinstance(value, dict):
        fail(f"{path} must contain a JSON object")
    return value


def newest_manifest(release_dir: Path) -> Path:
    manifests = sorted(
        release_dir.glob("seatgeist-*.manifest.json"),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    if not manifests:
        fail(f"no release manifest found in {release_dir}")
    return manifests[0]


def require_bool(obj: dict[str, Any], key: str, path: str) -> bool:
    value = obj.get(key)
    if not isinstance(value, bool):
        fail(f"{path}.{key} must be a boolean")
    return value


def require_int(obj: dict[str, Any], key: str, path: str) -> int:
    value = obj.get(key)
    if not isinstance(value, int):
        fail(f"{path}.{key} must be an integer")
    return value


def require_string(obj: dict[str, Any], key: str, path: str) -> str:
    value = obj.get(key)
    if not isinstance(value, str) or not value:
        fail(f"{path}.{key} must be a non-empty string")
    return value


def require_object(obj: dict[str, Any], key: str, path: str) -> dict[str, Any]:
    value = obj.get(key)
    if not isinstance(value, dict):
        fail(f"{path}.{key} must be an object")
    return value


def verify_manifest(manifest_path: Path) -> dict[str, Any]:
    manifest = load_json(manifest_path)
    if require_string(manifest, "name", "manifest") != "Seatgeist":
        fail("manifest.name must be Seatgeist")
    require_string(manifest, "version", "manifest")
    require_string(manifest, "git", "manifest")
    artifacts = require_object(manifest, "artifacts", "manifest")
    for key in ("bundle", "bundle_sha256", "plugin", "plugin_sha256", "source", "source_sha256"):
        artifact = require_string(artifacts, key, "manifest.artifacts")
        if "/" in artifact or artifact.startswith("."):
            fail(f"manifest.artifacts.{key} must be a plain filename")
    return manifest


def verify_readiness(path: Path, manifest_path: Path) -> None:
    readiness = load_json(path)
    if readiness.get("type") != "release_readiness":
        fail(f"{path}.type must be release_readiness")
    require_bool(readiness, "ready", "readiness")
    blocker_count = require_int(readiness, "blocker_count", "readiness")
    checks = readiness.get("checks")
    if not isinstance(checks, list) or not checks:
        fail("readiness.checks must be a non-empty list")
    if readiness.get("latest_manifest") != rel(manifest_path):
        fail(f"readiness.latest_manifest must point at {rel(manifest_path)}")

    seen: set[str] = set()
    blockers = 0
    for index, check in enumerate(checks):
        if not isinstance(check, dict):
            fail(f"readiness.checks[{index}] must be an object")
        name = require_string(check, "name", f"readiness.checks[{index}]")
        ok = require_bool(check, "ok", f"readiness.checks[{index}]")
        require_string(check, "summary", f"readiness.checks[{index}]")
        evidence = check.get("evidence")
        if not isinstance(evidence, list) or not all(isinstance(item, str) for item in evidence):
            fail(f"readiness.checks[{index}].evidence must be a string list")
        seen.add(name)
        if not ok:
            blockers += 1
    missing = sorted(REQUIRED_READINESS_CHECKS - seen)
    if missing:
        fail(f"readiness.checks is missing required checks: {', '.join(missing)}")
    if blocker_count != blockers:
        fail(f"readiness.blocker_count is {blocker_count}, expected {blockers}")
    if readiness["ready"] != (blockers == 0):
        fail("readiness.ready does not match blocker_count")


def verify_portal_status(path: Path) -> None:
    status = load_json(path)
    if status.get("type") != "portal_screenshot_v3_status":
        fail(f"{path}.type must be portal_screenshot_v3_status")
    require_bool(status, "ok", "portal_status")
    portal = require_object(status, "portal", "portal_status")
    packages = require_object(status, "packages", "portal_status")
    notes = status.get("notes")
    next_steps = status.get("next_steps")
    if not isinstance(notes, list) or not all(isinstance(item, str) for item in notes):
        fail("portal_status.notes must be a string list")
    if not any("read-only diagnostic" in note for note in notes):
        fail("portal_status.notes must document that the diagnostic is read-only")
    if not isinstance(next_steps, list) or not all(isinstance(item, str) for item in next_steps):
        fail("portal_status.next_steps must be a string list")

    for key in (
        "busctl_available",
        "target_option_supported",
        "version_probe_ok",
        "available_targets_probe_ok",
    ):
        require_bool(portal, key, "portal_status.portal")
    version = portal.get("screenshot_interface_version")
    target_mask = portal.get("available_targets_mask")
    if version is not None and not isinstance(version, int):
        fail("portal_status.portal.screenshot_interface_version must be null or integer")
    if target_mask is not None and not isinstance(target_mask, int):
        fail("portal_status.portal.available_targets_mask must be null or integer")
    targets = portal.get("available_targets")
    if not isinstance(targets, list) or not all(isinstance(item, str) for item in targets):
        fail("portal_status.portal.available_targets must be a string list")

    installed = require_object(packages, "installed", "portal_status.packages")
    missing_packages = sorted(REQUIRED_PORTAL_PACKAGES - set(installed))
    if missing_packages:
        fail(f"portal_status.packages.installed is missing packages: {', '.join(missing_packages)}")
    pending = require_object(packages, "pending_upgrades", "portal_status.packages")
    if not all(isinstance(key, str) and isinstance(value, str) for key, value in pending.items()):
        fail("portal_status.packages.pending_upgrades must map package names to version strings")
    require_bool(packages, "aur_step_available", "portal_status.packages")


def main() -> None:
    parser = argparse.ArgumentParser(description="Verify retained Seatgeist release evidence snapshots.")
    parser.add_argument("manifest", nargs="?", help="Path to a generated *.manifest.json file.")
    args = parser.parse_args()

    manifest_path = Path(args.manifest) if args.manifest else newest_manifest(DEFAULT_RELEASE_DIR)
    if not manifest_path.is_absolute():
        manifest_path = ROOT / manifest_path
    if not manifest_path.is_file():
        fail(f"manifest is missing: {manifest_path}")

    verify_manifest(manifest_path)
    prefix = Path(str(manifest_path).removesuffix(".manifest.json"))
    verify_readiness(prefix.with_name(prefix.name + ".readiness.json"), manifest_path)
    verify_portal_status(prefix.with_name(prefix.name + ".portal-screenshot-v3-status.json"))
    print(f"verify-release-evidence: ok {manifest_path}")


if __name__ == "__main__":
    main()
