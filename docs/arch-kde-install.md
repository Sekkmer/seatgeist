# Arch Linux KDE Install

This is the operator runbook for a local Arch Linux + KDE Plasma 6 + Wayland workstation. It keeps PlasmaPilot user-scoped by default, separates diagnostics from control, and treats KWin script installation plus uinput access as explicit opt-in steps.

Package references were checked on 2026-07-04 against Arch's official package pages for `plasma-meta`, `plasma-workspace`, `kde-cli-tools`, `spectacle`, `xdg-desktop-portal`, `xdg-desktop-portal-kde`, and `wl-clipboard`.

## Packages

Install or verify the host packages PlasmaPilot relies on:

```bash
sudo pacman -S --needed base-devel rust cargo jq plasma-meta plasma-workspace kde-cli-tools spectacle xdg-desktop-portal xdg-desktop-portal-kde wl-clipboard
```

If you manage Rust with `rustup`, keep using that toolchain instead of Arch's `rust` and `cargo` packages. PlasmaPilot currently targets Rust 2024 crates in a Cargo resolver 3 workspace.

The package roles are:

- `plasma-meta` / `plasma-workspace`: KDE Plasma session and KWin.
- `kde-cli-tools`: `kpackagetool6`, `kwriteconfig6`, and related KDE command tools.
- `spectacle`: screenshot tile backend and compatibility fallback.
- `xdg-desktop-portal` / `xdg-desktop-portal-kde`: portal services for current consented full-screen Screenshot capture, capture/control diagnostics, and future RemoteDesktop control backends.
- `wl-clipboard`: current Wayland clipboard command backend.
- `jq`: smoke target JSON checks.

Package reference pages:

- `plasma-meta`: <https://archlinux.org/packages/extra/any/plasma-meta/>
- `plasma-workspace`: <https://archlinux.org/packages/extra/x86_64/plasma-workspace/>
- `kde-cli-tools`: <https://archlinux.org/packages/extra/x86_64/kde-cli-tools/>
- `spectacle`: <https://archlinux.org/packages/extra/x86_64/spectacle/>
- `xdg-desktop-portal`: <https://archlinux.org/packages/extra/x86_64/xdg-desktop-portal/>
- `xdg-desktop-portal-kde`: <https://archlinux.org/packages/extra/x86_64/xdg-desktop-portal-kde/>
- `wl-clipboard`: <https://archlinux.org/packages/extra/x86_64/wl-clipboard/>

## Build Binaries

From the repository root:

```bash
cargo build --workspace
```

For a user-service install matching `systemd/plasma-pilotd.service`, install the binaries into `~/.cargo/bin`:

```bash
cargo install --locked --path crates/plasma-pilotd
cargo install --locked --path crates/plasma-pilot-cli
cargo install --locked --path crates/plasma-pilot-mcp
```

Verify the user session can find them:

```bash
command -v plasma-pilotd
command -v plasma-pilot-cli
command -v plasma-pilot-mcp
```

## Config

Create a conservative config first:

```bash
mkdir -p ~/.config/plasma-pilot
cat > ~/.config/plasma-pilot/config.toml <<'EOF'
[policy]
default_observe = "allow"
default_control = "prompt"
destructive_actions = "prompt"
secret_fields = "deny"
default_clipboard_read = "prompt"
default_clipboard_write = "allow"
full_resolution_screenshot = "prompt"

[safety]
require_focus_guard = true
pause_on_human_input = false
EOF
```

See `docs/config.md` for approval-file grants, app allow/deny lists, redaction regions, and local override flags.

## User Service

Install and start the socket-activated user service:

```bash
mkdir -p ~/.config/systemd/user
cp systemd/plasma-pilotd.service systemd/plasma-pilotd.socket ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now plasma-pilotd.socket
```

Check the daemon through the CLI:

```bash
plasma-pilot-cli doctor
plasma-pilot-cli capabilities
plasma-pilot-cli policy-status
```

The socket unit uses mode `0600` and directory mode `0700`. Keep the daemon running as the desktop user. Do not run it as root for ordinary operation.

## KDE Bridge

