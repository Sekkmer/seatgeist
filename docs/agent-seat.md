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

## Restartable planner, permanent seat host

The `kwin_agent_seat_v2` host is deliberately small: it owns native seat
lifetimes, real client bindings, target transforms, modifier mechanics, and
bounded event dispatch. Rust in `seatgeistd` plans chords, text, clicks, drags,
and scrolls into protocol-1 primitive event batches. The host validates the
entire batch before input: supported opcodes, bounded counts, balanced
key/button transitions, surface-local coordinates, and the original deadline.
There is no remotely loaded native code or live plugin unload.

Ordinary planner fixes now use `make deploy-seatgeistd-user`, after clients
close their retained sessions. This replaces/restarts only the daemon. On
daemon owner loss/replacement, the host cancels old queued work and clears
agent focus, but preserves every `wl_seat` global and client binding. The new
daemon's sessions get fresh lane IDs backed by the existing idle seats; clients
must reopen retained captures. A native host or KWin ABI change still needs a
normal logout/login. Activating this architecture from the old plugin needs
one such session restart; subsequent compatible planner updates do not.

For rolling deployment the daemon still negotiates `kwin_agent_seat_v1` and
sends its legacy high-level actions. Host v2 requires the matching new daemon;
an older daemon rejects registration rather than guessing a wire format.

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

Never unload a live agent-seat plugin to apply an update. Its `wl_seat`
globals outlive capture sessions, and removing them can disconnect clients
with fatal `wl_fixes` protocol errors. The reload helper refuses this operation;
save work and use a normal logout/login. Loading an absent, ABI-matched plugin
does not remove any existing seat globals.

Check `seatgeist.input_backend_status`; `implemented_available_backend` must be
`kwin_agent_seat`. If the plugin is absent, disabled, or ABI-incompatible, the
daemon refuses the action. To revert:

The nested `agent_seat` status reports the exact registered wire protocol,
source build fingerprint, heartbeat freshness/age, and the outcome of the most
recent real delivery. `heartbeat_only` means registration is live but this
daemon run has not yet observed an end-to-end action; `delivery_verified` and
`delivery_failed` distinguish actual client delivery from a heartbeat. The ABI
watcher compares the registered build fingerprint with the installed plugin
and reports `agent_seat_restart_required` when KWin is still executing an older
image. Legacy v1 hosts remain accepted, but cannot claim an exact build.

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
  At most four owners may hold live agent lanes. The plugin announces its four
  permanent seats at compositor startup, before applications connect. GTK 3
  processes deferred seat bindings only during display startup; creating seats
  on the first action leaves an already-running Firefox unable to bind them.
  A dequeue-time daemon roster permits reuse
  of released/expired lanes only; an idle-but-still-owned lane cannot be
  evicted by another agent. A fifth owner receives `agent_lane_quota`.
  Separate `seatgeist-mcp` processes (including independent Codex instances)
  have process-scoped ownership and cannot use, close, or claim each other's
  sessions. Manual `seatgeist-cli` invocations intentionally share a tool-scoped
  owner and are not a substitute for independent MCP connections. Close
  sessions when finished; abandoned sessions otherwise retain their normal TTL.
- The target is re-resolved by KWin window UUID, app id, and PID before each
  action. The UUID is the window identity; app id and PID are validation
  attributes, not selectors. This is important for Firefox, where several
  windows commonly share one process.
- Firefox uses a process-wide keyboard-focus target, so separate Wayland seats
  alone do not isolate its windows. Raw agent-seat input is refused if another
  normal window/dialog shares the Firefox target's Wayland client. Use a
  dedicated Firefox process/profile per agent (one top-level window), or exact
  semantic targets. This is checked again after the binding wait. The rule is a
  known-Firefox compatibility guard, not a claim that every other toolkit or
  Firefox-derived browser is multiseat-safe.
- Pointer move, click, and drag accept logical `window_local` coordinates. For
  positions read from a retained preview, callers should use `capture_output`
  with the session id and exact frame revision; the daemon maps those pixels
  to `window_local` before queuing the authorized action. The KWin plugin treats
  the result as client-surface-local, excluding server-side decorations, and
  lets `inputTransformation()` apply output scaling. Scroll uses the current
  target's last agent pointer position, or the client-surface center after
  startup or a target switch.
  Movement within the same surface updates its transform and sends a pointer
  motion event. Repeating a pointer-enter event alone is insufficient: KWin
  suppresses it for an already-focused surface, leaving the client's click
  position unchanged.
- Keyboard combos and `type_text` are delivered through the owner's agent seat.
  The plugin waits for the target client to bind the requested seat device
  before sending the first input event. Compositor keyboard focus alone does
  not mean a client has created its `wl_keyboard`: sending a key immediately
  after announcing a new seat can silently drop it. The non-blocking binding
  wait is capped at one second and fails closed on timeout, target/surface
  loss, bridge owner change, or same-target input through KWin's ordinary
  input pipeline. It never activates the physical workspace or sends a
  sacrificial warm-up key. Already-bound clients dispatch without this wait.
  Queued actions retain the daemon's original expiry time; the binding wait
  cannot extend a timed-out request into a later injection. Deploy the matching
  daemon with this plugin.
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
and `Window::inputTransformation`. Binding readiness uses public Wayland server
resource lifetime callbacks, `SeatInterface::get`, and `PointerInterface::get`.
Because KWin does not expose keyboard binding readiness, the plugin correlates
`wl_seat.get_keyboard` with the keymap event sent synchronously by KWin 6.7.4;
it retains only resource/client/seat identity, never keymap contents or input.
If that contract changes, or the client ignores the announced
seat, readiness times out without sending input. Rebuild/adapt the plugin for
the installed KWin ABI; no X11 emulation fallback is attempted.

`make check-kwin-agent-seat-plugin` builds and runs the isolated native Wayland
binding and event-batch tests. With the workspace binaries already built, run
`python scripts/agent-seat-first-input-eval.py` for the full
private-bus/headless-KWin Qt regression. It checks
first-key delivery, switching between two windows of the same Qt client,
simultaneous input from two independent MCP processes, cross-owner refusal,
released-lane churn, and a daemon-only restart. A live Wayland observer verifies
that native seat global IDs survive unchanged, with no global removal. Both
agent targets and the physical-focus sentinel stay alive throughout. It never
loads/reloads the live plugin or restarts the operator's compositor.

Append `--firefox` to exercise first-key and repeated concurrent input through
two real MCP processes into two disposable Firefox profiles, including modifier
isolation and a daemon-only restart with the browsers and MCP clients alive.
Append `--firefox-shared-client` to verify that two windows in one
Firefox process are refused before input, with the physical-focus sentinel
unchanged. Test profiles disable onboarding/splash UI explicitly; DOM load alone
does not prove the browser's modal startup overlay has disappeared. The native
Wayland unit test controls late device binding directly; stopping an already
connected Qt client no longer simulates that condition with startup seats.

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
or stalled-call watchdog. The first pull starts as soon as registration
succeeds, rather than waiting for that timer and timing out the first action.
The long-poll heartbeat keeps readiness fresh without
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
