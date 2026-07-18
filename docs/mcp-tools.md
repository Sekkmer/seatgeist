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
- Control: `seatgeist.focus_window`, `seatgeist.move_window`, `seatgeist.launch_window`, `seatgeist.resize_window`, `seatgeist.page_zoom`, `seatgeist.type_text`, `seatgeist.key_combo`, `seatgeist.move_pointer`, `seatgeist.click_pointer`, `seatgeist.drag_pointer`, `seatgeist.scroll_pointer`, `seatgeist.click_button`, `seatgeist.set_text_field`, `seatgeist.focus_text_field`, `seatgeist.select_menu`, `seatgeist.activate_tab`, `seatgeist.activate_link`, `seatgeist.toggle_check`, `seatgeist.set_value`, `seatgeist.select_item`.
- Clipboard: `seatgeist.clipboard_status`, `seatgeist.clipboard_get_text`, `seatgeist.clipboard_set_text`.
- Accessibility: `seatgeist.a11y_quality_status`, `seatgeist.a11y_focused_tree`, `seatgeist.a11y_find`, `seatgeist.a11y_text_attributes`, `seatgeist.a11y_invoke`, `seatgeist.a11y_set_text`, `seatgeist.a11y_insert_text`, `seatgeist.a11y_delete_text`, `seatgeist.a11y_copy_text`, `seatgeist.a11y_cut_text`, `seatgeist.a11y_paste_text`, `seatgeist.a11y_set_caret`, `seatgeist.a11y_set_selection`.
- Retained capture: expert `seatgeist.capture_open` for explicit window/monitor/virtual-output sources, plus `seatgeist.window_capture_open`, `seatgeist.capture_session_status`, `seatgeist.capture_session_renew`, `seatgeist.capture_snapshot`, `seatgeist.capture_wait`, `seatgeist.capture_session_close`.
- Safety/diagnostics: `seatgeist.policy_status`, `seatgeist.safety_status`, `seatgeist.desktop_session_status`, `seatgeist.computer_use_readiness`, `seatgeist.panic_stop_status`, `seatgeist.panic_stop_enable`, `seatgeist.panic_stop_disable`, `seatgeist.uinput_status`, `seatgeist.input_backend_status`, `seatgeist.remote_desktop_session_probe`, `seatgeist.remote_desktop_eis_probe`, `seatgeist.remote_desktop_eis_start`, `seatgeist.remote_desktop_eis_session_status`, `seatgeist.remote_desktop_eis_stop`, `seatgeist.capture_backend_status`, `seatgeist.pointer_calibration`, `seatgeist.journal_tail`.

All coordinate-bearing tools must require an explicit coordinate space. Full-resolution screenshots and clipboard reads are policy-gated and fail closed by default without explicit daemon approval.

`seatgeist.resize_window` takes a listed KWin `window_id` plus a width and height from 64 through 32768 logical pixels. It preserves the current position, passes `ControlSemantic` policy, panic-stop, optional active-window guard, app policy, rate limiting, and journal checks before the shared window backend queues a compositor action. The compact action result reports requested and actual geometry. Use `seatgeist-cli resize --window <id> --width <logical-width> --height <logical-height>` for the same daemon contract.

`seatgeist.move_window` moves an exact listed KWin id to explicit logical-pixel coordinates while preserving its size. `seatgeist.launch_window` accepts a desktop-entry id, never a shell command or path, and arms a one-shot KWin intent before invoking `gtk-launch`. The compositor matches the new window by desktop entry, anchors it inside the panel-aware placement area (`top_left`, `top_right`, `bottom_left`, `bottom_right`, or `center`), optionally applies monitor, margin, and size, and verifies the settled geometry. `activation=preserve_focus` restores and confirms the previously active window; `activate` deliberately focuses the new one. Both paths pass policy, panic-stop, optional active-window guard, app policy, rate limiting, and journaling. CLI equivalents are `seatgeist-cli move ...` and `seatgeist-cli launch --desktop-entry <id> --anchor top-right ...`.

