---
name: seatgeist-computer-use
description: Use KDE Plasma desktop computer-use tools through Seatgeist MCP when a task requires seeing, clicking, typing, controlling windows, reading screenshots, or operating GUI applications on Linux/KDE.
---

Use terminal commands, files, APIs, and structured integrations first when they solve the task directly.

When GUI state matters, use Seatgeist tools through MCP:

If the MCP server is running with `--tool-profile core`, use
`seatgeist.computer_status`, `seatgeist.window_session`, `seatgeist.snapshot`,
`seatgeist.act`, `seatgeist.wait`, and `seatgeist.panic_stop`. Open one
`window_session` when repeated visual work should stay on a user-approved
window. First call `window_session` with `operation=inventory`; reuse its
revision with `operation=wait_inventory` instead of polling. Pass a stable id
from that inventory as `requested_window_id`
so Seatgeist can authorize and capture that exact KWin window without opening a
portal chooser or ScreenCast stream. Confirm that status reports
`sticky_target_bound=true`, then pass the returned `session_id` to `snapshot`,
`wait`, and each raw `act`. This repeatedly captures only the compositor window
and lets the daemon reacquire the pinned target immediately before input; do
not add active-window guards or separate focus calls to a sticky raw action.
Core `snapshot` and `wait` require `session_id` and never fall back to the whole
desktop. Use the explicitly named expert screenshot tools only when a
whole-desktop compatibility capture is intended. Expert Screenshot v3
`portal_target` requests fail closed and never fall back to Spectacle. Use
`visible_window_crop_id` only as an explicit last-resort compatibility crop of
current visible pixels; treat `backend=visible_window_crop` as potentially
occluded and never as proof of hidden-window state. For a long but still active
pinned session, use `window_session` with `operation=renew` and the same
`session_id`; renewal revalidates the original KWin id/app/PID and fails closed
instead of binding a replacement. Close the session when
finished. If status reports `active=false`, discard the session id immediately.
Use expert `seatgeist.capture_open` only when the task explicitly needs a
user-approved monitor or virtual output rather than a window. Treat
`requested_source_id` as intent only, never portal authorization, and do not
expect sticky raw control or restore-token persistence for those source types.
Treat `last_end_reason=portal_closed` or `portal_monitor_failed` as revoked or
lost capture authority: do not retry capture, raw input, or another portal open
automatically. An open returning `consent_cancelled` is also a terminal user
choice for that attempt and must not trigger another automatic prompt.
`client_closed` is the expected explicit-close result. If status
clears `sticky_target_bound` after the target was closed or replaced, never
bind a same-application replacement implicitly; close any still-active capture
and open a new user-approved session only when the task still requires it. Stop
and re-open the session if an action returns `target_lost`; do not retry
`focus_lease_conflict` immediately. `seatgeist.act` performs exactly one
allowlisted action through the same daemon policy and journal path as the
expert tool and returns the same settled observation. Sticky raw results also
report `focus_restored` and a compact restoration reason. Restoration occurs
only when `computer_status`/`safety_status` reports the trusted
`kwin_input_spy_v1` or `kwin_input_spy_v2` activity backend; an unavailable backend safely leaves the
agent target focused. The numbered expert flow below applies when those tools
are advertised.

With `kwin_agent_seat`, one verified MCP process owns one opaque virtual-seat
lane and a target window has one agent owner. Treat `agent_target_in_use` and
`agent_lane_quota` as non-retryable until another session is explicitly closed
or expires. The physical user is never locked out: with
`kwin_input_spy_v2`, same-window activity produces `agent_target_user_active`
before delivery or `confirmation=user_preempted` for an in-flight collision.
Stop, wait for the user to finish, obtain a fresh retained frame, and make a
new decision; never automatically replay the prior click or keystroke. User
activity in another window does not invalidate the lane.

An agent-seat action must never acquire the physical workspace focus. Do not
call `focus_window` to prepare or recover a retained action: MCP does not
advertise that tool, and the daemon rejects MCP focus requests. Launches from
MCP always use `preserve_focus`; the independent seat supplies target-local
keyboard focus without activating, raising, or restacking the KWin window.
For window lifecycle operations, treat the KWin UUID as the window identity.
App id and PID only validate that identity: Firefox commonly gives several
windows the same process, so a PID is never sufficient to select one.

