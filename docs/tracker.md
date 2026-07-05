# Seatgeist Tracker

Legend:

- `[x]` done and covered by repo evidence.
- `[~]` partial or in progress.
- `[ ]` not done yet.

Current full-project estimate: `[~]` about one third complete. This estimate uses the full Seatgeist goal as the denominator: a reliable backend-neutral computer-use substrate for Codex, starting with the KDE/Plasma backend, with policy, journaling, MCP/plugin integration, real evals, installation docs, and room for alternate backends.

## Foundation

- [x] Cargo workspace with resolver 3 and Rust 2024 member crates.
- [x] Shared protocol/types crate.
- [x] Daemon crate with Unix socket protocol.
- [x] CLI crate for daemon operations.
- [x] MCP stdio crate for Codex tool access.
- [x] Backend traits and deterministic testkit mocks.
- [x] Plugin, skills, hooks, systemd, udev, and polkit skeletons.
- [x] Arch/KDE install documentation.

## Safety Contract

- [x] All daemon requests flow through policy classification before execution.
- [x] Prompt-level control fails closed by default.
- [x] Keyboard, pointer, semantic, clipboard-read, and full-resolution screenshot requests have separate safety classes.
- [x] Control actions are journaled with compact method, success, client, safety class, guard, and summary metadata.
- [x] Journals redact typed text, replacement text, clipboard contents, screenshots, and semantic target names.
- [x] Panic-stop blocks control.
- [x] Active-window guards default to required for control.
- [x] Human-input pause can block approved control when a fresh activity signal exists.
- [x] App allow/deny policy can block control before backend execution.
- [x] Control rate limiting exists for control-class requests.
- [~] Operator approval flow exists through approval files and broad local flags; desktop-native approval UX remains future work.

## Observation

- [x] Daemon health, capabilities, policy status, safety status, and readiness diagnostics.
- [x] Desktop session diagnostics for KDE/Wayland troubleshooting.
- [x] Window listing through KWin runner plus bridge enrichment.
- [x] Active-window bridge status and KWin script packaging.
- [x] Bounded screenshot preview capture with backend provenance.
- [x] Screenshot tile capture with 8K/scaled coordinate metadata.
- [x] Wait-for-change polling with timed-out versus failed-command distinction.
- [x] Capture backend status for portal, KWin metadata, and Spectacle fallback.
- [~] Portal Screenshot live eval exists but remains opt-in because it may request user consent.
- [~] Robust multi-monitor visual eval coverage is partial.

## Input Backends

- [x] uinput keyboard path for type text and key combos.
- [x] uinput pointer path for move, click, drag, and scroll.
- [x] Physical, global logical, and guarded window-local coordinate spaces.
- [x] Pointer calibration diagnostics for scaled and 8K displays.
- [x] Raw input backend trait boundary.
- [x] RemoteDesktop portal probe without sending input.
- [x] Transient RemoteDesktop EIS probe without sending input.
- [x] Retained RemoteDesktop EIS session lifecycle.
- [x] EIS/libei action planning and readiness gates.
- [x] Explicit portal/libei raw input routes through retained ready EIS sessions only after policy and safety gates.
- [~] Live portal/libei input execution is implemented but depends on consent/session/device readiness and remains opt-in for real input evals.
- [ ] Kernel or custom KDE module work has not started and is intentionally deferred until supported paths show a concrete gap.

## Clipboard

- [x] Safe clipboard backend diagnostics without reading contents.
- [x] Clipboard set through wl-clipboard or KDE Klipper fallback.
- [x] Clipboard get is policy-gated and bounded by default.
- [x] Clipboard summaries and journal entries report length/backend metadata only.
- [x] Clipboard smoke restores previous text when possible.

## Accessibility And Semantic Control

- [x] Focused AT-SPI tree reads with depth/node caps.
- [x] AT-SPI find by role/name/app/window filters.
- [x] AT-SPI text attributes with compact range/count output.
- [x] Direct AT-SPI invoke/edit/caret/selection operations behind semantic-control policy.
- [x] High-level semantic actions for buttons, text fields, menus, tabs, links, toggles, values, and items.
- [x] Ambiguity refusal returns bounded non-sensitive candidate choices.
- [x] Semantic candidate ids are stable across raw AT-SPI node-id churn.
- [x] Accessibility quality diagnostics identify weak or generic trees.
- [~] Semantic actions work through the implemented AT-SPI path, but broad common-app manual eval coverage is still partial.
- [ ] Screenshot/OCR fallback for semantic actions is not implemented.

