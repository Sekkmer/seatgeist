# Arming a launch for an already-admitted child

`seatgeist-cli launch-intent` supports a short pre-exec handshake for an existing
Native/resource-contained GUI child. It does not launch a process, execute a
desktop entry, reparent a child, transfer cgroup custody, or authorize capture.
Ordinary `launch` and its defaults are unchanged. No new MCP tool is required.

The same living controller must directly parent both the held child and every
CLI invocation below. Run these calls from that controller with `subprocess.run`,
not from separate interactive shells. The daemon derives the controller from the
trusted CLI peer's kernel parent; generic CLI tool scope is insufficient. It pins
controller and child pidfds, UID and `/proc/<pid>/stat` field22 start time. Exec of
the same child is allowed; replacement/forked GUI PIDs are refused.

After normal Source/resource/GPU admission, keep the same child at the maintained
bounded pre-exec gate. Read its start ticks and a fresh active-window UUID, then:

```sh
seatgeist-cli launch-intent arm \
  --child-pid "$child_pid" --child-start-ticks "$child_start_ticks" \
  --desktop-entry owned-viewer-entry --app-id eyrivo-viewer \
  --expected-active-window "$previous_window_uuid" \
  --anchor top-left --timeout-ms 30000
```

The desktop-entry ID and actual expected Wayland app ID are explicit identity
labels, both checked against app policy. No desktop entry is executed by this
operation. The compositor must match that app ID and the exact child PID on a
new window; existing windows are never adopted. The daemon also refuses arming
when the fresh inventory already contains a window for that child. Capture the
returned `data.launch_id`, controller/child generations and `expires_unix_ms`.
Release the same child's gate only after `data.state` is `armed`; an error is not
permission to release or retry. The original Source guard and execution wrapper
remain responsible for the exact executable. The launch intent cannot replace them.

Immediately after releasing the gate, from the same controller:

```sh
seatgeist-cli launch-intent wait --launch-id "$launch_id"
```

Success reports `data.state=confirmed`, the exact window and
`focus_preserved=true`. The timeout is at most30seconds **from arm**, including
bridge acknowledgement and settling; waiting later does not extend it. On an
abandoned gate or an interrupted controller workflow, cancel through:

```sh
seatgeist-cli launch-intent cancel --launch-id "$launch_id"
```

Wait/cancel are observation/cleanup operations and remain available under
panic-stop; arming and every pending placement require ControlSemantic authority.
All requests are journaled. Cancellation, expiry, owner/child exit or generation
change, panic-stop or expired approval fence pending callbacks before further
placement/focus effects. The script checks the original active window at arm time
and refuses if another user window becomes active during the pending launch.
The existing launch-intent focus preservation is applied only to the exact new
candidate; no separate physical refocus/input operation is introduced.

The daemon validates the live process generation and recorded app/window identity
through authenticated KWin callbacks before the initial effect, reapplication and
confirmation. A numeric PID, cached ACK, matching app name alone, or a completed
old ticket is insufficient. This requires the matching script's authenticated
`launch_window_arm_v1` registration; older scripts refuse before the gate can be
released. Tickets are daemon-local and cannot survive a daemon restart. The
permanent native-seat plugin and existing MCP connector need no replacement.

This handshake must follow admission, so queuing for GPU/CPU capacity cannot
consume the30-second compositor window. Keep the handshake automated and bounded;
do not hold an admission reservation across human input. The final GUI consumer
still proves application behavior and independent physical focus.
