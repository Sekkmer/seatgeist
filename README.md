# Seatgeist

Seatgeist lets Codex look at and use applications in a local KDE Plasma 6
Wayland session. It provides screenshots, window information, clipboard and
accessibility tools, plus several ways to send keyboard and pointer input.

This is an experimental KDE project and development checkout, not a
plug-and-play desktop package. You can use it locally, but expect to build the
Rust binaries, run a user service, and install the KDE integrations needed for
the features you want.

## What is included

- `seatgeistd`: the user-session daemon.
- `seatgeist-cli`: local setup, status, and troubleshooting commands.
- `seatgeist-mcp`: the MCP server used by Codex.
- `plugin/`: the Codex plugin, skills, and MCP configuration.
- `kwin/seatgeist-bridge`: a KWin script for window state and basic window
  actions.
- `kwin/seatgeist-activity`: a KWin binary plugin that distinguishes physical
  activity from Seatgeist activity.
- `kwin/seatgeist-agent-seat`: an experimental bounded pool of per-agent
  Wayland seats that can work in native Wayland windows while you use another.

Actions still pass through the daemon's policy checks and are recorded in its
compact journal. The defaults are deliberately cautious, but this is useful
local tooling rather than a security boundary for running untrusted agents.

## KDE pieces

The integrations are separate so you can install only the parts you need:

| Piece | What it is for | When you need it |
| --- | --- | --- |
| KWin script bridge | Window list, active-window updates, move, resize, and launch coordination | Baseline for the intended KDE experience |
| KWin screenshot authorization | Lets the user daemon use KWin's exact-window screenshot interface | Needed for direct capture of covered or background windows |
| Codex plugin | Seatgeist MCP tools and desktop-use skills inside Codex | When using Seatgeist from Codex |
| Activity plugin | Trusted physical activity reporting, including target-local user preemption | When using pause-on-human-input, cooperative focus handling, or parallel agent seats |
| Agent-seat plugin | Per-agent input routed to exclusively leased native Wayland windows without taking your normal focus | Optional and experimental |
| KDE portal services | Screenshots and portal/libei input sessions | Used by the portal backends |
| uinput setup | Virtual keyboard and pointer fallback | Optional; requires local system setup |

The two KWin binary plugins are built against the installed KWin version.
Rebuild them after KWin upgrades. Installing them does not restart the
compositor; load them at the next normal Plasma login.

The agent-seat path currently targets native Wayland windows. It is not an
XWayland input backend, and applications vary in how well they handle a second
Wayland seat.

## Try it from a checkout

The current setup guide is written for Arch Linux with Plasma 6 Wayland:

```bash
sudo pacman -S --needed base-devel cmake rust cargo jq plasma-meta plasma-workspace \
  kde-cli-tools spectacle xdg-desktop-portal xdg-desktop-portal-kde wl-clipboard

cargo build --workspace --release
mkdir -p ~/.local/bin
ln -sfn "$PWD/target/release/seatgeistd" ~/.local/bin/seatgeistd
ln -sfn "$PWD/target/release/seatgeist-cli" ~/.local/bin/seatgeist-cli
ln -sfn "$PWD/target/release/seatgeist-mcp" ~/.local/bin/seatgeist-mcp
```

Install the socket-activated user service and the baseline KWin bridge:

```bash
mkdir -p ~/.config/systemd/user
cp systemd/seatgeistd.service systemd/seatgeistd.socket ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now seatgeistd.socket

make install-kwin-script
scripts/install-kwin-screenshot-authorization.py
seatgeist-cli doctor
seatgeist-cli kwin-bridge-status
```

Install the local Codex plugin:

```bash
codex plugin marketplace add .
make refresh-local-codex-plugin
```

Start a new Codex session after installing or refreshing the plugin.

For physical-activity tracking:

```bash
make install-kwin-activity-user
```

For the experimental independent input lane:

```bash
make install-kwin-agent-seat-user
```

Then select it in `~/.config/seatgeist/config.toml`:

```toml
[backends]
input = "kwin_agent_seat"
```

Log out and back in normally after installing either KWin binary plugin. Do
not restart `plasma-kwin_wayland.service` in place: on a normal DRM-backed
Plasma session it may be unable to reacquire the display and SDDM will need to
create a new login session.

The complete walkthrough, including configuration, daemon deployment, portal
diagnostics, optional uinput, and rollback commands, is in the
[Arch KDE install guide](docs/arch-kde-install.md).

## Current rough edges

- KDE Plasma 6 Wayland is the environment being developed and tested.
- There is no polished distro package or one-command installer yet.
- KWin private binary interfaces can change between Plasma updates.
- Native Wayland, XWayland, accessibility, and portal paths do not all offer
  the same features.
- The agent-seat work is still experimental, especially with applications that
  do not expect multiple seats.
- Some GUI checks require an active desktop session and deliberate operator
  interaction.

If something does not work, these commands usually show which piece is
missing:

```bash
seatgeist-cli doctor
seatgeist-cli readiness
seatgeist-cli kwin-bridge-status
seatgeist-cli capture-backends
seatgeist-cli input status
seatgeist-cli journal tail --limit 20
```

## Development

Run the normal workspace checks with:

```bash
cargo fmt --all
cargo test --workspace
cargo check --workspace
```

The larger repository validation is:

```bash
make verify
```

`make verify` does not install KDE assets or send live desktop input. Live GUI
evaluations and install targets are separate.

Useful longer references:

- [Architecture](docs/architecture.md)
- [Configuration](docs/config.md)
- [Backends](docs/backends.md)
- [Agent seat](docs/agent-seat.md)
- [Human-input activity](docs/human-input-activity.md)
- [Codex plugin](docs/plugin.md)

## License

Seatgeist is available under the Apache License 2.0 or the MIT License, at your
option. See [LICENSE-APACHE](LICENSE-APACHE) and
[LICENSE-MIT](LICENSE-MIT).
