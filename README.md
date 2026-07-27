# Seatgeist

Seatgeist is a local, policy-gated desktop-control substrate for Codex on KDE
Plasma 6 Wayland. It combines a user-scoped daemon, CLI, MCP server, Codex
plugin, and KDE integration so an agent can observe and operate desktop apps
without turning ordinary automation into an unbounded input channel.

The project is usable for local development on its supported KDE baseline, but
the repository is private and it is not yet a public v0.1 release. GitHub
Actions intentionally remains disabled until the repository is made public.
Signed release artifacts and final live-evaluation evidence also remain release
blockers; see the [release checklist](docs/release-checklist.md) for the
authoritative state.

## What it provides

- Window inventory, active-window state, monitor metadata, and bounded
  high-DPI screenshots.
- Exact KWin window capture, retained capture sessions, visual change waits,
  and target-bound post-action images.
- Policy-gated keyboard, pointer, clipboard, AT-SPI semantic control, window
  move/resize, desktop-entry launch, and guarded browser page zoom.
- Active-window guards, app allow/deny rules, panic-stop, human-input pause,
  control rate limits, and compact action journaling.
- A bounded core MCP profile for normal agent use and an expert profile for
  diagnostics and compatibility tools.
- A local Codex plugin with computer-use, GUI-testing, browser-debugging, and
  desktop-triage skills.

All input and semantic-control actions go through daemon policy checks and are
journaled. Secret fields fail closed by default, control defaults to prompt,
and screenshots default to bounded previews instead of full-resolution 8K
payloads. See [Safety](docs/safety.md) and the
[threat model](docs/threat-model.md) before enabling control.

## Architecture

The main components are:

- `seatgeistd`: owns policy, journaling, sessions, and desktop backends.
- `seatgeist-cli`: operator diagnostics and explicit local control.
- `seatgeist-mcp`: compact model-facing tools over the daemon protocol.
- `plugin/`: Codex manifest, MCP configuration, skills, and audit hook.
- `kwin/`: KWin script and optional activity helper integration.
- `crates/seatgeist-*`: backend traits and KDE/Wayland implementations.

KDE Plasma 6 Wayland is the supported baseline. The protocol and backend traits
are intentionally desktop-neutral, but GNOME, wlroots/Sway, and X11 are not
supported yet. The exact boundaries are documented in
[Unsupported paths](docs/unsupported-paths.md).

## Build and verify

The workspace targets Rust 1.96 with the 2024 edition. From the repository
root:

```bash
cargo build --workspace
cargo test --workspace
cargo check --workspace
```

The complete safe repository gate is:

```bash
make verify
```

It does not send live desktop input, open consent dialogs, install KDE assets,
or mutate system policy. Live GUI evaluations and installation steps are
separate opt-in targets.

For the full Arch Linux/KDE setup, user service, KWin bridge, diagnostics, and
local Codex plugin install, follow the
[Arch KDE install guide](docs/arch-kde-install.md). Plugin-only development and
validation are covered in [Codex plugin](docs/plugin.md).

## Documentation

- [Architecture](docs/architecture.md)
- [MCP tool contracts](docs/mcp-tools.md)
- [Configuration](docs/config.md)
- [Capture backends](docs/capture-backends.md)
- [Project status](docs/status.md)
- [Release checklist](docs/release-checklist.md)

## License

Seatgeist is dual-licensed under the Apache License 2.0 or the MIT License, at
your option. See [LICENSE-APACHE](LICENSE-APACHE) and
[LICENSE-MIT](LICENSE-MIT).
