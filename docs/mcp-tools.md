# MCP Tools

`seatgeist-mcp` supports `--tool-profile core|expert|all` (or
`SEATGEIST_TOOL_PROFILE`). `all` remains the compatibility default. `core`
exposes six model-facing tools: `seatgeist.computer_status`,
`seatgeist.window_session`, `seatgeist.snapshot`, `seatgeist.act`, `seatgeist.wait`, and
`seatgeist.panic_stop`; it rejects calls to hidden expert tools instead of
merely omitting them from discovery. `expert` exposes the existing low-level
surface without the aliases. `seatgeist.act` accepts exactly one allowlisted
logical action and maps it to the same daemon request, policy checks, action
journal, and post-action settle path as the corresponding expert tool. It is
not a batch or policy bypass. The stdio server also answers MCP
`resources/list` and `resources/templates/list` with valid empty collections.
Seatgeist currently publishes tools and plugin skills rather than MCP
resources, but generic discovery no longer produces avoidable method-not-found
errors. `window_session` keeps authoritative capture and pinned-target state in
the daemon rather than the MCP process. Core `open` requires
`requested_window_id`: the daemon resolves and authorizes that exact KWin
app/PID, then uses KWin ScreenShot2 to recapture only that UUID without a portal
chooser or ScreenCast stream. `status` is side-effect free and reports the
sticky target identity and expiry. `renew` requires the active opaque session id, revalidates the pinned
KWin id/app/PID and app policy, and extends only the bounded interaction-target
lease; it neither opens a portal dialog nor sends input. `close` requires the active opaque session id and clears both capture
and target state.

Session status also carries a compact execution summary: `capture_exec`,
`semantic`, `raw`, `last_method`, `last_backend`, `last_policy`, `focus_policy`,
trusted activity state, focus reacquisition/restoration outcome, and the final
settle backend/result. This lets a client reuse the daemon-owned sticky target
without polling active focus or rediscovering the selected backend. It is
diagnostic state, not reusable authorization: every next control request still
runs current policy, panic-stop, activity, target, focus, and rate checks.
Status, snapshot, wait, and renew do not replace the last control-action
summary.

`seatgeist.snapshot` and `seatgeist.wait` use repeated compositor-side KWin
exact-window screenshots and require the session id. They return content
revisions without a portal chooser or ScreenCast stream. Core
mode cannot silently fall back to Screenshot v2 or whole-desktop polling;
explicit `seatgeist.screenshot` and `seatgeist.wait_for_change` compatibility
tools remain available in the expert and all profiles.

Expert `seatgeist.screenshot` and screenshot-bearing `seatgeist.observe` expose
two explicit one-shot compatibility modes behind `ScreenBackend`. A
`portal_target=window|active_window|area|screen` request requires Screenshot v3
target support and never falls back to Spectacle after a portal failure.
`visible_window_crop_id=<kwin-id>` instead crops current visible pixels from a
composed desktop, reports `backend=visible_window_crop` in window-local
coordinates, and can include overlapping windows. It cannot be combined with
`portal_target` or `portal_interactive` and fails closed for uncertain monitor
or geometry mapping. Core `snapshot`/`wait` never use either compatibility
mode.

Action tools accept an opt-in `include_image=true` post-action result. Supply
`capture_session_id` (or use the same sticky `session_id` on a raw action); the
daemon validates that the retained session is live, pinned, and matches the
action's sticky target, target-window guard, focus destination, or exact
active-window guard before executing the action. After settling it reads one
bounded retained frame, places the frame revision and path in the observation,
and returns native MCP image content in the same call. This path never opens a
portal dialog or falls back to desktop capture. Capture start/finish records
share the parent action id; image artifact metadata remains opt-in.

Action `ok=true` means the backend dispatch succeeded; it is not visual proof.
Compact and structured settle output separately reports confirmation as
`confirmed`, `unconfirmed_timeout`, or `not_requested`. A timeout does not
retroactively mean dispatch failed, so inspect the returned observation or use
the retained-session wait before deciding whether a retry is safe.

