#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tarfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_RELEASE_DIR = ROOT / "target" / "seatgeist-release"


def fail(message: str) -> None:
    print(f"verify-release-artifacts: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        fail(f"{path} is not valid JSON: {err}")
    if not isinstance(value, dict):
        fail(f"{path} must contain a JSON object")
    return value


def newest_manifest(release_dir: Path) -> Path:
    manifests = sorted(
        release_dir.glob("seatgeist-*-linux_*.manifest.json"),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    if not manifests:
        manifests = sorted(
            release_dir.glob("seatgeist-*-linux-x86_64.manifest.json"),
            key=lambda path: path.stat().st_mtime,
            reverse=True,
        )
    if not manifests:
        fail(f"no release manifest found in {release_dir}")
    return manifests[0]


def require_string(obj: dict[str, Any], key: str, path: str) -> str:
    value = obj.get(key)
    if not isinstance(value, str) or not value:
        fail(f"{path}.{key} must be a non-empty string")
    return value


def require_artifact(release_dir: Path, name: str) -> Path:
    if "/" in name or name.startswith("."):
        fail(f"artifact name must be a plain filename: {name}")
    path = release_dir / name
    if not path.is_file():
        fail(f"artifact is missing: {path}")
    return path


def verify_checksum(artifact: Path, checksum_file: Path) -> None:
    parts = checksum_file.read_text(encoding="utf-8").split()
    if len(parts) < 2:
        fail(f"{checksum_file} must contain '<sha256> <path>'")
    expected_hash = parts[0]
    expected_name = Path(parts[1]).name
    if expected_name != artifact.name:
        fail(f"{checksum_file} points at {expected_name}, expected {artifact.name}")
    actual_hash = hashlib.sha256(artifact.read_bytes()).hexdigest()
    if actual_hash != expected_hash:
        fail(f"{artifact} sha256 mismatch: expected {expected_hash}, got {actual_hash}")


def tar_names(path: Path) -> set[str]:
    try:
        with tarfile.open(path, "r:gz") as archive:
            names = set(archive.getnames())
    except tarfile.TarError as err:
        fail(f"{path} is not a valid gzip tar archive: {err}")
    return names


def read_tar_json(path: Path, member: str) -> dict[str, Any]:
    try:
        with tarfile.open(path, "r:gz") as archive:
            handle = archive.extractfile(member)
            if handle is None:
                fail(f"{path} member {member} is not a regular file")
            return json.loads(handle.read().decode("utf-8"))
    except KeyError:
        fail(f"{path} is missing {member}")
    except (json.JSONDecodeError, UnicodeDecodeError) as err:
        fail(f"{path} member {member} is not valid JSON: {err}")
    except tarfile.TarError as err:
        fail(f"{path} is not a valid gzip tar archive: {err}")


def require_members(path: Path, names: set[str], members: list[str]) -> None:
    missing = [member for member in members if member not in names]
    if missing:
        fail(f"{path} is missing archive members: {', '.join(missing)}")


def reject_python_cache_members(path: Path, names: set[str]) -> None:
    forbidden = [
        name
        for name in names
        if "/__pycache__/" in name
        or name.endswith("/__pycache__")
        or name.endswith(".pyc")
        or name.endswith(".pyo")
    ]
    if forbidden:
        fail(f"{path} contains generated Python cache files: {forbidden[:3]}")


def verify_bundle(bundle: Path, manifest: dict[str, Any]) -> None:
    package = require_string(manifest, "package", "manifest")
    names = tar_names(bundle)
    reject_python_cache_members(bundle, names)
    prefix = f"{package}/"
    require_members(
        bundle,
        names,
        [
            f"{prefix}MANIFEST.files",
            f"{prefix}MANIFEST.json",
            f"{prefix}README.md",
            f"{prefix}SECURITY.md",
            f"{prefix}LICENSE-APACHE",
            f"{prefix}LICENSE-MIT",
            f"{prefix}bin/seatgeistd",
            f"{prefix}bin/seatgeist-cli",
            f"{prefix}bin/seatgeist-mcp",
            f"{prefix}desktop/org.seatgeist.daemon.desktop.in",
            f"{prefix}plugin/.codex-plugin/plugin.json",
            f"{prefix}plugin/.mcp.json",
            f"{prefix}docs/release-checklist.md",
            f"{prefix}scripts/check-local-codex-install.py",
            f"{prefix}scripts/deploy-seatgeistd-user.py",
            f"{prefix}scripts/deploy_user_daemon.py",
            f"{prefix}scripts/install-kwin-screenshot-authorization.py",
            f"{prefix}scripts/portal-screenshot-v3-status.py",
            f"{prefix}scripts/release-external-preflight.py",
            f"{prefix}scripts/run-release-live-evals.sh",
            f"{prefix}scripts/seatgeist-panic-stop-hotkey",
            f"{prefix}scripts/smoke-codex-plugin-install.sh",
            f"{prefix}scripts/write-release-evidence.sh",
            f"{prefix}scripts/verify-release-evidence.py",
            f"{prefix}systemd/seatgeistd.service",
            f"{prefix}udev/99-seatgeist-uinput.rules",
        ],
    )
    embedded_manifest = read_tar_json(bundle, f"{prefix}MANIFEST.json")
    if embedded_manifest != manifest:
        fail(f"{bundle} embedded MANIFEST.json differs from external manifest")


def verify_plugin(plugin: Path, manifest: dict[str, Any]) -> None:
    version = require_string(manifest, "version", "manifest")
    git = require_string(manifest, "git", "manifest")
    plugin_name = f"seatgeist-{version}-{git}-plugin"
    names = tar_names(plugin)
    reject_python_cache_members(plugin, names)
    prefix = f"{plugin_name}/"
    require_members(
        plugin,
        names,
        [
            f"{prefix}MANIFEST.files",
            f"{prefix}MANIFEST.json",
            f"{prefix}.codex-plugin/plugin.json",
            f"{prefix}.mcp.json",
            f"{prefix}hooks/hooks.json",
            f"{prefix}hooks/seatgeist_audit_summary.py",
            f"{prefix}skills/seatgeist-browser-debugging/SKILL.md",
            f"{prefix}skills/seatgeist-computer-use/SKILL.md",
            f"{prefix}skills/seatgeist-desktop-triage/SKILL.md",
            f"{prefix}skills/seatgeist-gui-testing/SKILL.md",
        ],
    )
    embedded_manifest = read_tar_json(plugin, f"{prefix}MANIFEST.json")
    if embedded_manifest.get("type") != "codex-plugin":
        fail(f"{plugin} embedded MANIFEST.json type must be codex-plugin")
    if embedded_manifest.get("version") != version:
        fail(f"{plugin} embedded MANIFEST.json version differs from release manifest")
    if embedded_manifest.get("git") != git:
        fail(f"{plugin} embedded MANIFEST.json git differs from release manifest")
    forbidden = [name for name in names if "/target/" in name or "/.git/" in name or "/bin/" in name]
    if forbidden:
        fail(f"{plugin} contains generated, VCS, or binary paths: {forbidden[:3]}")


def verify_source(source: Path, manifest: dict[str, Any]) -> None:
    version = require_string(manifest, "version", "manifest")
    git = require_string(manifest, "git", "manifest")
    source_name = f"seatgeist-{version}-{git}-source"
    names = tar_names(source)
    reject_python_cache_members(source, names)
    prefix = f"{source_name}/"
    require_members(
        source,
        names,
        [
            f"{prefix}.agents/plugins/marketplace.json",
            f"{prefix}Cargo.lock",
            f"{prefix}Cargo.toml",
            f"{prefix}README.md",
            f"{prefix}SECURITY.md",
            f"{prefix}LICENSE-APACHE",
            f"{prefix}LICENSE-MIT",
            f"{prefix}Makefile",
            f"{prefix}crates/libseatgeist/src/protocol.rs",
            f"{prefix}crates/seatgeistd/src/main.rs",
            f"{prefix}desktop/org.seatgeist.daemon.desktop.in",
            f"{prefix}plugin/.mcp.json",
            f"{prefix}scripts/package-release.sh",
            f"{prefix}scripts/verify-release-artifacts.py",
            f"{prefix}scripts/verify-release-install.sh",
            f"{prefix}scripts/sign-release-artifacts.sh",
            f"{prefix}scripts/verify-release-signatures.sh",
            f"{prefix}scripts/portal-screenshot-v3-status.py",
            f"{prefix}scripts/check-local-codex-install.py",
            f"{prefix}scripts/install-kwin-screenshot-authorization.py",
            f"{prefix}scripts/run-release-live-evals.sh",
            f"{prefix}scripts/write-release-evidence.sh",
            f"{prefix}scripts/verify-release-evidence.py",
            f"{prefix}scripts/smoke-codex-plugin-install.sh",
            f"{prefix}scripts/check-public-name.py",
            f"{prefix}scripts/release-readiness.py",
            f"{prefix}scripts/release-external-preflight.py",
            f"{prefix}scripts/write-eval-evidence.py",
        ],
    )
    forbidden = [name for name in names if "/target/" in name or "/.git/" in name]
    if forbidden:
        fail(f"{source} contains generated or VCS paths: {forbidden[:3]}")
    github_automation = [name for name in names if "/.github/" in name]
    if github_automation:
        fail(f"{source} contains private-state GitHub automation: {github_automation[:3]}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Verify generated Seatgeist release artifacts.")
    parser.add_argument("manifest", nargs="?", help="Path to a generated *.manifest.json file.")
    args = parser.parse_args()

    manifest_path = Path(args.manifest) if args.manifest else newest_manifest(DEFAULT_RELEASE_DIR)
    if not manifest_path.is_absolute():
        manifest_path = ROOT / manifest_path
    if not manifest_path.is_file():
        fail(f"manifest is missing: {manifest_path}")

    release_dir = manifest_path.parent
    manifest = load_json(manifest_path)
    if require_string(manifest, "name", "manifest") != "Seatgeist":
        fail("manifest.name must be Seatgeist")

    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, dict):
        fail("manifest.artifacts must be an object")

    bundle = require_artifact(release_dir, require_string(artifacts, "bundle", "manifest.artifacts"))
    bundle_checksum = require_artifact(
        release_dir, require_string(artifacts, "bundle_sha256", "manifest.artifacts")
    )
    plugin = require_artifact(release_dir, require_string(artifacts, "plugin", "manifest.artifacts"))
    plugin_checksum = require_artifact(
        release_dir, require_string(artifacts, "plugin_sha256", "manifest.artifacts")
    )
    source = require_artifact(release_dir, require_string(artifacts, "source", "manifest.artifacts"))
    source_checksum = require_artifact(
        release_dir, require_string(artifacts, "source_sha256", "manifest.artifacts")
    )

    verify_checksum(bundle, bundle_checksum)
    verify_checksum(plugin, plugin_checksum)
    verify_checksum(source, source_checksum)
    verify_bundle(bundle, manifest)
    verify_plugin(plugin, manifest)
    verify_source(source, manifest)

    print(f"verify-release-artifacts: ok {manifest_path}")


if __name__ == "__main__":
    main()
