# Backends

Preferred KDE Plasma 6 Wayland order:

1. Semantic AT-SPI actions when an accessible node is available.
2. KWin metadata through DBus or KWin scripting for window state, focus, scaling, and geometry.
3. xdg-desktop-portal ScreenCast/Screenshot and RemoteDesktop for supported consented capture/control flows.
4. libei where the compositor exposes a suitable emulated-input server path.
5. Controlled uinput virtual devices for privileged local fallback.
6. Custom KWin plugin, KDE patch, or kernel module only after a measured gap remains.

Every backend must report capabilities and provenance. The daemon should refuse ambiguous fallback behavior.

Safe capture capability probing is isolated in
`crates/seatgeistd/src/capture_diagnostics.rs`. It owns portal Screenshot and
ScreenCast introspection, v3 property decoding, KWin metadata availability,
Spectacle availability, backend preference, and compact setup hints without
starting capture. Process lookup/status/stdout probing shared by capture,
screenshot orchestration, readiness, and clipboard lives in
`crates/seatgeistd/src/commands.rs` rather than being reimplemented per
backend.

Current session diagnostics report sanitized KDE/Wayland environment facts through `seatgeist-cli desktop-session-status` and MCP `seatgeist.desktop_session_status`, including session type, desktop name, KDE session hints, display names, and boolean-only DBus/runtime-directory presence. Current capture diagnostics report xdg-desktop-portal Screenshot and ScreenCast interface visibility, the Screenshot interface version when readable, the Screenshot v3 `AvailableTargets` mask when exported, KWin `supportInformation` metadata availability, and Spectacle command fallback availability through `seatgeist-cli capture-backends` and MCP `seatgeist.capture_backend_status`. These probes do not start a portal session, request consent, capture pixels, or expose raw session bus paths. Capture status distinguishes `preferred_available_backend` from `implemented_available_backend`; when portal Screenshot is visible, full-screen screenshot execution uses the portal, follows the returned Request handle, bounds the Response wait, copies the returned screenshot URI into the requested PNG output, and then applies the same downscaling, 8K transform metadata, monitor metadata, redaction, and backend provenance as the previous command path. `seatgeist-cli screenshot` and MCP `seatgeist.screenshot` can pass an optional Screenshot v3 `portal_target` hint (`screen`, `window`, `area`, or `active_window`) only when `screenshot_target_option_supported=true` and the requested target is advertised by `AvailableTargets`; otherwise the daemon fails closed before falling back to Spectacle. KDE portal builds may currently expose Screenshot v2 even when the frontend specification documents v3; in that case `screenshot_target_option_supported=false`, `AvailableTargets` is absent, and Seatgeist keeps using the v2 full-screen contract instead of assuming active-window or region target support. `seatgeist-cli screenshot-tile` uses the same portal Screenshot source when available, then crops/downscales only the requested physical-pixel tile before applying redactions and transform metadata; if the portal backend fails before user cancellation and Spectacle is available, it falls back to Spectacle for the full-source intermediate. `seatgeist-portal` provides the tested boundary for the official Screenshot method contract: `org.freedesktop.portal.Desktop` at `/org/freedesktop/portal/desktop`, `Screenshot(parent_window, options) -> handle`, handle-token/path validation, Request `Response(response, results)` completion, screenshot `uri` extraction, file URI decoding, and zbus transport execution. Spectacle remains the compatibility fallback when portal Screenshot is unavailable or fails before a user response. Future KWin-native capture implementations should preserve this compact provenance surface before replacing either backend.

Backend-independent screenshot file processing lives in
`crates/seatgeistd/src/screenshot_image.rs`. It owns safe PNG destination
preparation, temporary full-resolution paths, bounded preview resize/copy,
tile validation and cropping, PNG dimension retry reads, transform-aware
redaction mapping, and physical pixel blacking. Portal, Spectacle, and
visible-window compatibility adapters share this boundary, including the
same scaling helper. Deterministic module tests cover resize and tile bounds,
redaction transforms and pixels, extension checks, and symlink refusal.

One-shot screenshot orchestration lives in
`crates/seatgeistd/src/screenshot.rs`. The module owns explicit source-mode
validation, Screenshot v3 target checks, portal/Spectacle selection and
fallback rules, tile-source lifecycle, capture metadata assembly, and the
bounded `wait_for_change` loop. Policy remains in the daemon dispatcher and
image mutation remains in `screenshot_image`; the wait interval now yields
through Tokio instead of blocking a daemon worker thread.