`seatgeist.computer_status` caches one successful readiness result inside the
MCP process for at most 30 seconds while the daemon socket identity is
unchanged. Any intervening tool call invalidates it, as does daemon socket
replacement. This removes accidental repeated preflight round trips without
weakening action safety: every control request still reaches the daemon and
reruns current policy, panic-stop, human-activity, target, focus, and rate
checks.

Readiness uses explicit `ready`, `needs_guard`, `needs_approval`, `blocked`, or
`unavailable` states for every action family. The compatibility booleans are
true only for `ready`. A successful result includes an opaque
`desktop_revision`; pass it back as the action `desktop_revision` guard instead
of copying the active window id, app id, and title.

Core `window_session operation=inventory` returns the current sorted KWin
window inventory, active window, and an opaque revision. Use
`operation=wait_inventory` with `after_revision` for one bounded server-side
wait instead of repeatedly calling list/active-window tools. Each inventory
also includes owner-bound, one-shot `semantic_handle` values. A semantic
`seatgeist.act` can use one within 10 seconds instead of copying target window
metadata; the daemon consumes it once and still resolves, correlates,
policy-checks, and invokes the AT-SPI target atomically.

Initial tool groups:

- Observation: `seatgeist.health`, `seatgeist.capabilities`, `seatgeist.list_monitors`, `seatgeist.list_windows`, `seatgeist.active_window`, `seatgeist.screenshot`, `seatgeist.screenshot_tile`, `seatgeist.observe`, `seatgeist.wait_for_change`.
- Control: `seatgeist.close_window`, `seatgeist.move_window`, `seatgeist.launch_window`, `seatgeist.resize_window`, `seatgeist.page_zoom`, `seatgeist.type_text`, `seatgeist.key_combo`, `seatgeist.move_pointer`, `seatgeist.click_pointer`, `seatgeist.drag_pointer`, `seatgeist.scroll_pointer`, `seatgeist.click_button`, `seatgeist.set_text_field`, `seatgeist.focus_text_field`, `seatgeist.select_menu`, `seatgeist.activate_tab`, `seatgeist.activate_link`, `seatgeist.toggle_check`, `seatgeist.set_value`, `seatgeist.select_item`. MCP deliberately does not advertise `focus_window`; changing the physical user's workspace focus remains an explicit CLI/operator operation.
- Clipboard: `seatgeist.clipboard_status`, `seatgeist.clipboard_get_text`, `seatgeist.clipboard_set_text`.
- Accessibility: `seatgeist.a11y_quality_status`, `seatgeist.a11y_focused_tree`, `seatgeist.a11y_find`, `seatgeist.a11y_text_attributes`, `seatgeist.a11y_invoke`, `seatgeist.a11y_set_text`, `seatgeist.a11y_insert_text`, `seatgeist.a11y_delete_text`, `seatgeist.a11y_copy_text`, `seatgeist.a11y_cut_text`, `seatgeist.a11y_paste_text`, `seatgeist.a11y_set_caret`, `seatgeist.a11y_set_selection`.
- Retained capture: expert `seatgeist.capture_open` for explicit window/monitor/virtual-output sources, plus `seatgeist.window_capture_open`, `seatgeist.capture_session_status`, `seatgeist.capture_session_renew`, `seatgeist.capture_snapshot`, `seatgeist.capture_wait`, `seatgeist.capture_session_close`.
- Safety/diagnostics: `seatgeist.policy_status`, `seatgeist.safety_status`, `seatgeist.desktop_session_status`, `seatgeist.computer_use_readiness`, `seatgeist.panic_stop_status`, `seatgeist.panic_stop_enable`, `seatgeist.panic_stop_disable`, `seatgeist.uinput_status`, `seatgeist.input_backend_status`, `seatgeist.remote_desktop_session_probe`, `seatgeist.remote_desktop_eis_probe`, `seatgeist.remote_desktop_eis_start`, `seatgeist.remote_desktop_eis_session_status`, `seatgeist.remote_desktop_eis_stop`, `seatgeist.capture_backend_status`, `seatgeist.pointer_calibration`, `seatgeist.journal_tail`.

All coordinate-bearing tools must require an explicit coordinate space. Full-resolution screenshots and clipboard reads are policy-gated and fail closed by default without explicit daemon approval.

