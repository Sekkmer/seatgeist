# Experimental KWin Agent Seat

The `kwin_agent_seat` input backend is a first native-Wayland vertical slice
for using Seatgeist while the operator works in another window. It creates a
second KWin `wl_seat`, routes input directly to the exact window pinned by a
retained capture session, and never calls KWin activation, raise, or stacking
APIs. The operator's focused window and normal pointer are therefore outside
the agent lane.

This backend is opt-in and fails closed. Policy, panic-stop, app allow/deny,
capture-session ownership, rate limiting, and journaling remain in
`seatgeistd`. The binary KWin plugin has no input-enqueue D-Bus method. It only
pulls actions which the daemon has already authorized and correlates completion
with the daemon-generated action id.

## Build and install

The plugin ABI is tied to the installed KWin version. Rebuild it after every
KWin update:

```bash
make check-kwin-agent-seat-plugin
make install-kwin-agent-seat-user
```

The user installer does not restart the active compositor. Restart the normal
Plasma session, then explicitly select the backend:

```toml
[backends]
input = "kwin_agent_seat"
```

Check `seatgeist.input_backend_status`; `implemented_available_backend` must be
`kwin_agent_seat`. If the plugin is absent, disabled, or ABI-incompatible, the
daemon refuses the action. To revert:

```bash
make uninstall-kwin-agent-seat-user
```

The supported fallback is the nested KWin plus portal/libei lane. Seatgeist
does not fall back from an explicitly selected agent seat to shared uinput.

## Current contract

- Every raw action requires `session_id` from an exact retained window capture.
- The target is re-resolved by KWin window UUID, app id, and PID before each
  action.
- Pointer move, click, and drag require `window_local` coordinates. Scroll uses
  the current target's last agent pointer position, or the window center after
  startup or a target switch.
- Keyboard combos and `type_text` are delivered through the second seat.
  `type_text` currently supports characters present on a fixed US PC-105
  keymap; unmappable Unicode fails before delivery.
- The second seat advertises the primary KDE seat's keyboard repeat rate and
  delay when the plugin starts. This is required because clients such as
  XWayland bind every advertised seat and may otherwise leave their shared
  master keyboard repeat state incomplete. If the primary seat is unexpectedly
  unavailable during plugin construction, the plugin advertises KDE's standard
  fallback of 25 characters per second after a 600 ms delay.
- The plugin supports native Wayland client windows only. XWayland, internal
  KWin windows, clipboard, drag-and-drop, input methods, and popup/grab routing
  are not yet supported.
- This slice exposes one agent seat and does not route `page_zoom`; multiple
  concurrent agent seats and lane lifecycle controls remain follow-up work.
- Agent actions are serialized by the plugin queue. Human input remains
  independent unless the operator and agent use the same target window (or an
  application internally shares focus state across several windows).
- Exact KWin window capture remains independent of occlusion, so observing the
  pinned lower-layer window does not require activation or restacking.

KWin's private binary plugin ABI and public-in-this-build server interfaces are
version-sensitive. The implementation uses `SeatInterface`, `Window::surface`,
and `Window::inputTransformation`. If those interfaces change or the target
does not bind the dynamically announced seat, the action fails or produces no
client event; use the nested compositor backend until the plugin is rebuilt or
adapted. No X11 emulation fallback is attempted.

## Security and journal boundary

The daemon owns `org.seatgeist.KWinBridge` and queues only requests which have
passed the ordinary control policy. The plugin calls
`TakePendingAgentSeatAction` and returns `CompleteAgentSeatAction`; another
local process cannot enqueue an input action through the plugin. Registration
is accepted only from the unique D-Bus connection which currently owns
`org.kde.KWin`; subsequent pull and completion calls must use that same
connection, preventing another session process from stealing queued key or
pointer data. Successful delivery adds an `agent_seat_delivery` journal record
containing the session, action id, window id, app id, PID, and backend, but no
text, title, coordinates, or keycodes. The normal request journal record
remains the authoritative policy result.

The plugin watches the bridge's well-known-name owner and stops its timer while
the daemon is absent, so daemon downtime cannot create an ownerless D-Bus call
storm. While connected it polls at 50 ms, refreshes a one-second readiness
heartbeat, and resumes automatically when the daemon returns. The daemon's
default post-action confirmation for this exact-target lane is the compositor
completion (`delivery_ack`); an explicit settle condition still requests
additional observation-based verification.