Current KWin window-list implementation keeps the `WindowsRunner.Match` plus `org.kde.KWin.getWindowInfo` command path as the compatibility baseline. It deduplicates repeated runner matches by stable KWin UUID and enriches each window with public `getWindowInfo` PID, app, title, and geometry metadata before merging updates from the packaged KWin script bridge. This public DBus PID path is the fallback when the KWin JavaScript API does not expose `window.pid`; target binding still fails closed when neither path supplies a PID. The daemon can fall back to the latest bridge list if the runner path fails after a bridge update. Current KWin focus implementation uses `org.kde.krunner1.Run` on KWin's `WindowsRunner` service with a window id previously discovered from `WindowsRunner.Match`. This is kept behind the window backend boundary and policy-gated as a control action. A future KWin script/plugin focus path remains a fallback if `WindowsRunner` proves unstable across Plasma versions.

Current clipboard text implementation uses the standard Wayland `wl-copy` and `wl-paste` commands first, then falls back to KDE Klipper DBus (`org.kde.klipper`) when the Wayland command backend is unavailable. Production execution is isolated in `crates/seatgeistd/src/clipboard.rs`; its Wayland and Klipper adapters implement the shared `seatgeist_backend::ClipboardBackend` trait, while the module owns selection, availability/status probing, UTF-8 bounding, and compact provenance. Policy enforcement remains in the daemon dispatcher before the trait-backed adapter is called. The daemon reports `clipboard_text` capability when it can both read and write through one of those local backends. `seatgeist-cli clipboard status` and MCP `seatgeist.clipboard_status` report `wl-paste`, `wl-copy`, and KDE Klipper DBus availability plus selected read/write backend names and setup hints without reading clipboard contents. Clipboard reads are policy-gated separately from clipboard writes, bounded to 64 KiB by default, and compact daemon/MCP summaries report only text length, truncation metadata, original byte count, and backend provenance. Future backend work should add portal clipboard integration behind the same trait if a suitable stable interface becomes available.

