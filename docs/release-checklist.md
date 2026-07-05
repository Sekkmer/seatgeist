# PlasmaPilot Release Checklist

This checklist defines the minimum evidence required before calling a public release ready. It is intentionally stricter than the local development tracker because public users need repeatable install, safety, and troubleshooting paths.

## Release Blocking Evidence

- [x] Safe workspace verification exists as `make verify`.
- [x] External CI runs the safe verification gate on push and pull requests.
- [x] Policy-denied raw input, semantic control, clipboard read, full-resolution screenshot, and panic-stop paths are covered by tests, replay traces, or safe GUI evals.
- [x] Arch Linux/KDE Plasma 6 operator installation docs exist.
- [x] Plugin manifest, MCP config, skills, and hook assets validate locally.
- [~] Manual KDE Plasma 6 Wayland evals exist, but broader repeated passes are still required before a public v0.1 release.
- [~] Versioned release artifacts are not produced yet.
- [ ] Add real public repository metadata before publishing, replacing placeholder `example.invalid` Cargo package URLs.
- [ ] Add final license files matching the workspace `MIT OR Apache-2.0` declaration.
- [ ] Decide whether the public project name remains `PlasmaPilot` or moves to a backend-neutral name.
- [ ] Run and record the opt-in live evals on the target KDE machine: KWrite/Kate input, KCalc visual input, Firefox localhost click, portal Screenshot, RemoteDesktop probe, and retained RemoteDesktop EIS session.
- [ ] Document known unsupported paths for GNOME, wlroots/Sway, X11, kernel modules, OCR fallback, and native desktop approval UX.
- [ ] Publish signed or checksummed binaries, plugin bundle, and source archive for the release tag.

## CI Scope

The CI workflow runs only safe, non-opt-in gates. It does not send real desktop input, start portal consent flows, install the KWin script, mutate system policy, or require a graphical KDE session. The live GUI and portal evals remain local operator release evidence until a reliable desktop-integration runner exists.

## Release Cut Procedure

1. Update `Cargo.toml` workspace metadata and package URLs for the target public repository.
2. Update this checklist and `docs/tracker.md` with current release evidence.
3. Run `make verify` locally.
4. Run each opt-in live eval intentionally on the supported KDE Plasma 6 Wayland workstation and save the artifact paths or summaries.
5. Create a signed release tag.
6. Build release binaries with `cargo build --workspace --release`.
7. Package the Codex plugin directory and release binaries.
8. Publish checksums for every uploaded artifact.
9. Verify a clean install from the released artifacts, not from the working tree.
