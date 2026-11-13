# Finite gestures on permanent agent seats

The native seat host keeps the same Wayland seats alive for the login. The Rust
planner can construct new combinations without replacing that host. This contract
adds timing and client-state boundaries, rather than dynamically unloading code
inside KWin. Existing click, drag, relative-pointer and Alt-point-click contracts
are unchanged.

## Availability and authorization

`seatgeist.native_gesture` and `seatgeist.seat_state` require an owned, active exact
window session and a native Wayland target. There is no shared-seat, physical
focus, Xwayland, clipboard or accessibility fallback. Clients first check
`health.native_gesture_supported`; the daemon also requires the authenticated
host capability `native_gesture_v1`. The health bit describes the daemon contract,
not installed native readiness. Unsupported hosts refuse before queuing input.

Every gesture requires both ControlKeyboard and ControlPointer authorization,
app policy, panic-stop, session ownership and the existing interference checks.
The request and its delivery are journaled; step counts are retained rather than
key contents. Known close/quit chords remain forbidden in these sessions.
Observation requires Observe authorization and the same exact-session ownership;
it neither focuses devices nor claims successful input delivery.

## Request

MCP expert tool `seatgeist.native_gesture`:

```json
{"session_id":"<owned exact session>","steps":[
  {"op":"key","code":42,"pressed":true},
  {"op":"motion","x":50,"y":60},
  {"op":"button","button":"left","pressed":true},
  {"op":"wait","duration_ms":100},
  {"op":"motion","x":100,"y":90},
  {"op":"wait","duration_ms":100},
  {"op":"button","button":"left","pressed":false},
  {"op":"key","code":42,"pressed":false}
]}
```

CLI uses the same steps:
`seatgeist-cli input native-gesture --session-id <session> --steps-json '<array>'`.

The full sequence is validated before delivery. It must contain 1..256 steps,
include actual input, balance every key and button, and reserve at most 1500 ms
for all waits and condition timeouts combined. The native execution deadline is
2500 ms, additionally bounded by the daemon action's absolute expiry. Timings are
minimum waits, not real-time scheduling or proof the application processed input.
Slow scheduling or authorization may cause a deadline refusal. No held state or
handle survives a request; separate concurrent calls cannot build a held gesture.

Supported steps:

| Operation | Fields and meaning |
|---|---|
| `key` | Linux evdev `code` 1..767 and `pressed`; codes describe keys on the host's agent keymap, not Unicode characters |
| `button` | `left`, `right`, `middle`, `back` (BTN_SIDE), `forward` (BTN_EXTRA), and `pressed` |
| `motion` | Finite nonnegative window-client-local `x`, `y`; all points must fit the original client geometry |
| `relative_motion` | Finite nonzero `dx`, `dy`, each within ±4096 logical units; accelerated and unaccelerated deltas are identical |
| `scroll` | `axis` horizontal/vertical, finite `delta` within ±1000000, signed i32 `delta_v120`, `source` wheel/finger/continuous, optional `inverted` false |
| `wait` | Positive `duration_ms` |
| `await` | `condition` bindings_ready/pointer_locked/client_unlocked, positive `timeout_ms` |

Native scroll deltas increase down/right; this differs from the older detent API
which accepts positive up/left. Use zero delta and zero v120 with source finger
to terminate a finger scroll. An unfinished non-wheel scroll is terminated automatically on every exit. No application momentum is fabricated.

Relative gestures require an already focused exact agent lane, actual same-seat
client relative-pointer binding and an active lock at each relative event. They
do not acquire a lock, warp the pointer or fall back to absolute motion. Absolute
motion refuses an active lock rather than acknowledging a suppressed move.

`pointer_locked` observes the same seat/client's active lock; `client_unlocked`
requires destruction of that original observed lock by its client and absence of
a replacement lock. A compositor-only deactivation or elapsed delay is not an
unlock. A successful condition is checked again after asynchronous authorization
before the next step. Each input step obtains a fresh authenticated daemon check;
waits request revalidation every 50 ms while KWin's event loop runs. Pending revalidation blocks further input; policy grants and panic-stop are checked again.

Target surface, app/PID, geometry, transform and backend generation remain pinned.
Target loss/change, physical-user interference, cancellation, daemon loss, expired
lease or deadline terminates the sequence and releases held virtual input.
Cleanup never delivers releases into a replacement target. Delivery refusal after
a partial gesture is uncertain: observe the application; do not replay the whole
sequence automatically. A native delivery acknowledgement is not application
acceptance.

## Seat observation

`seatgeist.seat_state` takes only `session_id`. CLI:
`seatgeist-cli input seat-state --session-id <session>`.

The compact response distinguishes actual keyboard/pointer focus from client device
bindings, and includes the active same-seat lock, current transformed local pointer
position when pointer-focused, and cursor
metadata. Named cursors retain their reported name. Surface cursors are reported
as `surface`, hidden cursors as `hidden`, and an unobserved cursor as `unknown`.
A per-lane revision increases on observed cursor changes and lane reset. It is
not pixel capture, a cursor-shape inference, a full history or a timestamped app
hover confirmation. No physical user cursor is moved or read.

## Future input families and international text

These primitives cover paced selection, long press, multiple clicks, modifier
scrolling, finite movement with mouse-look, and camera drags. Their high-level
combinations belong in the daemon. No generic loops, code evaluation, arbitrary
callbacks or persistent held-key API are introduced.

International text remains a separate design item: current native typing lowers
characters to US PC105 key chords. Supporting Hungarian accented letters or other
scripts needs a negotiated per-lane keymap/text-input contract and actual client
support. Changing a keymap while keys are held must be forbidden. Text-input
commit/preedit is distinct from key injection, is not universally implemented by
clients, and must not silently substitute clipboard paste. Before implementation,
qualify real supported Qt/GTK/Chromium clients and preserve secret-field policy.
Reserve a separate capability/version for that extension rather than claiming
arbitrary Unicode support from the current key-event envelope.

Touchscreen contacts, touchpad pinch/swipe, tablet pressure/tilt and cross-window
MIME drag-and-drop are deferred. They require additional native protocol families,
client bindings and, for transfers, authorization of both endpoints. The current
single-target pointer path does not provide them. KWin ABI or host primitive
changes still require a normal saved-work logout/login; no live seat unload.