## MCP And Plugin

- [x] MCP initialize, ping, tools/list, and tools/call.
- [x] MCP exposes current observation, screenshot, window, safety, clipboard, input, RemoteDesktop/EIS, AT-SPI, semantic, and journal tools.
- [x] MCP tool outputs are compact and include structured daemon responses.
- [x] MCP stdio integration tests cover real daemon calls, configured denial kinds, raw-input denial journaling, and journal visibility.
- [x] Plugin manifest and bundled MCP config validate.
- [x] Four plugin skills describe current Seatgeist workflows and safety rules.
- [x] Stop audit hook writes compact repo/journal audit summaries.
- [~] Installed-plugin end-to-end use in a real Codex session is documented and scaffolded, but broader field validation remains partial.

## Evals And Regression Gates

- [x] `make verify` runs formatting, check, tests, clippy, plugin/install validation, trace validation, smoke tests, MCP smoke, and safe GUI evals.
- [x] GitHub Actions CI runs the safe verification gate on push and pull requests.
- [x] Checked-in replay traces cover status, journal filters, protected policy denials, semantic denials, raw input denials, and panic-stop transitions.
- [x] `make smoke-trace-replay` explicitly validates and replays the checked-in trace categories, including semantic denials.
- [x] Safe GUI evals cover status, session preflight, observe, AT-SPI diagnostics, denial paths, clipboard status/denial, KWin bridge status, keymap status, full-resolution denial, and control safety.
- [~] Opt-in GUI evals exist for KWrite/Kate, KCalc, Firefox localhost button, portal Screenshot, RemoteDesktop probe, and retained RemoteDesktop EIS session.
- [~] Manual KDE Plasma 6 Wayland eval suite is useful but not complete enough to call v0.1 done.
- [~] Public release checklist, external CI, dual license files, local Seatgeist binary/plugin plus source release packaging, artifact verification, clean-install validation, optional GPG signing targets, and a release-readiness blocker audit exist; public uploads and signed release tags are not done.

## v0.1 Definition Of Done

- [x] Seatgeist daemon runs under the user session.
- [x] Codex can connect through MCP.
- [x] Codex can observe the screen.
- [x] Codex can list and focus windows.
- [x] Codex can click/type through the implemented local input paths when explicitly approved and configured.
- [x] Actions are policy-checked and journaled.
- [x] Panic-stop works.
- [x] Clipboard get/set exists with policy checks.
- [x] Plugin bundle contains working MCP config and useful skills.
- [~] Basic KDE Plasma 6 Wayland manual evals exist but need broader repeated passes before v0.1 is complete.

## v0.2 Definition Of Done

- [x] AT-SPI focused tree works.
- [~] Semantic button/text/menu actions are implemented; broad common-KDE-app reliability remains partial.
- [x] Coordinate mapping supports scaling and 8K physical-pixel source metadata.
- [x] Window guards are used by default.
- [x] Plugin hooks provide useful audit summaries.
- [x] Docs explain installation on Arch Linux/KDE Plasma 6.
- [ ] v0.2 is not complete until semantic reliability and manual eval maturity are stronger.

## Public/General Backend Future

- [~] Backend traits already allow KDE, Wayland/X11, mock, uinput, portal/libei, and future implementations to coexist.
- [~] KDE is the only first-class desktop target right now.
- [x] Backend-neutral public name and package/binary prefixes are `Seatgeist` / `seatgeist-*`.
- [x] Unsupported public paths are documented for GNOME, wlroots/Sway, X11, kernel modules, OCR fallback, custom KDE/KWin work, and native desktop approval UX.
- [ ] GNOME backend is not implemented.
- [x] Public project naming is finalized as `Seatgeist`.
- [ ] Hardening for public distribution, packaging, signing, and support expectations is not done.