`seatgeist.page_zoom` takes `operation=in|out|reset`, an optional 1-20 step count, and a required exact active-window id guard. Immediately before input it rechecks that the active app id belongs to Firefox or a Chromium-family browser, then sends the standard Linux browser zoom shortcut through the configured keyboard backend under `ControlKeyboard` policy. It does not claim an exact percentage: Firefox and Chromium can customize shortcuts or zoom ladders, and Firefox persists zoom per hostname. Use `seatgeist-cli page-zoom --operation out --steps 2 --expected-active-window <id>` for the same path.

Daemon and MCP error responses now include a compact structured `kind` next to the human message. Current kinds distinguish missing approval (`policy_prompt_required`), explicit policy denial, app-policy denial, focus/active-window guard failure, resolved target mismatch (`target_mismatch`), expired or invalid pinned targets (`target_lost`), foreign retained-session access (`session_owner_mismatch`), a busy per-seat lease (`focus_lease_conflict`), human-input pause, panic-stop, rate limiting, deliberate portal cancellation (`consent_cancelled`), portal/backend unavailability, backend failure, accessibility unavailability or weak trees, validation, and unknown failures. MCP compact text includes the kind, and structured content keeps the daemon JSON shape as `{"type":"error","data":{"kind":"...","message":"..."}}`.

`seatgeist.journal_tail` returns compact daemon journal entries. Entries may include explicit client tool identity such as `seatgeist-mcp` plus best-effort pid/process-name metadata from Unix peer credentials; Linux process names can be kernel-truncated, and callers cannot self-report pid/process fields. Control entries may also include a structured `control` object with action id, policy result, backend provenance, and redacted requested-target metadata such as coordinates, offsets, node ids, text/key counts, app filters, and device hints. Screenshot-bearing entries may include `artifacts` with path, byte count, and SHA-256 only when `[journal].include_artifact_metadata = true`. It must not return typed text, replacement text, clipboard contents, screenshot contents, or semantic target names.

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

Capture status includes an optional compact `last_end_reason`. Portal-driven
`Session.Closed` is reported as `portal_closed`; explicit id-checked close is
reported as `client_closed`; a failed portal closure monitor fails closed as
`portal_monitor_failed`. Ended sessions expose no session id or sticky target,
are removed before reuse, and cannot authorize sticky raw input.

Status also reports compact `owner_tool`, `owner_pid`, and `owner_scope`
metadata. A session opened by one MCP server process cannot be renewed, read,
closed, used for sticky raw control, or used for a post-action image by another
process. A verified CLI session remains usable by later CLI invocations.

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
content, coordinates, or device identity. A trusted `kwin_input_spy_v1`
backend lets a sticky raw action restore the previously focused window when no
physical or unknown activity occurred; otherwise restoration is skipped and
the action result reports the compact reason.

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