`seatgeist.resize_window` takes a listed KWin `window_id` plus a width and height from 64 through 32768 logical pixels. It preserves the current position, passes `ControlSemantic` policy, panic-stop, optional active-window guard, app policy, rate limiting, and journal checks before the shared window backend queues a compositor action. The compact action result reports requested and actual geometry. Use `seatgeist-cli resize --window <id> --width <logical-width> --height <logical-height>` for the same daemon contract.

`seatgeist.move_window` moves an exact listed KWin id to explicit logical-pixel coordinates while preserving its size. `seatgeist.launch_window` accepts a desktop-entry id, never a shell command or path, and arms a one-shot KWin intent before invoking `gtk-launch`. The compositor matches the new window by desktop entry, anchors it inside the panel-aware placement area (`top_left`, `top_right`, `bottom_left`, `bottom_right`, or `center`), optionally applies monitor, margin, and size, and verifies the settled geometry. MCP accepts only `activation=preserve_focus` and confirms the previously active physical window stayed active. The protocol and CLI retain `activate` for deliberate operator use, but an MCP request for it fails before launch. Both paths pass policy, panic-stop, optional active-window guard, app policy, rate limiting, and journaling. CLI equivalents are `seatgeist-cli move ...` and `seatgeist-cli launch --desktop-entry <id> --anchor top-right ...`.

`seatgeist.close_window` is the target-safe lifecycle path for an owned retained window. It requires the retained `session_id` and exact pinned KWin `window_id`, classifies the request as `DestructiveAction`, re-resolves UUID, app id, and PID, asks the KWin bridge to close that exact UUID, and returns success only after that UUID disappears. It never falls back to a key combination. This distinction matters for Firefox: several browser windows can share one PID, while window-global shortcuts can be consumed by the physically active same-process window. Retained-seat `Alt+F4`, `Ctrl+W`, `Ctrl+Shift+W`, and `Ctrl+Q` requests therefore fail closed before input.

`seatgeist.page_zoom` takes `operation=in|out|reset`, an optional 1-20 step count, and a required exact active-window id guard. Immediately before input it rechecks that the active app id belongs to Firefox or a Chromium-family browser, then sends the standard Linux browser zoom shortcut through the configured keyboard backend under `ControlKeyboard` policy. It does not claim an exact percentage: Firefox and Chromium can customize shortcuts or zoom ladders, and Firefox persists zoom per hostname. Use `seatgeist-cli page-zoom --operation out --steps 2 --expected-active-window <id>` for the same path.

Daemon and MCP error responses include a compact structured `kind`, a stable
`reason_code`, and the human message. Current kinds distinguish missing
approval (`policy_prompt_required`), explicit policy denial, app-policy denial,
focus/active-window guard failure, resolved target mismatch
(`target_mismatch`), expired or invalid pinned targets (`target_lost`), foreign
retained-session access (`session_owner_mismatch`), a busy per-seat lease
(`focus_lease_conflict`), human-input pause, panic-stop, rate limiting,
deliberate portal cancellation (`consent_cancelled`), portal/backend
unavailability, backend failure, accessibility unavailability or weak trees,
validation, and unknown failures. Stable reasons preserve actionable causes
inside those broad groups, such as `protected_application`,
`atspi_registry_unreachable`, `kwin_bridge_unavailable`,
`agent_target_in_use`, `agent_lane_quota`, `agent_target_user_active`, or
`capture_frame_invalidated_by_user`. MCP compact text
includes both fields. An `app_denied` result is rendered as `POLICY DENIED`
with an instruction to stop and not retry through another control backend;
structured content keeps the full daemon error JSON.

`seatgeist.journal_tail` returns compact daemon journal entries. Every new
entry carries daemon `run_id` and `build_id` correlation, and daemon lifecycle
records use the methods `daemon_start` and `daemon_stop`. Entries may include
explicit client tool identity such as `seatgeist-mcp` plus best-effort
pid/process-name metadata from Unix peer credentials; Linux process names can
be kernel-truncated, and callers cannot self-report pid/process fields.
Control entries may also include a structured `control` object with action id,
policy result, backend provenance, and redacted requested-target metadata such
as coordinates, offsets, node ids, text/key counts, app filters, and device
hints. Failed summaries retain error kind and reason even when detailed error
text is disabled. Screenshot-bearing entries may include `artifacts` with
path, byte count, and SHA-256 only when
`[journal].include_artifact_metadata = true`. It must not return typed text,
replacement text, clipboard contents, screenshot contents, or semantic target
names.