Current keyboard and pointer input implementation uses a localized Linux uinput backend in `seatgeist-uinput`. It creates short-lived virtual keyboard and pointer devices through `/dev/uinput` and uses `UI_DEV_SETUP` plus absolute-axis setup for pointer motion. Keyboard support covers US evdev key positions for ASCII text plus newline/tab using explicit key-code mappings, and named evdev key combos such as `Ctrl+L`, `Alt+F4`, and `Super+Space`. Pointer support covers absolute physical-pixel, global logical-pixel, and guarded active-window-local move/click/drag coordinates, one-click or double-click with left/middle/right buttons, bounded press-move-release drag with left/middle/right buttons, and vertical/horizontal wheel deltas. The daemon exposes keyboard through policy-gated `ControlKeyboard` requests and pointer through policy-gated `ControlPointer` requests; active panic-stop blocks both, and compact summaries report metadata rather than typed text or UI content. Raw input commands now resolve an `InputExecutionBackend` trait before backend execution; `auto` and `uinput` use the uinput adapter, explicit `portal_remote_desktop` and `libei` use the stored daemon EIS session when it is ready, and successful action summaries include backend provenance. The daemon reports `keyboard_input` and `pointer_input` capabilities only when the configured backend has an executable path: `/dev/uinput` can be opened read/write for uinput, or a stored EIS session is active and ready for explicit portal/libei. `seatgeist-cli input status` and MCP `seatgeist.uinput_status` expose the uinput access state with file metadata and setup hints, and `docs/uinput-setup.md` documents the optional udev and user-service path. `seatgeist-cli input backends` and MCP `seatgeist.input_backend_status` probe xdg-desktop-portal RemoteDesktop, KDE portal service visibility, libei client metadata/socket hints, uinput fallback availability, stored EIS session availability, and compact EIS keymap source metadata without starting a consent/session flow. Input status distinguishes an operator-requested `configured_backend` from the auto-detected `preferred_available_backend` and the currently executable `implemented_available_backend`. `[backends].input`, `--input-backend`, or `SEATGEIST_INPUT_BACKEND` can request `auto`, `uinput`, `portal_remote_desktop`, or `libei`; explicit portal/libei selections build EIS plans for text, named evdev or single-symbol key combos, and pointer requests, then execute them through the stored daemon EIS session only after policy, active-window, panic-stop, and per-plan readiness gates pass. Without a stored session or ready selected device, explicit EIS input fails closed before side effects instead of silently falling back to uinput. `seatgeist-portal` now includes a tested xdg-desktop-portal RemoteDesktop boundary for `CreateSession`, `SelectDevices`, `Start`, and `ConnectToEIS`: it models device-type bitmasks, persist modes, Request and Session handle paths, start/session response parsing, EIS FD return validation, a mockable lifecycle/EIS transport, a zbus lifecycle that pre-subscribes to expected Request responses before each portal call through `Start`, and same-connection zbus helpers that return an owned EIS FD. `seatgeist-cli input remote-desktop-probe` and MCP `seatgeist.remote_desktop_session_probe` expose the start lifecycle as an explicit control-class probe: it may open a portal consent dialog, supports device subset/restore/persist options plus active-window guards, reports only selected device and handle metadata, closes the transient session after the probe, and still does not call `ConnectToEIS` or send Notify*/EIS input. `seatgeist-cli input remote-desktop-eis-probe` and MCP `seatgeist.remote_desktop_eis_probe` extend that diagnostic by calling `ConnectToEIS` after `Start`, initializing a transient daemon EIS runtime from the returned FD, polling pending events, closing the runtime, and reporting compact runtime state without sending EIS or Notify* input. `seatgeist-cli input remote-desktop-eis-start`, `remote-desktop-eis-session-status`, and `remote-desktop-eis-stop` expose the retained daemon-owned EIS lifecycle used by explicit portal/libei raw input. `seatgeist-eis` models the libei sender event stream as typed, tested plans: start emulation, generated events, frame boundaries, key/button release before stop, and stop emulation. It covers UTF-8 text events, XKB-compatible text keysym events through xkbcommon conversion, evdev keyboard key events, absolute pointer motion, Linux input-event-codes button events, drag, and 120-unit discrete scroll. It also includes a tested xkbcommon keymap wrapper that builds an explicit RMLVO keymap, reads level-0 keysyms, and converts XKB keycodes to evdev keycodes with the documented 8-code offset; explicit EIS `key_combo` planning now preserves the named evdev parser first and then uses pinned `[backends.keymap]` RMLVO names, KDE current-layout DBus metadata, KDE `kxkbrc` config via `kreadconfig6`, or xkbcommon defaults for unsupported single-character symbol parts such as `Ctrl+;`. It also exposes an `EisEventSink` boundary plus a guarded `LibeiDeviceSink` FFI adapter for translating a validated plan into libei sender calls once a caller-owned context has selected a resumed capable device. Its selector requires a resumed device with every plan capability and, for absolute pointer plans, a virtual-device region covering all target coordinates; it rejects paused devices, missing capabilities, out-of-region targets, cross-region drags, and physical absolute devices until explicit physical-unit mapping exists. `seatgeist-cli input pointer-calibration` and MCP `seatgeist.pointer_calibration` report the physical-pixel pointer bounds and per-monitor physical origins derived from KWin monitor metadata before any pointer action is attempted. Pointer coordinates require explicit coordinate space. `physical_pixel` coordinates are validated against monitor-derived physical desktop bounds, including scaled 8K layouts. Global `logical_pixel` coordinates map through monitor logical origins and scale factors before the same physical bounds check. `window_local` coordinates are active-window-relative logical pixels, require an active-window guard, require active-window geometry from the KWin bridge/fallback path, and are mapped to physical pixels through the active window's monitor scale before input execution. `make smoke-gui-input` validates a guarded uinput click/type/save flow in a disposable KWrite/Kate document when run in the intended KDE session, while `make gui-eval-remote-desktop-eis-session` validates the opt-in live retained EIS path with minimal scroll and `Shift` key-combo attempts when the portal returns a ready session. Future backend work should broaden live portal/libei eval coverage before expanding additional coordinate spaces.

Safe input probing and backend-selection reporting live in
`crates/seatgeistd/src/input_diagnostics.rs`. The module owns `/dev/uinput`
metadata/access checks, RemoteDesktop portal introspection, libei metadata and
socket visibility, configured/preferred/implemented selection, and setup
hints. It accepts only the retained-session active bit and resolved XKB status;
the retained portal/EIS lifecycle and all policy-gated input execution remain
outside this read-only boundary.

