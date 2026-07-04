# Backends

Preferred KDE Plasma 6 Wayland order:

1. Semantic AT-SPI actions when an accessible node is available.
2. KWin metadata through DBus or KWin scripting for window state, focus, scaling, and geometry.
3. xdg-desktop-portal ScreenCast/Screenshot and RemoteDesktop for supported consented capture/control flows.
4. libei where the compositor exposes a suitable emulated-input server path.
5. Controlled uinput virtual devices for privileged local fallback.
6. Custom KWin plugin, KDE patch, or kernel module only after a measured gap remains.

Every backend must report capabilities and provenance. The daemon should refuse ambiguous fallback behavior.

Current KWin focus implementation uses `org.kde.krunner1.Run` on KWin's `WindowsRunner` service with a window id previously discovered from `WindowsRunner.Match`. This is kept behind the window backend boundary and policy-gated as a control action. A future KWin script/plugin focus path remains a fallback if `WindowsRunner` proves unstable across Plasma versions.

## KWin Script Bridge

The repository includes `kwin/plasma-pilot-bridge`, a packaged KWin script that publishes active-window metadata to the user-session daemon over the session bus:

- DBus service: `org.plasmapilot.KWinBridge`
- DBus path: `/org/plasmapilot/KWinBridge1`
- DBus interface: `org.plasmapilot.KWinBridge1`
- Method: `UpdateActiveWindow(payload: string)`

The payload is compact JSON containing active state, stable KWin window id, title, app id, pid, and logical window geometry. The daemon keeps the latest update in memory and serves it through `plasma-pilot-cli active-window`.

Install or update the script explicitly with:

```bash
make install-kwin-script
```

Do not make this target part of normal verification because it mutates the user's KWin configuration.