`seatgeist.observe` returns monitors, windows, active-window state when available, and optional bounded screenshot metadata if `screenshot_output` is provided. `seatgeist.screenshot`, `seatgeist.screenshot_tile`, and `seatgeist.wait_for_change` accept optional output paths; when omitted, MCP writes timestamped PNGs under the Seatgeist runtime screenshot directory. Bounded screenshot, tile, and wait results attach native MCP `image/png` content by default so the model can inspect the result without a second filesystem-image call; `seatgeist.observe` does the same when `screenshot_output` is present. Pass `include_image=false` for metadata/path-only results. Attachments are limited to 2048 pixels per edge and 16 MiB, are opened without following symlinks, and remain absent from journals and structured JSON. Full-resolution screenshots are never embedded; their separately policy-gated output must be inspected explicitly. If `full_resolution` is requested through `seatgeist.observe` or `seatgeist.screenshot`, the daemon classifies the request as full-resolution screenshot access and rejects it by default until started with explicit approval. `seatgeist.observe`, `seatgeist.screenshot`, and `seatgeist.screenshot_tile` also accept `portal_interactive=true`, which passes the XDG portal Screenshot `interactive` hint only when the portal backend is selected; the default remains noninteractive. `seatgeist.observe` and `seatgeist.screenshot` accept optional `portal_target` values `screen`, `window`, `area`, and `active_window` for Screenshot v3 backends; if status reports `screenshot_target_option_supported=false` or the target is not advertised by `AvailableTargets`, the daemon fails closed instead of falling back to a different capture shape. Window and active-window objects include `monitor_id` when their logical geometry overlaps known KWin monitor geometry; active-window objects include pid when the KWin bridge publishes it. Tile coordinates are physical screenshot pixels. Screenshot responses include backend provenance, full source dimensions, output dimensions, source origin, scale factors, and monitor metadata when KWin responds; configured redaction regions are applied to the output PNG before metadata is returned. Default preview, observe screenshot, wait-for-change, and tile max-edge bounds come from `[safety].preview_max_edge` and `[safety].tile_max_edge`, unless a tool request supplies its own `max_edge`; full-resolution screenshot requests remain explicit and separately policy-gated. `seatgeist.wait_for_change` polls bounded screenshots until normalized RGB delta reaches `threshold` or `timeout_ms` expires; it returns changed/timed-out/captures/elapsed/timeout/interval/score metadata and the latest screenshot metadata, so watchdog no-change results are distinct from failed commands or capture backend errors. `seatgeist.safety_status` reports whether focus guards are required, whether human-input pause is enabled, whether the activity signal is currently fresh, the quiet interval, the optional signal-file path, the configured control rate limit, configured screenshot preview/tile max-edge defaults, the configured screenshot redaction count without exposing redaction geometry, and whether opt-in journal artifact metadata is enabled. `seatgeist.desktop_session_status` reports sanitized KDE/Wayland/session environment facts, including session type, desktop name, KDE session hints, display names, and only boolean presence for DBus and runtime-directory variables. `seatgeist.computer_use_readiness` aggregates safe preflight diagnostics without screenshots, portal sessions, clipboard reads, or input; it reports readiness booleans for observe, screenshot, window control, keyboard, pointer, semantic actions, clipboard read/write, active safety blockers, selected backend names, issue lists, and suggested next diagnostic tools. `seatgeist.capture_backend_status` probes xdg-desktop-portal Screenshot/ScreenCast interface visibility, Screenshot interface version, Screenshot v3 `AvailableTargets` when exported, KWin supportInformation metadata availability, and Spectacle fallback availability without starting a portal session or capturing pixels; it reports both the preferred visible backend and the currently implemented capture backend. When portal Screenshot is visible, full-screen and tile screenshot execution use the portal through the daemon; if status reports `screenshot_target_option_supported=false`, callers must assume the v2 full-screen Screenshot contract even if a newer frontend spec documents target-specific capture. Spectacle remains the compatibility fallback when portal Screenshot is unavailable or fails before a user response. `seatgeist.kwin_bridge_status` reports whether the daemon DBus receiver registered, whether the KWin script has published active-window and window-list updates, the latest bridge window count, and whether the user-local script package/config appear installed and enabled. `seatgeist.uinput_status` reports whether the daemon can open `/dev/uinput` read/write, whether the path exists and is a character device, mode/owner ids, daemon effective uid/gid, and a setup hint. `seatgeist.input_backend_status` probes xdg-desktop-portal RemoteDesktop, KDE portal service visibility, libei client metadata/socket hints, uinput fallback availability, and EIS keymap source metadata without starting a portal session or sending input; it reports `configured_backend`, `preferred_available_backend`, `implemented_available_backend`, and `eis_keymap`. `seatgeist.remote_desktop_session_probe` is an explicit policy-gated control-class probe that can request keyboard, pointer, and/or touchscreen devices through xdg-desktop-portal RemoteDesktop. It may show a portal consent dialog, accepts optional restore token, persist mode, parent window, timeout, and active-window guard arguments, reports selected devices/clipboard/restore metadata and request/session handles, then closes the transient session without calling `ConnectToEIS` or sending Notify*/EIS input. `seatgeist.remote_desktop_eis_probe` is a separate explicit control-class probe for the EIS handoff path: after `Start`, it calls `ConnectToEIS` on the same transient session, initializes a transient daemon EIS runtime from the returned FD, polls pending events, closes the runtime, reports compact metadata including `eis_fd_closed`, `eis_runtime_connected`, `eis_event_count`, `eis_bound_capabilities`, and `eis_resumed_device_count`, and still sends no EIS or Notify* input. `seatgeist.remote_desktop_eis_start` uses the same portal arguments and active-window guards, retains a single daemon-owned EIS session, polls initial runtime state, and reports active/runtime/bound-capability/resumed-device/selected-device metadata without sending input. `seatgeist.remote_desktop_eis_session_status` reports the retained session state without opening a portal dialog, and `seatgeist.remote_desktop_eis_stop` drops the retained session if present. Current executable raw keyboard/pointer control resolves through an input-executor trait and uses the uinput adapter; explicit `portal_remote_desktop` or `libei` configuration routes EIS plans through the stored daemon EIS session and fails closed before side effects when no stored session or ready selected device exists. `seatgeist.pointer_calibration` reports monitor-derived physical-pixel bounds, per-monitor physical origins, and representative sample points without moving the pointer. Focus, keyboard input, pointer input, and semantic actions are policy-gated control, rate-limited by `[safety].control_rate_limit_per_minute`, and blocked while panic-stop is active. Current control tools accept optional active-window guards: `expected_active_window`, `expected_active_app`, and `active_title_contains`. `seatgeist.type_text` types US-keyboard-mapped ASCII plus newline/tab through the current input executor and reports text length plus backend only. `seatgeist.key_combo` sends named combos such as `Ctrl+L` or `Alt+F4` through the current input executor and, for explicit EIS backends only, can resolve single-character level-0 symbol parts such as `Ctrl+;` through configured, KDE-discovered, or default xkbcommon keymap lookup; it reports key count plus backend only. `seatgeist.move_pointer`, `seatgeist.click_pointer`, and `seatgeist.drag_pointer` require explicit coordinates and `coordinate_space`; daemon support accepts `physical_pixel`, global `logical_pixel`, and guarded active-window `window_local`. Physical coordinates are validated against monitor-derived physical desktop bounds, which supports 8K and scaled displays. Logical coordinates are global compositor logical pixels and map through monitor logical origins plus scale factors before physical bounds validation. Window-local coordinates are active-window-relative logical pixels and are mapped through active-window geometry plus monitor scale before input execution; `expected_active_window`, `expected_active_app`, or `active_title_contains` is required for `window_local`. `seatgeist.click_pointer` supports left/middle/right buttons and one or two clicks. `seatgeist.drag_pointer` presses a left/middle/right button, interpolates to the target over a bounded `duration_ms`, releases the button, and reports coordinates/duration/backend only. `seatgeist.scroll_pointer` emits vertical and/or horizontal wheel deltas at the current pointer position and reports deltas/backend only. `seatgeist.click_button` finds a named non-sensitive AT-SPI button with optional app/window guards, invokes press only when exactly one viable match remains, and refuses ambiguous matches. `seatgeist.click_button`, `seatgeist.select_menu`, and `seatgeist.a11y_invoke` accept `destructive=true`; obvious destructive button/menu labels are also routed through destructive-action policy. `seatgeist.set_text_field` finds a named non-sensitive AT-SPI text field with optional app/window guards, requires one viable match, uses `EditableText` set-text, and reports replacement length only; secret-looking target names are routed through secret-field policy before matching. `seatgeist.focus_text_field` finds a named non-sensitive text field that advertises a focus action, requires one viable match, invokes focus, and routes secret-looking target names through secret-field policy before matching. `seatgeist.select_menu` selects a visible AT-SPI menu path, such as `["File", "Open"]`, with optional app/window guards, requires one non-sensitive activatable item, and refuses missing or ambiguous paths. `seatgeist.activate_tab` finds a named non-sensitive AT-SPI tab with optional app/window guards, requires one viable match, and invokes select or press. `seatgeist.activate_link` finds a named non-sensitive AT-SPI link with optional app/window guards, requires one viable match, and invokes press or select. `seatgeist.clipboard_status` reports `wl-paste`, `wl-copy`, and KDE Klipper DBus availability plus selected read/write backends and a setup hint without reading clipboard contents. Clipboard reads are policy-gated and fail closed by default until the daemon is started with an explicit clipboard-read approval mode. `seatgeist.clipboard_get_text` defaults to `max_bytes = 65536`, preserves UTF-8 boundaries when truncating, supports `full = true` for an explicit unbounded read, and returns backend provenance such as `wl-clipboard` or `kde-klipper`. `seatgeist.clipboard_set_text` uses the same local backend preference and reports backend provenance in action summaries without echoing content. `seatgeist.a11y_quality_status` samples a bounded focused AT-SPI tree, reports AT-SPI availability, node/name/action/text/generic-role counts, whether the tree is flat, whether semantic targeting looks reliable, and a compact fallback label such as `atspi_semantic` or `screenshot_tile_or_structured_integration`. `seatgeist.a11y_focused_tree` returns role/name/value/states/bounds/action names/children with `depth` and `max_nodes` caps; AT-SPI text values are capped at 512 characters and password roles suppress values. `seatgeist.a11y_find` filters by role, accessible-name substring, app name, and containing window name with result/depth/node caps. `seatgeist.a11y_text_attributes` reads the `org.a11y.atspi.Text.GetAttributeRun` dictionary and start/end offsets for a non-sensitive text node and compact summaries report only range and attribute count. `seatgeist.a11y_invoke` invokes a normalized advertised action on an AT-SPI node id and is policy-gated as semantic control. `seatgeist.a11y_set_text` replaces non-sensitive `EditableText` contents, is capped at 8192 characters, and reports text length only in summaries. `seatgeist.a11y_insert_text` inserts UTF-8 text at a character offset on a non-sensitive `EditableText` node, passes the AT-SPI byte length to `InsertText`, is capped at 8192 characters, and reports text length plus offset only. `seatgeist.a11y_delete_text` deletes a character-offset range from a non-sensitive `EditableText` node without copying to clipboard, validates `end_offset > start_offset`, and reports offsets only. `seatgeist.a11y_copy_text` copies a character-offset range from a non-sensitive `EditableText` node into the system clipboard, validates `end_offset > start_offset`, does not read clipboard contents, and reports offsets only. `seatgeist.a11y_cut_text` cuts a character-offset range from a non-sensitive `EditableText` node into the system clipboard, validates `end_offset > start_offset`, does not read clipboard contents, and reports offsets only. `seatgeist.a11y_paste_text` pastes current system clipboard text at a character offset on a non-sensitive `EditableText` node, validates the offset, does not read clipboard contents, and reports offset only. `seatgeist.journal_tail` supports `limit`, `method`, and `ok` filters; entries include best-effort client pid/process-name metadata, safety class, guard presence, best-effort active-window context for control-class requests, and optional structured `control` metadata with action id, policy result, backend provenance, and redacted requested-target fields while keeping request payload text and screenshot contents out of the journal. MCP tool responses return compact text plus structured JSON from the daemon; screenshot and wait-for-change compact text include backend provenance, readiness compact text reports booleans/backend names/issue count, and clipboard compact text and journal summaries report lengths rather than echoing clipboard content.

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
the `atspi_event`, `target_read`, or `polling` backend, whether it is
target-scoped, and the non-content event class/member when present. If event
subscription is unavailable, a bounded target-node read/poll fallback is used;
it never retries through pointer or keyboard input.

`settle_condition=auto` resolves to `active_window_change` for
`seatgeist.focus_window`, `accessibility_change` through target-scoped events for
guarded high-level semantic actions, and `stable` polling otherwise. Explicit conditions are
`none`, `stable`, `active_window_change`, `accessibility_change`, and
`any_change`; timeout watchdog results remain successful actions with
`settled=false` and `timed_out=true`. Defaults are 1500 ms timeout and 100 ms
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
