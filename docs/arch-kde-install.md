# Arch Linux KDE Install

This is the operator runbook for a local Arch Linux + KDE Plasma 6 + Wayland workstation. It keeps Seatgeist user-scoped by default, separates diagnostics from control, and treats KWin script installation plus uinput access as explicit opt-in steps.

Package references were checked on 2026-07-04 against Arch's official package pages for `plasma-meta`, `plasma-workspace`, `kde-cli-tools`, `spectacle`, `xdg-desktop-portal`, `xdg-desktop-portal-kde`, and `wl-clipboard`.

## Packages

Install or verify the host packages Seatgeist relies on:

```bash
sudo pacman -S --needed base-devel cmake rust cargo jq plasma-meta plasma-workspace kde-cli-tools spectacle xdg-desktop-portal xdg-desktop-portal-kde wl-clipboard
```

If you manage Rust with `rustup`, keep using that toolchain instead of Arch's `rust` and `cargo` packages. Seatgeist currently targets Rust 2024 crates in a Cargo resolver 3 workspace.

The package roles are:

- `plasma-meta` / `plasma-workspace`: KDE Plasma session and KWin.
- `cmake`: builds the optional version-matched KWin binary plugins.
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

For a user-service install matching `systemd/seatgeistd.service`, expose the
release binaries under `~/.local/bin`. Development checkouts can use symlinks
so rebuilding `target/release` updates the CLI, MCP server, and next daemon
restart together:

```bash
cargo build --workspace --release
mkdir -p ~/.local/bin
ln -sfn "$PWD/target/release/seatgeistd" ~/.local/bin/seatgeistd
ln -sfn "$PWD/target/release/seatgeist-cli" ~/.local/bin/seatgeist-cli
ln -sfn "$PWD/target/release/seatgeist-mcp" ~/.local/bin/seatgeist-mcp
```

For a fixed release installation, copy the three release binaries into the
same directory instead of linking them to a development checkout.

Verify the user session can find them:

```bash
command -v seatgeistd
command -v seatgeist-cli
command -v seatgeist-mcp
```

## Config

Create a conservative config first:

```bash
mkdir -p ~/.config/seatgeist
cat > ~/.config/seatgeist/config.toml <<'EOF'
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
cp systemd/seatgeistd.service systemd/seatgeistd.socket ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now seatgeistd.socket
```

Check the daemon through the CLI:

```bash
seatgeist-cli doctor
seatgeist-cli capabilities
seatgeist-cli policy-status
```

The socket unit uses mode `0600` and directory mode `0700`. Keep the daemon running as the desktop user. Do not run it as root for ordinary operation.
The service limits starts to five per five minutes and uses stepped restart
backoff from 2 to 30 seconds, so a persistent startup fault cannot create an
unbounded one-second failure loop.

### Safe daemon update

From a development checkout, use the explicit deployment target instead of
copying and restarting the daemon by hand:

```bash
make deploy-user-daemon
```

The deployment build embeds source/build provenance, atomically replaces the
user binary, restarts only `seatgeistd.service`, and verifies that health
reports protocol version `1`, a fresh run id, a config fingerprint, and the
same executable SHA-256 as the deployed file. It does not restart KWin or the
Plasma session.

The deployment helper builds the current CLI and release daemon, refuses to
restart while a retained capture or RemoteDesktop EIS session is active,
installs a user-local KDE desktop authorization for the daemon's restricted
`org.kde.KWin.ScreenShot2` exact-window capture interface, atomically installs
the daemon at `~/.local/bin/seatgeistd`, and restarts only
`seatgeistd.service`. It waits for a successful daemon request rather than
socket existence, separately waits for the KWin bridge heartbeat, verifies
that release, installed, and running executable hashes match, and rechecks
retained-session idleness. It does not capture pixels, open a portal session,
focus a window, or send input.

## Panic-Stop Shortcut

Install the panic-stop helper somewhere KDE's global shortcut runner can execute:

```bash
install -Dm755 scripts/seatgeist-panic-stop-hotkey ~/.local/bin/seatgeist-panic-stop-hotkey
```

Bind `~/.local/bin/seatgeist-panic-stop-hotkey` to a KDE custom shortcut if you want a keyboard emergency stop. With no arguments it runs `seatgeist-cli panic-stop enable`, so the request still goes through the daemon and is journaled. If KDE's shortcut environment cannot find the CLI, set `SEATGEIST_CLI=/home/$USER/.cargo/bin/seatgeist-cli` in the shortcut command or wrap it in a small shell command.