RemoteDesktop probing and retained EIS ownership are split across two focused
daemon modules. `crates/seatgeistd/src/portal_eis_probe.rs` owns transient
portal/EIS probes, request option validation, device metadata, and bounded
timeouts. `crates/seatgeistd/src/portal_eis_session.rs` owns the daemon-retained
session metadata, mutex-backed store, start/status/stop lifecycle, and
ready-plan execution. The dispatcher continues to apply policy, panic-stop,
and active-window guards before either control path; the modules do not create
an alternate input route.

Raw backend selection and execution adapters live in
`crates/seatgeistd/src/input_execution.rs`. The module owns the shared executor
trait, uinput adapter, stored-session EIS adapter, and the common EIS plan
translation used by keyboard and pointer actions. Named evdev parsing plus XKB
single-symbol fallback is isolated in `crates/seatgeistd/src/eis_key_combo.rs`.
Request validation, coordinate resolution, policy, focus guards, panic-stop,
human-activity checks, and journaling remain outside these adapters, so
selecting a backend cannot bypass the daemon's control gates.

The six raw keyboard and pointer action handlers are grouped in
`crates/seatgeistd/src/input_actions.rs`. This layer owns request-local bounds,
coordinate-context loading, one backend call, and compact `ActionResult`
construction. It deliberately does not own authorization, sticky-session
focus acquisition, capture ownership, human-activity arbitration, rate limits,
or journal writes; those remain ordered in the central request dispatcher
before and after the handler call.

Pointer coordinate preparation now lives in the daemon's
`pointer_coordinates` module. Each action loads monitor metadata through the
injected `ScreenBackend`; window-local actions read the active window through
the injected `WindowBackend` only after sticky focus acquisition and
verification. A move or click uses one coordinate context, and both drag
endpoints share one monitor/active-window snapshot, avoiding duplicate
round-trips and geometry races. Calibration uses the same monitor path, and
visible-window crop reuses the module's scaled-origin calculation.

Daemon-side RMLVO configuration, KDE layout discovery, normalization, source
selection, and compact status now live in
`crates/seatgeistd/src/keymap.rs`. Live `qdbus6`/`kreadconfig6` collection is
separate from the pure resolver, whose tests prove explicit-config priority,
current KDE layout priority, `kxkbrc` fallback, invalid-current-layout
handling, and the final xkbcommon-default path. Input execution consumes only
the resolved settings, keeping layout probing out of the dispatcher.

`seatgeist-eis` now also has a live sender context boundary: `LibeiSenderContext` takes ownership of a portal-returned EIS FD, configures the libei sender name, exposes the libei event FD for polling, dispatches pending events, binds the intersection of plan-required and seat-available capabilities on `SeatAdded`, snapshots connect/seat/device/resume/pause/remove events into compact device metadata, retains refcounted resumed libei device handles until pause/remove/seat removal/disconnect, and is marked `Send` for daemon mutex-serialized ownership transfer. `EisRuntimeState` consumes those snapshots, tracks connection/seat/bound-capability state plus the current device list, applies pause/remove/disconnect events, selects a resumed device for a plan from live-style state, and exposes stricter execution readiness that also requires a connected session plus every plan-required capability bound by the EIS seat. `EisSessionRuntime` wraps an event source plus runtime state so daemon-owned EIS sessions can poll, update state, report planning or execution readiness, and hand a ready plan to a selected-device executor only after readiness passes. The live `LibeiSenderContext` selected-device executor applies validated UTF-8 text, XKB text-keysym, evdev keyboard, pointer, button, and scroll plans only to retained selected devices and fails closed if the selected runtime device is no longer retained. The daemon now has a `DaemonPortalEisSession` wrapper that preserves portal session metadata while owning an EIS runtime, a mutex-backed single-session store with start/status/stop daemon protocol, plus a tested session-backed input executor that maps daemon text/keyboard/pointer requests to EIS plans and calls the ready selected-device executor only after the runtime readiness gate passes. The transient EIS probe initializes a session wrapper, polls, drops it, and still sends no input. Explicit portal/libei raw-input selections now use the stored daemon EIS session after policy, panic-stop, active-window guard, and per-plan readiness checks. CLI/MCP wrappers expose stored-session start, status, and stop; explicit EIS key combos now use the XKB-to-evdev translator with configured, KDE-discovered, or default keymaps for single-symbol fallback.

