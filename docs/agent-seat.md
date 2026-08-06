# Experimental KWin Agent Seat

The `kwin_agent_seat` input backend is a native-Wayland vertical slice
for using Seatgeist while the operator works in another window. It creates a
bounded pool of KWin `wl_seat` lanes, routes input directly to the exact window
pinned by a retained capture session, and never calls KWin activation, raise,
or stacking APIs. Each verified agent process reuses one opaque lane, including
its own pointer, keyboard focus, modifier state, and target history. The
operator's focused window and normal pointer are therefore outside every agent
lane.

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
- A window interaction lease is exclusive across agent owners. A second agent
  attempting to bind the same window receives `agent_target_in_use`; closing or
  expiring the first session releases the lease. The physical user is never
  locked out.
- One verified agent owner reuses one virtual seat across its retained windows.
  At most four owners may hold live agent lanes. The plugin creates seats
  lazily, retains at most four, and clears the least-recently-used idle lane
  before replacement.
- The target is re-resolved by KWin window UUID, app id, and PID before each
  action. The UUID is the window identity; app id and PID are validation
  attributes, not selectors. This is important for Firefox, where several
  windows commonly share one process.
- Pointer move, click, and drag accept logical `window_local` coordinates. For
  positions read from a retained preview, callers should use `capture_output`
  with the session id and exact frame revision; the daemon maps those pixels
  to `window_local` before queuing the authorized action. The KWin plugin treats
  the result as client-surface-local, excluding server-side decorations, and
  lets `inputTransformation()` apply output scaling. Scroll uses the current
  target's last agent pointer position, or the client-surface center after
  startup or a target switch.
- Keyboard combos and `type_text` are delivered through the owner's agent seat.
  `type_text` currently supports characters present on a fixed US PC-105
  keymap; unmappable Unicode fails before delivery. Same-window physical input
  still enforces the target quiet period and in-flight preemption, but stale
  screenshot metadata does not block keyboard input. Fresh-frame enforcement
  applies only to pointer coordinates derived from a retained preview.
- The lane never activates, raises, or restacks the target and must not acquire
  the physical user's workspace focus. MCP does not advertise `focus_window`,
  MCP launches always preserve physical focus, and the daemon rejects either
  kind of focus-changing MCP request before compositor control.
- Application or window-management shortcuts are not a window lifecycle API.
  Retained-seat close/quit combinations such as `Alt+F4`, `Ctrl+W`,
  `Ctrl+Shift+W`, and `Ctrl+Q` fail closed before delivery. Exact close instead
  requires the owned session and pinned KWin UUID, uses the KWin bridge's
  `closeWindow()` path, and waits until that UUID is absent. There is no
  keyboard fallback.
- Every agent seat advertises the primary KDE seat's keyboard repeat rate and
  delay when the plugin starts. This is required because clients such as
  XWayland bind every advertised seat and may otherwise leave their shared
  master keyboard repeat state incomplete. If the primary seat is unexpectedly
  unavailable during plugin construction, the plugin advertises KDE's standard
  fallback of 25 characters per second after a 600 ms delay.
- The plugin supports native Wayland client windows only. XWayland, internal
  KWin windows, clipboard, drag-and-drop, input methods, and popup/grab routing
  are not yet supported.
- The target-aware `kwin_input_spy_v2` activity plugin scopes physical input to
  the KWin window UUID. Input in another window does not pause an agent. Input
  in the same window cancels queued delivery and enforces a 350 ms quiet period
  before another action. It also invalidates retained preview metadata for that
  target, so preview-based input requires a fresh frame. If input races with in-flight delivery, the action
  returns `confirmation=user_preempted` and must not be replayed without a
  fresh frame.
- Small lane actions are pulled through one compositor queue, while agents keep
  independent seat state and may perform capture and post-action work in
  parallel. Human input remains independent unless the operator and agent use
  the same target window, or an application internally shares state across
  several windows.
- This slice does not route `page_zoom`; input methods and popup/grab routing
  remain follow-up work.
- Exact KWin window capture remains independent of occlusion, so observing the
  pinned lower-layer window does not require activation or restacking.

KWin's private binary plugin ABI and public-in-this-build server interfaces are
version-sensitive. The implementation uses `SeatInterface`, `Window::surface`,
and `Window::inputTransformation`. If those interfaces change or the target
does not bind the dynamically announced seat, the action fails or produces no
client event; use the nested compositor backend until the plugin is rebuilt or
adapted. No X11 emulation fallback is attempted.

The daemon accepts the legacy `kwin_input_spy_v1` contract so a daemon-only
restart does not disable the currently loaded plugin. In that compatibility
mode global activity provenance remains trusted, but target-local preemption
reports `unavailable`. The fallback is exclusive window leases plus the legacy
serialized seat until the updated plugins load at the next normal Plasma
session start.

## Security and journal boundary

The daemon owns `org.seatgeist.KWinBridge` and queues only requests which have
passed the ordinary control policy. The plugin calls
`TakePendingAgentSeatAction` and returns `CompleteAgentSeatAction`; another
local process cannot enqueue an input action through the plugin. Registration
is accepted only from the unique D-Bus connection which currently owns
`org.kde.KWin`; subsequent pull and completion calls must use that same
connection, preventing another session process from stealing queued key or
pointer data. Successful delivery adds an `agent_seat_delivery` journal record
containing the session, opaque lane id, action id, window id, app id, PID, and
backend, but no text, title, coordinates, or keycodes. The normal request journal record
remains the authoritative policy result.

The plugin watches the bridge's well-known-name owner and stops its timer while
the daemon is absent, so daemon downtime cannot create an ownerless D-Bus call
storm. While connected it holds a daemon-side long poll, re-arms immediately
after a successful response, and retains a five-second timer only as an error
or stalled-call watchdog. The long-poll heartbeat keeps readiness fresh without
a 50 ms idle loop, and the plugin resumes automatically when the daemon
returns. The daemon's default post-action confirmation for this exact-target
lane is the compositor completion (`delivery_ack`). Delivery-only and explicit
`settle_condition=none` responses collect only the bounded window observation
and do not wait on AT-SPI. An explicit observation settle condition still
requests accessibility verification, with daemon-side AT-SPI deadlines.
`delivery_ack` proves that KWin dispatched the authorized seat events; it does
not prove which application command, if any, interpreted a shortcut. Callers
must use exact lifecycle tools for lifecycle effects and inspect the settled
target observation for ordinary input.