Current executable raw keyboard/pointer control uses the uinput adapter for `auto` and `uinput`. Explicit `portal_remote_desktop` or `libei` backend configuration now builds EIS plans for UTF-8 text, modeled XKB text keysyms, named evdev or single-symbol key combos, and pointer requests and executes currently wired daemon requests through the stored daemon EIS session only after the per-plan readiness gate passes. Explicit EIS key combos keep the named evdev parser first, then fall back to xkbcommon lookup for single-character symbol parts such as `Ctrl+;`; operators can pin `[backends.keymap]` RMLVO names in daemon config. Without that config override, the daemon resolves EIS keymap names from KDE's current keyboard-layout DBus metadata, then KDE `kxkbrc` config via `kreadconfig6`, then xkbcommon defaults. `seatgeist.input_backend_status` reports the compact `eis_keymap` source/layout metadata. `seatgeist.remote_desktop_eis_probe` initializes a transient daemon EIS runtime from the returned portal FD, polls pending events, reports compact runtime connected/event/bound-capability/resumed-device state, closes the runtime, and still sends no input. `seatgeist.remote_desktop_eis_start`, `seatgeist.remote_desktop_eis_session_status`, and `seatgeist.remote_desktop_eis_stop` expose the daemon-owned EIS session lifecycle; start may open a portal dialog; retained sessions are used only by separate policy-gated raw input requests after per-plan readiness passes.

Failed journal summaries store only the structured error kind by default, so
target labels, window identifiers, and backend diagnostics are not duplicated
into the audit log. Full error text requires the private-run opt-in
`[journal].include_error_details = true`.

The retained capture daemon protocol methods are `window_capture_open`,
generic expert `capture_open`, `capture_session_status`, `capture_session_renew`, `capture_snapshot`, `capture_wait`, and
`capture_session_close`. The CLI groups these as `seatgeist-cli capture
open|status|renew|snapshot|wait|close`; `capture open --source
window|monitor|virtual-output` selects the exact retained source contract.
Open and frame acquisition are observe-policy
requests; status, identity-validated renew, and id-checked close are policy-class lifecycle operations.
Every call is journaled, while output paths become journal artifacts only when
artifact metadata is explicitly enabled.

Exact KWin window sessions are parallel: each trusted client owner may keep up
to four, and different owners do not block one another. Chooser-backed portal
sessions remain globally serialized. A retained frame reports its preview
dimensions, source extent, scale, and `capture_output -> source` transform.
When a pointer coordinate was measured on that preview, pass
`coordinate_space="capture_output"`, the frame's `session_id`, and its exact
`capture_revision`. The daemon rejects stale revisions and maps the point
atomically, including fractional-DPI and preview downscaling; do not copy
preview pixels into `window_local`.

Capture status includes an optional compact `last_end_reason`. Portal-driven
`Session.Closed` is reported as `portal_closed`; explicit id-checked close is
reported as `client_closed`; a failed portal closure monitor fails closed as
`portal_monitor_failed`. Ended sessions expose no session id or sticky target,
are removed before reuse, and cannot authorize sticky raw input.

Status also reports compact `owner_tool`, `owner_pid`, and `owner_scope`
metadata. A session opened by one MCP server process cannot be renewed, read,
closed, used for sticky raw control, or used for a post-action image by another
process. MCP `capture_session_status` is therefore an owner-scoped view:
`active=false` says that the calling MCP process owns no session, not that the
daemon has no sessions belonging to other agents. A verified CLI session
remains usable by later CLI invocations, and `seatgeist-cli capture status` is
the explicit global operator view used by the guarded daemon deployment
workflow. Agents must not close another owner's session merely to unblock a
deployment.