Current AT-SPI implementation discovers the accessibility bus through `org.a11y.Bus.GetAddress`, then queries the separate accessibility bus with `busctl --address`. It walks application roots from `/org/a11y/atspi/accessible/root`, detects the focused node from the installed AT-SPI state bitset, and returns bounded compact role/name/value/state/bounds/action metadata. Text values use `org.a11y.atspi.Text.GetText` with a 512-character cap, scalar values use `org.a11y.atspi.Value.CurrentValue`, and password roles suppress value reads. Text-attribute inspection uses `org.a11y.atspi.Text.GetAttributeRun(offset, includeDefaults)` on non-sensitive text nodes and reports compact range/count summaries. The same traversal can find nodes by role, accessible-name substring, application name, and containing frame/dialog/window name. Policy-gated semantic invoke resolves a normalized action to the node's advertised `org.a11y.atspi.Action.GetActions` index and calls `DoAction(index)`. Policy-gated set-text requires a non-sensitive node exposing `org.a11y.atspi.EditableText`, replaces contents with `SetTextContents`, and caps replacement text at 8192 characters. Policy-gated insert-text requires the same non-sensitive `EditableText` interface, inserts UTF-8 text at a character offset with the AT-SPI byte length, and journals only text length plus offset. Policy-gated delete-text requires the same non-sensitive `EditableText` interface, deletes a character-offset range without copying to the clipboard, and journals only offsets. Policy-gated copy-text and cut-text require the same non-sensitive `EditableText` interface, call `CopyText(start, end)` or `CutText(start, end)` so the application writes the selected range to the system clipboard, do not read clipboard contents, and journal only offsets. Policy-gated paste-text requires the same non-sensitive `EditableText` interface, calls `PasteText(position)` so the application inserts current system clipboard text, does not read clipboard contents, and journals only the offset. Policy-gated set-value requires a non-sensitive node exposing `org.a11y.atspi.Value`, writes its `CurrentValue` double property, and is currently limited to slider, spin button, scrollbar, and dial roles. High-level `click_button`, `set_text_field`, `focus_text_field`, `select_menu`, `activate_tab`, `activate_link`, `toggle_check`, `set_value`, and `select_item` use the same AT-SPI find/action path and refuse zero, sensitive, non-viable, or ambiguous matches. Ambiguity choices include a 1-based choice index, deterministic candidate id derived from semantic fields rather than volatile AT-SPI node id, deterministic name-match score, raw node id, role, accessible name, and action metadata so callers can make a narrower follow-up request without Seatgeist choosing implicitly. `focus_text_field` requires the matched text field to advertise a focus action and invokes that action without mutating text, which gives keyboard-entry workflows a semantic focus step before falling back to pointer clicks. `activate_link` targets AT-SPI `link` role nodes and invokes press or select. `toggle_check` can take an optional desired checked state and skips the AT-SPI action when the current checked/selected state already matches. `select_item` targets list items, tree items, table rows, combo boxes, options, and menu-item-like choices with select or press actions. `select_menu` currently requires the menu path to be visible in AT-SPI; future work can add a multi-step open-then-select flow with the same ambiguity rules. The backend trait now covers focused-tree reads, find, text-attribute reads, invoke, set-text, insert-text, delete-text, copy-text, cut-text, paste-text, and numeric set-value so mock and future native zbus/libatspi implementations can share the same boundary. This command-backed implementation is intentionally isolated inside `seatgeist-atspi`; a future native zbus/libatspi backend should preserve the same daemon protocol and add richer edit operations such as run-attribute-aware editing when needed.

Focused-tree discovery and named, app-filtered, depth-zero lookups first request the application's standard
`org.a11y.atspi.Cache.GetItems` snapshot over its direct application bus. The
backend reads focus-state bits or filters names from that bulk snapshot, queries detailed role/action and
component data only for candidate nodes, and follows cached parents to recover
the containing window required for PID/title correlation. Applications without
a usable cache, malformed cache responses, and queries requiring deeper or
containing-window-name traversal retain the bounded recursive `Accessible`
fallback. The fast path therefore reduces bus/process round trips without
weakening semantic authorization. Cache responses have an independent 65,536
item safety cap. When an explicit application filter matches multiple running
instances, each matching application receives its own `max_nodes` detail-query
budget so a large long-lived browser cannot starve a second disposable window;
bulk name prefiltering does not consume that detail budget.

