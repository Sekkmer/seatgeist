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

Current clipboard text implementation uses the standard Wayland `wl-copy` and `wl-paste` commands when both are available. The daemon reports `clipboard_text` capability only in that case. Clipboard reads are policy-gated separately from clipboard writes, bounded to 64 KiB by default, and compact daemon/MCP summaries report only text length and truncation metadata. Future backend work should add portal or KDE-native clipboard integration and expose provenance/fallback diagnostics when the Wayland command backend is unavailable.

Current keyboard and pointer input implementation uses a localized Linux uinput backend in `plasma-pilot-uinput`. It creates short-lived virtual keyboard and pointer devices through `/dev/uinput` and uses `UI_DEV_SETUP` plus absolute-axis setup for pointer motion. Keyboard support covers US evdev key positions for ASCII text plus newline/tab and named key combos such as `Ctrl+L`, `Alt+F4`, and `Super+Space`. Pointer support covers absolute physical-pixel move, one-click or double-click with left/middle/right buttons, and vertical/horizontal wheel deltas. The daemon exposes keyboard through policy-gated `ControlKeyboard` requests and pointer through policy-gated `ControlPointer` requests; active panic-stop blocks both, and compact summaries report metadata rather than typed text. The daemon reports `keyboard_input` and `pointer_input` capabilities only when `/dev/uinput` can be opened read/write. Pointer coordinates currently require explicit `physical_pixel` space and are validated against monitor-derived physical desktop bounds, including scaled 8K layouts. Future backend work should add xdg-desktop-portal RemoteDesktop/libei probing, calibration diagnostics, and fallback provenance before expanding non-physical coordinate spaces.

Current AT-SPI implementation discovers the accessibility bus through `org.a11y.Bus.GetAddress`, then queries the separate accessibility bus with `busctl --address`. It walks application roots from `/org/a11y/atspi/accessible/root`, detects the focused node from the installed AT-SPI state bitset, and returns bounded compact role/name/value/state/bounds/action metadata. Text values use `org.a11y.atspi.Text.GetText` with a 512-character cap, scalar values use `org.a11y.atspi.Value.CurrentValue`, and password roles suppress value reads. The same traversal can find nodes by role, accessible-name substring, application name, and containing frame/dialog/window name. Policy-gated semantic invoke resolves a normalized action to the node's advertised `org.a11y.atspi.Action.GetActions` index and calls `DoAction(index)`. Policy-gated set-text requires a non-sensitive node exposing `org.a11y.atspi.EditableText`, replaces contents with `SetTextContents`, and caps replacement text at 8192 characters. High-level `click_button`, `set_text_field`, `select_menu`, and `activate_tab` use the same AT-SPI find/action path and refuse zero, sensitive, non-viable, or ambiguous matches. `select_menu` currently requires the menu path to be visible in AT-SPI; future work can add a multi-step open-then-select flow with the same ambiguity rules. This command-backed implementation is intentionally isolated inside `plasma-pilot-atspi`; a future native zbus/libatspi backend should preserve the same daemon protocol and add richer edit operations such as insert/delete/paste when needed.

## KWin Script Bridge

The repository includes `kwin/plasma-pilot-bridge`, a packaged KWin script that publishes active-window metadata to the user-session daemon over the session bus:

- DBus service: `org.plasmapilot.KWinBridge`
- DBus path: `/org/plasmapilot/KWinBridge1`
- DBus interface: `org.plasmapilot.KWinBridge1`
- Method: `UpdateActiveWindow(payload: string)`

The payload is compact JSON containing active state, stable KWin window id, title, app id, pid, and logical window geometry. The daemon keeps the latest update in memory and serves it through `plasma-pilot-cli active-window`.

`plasma-pilot-cli kwin-bridge-status` and MCP `plasma.kwin_bridge_status` report the daemon DBus receiver state, whether the script has published an update, and the user-local package/config paths checked for persistent installation.

Install or update the script explicitly with:

```bash
make install-kwin-script
```

Do not make this target part of normal verification because it mutates the user's KWin configuration.