High-level semantic actions accept a target-window guard independently of the
active-window guard. MCP and the core `seatgeist.act` facade use
`target_window_id`, with optional `target_app_id`, `target_pid`, and
`target_title_contains`; the CLI exposes the equivalent `--target-*` flags.
Supplying any optional target constraint without the window id is rejected.
The daemon resolves the AT-SPI node without side effects, correlates its DBus
process id, application, and containing window with the exact KWin window, then
applies app policy to that resolved window before invoking the semantic action.
This permits proven background semantics while another window is active.
Missing, reopened, or mismatched windows fail closed as `target_mismatch`.
Raw seat input still requires an active-window guard when focus guards are
enabled; a target guard is not accepted as a replacement for raw input. Journal
records include the target window id, app id, pid, and only whether a title
constraint was present, never the title constraint or semantic target name.

An opt-in live evaluator is available for this contract:

```bash
SCENARIO=<firefox-or-kde> TARGET_WINDOW_ID=<background-target-id> \
  USER_WINDOW_ID=<active-work-window-id> BUTTON_NAME='<safe accessible button>' \
  make gui-eval-background-semantic
```

It performs one policy-approved `click_button`, verifies that a non-target user
window was active before and after the action, and asks the operator to confirm the target never received
focus. A physical user switch between non-target work windows is accepted and
recorded rather than misclassified as a semantic failure. The runner also
verifies the action journal. It does not focus, inject raw input, or take
a screenshot, so it proves a genuinely background-capable AT-SPI action rather
than a hidden focus fallback. It remains opt-in because it performs a real
semantic side effect.

`seatgeist.safety_status` includes the registered activity backend and trust
state plus the last redacted activity class/provenance. It never exposes input
content, coordinates, titles, or device identity. A trusted
`kwin_input_spy_v1` or `kwin_input_spy_v2`
backend lets a sticky raw action restore the previously focused window when no
physical or unknown activity occurred; otherwise restoration is skipped and
the action result reports the compact reason. Version 2 additionally reports
only the KWin window UUID, allowing independent agent seats to ignore activity
in other windows, pause 350 ms after same-window input, and return
`confirmation=user_preempted` for an in-flight collision.

A raw keyboard or pointer action may instead carry the `session_id` of a
window session with a pinned target. The daemon re-resolves the exact KWin
window/app/PID, reapplies app policy, acquires a short per-seat lease, and, only
when required, applies the ordinary focus policy before asking the focus backend
to activate that target. Input is sent only after the KWin bridge confirms the
target as active. The focus policy, focus request, focus verification, and raw
input journal records share one action id and contain no window title. A stale,
closed, replaced, or expired target returns `target_lost`; lease contention
returns `focus_lease_conflict`. Do not combine a sticky session with an active
window guard. Step 10 deliberately leaves the focused target active; physical
activity provenance and safe restoration of the user's prior focus belong to
the following cooperative-restoration slice.

Capture status revalidates the pinned KWin id/app/PID and clears the sticky
binding when that target was closed or replaced. A new window from the same
application is never treated as the old session target.

The core facade rejects `session_id` combined with any active-window guard
locally, before making a daemon round trip. A sticky session already supplies
the exact target identity; combining both guard models is contradictory and
must not become a recoverable focus/retry loop.

Current daemon protocol exposes `observe`, `screenshot`, `screenshot-tile`, `wait_for_change`, window listing, active-window bridge reads, KWin bridge status, safety status, desktop session status, computer-use readiness, uinput status, input backend status, RemoteDesktop session probe, RemoteDesktop EIS probe, RemoteDesktop EIS session start/status/stop, capture backend status, pointer calibration, panic-stop status/set, `focus_window`, `move_window`, `launch_window`, `resize_window`, `page_zoom`, `type_text`, `key_combo`, `move_pointer`, `click_pointer`, `drag_pointer`, `scroll_pointer`, high-level semantic actions, clipboard operations, AT-SPI operations, and `journal_tail` through the CLI. The MCP stdio server exposes the corresponding expert tools; core `seatgeist.act` allowlists the window and browser actions as one-action requests through the same daemon policy and journal path.