Firefox can expose a single button Action entry whose name, description, and
keybinding are all the placeholder `;;`. Seatgeist maps this to `press` only
when the resolved role is `button` or `push button` and exactly one unlabeled
action exists. Other roles, multiple unlabeled actions, and non-press requests
remain unavailable; `DoAction(0)` still runs only after normal semantic policy,
identity, sensitivity, and ambiguity checks.
The high-level generic `button` query also accepts Qt's standard `push button`
role spelling; it does not broaden to toggle buttons or other control roles.

Policy-gated caret and selection control use the documented `org.a11y.atspi.Text` interface rather than raw keyboard input. `a11y_set_caret` calls `SetCaretOffset(offset)`, and `a11y_set_selection` calls `SetSelection(selectionNum, start, end)` for an existing text selection index. Both reject sensitive roles, validate offsets/ranges, and journal only offsets. The current D-Bus `org.a11y.atspi.EditableText` method set covers contents, insert, copy, cut, delete, and paste; run-attribute mutation remains future work until a supported interface is identified.

## KWin Script Bridge

The repository includes `kwin/seatgeist-bridge`, a packaged KWin script that publishes active-window metadata to the user-session daemon over the session bus:

- DBus service: `org.seatgeist.KWinBridge`
- DBus path: `/org/seatgeist/KWinBridge1`
- DBus interface: `org.seatgeist.KWinBridge1`
- Methods: `UpdateActiveWindow(payload: string)` and `UpdateWindows(payload: string)`

The same interface also carries the bounded compositor-control half of the bridge through `RegisterActionCapabilities(capabilities: string)`, `TakePendingAction() -> string`, `AcknowledgeAction(id: string)`, and `CompleteAction(payload: string)`. The daemon advertises resize, move, and launch only after the currently running script registers the corresponding capability; an installed but stale script therefore fails immediately with an actionable setup error. Resize and move resolve an exact stable KWin id, reject special or unsupported windows, update writable `frameGeometry`, and acknowledge actual geometry. Launch uses a two-phase one-shot intent: KWin acknowledges that desktop-entry matching, work-area anchoring, and activation policy are armed before the daemon invokes validated `gtk-launch`; after `windowAdded`, KWin applies and rechecks the placement and reports whether focus was preserved. Failed daemon launches enqueue cancellation and every remaining intent has a bounded expiry. The shared `WindowBackend` trait exposes direct resize/move independently so future Wayland/X11/mock implementations can coexist; desktop-entry launch remains a daemon/KWin transaction because it must be armed before process activation.

The active-window payload is compact JSON containing active state, stable KWin window id, title, app id, pid, and logical window geometry. The window-list payload contains a compact `windows` array with the same non-active window fields. The daemon keeps the latest updates in memory, serves active-window state through `seatgeist-cli active-window`, and merges bridge window-list metadata into `seatgeist-cli windows`/MCP `seatgeist.list_windows` while preserving the `WindowsRunner` fallback.

The script publishes immediately at startup and on KWin window
activation/add/remove events. It also sends the same snapshot every two
seconds through KWin's script-exposed `QTimer`, because KWin's asynchronous
`callDBus` API does not notify the script when a previously absent destination
service returns. This heartbeat reseeds active-window and window-list state
after a daemon-only restart without focusing a window. `SnapshotIntervalMs` in
the script's KWin configuration group can tune the interval from 250 to 60000
milliseconds. If a KWin implementation does not expose `QTimer`, startup and
event publishing remain the documented compatibility fallback, but first-class
resize is unavailable because there is no safe daemon-to-script action poll.
The action poll runs every 50 milliseconds, permits concurrent daemon callers
through opaque action ids, and fails closed after a bounded acknowledgment
timeout rather than falling back to pointer dragging or a private KWin DBus
method.

The daemon-side DBus receiver, payload validation, synchronized
active-window/window-list snapshots, package/config discovery, `kwinrc`
enabled-state parsing, and compact status assembly are isolated in
`crates/seatgeistd/src/kwin_bridge.rs`. Request dispatch consumes only its
clonable state handles and status function, keeping KWin wire and installation
details out of the daemon entry point while preserving the existing fallback
and policy boundaries. Shared XDG data/config home resolution lives in the
small `crates/seatgeistd/src/xdg.rs` module so daemon config discovery and the
KWin package probe use one fallback contract.

