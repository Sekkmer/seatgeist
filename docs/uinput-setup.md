# Uinput Setup

Seatgeist can use Linux uinput as a privileged local fallback for keyboard and pointer control. Prefer portal or libei backends when they are available and consented; uinput is the fallback that makes local KDE Wayland control possible when supported desktop mediation is not enough.

## Check Status

Start the daemon normally, then run:

```bash
seatgeist-cli input uinput-status
```

The same diagnostic is available to MCP as `seatgeist.uinput_status`. It reports whether the daemon can open `/dev/uinput` read/write, whether the path exists and is a character device, file mode and owner ids when available, daemon effective uid/gid, and a short setup hint.

To compare the supported input paths before relying on uinput, run:

```bash
seatgeist-cli input status
```

The same aggregate probe is available to MCP as `seatgeist.input_backend_status`. It checks xdg-desktop-portal RemoteDesktop interface visibility, KDE portal service visibility, libei client metadata/socket hints, and uinput fallback availability without starting a portal consent flow or sending input.

To intentionally test the RemoteDesktop consent path, use `seatgeist-cli input remote-desktop-probe` or MCP `seatgeist.remote_desktop_session_probe` with a method approval and active-window guard. This may show a portal dialog, reports selected devices, closes the transient session, and still does not send input. If you need to test the EIS handoff too, use `seatgeist-cli input remote-desktop-eis-probe` or MCP `seatgeist.remote_desktop_eis_probe`; it calls `ConnectToEIS`, reports compact libei runtime connected/event/bound-capability/resumed-device state, immediately closes the returned FD, and sends no input. The retained EIS lifecycle is exposed separately as `seatgeist-cli input remote-desktop-eis-start`, `seatgeist-cli input remote-desktop-eis-session-status`, and `seatgeist-cli input remote-desktop-eis-stop`; explicit `portal_remote_desktop` or `libei` raw-input backends route through that stored session only after policy, active-window, and readiness gates pass. Live retained-session input is opt-in through `make gui-eval-remote-desktop-eis-session`.

Before any real pointer action, check monitor-derived physical pointer bounds:

```bash
seatgeist-cli input pointer-calibration
```

The same diagnostic is available to MCP as `seatgeist.pointer_calibration`. It reports the physical-pixel desktop bounds, per-monitor physical origins, and representative top-left, center, and bottom-right sample points. This is a preflight diagnostic only; it does not move the pointer.

## Optional Udev Rule

The repository includes `udev/99-seatgeist-uinput.rules`:

```udev
KERNEL=="uinput", GROUP="uinput", MODE="0660", OPTIONS+="static_node=uinput"
```

Install it only on a machine where the local operator accepts that members of the configured group can create virtual input devices:

```bash
sudo groupadd --system uinput || true
sudo install -m 0644 udev/99-seatgeist-uinput.rules /etc/udev/rules.d/99-seatgeist-uinput.rules
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=misc --sysname-match=uinput
sudo usermod -aG uinput "$USER"
```

Log out and back in, or restart the user service after group membership changes. Then run `seatgeist-cli input uinput-status` again.

## Systemd User Service

The user service skeleton runs `seatgeistd` without extra privileges:

```bash
mkdir -p ~/.config/systemd/user
cp systemd/seatgeistd.service systemd/seatgeistd.socket ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now seatgeistd.socket
```

Keep the service user-scoped. Do not run the daemon as root just to access uinput; use a narrow udev/group rule or a future portal/libei backend instead.

## Polkit State

`polkit/org.seatgeist.policy` is a placeholder for future privilege-brokered setup flows. The current daemon does not use polkit to elevate itself. This is intentional: input actions must still flow through the daemon policy engine, active-window guards when supplied, panic-stop, and the journal.

## Smoke Check

The safe smoke target checks status only and does not move the pointer or type:

```bash
make smoke-uinput-status
```

On a KDE session with monitor metadata available, the pointer calibration smoke checks coordinate metadata without moving the pointer:

```bash
make smoke-pointer-calibration
```

Actual click/type GUI smoke should be run only in a disposable test window with short-lived method-scoped approval grants and a known active-window guard.

The opt-in host GUI smoke does exactly that. It starts a private daemon with an approval file, grants only the focus, click, type, and save methods it uses, opens a disposable KWrite/Kate file, focuses the matching KWin window, validates the active-window guard, maps a safe point inside the window through pointer calibration, clicks, types a sentinel, saves the file, verifies the saved content, and confirms journal entries:

```bash
make smoke-gui-input
```

This target sends real pointer and keyboard input. Run it only from the intended KDE session.