Verify the binding target manually before assigning a shortcut:

```bash
~/.local/bin/seatgeist-panic-stop-hotkey
seatgeist-cli panic-stop status
seatgeist-cli panic-stop disable
```

## KDE Bridge

The KWin bridge is an explicit KDE configuration mutation. Install it only from the target KDE session:

```bash
make install-kwin-script
seatgeist-cli kwin-bridge-status
```

The install target updates the user-local package and enabled flag, then
reloads only the Seatgeist script when the KWin scripting service is live. If
it reports that loading was deferred, log into the target KDE session and run
the target again, or let the enabled script load at the next session start.

Before the script publishes its first active-window update, active-window reads can report the documented bridge-not-yet-reporting state. The script republishes its current snapshot every two seconds, so a daemon-only restart should recover without focusing a window. If it remains empty, update/reload the installed script with `make install-kwin-script`, then re-check status.

### KDE Connect recovery after KWin restarts

KDE Connect receives remote mouse and keyboard packets through a
portal-mediated RemoteDesktop/EIS session owned by KWin. A `kdeconnectd`
process that survives a compositor restart can retain a dead session and stop
processing device traffic. Install the user-scoped recovery integration with:

```bash
make install-kdeconnect-kwin-recovery-user
```

The installer routes KDE Connect D-Bus activation into its generated,
systemd-managed autostart unit, orders that unit after KWin and the KDE portal,
and installs an asynchronous KWin `ExecStartPost` hook. After a future KWin
start, the hook restarts only an already-active KDE Connect service and asks it
to refresh network links. Initial login does not start KDE Connect early, and
the installer never restarts KWin.

Installation restarts the existing `kdeconnectd` once so it moves out of an
unmanaged transient D-Bus scope. Existing pairing and the KDE portal's stored
remote-desktop permission are preserved. Remove the integration with:

```bash
make uninstall-kdeconnect-kwin-recovery-user
```

## Backend Diagnostics

Run safe read-only diagnostics before enabling control:

```bash
seatgeist-cli capture-backends
seatgeist-cli input backends
seatgeist-cli input status
seatgeist-cli input pointer-calibration
seatgeist-cli atspi tree --focused
```

The matching safe smoke targets are:

```bash
make smoke
make validate-install-assets
make validate-traces
make smoke-trace-replay
make smoke-mcp
make smoke-capture-backends
make smoke-uinput-status
make smoke-pointer-calibration
```

When `--output` is omitted for direct CLI `screenshot`, `screenshot-tile`, or `wait-for-change` commands, Seatgeist writes a timestamped PNG under `$XDG_RUNTIME_DIR/seatgeist/screenshots/`, using the same `/run/user/<uid>` fallback as the daemon socket defaults if `XDG_RUNTIME_DIR` is missing.

`make smoke-monitors`, `make smoke-windows`, `make smoke-clipboard`, and `make smoke-atspi` require a real KDE user session and may observe session state. `make gui-eval-portal-screenshot` validates live portal Screenshot capture when the portal interface is visible, first through the default noninteractive request and then through `--portal-interactive` if the portal cancels; it may show a desktop consent dialog, and `SEATGEIST_PORTAL_SCREENSHOT_STRICT=1` makes cancellation fail instead of skip. `make gui-eval-remote-desktop-probe` validates the live RemoteDesktop consent path when the interface and active-window guard metadata are visible; set `SEATGEIST_REMOTE_DESKTOP_STRICT=1` to require a started session instead of accepting a cancelled/ended probe. `make gui-eval-remote-desktop-eis-session` validates the retained RemoteDesktop EIS session lifecycle and minimal explicit-backend input attempts; it may show a portal dialog and can send one minimal scroll plus one minimal `Shift` key-combo only after method approval, an active-window guard, and EIS readiness checks pass. Set `SEATGEIST_REMOTE_DESKTOP_EIS_STRICT=1` to require the stored session to start, and set `SEATGEIST_REMOTE_DESKTOP_EIS_INPUT_STRICT=1` to require both minimal input attempts to succeed. `seatgeist-cli input remote-desktop-probe` is an explicit policy-gated RemoteDesktop consent-path probe that may show a portal dialog and closes the transient session without sending input. `seatgeist-cli input remote-desktop-eis-probe` uses the same consent path, calls `ConnectToEIS`, reports compact libei runtime state, immediately closes the returned FD, and still sends no input. `make smoke-gui-input` sends real keyboard and pointer input into a disposable KWrite/Kate document and should only be run intentionally. `make gui-eval-kcalc-visual` sends real keyboard input into KCalc through method-scoped approvals and writes a KCalc active-window screenshot artifact when Spectacle is available. `make gui-eval-firefox-localhost-button` launches Firefox with a disposable profile against a temporary localhost page, clicks a guarded window-local test button, verifies the local server received the click, and writes a Firefox active-window screenshot artifact when Spectacle is available.