Never close or quit a retained window with `Alt+F4`, `Ctrl+W`,
`Ctrl+Shift+W`, `Ctrl+Q`, or a similar key combination. Such shortcuts may be
handled by the application's physically active same-process window rather than
the independent agent-seat target, so the daemon rejects them before input.
Use `seatgeist.close_window` with the owned `session_id` and its exact KWin
`window_id`; it is a destructive policy action and succeeds only after KWin
confirms that exact UUID disappeared. There is no keyboard-shortcut fallback.

Treat `session_owner_mismatch` as terminal for the current client process. Do
not retry the old id, probe it with other lifecycle calls, or open another
portal prompt automatically. A new MCP process or Codex thread must open and
own a fresh session; explicit `seatgeist-cli` lifecycle commands are the only
supported cross-process continuation and status reports their `tool` owner
scope.

`seatgeist.capture_session_status` is owner-scoped for MCP. Its
`active=false` result means only that the current MCP process owns no retained
session; it does not prove that every other agent is idle. Never use that MCP
result to decide that restarting or deploying the daemon is safe. The
explicit `seatgeist-cli capture status` path is the global operator view, and
the deployment helper rechecks both global capture and EIS state immediately
before any restart. Do not close another owner's session to make deployment
proceed.

Use the compact session execution fields to avoid redundant model preflights.
`capture_exec`, `semantic`, and `raw` report the selected backends;
`last_method`, `last_backend`, `last_policy`, `focus_policy`, trusted activity,
focus reacquisition/restoration, and settle fields describe the last successful
control action. Status, snapshot, wait, and renew do not overwrite that action
record. Do not poll active focus before the next sticky raw action or issue a
separate focus call: the daemon reacquires and verifies the pinned target when
needed. These fields are diagnostics, not reusable approval; every action still
runs the daemon's current safety and policy checks.

After a daemon restart, open the same exact requested window again rather than
reusing the expired session id. Exact KWin UUID capture is newly authorized and
does not require portal restore tokens or a chooser. Portal persistence rules
still apply only to explicitly requested monitor, virtual-output, or generic
window ScreenCast sessions.

1. Call `seatgeist.computer_use_readiness` before acting. Treat the explicit action-family state as canonical: satisfy `needs_approval`, reuse the returned opaque `desktop_revision` for `needs_guard`, and do not attempt `blocked` or `unavailable` actions.
2. Call `seatgeist.observe` before acting. Include a bounded screenshot only when visual state matters.
   Bounded screenshot tools attach the PNG directly to the MCP result by default, so do not make a separate filesystem image-view call. Use `include_image=false` only when metadata/path output is sufficient.
3. Prefer `seatgeist.click_button`, `seatgeist.focus_text_field`, `seatgeist.set_text_field`, `seatgeist.select_menu`, `seatgeist.select_item`, `seatgeist.activate_tab`, `seatgeist.activate_link`, `seatgeist.toggle_check`, `seatgeist.set_value`, `seatgeist.close_window`, `seatgeist.move_window`, `seatgeist.resize_window`, and `seatgeist.launch_window` over raw coordinates, titlebar dragging, keyboard-based window management, or shell-based application launch.
4. Use `seatgeist.a11y_focused_tree` or `seatgeist.a11y_find` before semantic actions when the target is not obvious from `seatgeist.observe`.
5. Call `seatgeist.safety_status` before the first control action in a run if readiness did not already include the current safety state. If `focus_guard=true`, include an active-window guard for shared-seat raw input. Retained `kwin_agent_seat` input and exact close use the owned session's pinned KWin identity instead of the user's active-window focus. A fully correlated target-window guard can replace it only for the high-level semantic actions listed in step 3.
6. For a pointer point measured on a retained `snapshot` or `wait` image, use `coordinate_space=capture_output` with that frame's exact `session_id` and `revision`; never copy preview pixels into `window_local`. This lets the daemon apply preview downscaling, client-surface decoration offsets, and fractional DPI atomically, and rejects stale frames. Use direct `window_local`, `logical_pixel`, or `physical_pixel` coordinates only when they came from an independent coordinate source rather than screenshot pixels. Before direct desktop-coordinate actions, call `seatgeist.pointer_calibration`.
7. For a high-level semantic action on a known KWin window, prefer the owner-bound, one-shot `semantic_handle` returned by `window_session operation=inventory`; use it within 10 seconds. The daemon consumes it and still resolves and invokes the accessible target atomically. Copied target-window metadata remains a compatibility fallback. Use `desktop_revision` for shared-seat keyboard, pointer, and scroll actions.
8. Action tools return a compact settled post-action observation by default. Read `dispatch=accepted` separately from `confirmation=confirmed|unconfirmed_timeout|not_requested`; never treat dispatch alone as proof of the intended UI change. A target-guarded semantic action should report `target_scoped=true`, `backend=atspi_event` (or the bounded `target_read` fallback), `target_window`, and `target_accessibility`; it does not require the user's active window to change. Use that result instead of immediately calling `seatgeist.observe` again. Observe separately only when confirmation is unconfirmed, an availability issue exists, or visual evidence is insufficient. Use `observe_after=false` only for a deliberately latency-sensitive low-level sequence.
   When visual confirmation is needed and a matching pinned window session is already open, set `include_image=true` and `capture_session_id=<session>` (a sticky raw action may reuse its `session_id`). This returns one bounded retained frame after settling without another tool call or portal prompt. Do not use a capture session for a different target; the daemon rejects the mismatch before acting.