The KWin bridge is an explicit KDE configuration mutation. Install it only from the target KDE session:

```bash
make install-kwin-script
plasma-pilot-cli kwin-bridge-status
```

Before the script publishes its first active-window update, active-window reads can report the documented bridge-not-yet-reporting state. Open or focus a normal application window, then re-check status.

## Backend Diagnostics

Run safe read-only diagnostics before enabling control:

```bash
plasma-pilot-cli capture-backends
plasma-pilot-cli input backends
plasma-pilot-cli input status
plasma-pilot-cli input pointer-calibration
plasma-pilot-cli atspi tree --focused
```

The matching safe smoke targets are:

```bash
make smoke
make validate-traces
make smoke-trace-replay
make smoke-mcp
make smoke-capture-backends
make smoke-uinput-status
make smoke-pointer-calibration
```

`make smoke-monitors`, `make smoke-windows`, `make smoke-clipboard`, and `make smoke-atspi` require a real KDE user session and may observe session state. `make gui-eval-portal-screenshot` validates live portal Screenshot capture when the portal interface is visible and may show a desktop consent dialog; set `PLASMA_PILOT_PORTAL_SCREENSHOT_STRICT=1` to fail instead of skip when the portal cancels. `make gui-eval-remote-desktop-probe` validates the live RemoteDesktop consent path when the interface and active-window guard metadata are visible; set `PLASMA_PILOT_REMOTE_DESKTOP_STRICT=1` to require a started session instead of accepting a cancelled/ended probe. `make gui-eval-remote-desktop-eis-session` validates the retained RemoteDesktop EIS session lifecycle and a minimal explicit-backend input attempt; it may show a portal dialog and can send one minimal scroll only after method approval, an active-window guard, and EIS readiness checks pass. Set `PLASMA_PILOT_REMOTE_DESKTOP_EIS_STRICT=1` to require the stored session to start, and set `PLASMA_PILOT_REMOTE_DESKTOP_EIS_INPUT_STRICT=1` to require the minimal scroll to succeed. `plasma-pilot-cli input remote-desktop-probe` is an explicit policy-gated RemoteDesktop consent-path probe that may show a portal dialog and closes the transient session without sending input. `plasma-pilot-cli input remote-desktop-eis-probe` uses the same consent path, calls `ConnectToEIS`, reports compact libei runtime state, immediately closes the returned FD, and still sends no input. `make smoke-gui-input` sends real keyboard and pointer input into a disposable KWrite/Kate document and should only be run intentionally.

## Optional Uinput

Use uinput only when the local operator accepts a privileged virtual-input fallback. Install the packaged udev rule and add the user to the narrow `uinput` group as documented in `docs/uinput-setup.md`, then log out and back in before retrying:

```bash
plasma-pilot-cli input status
plasma-pilot-cli input backends
```

All keyboard and pointer actions still flow through daemon policy, panic-stop, active-window guards when supplied, and the journal.

## Codex Plugin

Validate the plugin bundle:

```bash
make validate-plugin
```

Install or load the repository `plugin/` directory through the Codex plugin workflow for the local Codex version. The plugin expects `plasma-pilot-mcp` on `PATH` and uses the daemon socket from `PLASMA_PILOT_SOCKET` or the built-in default.

After Codex sees the plugin, review plugin hooks through Codex's normal hook trust flow before expecting the bundled Stop audit hook to run.

## Approval Flow

Prefer method-scoped, short-lived approval grants:

```bash
plasma-pilot-cli approve --safety-class control-semantic --method focus_window --ttl-ms 60000
```

Control actions should include active-window guards when possible. Full-resolution screenshots, clipboard reads, destructive actions, and secret-looking text fields remain separately gated.

## Troubleshooting

Use journal filters to distinguish policy denials from backend failures:

```bash
plasma-pilot-cli journal tail --limit 20
plasma-pilot-cli journal tail --method focus_window --ok false
```

If capture fails, check `plasma-pilot-cli capture-backends` first. If input fails, check `plasma-pilot-cli input backends` and `plasma-pilot-cli input status` before changing udev, groups, or services.
