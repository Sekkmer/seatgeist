# Seatgeist Release Checklist

This checklist defines the minimum evidence required before calling a public release ready. It is intentionally stricter than the local development tracker because public users need repeatable install, safety, and troubleshooting paths.

Run `make release-readiness` to summarize current blockers from local repo metadata, generated release artifacts, public-name collision evidence, signatures, and opt-in live eval evidence. Live eval evidence must include a matching `evidence.json` pass record written by the eval scripts, not only loose artifact files. For a release cut, run `scripts/release-readiness.py --strict` after the checklist items below are complete.

## Release Blocking Evidence

- [x] Safe workspace verification exists as `make verify`.
- [x] External CI runs the safe verification gate on push and pull requests.
- [x] Policy-denied raw input, semantic control, clipboard read, full-resolution screenshot, and panic-stop paths are covered by tests, replay traces, or safe GUI evals.
- [x] Arch Linux/KDE Plasma 6 operator installation docs exist.
- [x] Plugin manifest, MCP config, skills, and hook assets validate locally.
- [~] Manual KDE Plasma 6 Wayland evals exist, but broader repeated passes are still required before a public v0.1 release.
- [~] Versioned local release artifact packaging, standalone plugin bundle packaging, verification, clean-install validation, and optional GPG signing exist through `make verify-release-artifacts`, `make verify-release-install`, `make sign-release-artifacts`, and `make verify-release-signatures`; public uploads and signed release tags are not done yet.
- [ ] Add real public repository metadata before publishing, replacing placeholder `example.invalid` Cargo package URLs.
- [x] Final license files match the workspace `MIT OR Apache-2.0` declaration.
- [x] Public project name and package/binary prefixes are `Seatgeist` / `seatgeist-*`.
- [~] Exact-name web search on 2026-07-05 did not show an obvious existing Seatgeist software/project collision; `make check-public-name` now writes repeatable crates.io, npm, PyPI, and GitHub exact-name evidence, but formal trademark and domain checks remain before publishing.
- [ ] Run and record the opt-in live evals on the target KDE machine: KWrite/Kate input, KCalc visual input, Firefox localhost click, portal Screenshot, RemoteDesktop probe, and retained RemoteDesktop EIS session.
- [x] Known unsupported paths are documented for GNOME, wlroots/Sway, X11, kernel modules, OCR fallback, and native desktop approval UX.
- [ ] Publish signed or checksummed binaries, plugin bundle, and source archive for the release tag.

## CI Scope

The CI workflow runs only safe, non-opt-in gates. It does not send real desktop input, start portal consent flows, install the KWin script, mutate system policy, or require a graphical KDE session. The live GUI and portal evals remain local operator release evidence until a reliable desktop-integration runner exists.

## Release Cut Procedure

1. Update `Cargo.toml` workspace metadata and package URLs for the target public repository.
2. Update this checklist and `docs/tracker.md` with current release evidence.
3. Run `make check-public-name` and review the generated exact-name collision report.
4. Run `make release-readiness` to capture the current blocker list before release work.
5. Run `make verify` locally.
6. Run each opt-in live eval intentionally on the supported KDE Plasma 6 Wayland workstation and keep the generated `evidence.json` records.
7. Create a signed release tag.
8. Build and verify local release artifacts with `make verify-release-artifacts`.
9. Verify a clean extraction/install smoke from the binary archive and standalone plugin archive with `make verify-release-install`.
10. Sign the generated artifacts with `SEATGEIST_RELEASE_SIGNING_KEY=<key> make sign-release-artifacts`, then run `make verify-release-signatures`.
11. Run `scripts/release-readiness.py --strict`; it must report no blockers before upload.
12. Upload the generated Seatgeist binary/plugin tarball, source tarball, manifest, checksums, detached signatures, and signature checksum manifest from `target/seatgeist-release/`.
13. Publish checksums and signatures for every uploaded artifact.
14. Verify a clean install from the released artifacts on a separate release-test checkout or machine, not from the working tree.