`seatgeist-cli kwin-bridge-status` and MCP `seatgeist.kwin_bridge_status` report the daemon DBus receiver state, registered `window_resize_supported` state, active-window update state, window-list update state, latest bridge window count, and the user-local package/config paths checked for persistent installation.

`make validate-kwin-bridge` runs the bridge in a mocked KWin JavaScript
runtime and verifies initial, event, restart-heartbeat, interval-bound,
timer-unavailable, and acknowledged resize behavior without touching the live
desktop.

Install or update the script explicitly with:

```bash
make install-kwin-script
```

The target delegates to `scripts/install-kwin-bridge.py`. It installs or
updates the package, persists the enabled flag, and, when a live KWin scripting
service is present, unloads and reloads only `seatgeist-bridge` before running
the new instance. This targeted refresh matters because replacing the package
and calling the general KWin reconfigure method does not guarantee that an
already-loaded JavaScript instance is replaced. Outside a live KDE session,
installation still succeeds and loading is explicitly deferred to the next
session. `scripts/test-install-kwin-bridge.py` verifies installed/not-installed,
already-loaded/not-loaded, and offline paths with fake commands.

Do not make this target part of normal verification because it mutates the user's KWin configuration.

Ordinary list-window, active-window, and direct-focus requests use the shared
`WindowBackend` trait. The production `KwinWindowBackend` owns KWin runner and
bridge-state merging plus monitor correlation, and delegates its focus method
to the internal KWin executor. Direct focus and sticky raw-input leases both
call the shared window backend; neither request handler nor interaction
transaction calls KWin directly. Model-facing desktop observations and polling
post-action observations also read window state through this injected backend. Their
assembly, compact accessibility projection, and revision generation live in
the dedicated daemon `observation` module. Pre-execution active-window guards
and app-policy target reads also consume `WindowBackend`; their fail-closed
validation is isolated in `window_safety`, after policy, panic stop, trusted
human-input pause, ownership, and required-guard checks and before rate-limit
acceptance or any action. Journal before/after context remains a best-effort
synchronous bridge snapshot and is not an authorization input. The current
`KwinFocusBackend` uses KWin's `WindowsRunner` through `qdbus6`, with the
existing KWin bridge as the independent active-window confirmation source for
sticky focus leases. There is intentionally no silent X11, portal, or other
focus fallback: if the KWin command is missing, rejected, fails, or cannot be
confirmed before the bounded lease deadline, the transaction fails closed
before input. Policy remains in the dispatcher, and every direct or internal
focus operation remains journaled. Internal focus-policy, focus, and
verification records share the lease action id with the raw-input request.
Sticky target binding, session renewal, status invalidation, pre-action
re-resolution, active-target verification, cooperative restoration, and
post-action target validation all consume the same `WindowBackend`. Its
`backend_name` is recorded for focus and verification journal steps, so a mock
or future fallback cannot be mislabeled as KWin.
All nine guarded high-level semantic actions also obtain their KWin correlation
set through `WindowBackend` before PID, title, application, target-window, and
app-policy validation. The async authorization helper lives with the pure
identity resolver in `target.rs`; unguarded semantic actions retain their
explicit uncorrelated behavior and do not perform a window lookup.
Future KWin-native, X11, or mock window and focus implementations can be added
behind these traits without changing daemon protocol or weakening policy and
journal requirements.

Trusted human-input provenance is provided by the separately compiled
`kwin/seatgeist-activity` binary plugin and the daemon's `ActivityTracker`, as
documented in `docs/human-input-activity.md`. KWin's binary plugin interface is
versioned to the exact compositor release, so the helper must be rebuilt after
KWin upgrades. When it is missing or incompatible, activity provenance is
reported unavailable and cooperative restoration is disabled; Seatgeist does
not reinterpret the JavaScript cursor signal, idle time, or the legacy file as
trusted provenance.

`make kwin-activity-preflight` compares the plugin's embedded KWin factory IID,
the installed `libkwin` ABI, the ABI mapped into the active compositor process,
plugin installation hash, and KWin's available/loaded plugin lists. This avoids
attempting a dynamic load across KWin upgrades. The running compositor exposes
`org.kde.KWin.Plugins.LoadPlugin`, but it is used only after installation and
an exact ABI match; otherwise a normal session restart is required.