9. Stop and re-resolve the window if an action returns `target_mismatch`, or if a post-action observation, `seatgeist.active_window`, or `seatgeist.observe` reports a different target than expected. Do not retry a failed semantic action as raw pointer or keyboard input without a new guarded decision.
10. Check `seatgeist.panic_stop_status` if control actions are unexpectedly denied or the desktop appears unsafe.
11. Do not interact with password fields, payment flows, account-security settings, or destructive dialogs without explicit user approval.
12. Set `destructive=true` on `seatgeist.click_button`, `seatgeist.select_menu`, or `seatgeist.a11y_invoke` when the action may delete, discard, close, quit, overwrite, or otherwise lose state. Close an owned window only with exact `seatgeist.close_window`; never substitute a close/quit key combination.

Useful control tools:

- `seatgeist.resize_window` for explicit logical-pixel window sizing after listing the exact KWin id; preserve optional active-window guards when the user's current focus must not change.
- `seatgeist.move_window` for exact logical-pixel placement of an existing listed KWin id while preserving its size.
- `seatgeist.launch_window` for desktop-entry-only launch with a panel-aware corner/center anchor. MCP accepts only `activation=preserve_focus`; pass an exact readiness guard and rely on its returned KWin window id and settled focus-preservation result instead of guessing from a process id. Treat `launch_no_new_window` as non-retryable: a single-instance application such as Firefox reused an existing process/window, so inventory and retain the intended existing KWin UUID instead.
- `seatgeist.close_window` for destructive, session-bound close of one exact KWin UUID. This is the only target-safe way to close a retained Firefox window when several Firefox windows share a PID.
- `seatgeist.page_zoom` for guarded Firefox or Chromium-family page zoom steps. Always pass the exact active-window id and treat the resulting percentage as browser-configured rather than inferred from the step count.
- `seatgeist.type_text` and `seatgeist.key_combo` for guarded text entry and non-window-management shortcuts. `delivery_ack` confirms compositor dispatch, not that an application interpreted the shortcut as intended.
- `seatgeist.focus_text_field` before guarded keyboard entry when AT-SPI exposes a named non-sensitive focusable text field.
- `seatgeist.a11y_text_attributes` when a known non-sensitive text node needs formatting or attribute-run inspection before choosing an edit path.
- `seatgeist.a11y_insert_text` only when a known non-sensitive `EditableText` node needs insertion at a specific character offset and high-level `seatgeist.set_text_field` is not appropriate.
- `seatgeist.a11y_delete_text` only when a known non-sensitive `EditableText` node needs range deletion at specific character offsets.
- `seatgeist.a11y_copy_text` and `seatgeist.a11y_cut_text` only when a known non-sensitive `EditableText` node needs clipboard copy/cut at specific character offsets.
- `seatgeist.a11y_paste_text` only when a known non-sensitive `EditableText` node needs clipboard paste at a specific character offset and the clipboard was intentionally prepared.
- `seatgeist.a11y_set_caret` and `seatgeist.a11y_set_selection` only when a known non-sensitive text node needs caret movement or an existing text-selection range changed at specific character offsets.
- `seatgeist.move_pointer`, `seatgeist.click_pointer`, `seatgeist.drag_pointer`, and `seatgeist.scroll_pointer` only after semantic routes are unavailable.
- `seatgeist.wait_for_change` to confirm bounded visual changes without repeatedly dumping screenshots; omit `output` unless a task-specific artifact path is needed.
- `seatgeist.journal_tail` to inspect compact action history when debugging a run.
