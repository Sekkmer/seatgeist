#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    print(f"validate-release: {message}", file=sys.stderr)
    raise SystemExit(1)


def read(path: str) -> str:
    file_path = ROOT / path
    if not file_path.exists():
        fail(f"{path} is missing")
    return file_path.read_text(encoding="utf-8")


def require_contains(path: str, text: str, needle: str) -> None:
    if needle not in text:
        fail(f"{path} does not contain expected text: {needle}")


def require_absent(path: str, text: str, needle: str) -> None:
    if needle in text:
        fail(f"{path} still contains placeholder text: {needle}")


def require_executable(path: str) -> str:
    file_path = ROOT / path
    if not file_path.exists():
        fail(f"{path} is missing")
    if file_path.stat().st_mode & 0o111 == 0:
        fail(f"{path} must be executable")
    return file_path.read_text(encoding="utf-8")


def main() -> None:
    cargo = read("Cargo.toml")
    require_contains("Cargo.toml", cargo, 'license = "MIT OR Apache-2.0"')

    mit = read("LICENSE-MIT")
    require_contains("LICENSE-MIT", mit, "MIT License")
    require_contains("LICENSE-MIT", mit, "Copyright (c) 2026 Sekkmer")
    require_contains("LICENSE-MIT", mit, "Permission is hereby granted")
    require_absent("LICENSE-MIT", mit, "<year>")
    require_absent("LICENSE-MIT", mit, "<copyright holders>")

    apache = read("LICENSE-APACHE")
    require_contains("LICENSE-APACHE", apache, "Apache License")
    require_contains("LICENSE-APACHE", apache, "Version 2.0, January 2004")
    require_contains("LICENSE-APACHE", apache, "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION")
    require_contains("LICENSE-APACHE", apache, "Copyright 2026 Sekkmer")
    require_absent("LICENSE-APACHE", apache, "Copyright [yyyy] [name of copyright owner]")

    makefile = read("Makefile")
    require_contains("Makefile", makefile, "package-release:")
    require_contains("Makefile", makefile, "scripts/package-release.sh")
    require_contains("Makefile", makefile, "verify-release-artifacts: package-release")
    require_contains("Makefile", makefile, "scripts/verify-release-artifacts.py")
    require_contains("Makefile", makefile, "verify-release-install: verify-release-artifacts")
    require_contains("Makefile", makefile, "scripts/verify-release-install.sh")
    require_contains("Makefile", makefile, "sign-release-artifacts: verify-release-artifacts")
    require_contains("Makefile", makefile, "scripts/sign-release-artifacts.sh")
    require_contains("Makefile", makefile, "verify-release-signatures:")
    require_contains("Makefile", makefile, "scripts/verify-release-signatures.sh")
    require_contains("Makefile", makefile, "write-release-evidence:")
    require_contains("Makefile", makefile, "scripts/write-release-evidence.sh")
    require_contains("Makefile", makefile, "verify-release-evidence:")
    require_contains("Makefile", makefile, "scripts/verify-release-evidence.py")
    require_contains("Makefile", makefile, "smoke-codex-plugin-install:")
    require_contains("Makefile", makefile, "scripts/smoke-codex-plugin-install.sh")
    require_contains("Makefile", makefile, "check-local-codex-install:")
    require_contains("Makefile", makefile, "scripts/check-local-codex-install.py --strict")
    require_contains("Makefile", makefile, "check-public-name:")
    require_contains("Makefile", makefile, "scripts/check-public-name.py")
    require_contains("Makefile", makefile, "release-readiness:")
    require_contains("Makefile", makefile, "scripts/release-readiness.py")
    require_contains("Makefile", makefile, "release-external-preflight:")
    require_contains("Makefile", makefile, "scripts/release-external-preflight.py")
    require_contains("Makefile", makefile, "release-live-evals:")
    require_contains("Makefile", makefile, "scripts/run-release-live-evals.sh")
    require_contains("Makefile", makefile, "portal-screenshot-v3-status:")
    require_contains("Makefile", makefile, "scripts/portal-screenshot-v3-status.py")

    package_release = require_executable("scripts/package-release.sh")
    for needle in [
        "cargo build --workspace --release",
        "package_name=\"seatgeist-${version}-${git_short}-${target_triple}\"",
        "source_name=\"seatgeist-${version}-${git_short}-source\"",
        "plugin_name=\"seatgeist-${version}-${git_short}-plugin\"",
        "target/seatgeist-release",
        "seatgeistd",
        "seatgeist-cli",
        "seatgeist-mcp",
        "\"plugin\": \"$(basename \"$plugin_archive\")\"",
        "\"source\": \"$(basename \"$source_archive\")\"",
        "cp -a scripts/. \"$stage/scripts/\"",
        "seatgeist-panic-stop-hotkey",
        "MANIFEST.json",
        "git ls-files -z",
        "find \"$stage\" \"$plugin_stage\" -type d -name __pycache__",
        "-name '*.pyc'",
        "plugin_archive=",
        "plugin_checksum=",
        "source_archive=",
        "source_checksum=",
        "sha256sum",
        "tar --sort=name",
    ]:
        require_contains("scripts/package-release.sh", package_release, needle)

    verify_release = require_executable("scripts/verify-release-artifacts.py")
    for needle in [
        "verify_checksum(bundle, bundle_checksum)",
        "verify_checksum(plugin, plugin_checksum)",
        "verify_checksum(source, source_checksum)",
        "verify_bundle(bundle, manifest)",
        "verify_plugin(plugin, manifest)",
        "verify_source(source, manifest)",
        "reject_python_cache_members",
        "__pycache__",
        ".pyc",
        "crates/seatgeistd/src/main.rs",
        "plugin/.mcp.json",
        ".agents/plugins/marketplace.json",
        "scripts/package-release.sh",
        "scripts/verify-release-artifacts.py",
        "scripts/verify-release-install.sh",
        "scripts/sign-release-artifacts.sh",
        "scripts/verify-release-signatures.sh",
        "scripts/portal-screenshot-v3-status.py",
        "scripts/check-public-name.py",
        "scripts/release-readiness.py",
        "scripts/release-external-preflight.py",
        "scripts/verify-release-evidence.py",
        "scripts/smoke-codex-plugin-install.sh",
        "scripts/check-local-codex-install.py",
        "scripts/run-release-live-evals.sh",
        "target/",
    ]:
        require_contains("scripts/verify-release-artifacts.py", verify_release, needle)

    install_verify = require_executable("scripts/verify-release-install.sh")
    for needle in [
        "validate-plugin.py",
        "validate-install-assets.py",
        "plugin_root=",
        "seatgeist-cli",
        "seatgeist-mcp",
        "seatgeistd",
        "--version",
        "verify-release-install: ok",
    ]:
        require_contains("scripts/verify-release-install.sh", install_verify, needle)

    sign_release = require_executable("scripts/sign-release-artifacts.sh")
    for needle in [
        "SEATGEIST_RELEASE_SIGNING_KEY",
        "gpg --batch --yes --armor --local-user",
        "--detach-sign",
        ".signatures.sha256",
    ]:
        require_contains("scripts/sign-release-artifacts.sh", sign_release, needle)

    verify_signatures = require_executable("scripts/verify-release-signatures.sh")
    for needle in [
        "gpg --batch --verify",
        "sha256sum --check",
        ".signatures.sha256",
        "verify-release-signatures: ok",
    ]:
        require_contains("scripts/verify-release-signatures.sh", verify_signatures, needle)

    write_release_evidence = require_executable("scripts/write-release-evidence.sh")
    for needle in [
        "scripts/release-readiness.py --json",
        "scripts/portal-screenshot-v3-status.py",
        "scripts/verify-release-evidence.py \"$manifest\"",
        ".readiness.json",
        ".portal-screenshot-v3-status.json",
    ]:
        require_contains("scripts/write-release-evidence.sh", write_release_evidence, needle)

    verify_release_evidence = require_executable("scripts/verify-release-evidence.py")
    for needle in [
        "release_readiness",
        "portal_screenshot_v3_status",
        "REQUIRED_READINESS_CHECKS",
        "REQUIRED_PORTAL_PACKAGES",
        "read-only diagnostic",
        "blocker_count",
        "latest_manifest",
        "verify-release-evidence: ok",
    ]:
        require_contains("scripts/verify-release-evidence.py", verify_release_evidence, needle)

    codex_plugin_smoke = require_executable("scripts/smoke-codex-plugin-install.sh")
    for needle in [
        "CODEX_HOME",
        "codex plugin marketplace add . --json",
        "codex plugin list --marketplace seatgeist-local --available --json",
        "codex plugin add seatgeist@seatgeist-local --json",
        "scripts/validate-plugin.py \"$installed_path\"",
        "seatgeist-mcp --help",
        "smoke-codex-plugin-install: ok",
    ]:
        require_contains("scripts/smoke-codex-plugin-install.sh", codex_plugin_smoke, needle)

    local_codex_install = require_executable("scripts/check-local-codex-install.py")
    for needle in [
        "seatgeist_local_codex_install",
        "seatgeist@seatgeist-local",
        "seatgeist-local",
        "marketplace_source",
        "installed_plugin_cache",
        "foreign_build_output_path",
        "seatgeist-mcp",
        "seatgeist-cli",
        "seatgeistd",
        "--strict",
    ]:
        require_contains("scripts/check-local-codex-install.py", local_codex_install, needle)

    release_live_evals = require_executable("scripts/run-release-live-evals.sh")
    for needle in [
        "SEATGEIST_RELEASE_LIVE_EVALS_APPROVED",
        "SEATGEIST_PORTAL_SCREENSHOT_STRICT=1",
        "SEATGEIST_REMOTE_DESKTOP_STRICT=1",
        "SEATGEIST_REMOTE_DESKTOP_EIS_STRICT=1",
        "SEATGEIST_REMOTE_DESKTOP_EIS_INPUT_STRICT=1",
        "make gui-eval-text-editor-input",
        "make gui-eval-kcalc-visual",
        "make gui-eval-firefox-localhost-button",
        "make gui-eval-portal-screenshot",
        "make gui-eval-remote-desktop-probe",
        "make gui-eval-remote-desktop-eis-session",
        "live_eval_evidence ok",
    ]:
        require_contains("scripts/run-release-live-evals.sh", release_live_evals, needle)

    readiness = require_executable("scripts/release-readiness.py")
    for needle in [
        "release_readiness",
        "public_metadata",
        "name_collision_report",
        "release_checklist",
        "release_artifacts",
        "release_signatures",
        "plugin_sha256",
        "live_eval_evidence",
        "seatgeist_eval_evidence",
        "evidence.json",
        "evidence is stale",
        "current-commit opt-in live KDE eval evidence exists",
        "latest release manifest is stale for the current commit",
        "--json",
        "--strict",
    ]:
        require_contains("scripts/release-readiness.py", readiness, needle)

    external_preflight = require_executable("scripts/release-external-preflight.py")
    for needle in [
        "release_external_preflight",
        "public_metadata",
        "local_codex_install",
        "signed_release_tag",
        "signed_release_artifacts",
        "live_eval_evidence",
        "name_collision_report",
        "scripts/check-local-codex-install.py",
        "make check-local-codex-install",
        "SEATGEIST_RELEASE_LIVE_EVALS_APPROVED=1 make release-live-evals",
        "git tag -s v0.1.0",
        "--strict",
    ]:
        require_contains("scripts/release-external-preflight.py", external_preflight, needle)

    eval_evidence = require_executable("scripts/write-eval-evidence.py")
    for needle in [
        "seatgeist_eval_evidence",
        "evidence.json",
        "write-eval-evidence",
        "--run-dir",
        "--case",
        "--kind",
    ]:
        require_contains("scripts/write-eval-evidence.py", eval_evidence, needle)

    name_check = require_executable("scripts/check-public-name.py")
    for needle in [
        "seatgeist_name_collision_check",
        "name-collision-check.json",
        "crates.io",
        "registry.npmjs.org",
        "pypi.org",
        "api.github.com/search/repositories",
        "--strict",
    ]:
        require_contains("scripts/check-public-name.py", name_check, needle)

    portal_v3_status = require_executable("scripts/portal-screenshot-v3-status.py")
    for needle in [
        "portal_screenshot_v3_status",
        "AvailableTargets",
        "screenshot_interface_version",
        "target_option_supported",
        "pacman",
        "aur-step",
        "read-only diagnostic",
    ]:
        require_contains("scripts/portal-screenshot-v3-status.py", portal_v3_status, needle)

    checklist = read("docs/release-checklist.md")
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [x] Public project name and package/binary prefixes are `Seatgeist` / `seatgeist-*`.",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "repo-local Codex marketplace entry validate locally",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "Local Codex install preflight exists as `make check-local-codex-install`",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [~] Versioned local release artifact packaging, standalone plugin bundle packaging, verification, clean-install validation, optional GPG signing, retained JSON release-evidence snapshots, and evidence-snapshot verification exist through `make verify-release-artifacts`, `make verify-release-install`, `make sign-release-artifacts`, `make verify-release-signatures`, `make write-release-evidence`, and `make verify-release-evidence`; public uploads and signed release tags are not done yet.",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [x] Retained release-evidence snapshots are shape-checked by `make verify-release-evidence`.",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "make write-release-evidence",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "SEATGEIST_RELEASE_LIVE_EVALS_APPROVED=1 make release-live-evals",
    )

    plugin_doc = read("docs/plugin.md")
    for needle in [
        "seatgeist@seatgeist-local",
        "make smoke-codex-plugin-install",
        "make check-local-codex-install",
        "codex exec --sandbox read-only",
        "$seatgeist-desktop-triage",
        "SKILL_OK",
        "seatgeist-mcp --version",
    ]:
        require_contains("docs/plugin.md", plugin_doc, needle)

    require_contains(
        "docs/release-checklist.md",
        checklist,
        "Run `make release-readiness` to summarize current blockers",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "Run `make release-external-preflight`",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "`make release-external-preflight` also reports this check as `local_codex_install`",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [x] Final license files match the workspace `MIT OR Apache-2.0` declaration.",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [ ] Add real public repository metadata before publishing",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [x] Known unsupported paths are documented for GNOME, wlroots/Sway, X11, kernel modules, OCR fallback, and native desktop approval UX.",
    )

    unsupported = read("docs/unsupported-paths.md")
    for label in [
        "GNOME is not implemented.",
        "wlroots and Sway are not implemented.",
        "X11 is not a supported baseline.",
        "Kernel modules are not implemented.",
        "Screenshot/OCR fallback for semantic actions is not implemented.",
        "Native desktop approval UX is not implemented.",
    ]:
        require_contains("docs/unsupported-paths.md", unsupported, label)

    ci = read(".github/workflows/ci.yml")
    require_contains(".github/workflows/ci.yml", ci, "make verify")
    require_contains(".github/workflows/ci.yml", ci, "libei-dev")
    require_contains(".github/workflows/ci.yml", ci, "libxkbcommon-dev")

    arch_install = read("docs/arch-kde-install.md")
    require_contains("docs/arch-kde-install.md", arch_install, "make portal-screenshot-v3-status")
    require_contains("docs/arch-kde-install.md", arch_install, "make check-local-codex-install")
    require_contains("docs/arch-kde-install.md", arch_install, "Screenshot v3")
    require_contains("docs/arch-kde-install.md", arch_install, "AvailableTargets")

    print("validate-release: ok")


if __name__ == "__main__":
    main()