Semantic action ambiguity is a refusal state, not an implicit selection. When `seatgeist.click_button`, `seatgeist.set_text_field`, `seatgeist.focus_text_field`, `seatgeist.select_menu`, `seatgeist.activate_tab`, `seatgeist.activate_link`, `seatgeist.toggle_check`, `seatgeist.set_value`, or `seatgeist.select_item` finds multiple viable non-sensitive matches, the daemon returns compact candidate choices with node id, role, accessible name, deterministic name-match score, and action metadata, capped at five choices plus an omitted-count marker. `score=1.00` means an exact case-insensitive name match, lower scores mean weaker prefix or substring matches, and the action still refuses until the caller disambiguates. `seatgeist.toggle_check` targets checkboxes, radio buttons, and checkable menu items; pass `checked=true` or `checked=false` when the desired state is known to avoid an unnecessary toggle if AT-SPI already reports the matching state. `seatgeist.set_value` targets sliders, spin buttons, scrollbars, and dials with numeric AT-SPI values and writes `org.a11y.atspi.Value.CurrentValue`. `seatgeist.select_item` targets list items, tree items, table rows, combo boxes, options, and menu-item-like choices with select or press actions.

`seatgeist.a11y_set_caret` calls `org.a11y.atspi.Text.SetCaretOffset` on a non-sensitive text node and reports offset only. `seatgeist.a11y_set_selection` calls `org.a11y.atspi.Text.SetSelection` for an existing selection index, validates `end_offset > start_offset`, and reports only the selection index and offsets. Both tools are policy-gated semantic control and accept the same active-window guard arguments as other control tools.

MCP action tools request one compact post-action observation by default. The
daemon captures it only after the action has passed all policy, approval,
panic-stop, human-activity, focus-guard, app-policy, and rate-limit checks. The
observation contains the current window identity, a compact focused AT-SPI node
with text values removed, a revision hash, best-effort issue codes, and bounded
settle metadata. It does not include a screenshot or echo typed/clipboard text.
For a target-guarded high-level semantic action, the observation additionally
contains `target_window` and a compact `target_accessibility` node. The daemon
registers object, window, and focus event interest before the side effect and
accepts only signals from the correlated application bus name whose source path
is the resolved node or its containing window. It then reads only that target
node once instead of rescanning the focused desktop tree. Settle metadata names
the `atspi_event`, `target_read`, `delivery_ack`, or `polling` backend, whether it is
target-scoped, and the non-content event class/member when present. If event
subscription is unavailable, a bounded target-node read/poll fallback is used;
it never retries through pointer or keyboard input.

`settle_condition=auto` resolves to exact requested-window activation for
`seatgeist.focus_window`, `accessibility_change` through target-scoped events for
guarded high-level semantic actions, compositor delivery acknowledgment for
session-bound independent agent-seat actions, and `stable` polling otherwise.
Agent-seat acknowledgment confirms exact-target delivery without waiting for an
unrelated foreground-window or accessibility change. Explicit conditions are
`none`, `stable`, `active_window_change`, `accessibility_change`, and
`any_change`. A default focus timeout returns `ok=false` because dispatch was
accepted but the requested target was never confirmed active; MCP marks that
result as a tool error. Other timeout watchdog results remain successful
actions with `settled=false` and `timed_out=true`. Defaults are 1500 ms timeout and 100 ms
sampling, bounded by daemon validation to 1-10000 ms and 10-1000 ms
respectively. Pass `observe_after=false` to opt out; it cannot be combined with
a non-`none` settle condition. Legacy daemon and CLI requests remain unchanged
unless they send response options. Journal summaries retain only settle status,
backend, target-scope flag, event type, sample count, and elapsed time, not
observation content.

## Installation

Manual Codex config uses a stdio MCP server entry:

```toml
[mcp_servers.seatgeist]
command = "seatgeist-mcp"
args = ["--stdio"]
```

The plugin bundle points at `plugin/.mcp.json` through `.codex-plugin/plugin.json`, so an installed plugin can provide the same MCP server config. The `seatgeist-mcp` binary must be on `PATH` for the current initial config.

The plugin bundle is validated by `make validate-plugin`, and the validator is included in `make verify`.