For release-blocking live evidence, use `SEATGEIST_RELEASE_LIVE_EVALS_APPROVED=1 make release-live-evals`. It refuses to run without the explicit environment variable, enables strict portal/RemoteDesktop modes, runs the KWrite/Kate, KCalc, Firefox localhost, portal Screenshot, RemoteDesktop probe, and retained EIS session evals, then checks that `make release-readiness` reports `live_eval_evidence` as complete.

## Portal Screenshot v3 Targets

Use the read-only v3 diagnostic before assuming an Arch/KDE package upgrade will make `--portal-target` work:

```bash
make portal-screenshot-v3-status
seatgeist-cli capture-backends
```

The diagnostic does not install packages, run `pacman -Syu`, call `aur-step`, restart services, request portal consent, or capture pixels. It reports the user-session `org.freedesktop.portal.Screenshot` `version`, the optional `AvailableTargets` bitmask, installed Arch package versions for the KDE/portal screenshot path, visible pending upgrades from the local pacman sync database, and whether `aur-step` is present.

The upstream xdg-desktop-portal frontend and backend documentation describes the Screenshot interface version 3 and states that `AvailableTargets` plus the `target` option were added in version 3. Seatgeist therefore treats missing `AvailableTargets` or a reported Screenshot version below 3 as a hard reason to reject `--portal-target` requests instead of silently falling back to a different capture shape. If the diagnostic still reports Screenshot v2 after normal system updates and a user-session restart, prefer Seatgeist's bounded full-screen/tile capture path or a future documented KWin-native fallback rather than weakening the policy check.

## Optional Uinput

Use uinput only when the local operator accepts a privileged virtual-input fallback. Install the packaged udev rule and add the user to the narrow `uinput` group as documented in `docs/uinput-setup.md`, then log out and back in before retrying:

```bash
seatgeist-cli input status
seatgeist-cli input backends
```

All keyboard and pointer actions still flow through daemon policy, panic-stop, active-window guards when supplied, and the journal.

## Codex Plugin

Validate the plugin bundle:

```bash
make validate-plugin
make check-local-codex-install
```

Install or load the repository `plugin/` directory through the Codex plugin workflow for the local Codex version. The plugin expects `seatgeist-mcp` on `PATH` and uses the daemon socket from `SEATGEIST_SOCKET` or the built-in default.

`make check-local-codex-install` verifies the real local Codex config after install or checkout renames. It checks that the `seatgeist-local` marketplace source points at this checkout, the installed plugin cache validates, and the `seatgeist-*` launchers resolve without stale old-checkout symlinks.

After Codex sees the plugin, review plugin hooks through Codex's normal hook trust flow before expecting the bundled Stop audit hook to run.

## Approval Flow

Prefer method-scoped, short-lived approval grants:

```bash
seatgeist-cli approve --safety-class control-semantic --method focus_window --ttl-ms 60000
seatgeist-cli approve --safety-class control-semantic --method move_window --ttl-ms 60000
seatgeist-cli approve --safety-class control-semantic --method launch_window --ttl-ms 60000
seatgeist-cli approve --safety-class control-semantic --method resize_window --ttl-ms 60000
seatgeist-cli approve --safety-class control-keyboard --method page_zoom --ttl-ms 60000
```

Control actions should include active-window guards when possible. Full-resolution screenshots, clipboard reads, destructive actions, and secret-looking text fields remain separately gated.

## Troubleshooting

Use journal filters to distinguish policy denials from backend failures:

```bash
seatgeist-cli journal tail --limit 20
seatgeist-cli journal tail --method focus_window --ok false
```

Health and journal output include run/build correlation. Structured errors
also include `reason_code`; for example, a KeePassXC target is
`kind=app_denied reason=protected_application`, which is a terminal policy
decision rather than a signal to try another backend.

If capture fails, check `seatgeist-cli capture-backends` first. If input fails, check `seatgeist-cli input backends` and `seatgeist-cli input status` before changing udev, groups, or services.
