# Uinput Setup

PlasmaPilot can use Linux uinput as a privileged local fallback for keyboard and pointer control. Prefer portal or libei backends when they are available and consented; uinput is the fallback that makes local KDE Wayland control possible when supported desktop mediation is not enough.

## Check Status

Start the daemon normally, then run:

```bash
plasma-pilot-cli input status
```

The same diagnostic is available to MCP as `plasma.uinput_status`. It reports whether the daemon can open `/dev/uinput` read/write, whether the path exists and is a character device, file mode and owner ids when available, daemon effective uid/gid, and a short setup hint.

To compare the supported input paths before relying on uinput, run:

```bash
plasma-pilot-cli input backends
```

The same aggregate probe is available to MCP as `plasma.input_backend_status`. It checks xdg-desktop-portal RemoteDesktop interface visibility, KDE portal service visibility, libei client metadata/socket hints, and uinput fallback availability without starting a portal consent flow or sending input.

Before any real pointer action, check monitor-derived physical pointer bounds:

```bash
plasma-pilot-cli input pointer-calibration
```

The same diagnostic is available to MCP as `plasma.pointer_calibration`. It reports the physical-pixel desktop bounds, per-monitor physical origins, and representative top-left, center, and bottom-right sample points. This is a preflight diagnostic only; it does not move the pointer.

## Optional Udev Rule

The repository includes `udev/99-plasma-pilot-uinput.rules`:

```udev
KERNEL=="uinput", GROUP="uinput", MODE="0660", OPTIONS+="static_node=uinput"
```

Install it only on a machine where the local operator accepts that members of the configured group can create virtual input devices:

```bash
sudo groupadd --system uinput || true
sudo install -m 0644 udev/99-plasma-pilot-uinput.rules /etc/udev/rules.d/99-plasma-pilot-uinput.rules
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=misc --sysname-match=uinput
sudo usermod -aG uinput "$USER"
```

Log out and back in, or restart the user service after group membership changes. Then run `plasma-pilot-cli input status` again.

## Systemd User Service

The user service skeleton runs `plasma-pilotd` without extra privileges:

```bash
mkdir -p ~/.config/systemd/user
cp systemd/plasma-pilotd.service systemd/plasma-pilotd.socket ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now plasma-pilotd.socket
```

Keep the service user-scoped. Do not run the daemon as root just to access uinput; use a narrow udev/group rule or a future portal/libei backend instead.

## Polkit State

`polkit/org.plasmapilot.policy` is a placeholder for future privilege-brokered setup flows. The current daemon does not use polkit to elevate itself. This is intentional: input actions must still flow through the daemon policy engine, active-window guards when supplied, panic-stop, and the journal.

## Smoke Check

The safe smoke target checks status only and does not move the pointer or type:

```bash
make smoke-uinput-status
```

On a KDE session with monitor metadata available, the pointer calibration smoke checks coordinate metadata without moving the pointer:

```bash
make smoke-pointer-calibration
```

Actual click/type GUI smoke should be run only in a disposable test window with explicit `--allow-control` and a known active-window guard.
