# Configuration

`seatgeistd` reads an optional TOML config from:

```text
~/.config/seatgeist/config.toml
```

Use `--config <path>` or `SEATGEIST_CONFIG=<path>` to point at another file.

Daemon configuration parsing and resolution live in
`crates/seatgeistd/src/config.rs`. The module owns the deserialized file
schema, CLI-over-file precedence helpers, backend selection, policy/app
normalization, safety bounds, and screenshot-redaction settings; runtime
dispatch consumes only the resolved settings types.

Implemented fields:

```toml
[daemon]
socket = "$XDG_RUNTIME_DIR/seatgeist/seatgeistd.sock"
journal = "$XDG_STATE_HOME/seatgeist/journal.jsonl"
panic_stop_file = "$XDG_RUNTIME_DIR/seatgeist/panic-stop"
approval_file = "$XDG_RUNTIME_DIR/seatgeist/approvals.jsonl"
capture_restore_file = "$XDG_STATE_HOME/seatgeist/capture-restore.json"

[journal]
include_artifact_metadata = false
include_error_details = false

[backends]
input = "auto"

[backends.keymap]
rules = "evdev"
model = "pc105"
layout = "us"
variant = ""
options = ""

[policy]
default_observe = "allow"
default_control = "prompt"
destructive_actions = "prompt"
secret_fields = "deny"
default_clipboard_read = "prompt"
default_clipboard_write = "allow"
full_resolution_screenshot = "prompt"

[apps]
allow = ["org.kde.kate", "org.mozilla.firefox"]
deny = ["org.keepassxc.KeePassXC"]

[safety]
require_focus_guard = true
pause_on_human_input = false
human_input_activity_file = "$XDG_RUNTIME_DIR/seatgeist/human-input-active"
human_input_quiet_ms = 1500
control_rate_limit_per_minute = 120
preview_max_edge = 1600
tile_max_edge = 1600

[[safety.redact_regions]]
x = 0
y = 0
width = 640
height = 120
```

Path values can use `$XDG_RUNTIME_DIR`, `$XDG_STATE_HOME`, `$XDG_CONFIG_HOME`, and `$HOME`.

`[daemon].capture_restore_file`, `--capture-restore-file`, and
`SEATGEIST_CAPTURE_RESTORE_FILE` select the private ScreenCast restore-token
store. The default is `$XDG_STATE_HOME/seatgeist/capture-restore.json`. For a
window session with an exact requested KWin target, Seatgeist asks the portal
for explicitly-revoked persistence, stores only the latest rotated token, and
uses it on the next open after a daemon restart. The file is atomic, owned by
the daemon user, and inaccessible to group/other; its target key is hashed and
only an opaque reference appears in status output. Missing state simply opens
the ordinary portal chooser. Malformed, symlinked, foreign-owned, or broadly
readable state fails closed instead of silently weakening capture identity.

`[journal].include_artifact_metadata` defaults to `false`. When enabled, screenshot-bearing journal entries can include compact artifact records with the output path, byte count, and SHA-256 of the written file. This is useful for local audit correlation, but it is opt-in because paths can reveal local project/session details.

`[journal].include_error_details` defaults to `false`. Failed entries normally store only the structured error kind, preventing window titles, target labels, identifiers, or backend diagnostics from being copied into the journal summary. Enable it only for a private, short-lived diagnostic run when full daemon error text is necessary.

Precedence is:

1. CLI arguments and environment-backed daemon flags.
2. Config file values.
3. Built-in defaults.

Prompt-level policy decisions fail closed unless the daemon is started with `--approval-file <path>` / `SEATGEIST_APPROVAL_FILE=<path>` or `[daemon].approval_file`, and that file contains a matching unexpired grant. The approval file is JSONL, must be owned by the daemon uid, must be a regular file, and must not be readable, writable, or executable by group/other. Its parent directory must also be owned by the daemon uid and not group/other writable. Missing approval files mean no approval is present. Malformed or insecure approval files fail closed.

Create a short-lived method-scoped grant with:

```bash
seatgeist-cli approve --safety-class control-semantic --method focus_window --ttl-ms 60000
```

The default CLI approval-file path is `$XDG_RUNTIME_DIR/seatgeist/approvals.jsonl`; the daemon only reads it when explicitly configured to do so. Grant records include `safety_class`, `method`, `expires_unix_ms`, and optional `reason`; `method = "*"` is supported for deliberate class-wide local grants. A matching grant only satisfies a prompt decision. Explicit `deny`, app policy, panic-stop, human-input pause, active-window guard checks, and backend validation still run.

Explicit local approval flags such as `--allow-control`, `--allow-clipboard-read`, and `--allow-full-resolution-screenshot` still override file policy defaults for that daemon run. Prefer short-lived approval-file grants for narrow local use.

`[backends].input` controls the requested raw keyboard/pointer backend and can be `auto`, `uinput`, `portal_remote_desktop`, or `libei`. `--input-backend` / `SEATGEIST_INPUT_BACKEND` override the config file. Raw input commands resolve a daemon input-executor trait before side effects, and successful action summaries include backend provenance. `auto` and `uinput` use the uinput adapter. Explicit `portal_remote_desktop` and `libei` selections build EIS plans for UTF-8 text, modeled XKB text keysyms, named evdev or single-symbol key combos, and pointer requests, then execute currently wired daemon requests through the stored daemon EIS session only after `seatgeist-cli input remote-desktop-eis-start` has retained a session and the per-plan readiness gate finds a connected session, bound capabilities, selected portal devices, and a matching resumed EIS device. Without a stored session or ready selected device, those explicit EIS backends fail closed before side effects. Explicit EIS `key_combo` planning first uses the named evdev parser and then resolves unsupported single-character symbol parts through xkbcommon keymap lookup. `[backends.keymap]` can pin explicit xkbcommon RMLVO names (`rules`, `model`, `layout`, `variant`, and `options`) for that lookup. Empty `options = ""` is preserved as xkbcommon's "no options" value. When `[backends.keymap]` is omitted, explicit EIS key combos try KDE's current keyboard-layout DBus metadata first, then KDE `kxkbrc` layout config via `kreadconfig6`, then xkbcommon defaults. This precedence is implemented in the isolated daemon `keymap` module and covered by pure resolver tests, including malformed current-layout fallback. `auto`/`uinput` still use the stricter named evdev/US mapping path. Use `seatgeist-cli input remote-desktop-eis-session-status` and `seatgeist-cli input remote-desktop-eis-stop` to inspect or drop the retained session. Use `seatgeist-cli input backends` or MCP `seatgeist.input_backend_status` to see `configured_backend`, `preferred_available_backend`, `implemented_available_backend`, `eis_keymap`, and setup hints. Use `seatgeist-cli input remote-desktop-probe` or MCP `seatgeist.remote_desktop_session_probe` only as an explicit, policy-gated RemoteDesktop consent-path test; it closes the transient session and does not send input. Use `seatgeist-cli input remote-desktop-eis-probe` or MCP `seatgeist.remote_desktop_eis_probe` only when you also need to prove the `ConnectToEIS` handoff path; it starts the transient session, calls `ConnectToEIS`, reports compact libei runtime connected/event/bound-capability/resumed-device state, immediately closes the returned FD, and sends no input.

`[policy].destructive_actions` applies after ordinary control policy for requests marked destructive and for obvious destructive labels in high-level semantic controls, such as delete, remove, discard, quit, shutdown, and restart. The default is `prompt`, which requires a matching approval-file grant or explicit local allow policy.

`[policy].secret_fields` applies to high-level text-field requests whose target name looks secret-related, such as password, passcode, token, API key, private key, seed phrase, card number, or CVV. The default is `deny`. AT-SPI nodes already marked sensitive remain non-viable for semantic actions regardless of text-field name matching.

`[apps].deny` blocks control-class actions when the relevant app id matches. Deny rules win over allow rules. If `[apps].allow` is non-empty, control-class actions are allowed only for matching app ids. For focus requests, the daemon checks the target window app id; for keyboard, pointer, and semantic control, it checks the active window app id and fails closed if app policy is configured but the app id is unavailable.

When `[safety].require_focus_guard = true`, every control-class request must include an active-window guard before the daemon will run backend control. This is the built-in default; set it to `false` only for a tightly scoped local development daemon. Observe, status, policy, and journal requests are unaffected. The guard is still checked against the active window after this presence check.

When `[safety].pause_on_human_input = true`, the daemon checks `human_input_activity_file` before control-class requests. If the file exists and its mtime is newer than `human_input_quiet_ms`, control is refused before backend execution. This is a file-backed signal for a future KDE/libinput watcher; observe, status, policy, and journal requests are unaffected.

`make smoke-human-input-pause` verifies this path through a private configured daemon: it writes a fresh activity signal, checks `safety-status` freshness metadata, then confirms an approved focus-control request is denied and journaled before backend focus execution.

`[safety].control_rate_limit_per_minute` defaults to `120` and caps accepted control-class daemon requests over a rolling 60-second window. Set it to `0` only for a tightly scoped local development daemon. Observe, status, policy, and journal requests are unaffected, and denied preflight requests do not consume the control budget.

`[safety].preview_max_edge` and `[safety].tile_max_edge` default to `1600` and must be greater than zero. These values bound default screenshot previews, observe-attached screenshots, wait-for-change captures, and screenshot tiles on high-resolution displays. Per-request `max_edge` values can still override the configured defaults, and full-resolution screenshot requests remain explicit and separately policy-gated.

Use `seatgeist-cli readiness` or MCP `seatgeist.computer_use_readiness` for a compact computer-use preflight before attempting control. It aggregates safe status only, without screenshots, portal sessions, clipboard reads, or input. Use `seatgeist-cli safety-status` or MCP `seatgeist.safety_status` for detailed active safety gates; the response includes focus-guard enforcement, human-input pause state, whether the activity signal is currently fresh, the quiet interval, the optional signal-file path, the control rate limit, screenshot preview/tile max-edge defaults, the count of configured screenshot redaction regions without exposing redaction geometry, and whether opt-in journal artifact metadata is enabled. Use `seatgeist-cli desktop-session-status` or MCP `seatgeist.desktop_session_status` when diagnosing KDE, Wayland, DBus, portal, KWin, or AT-SPI setup; it reports sanitized session values and boolean DBus/runtime presence instead of raw paths.

`[[safety.redact_regions]]` entries define physical-pixel source screenshot rectangles. The daemon maps each rectangle through the screenshot transform and black-fills the matching output pixels before returning screenshot, screenshot-tile, observe screenshot, or wait-for-change outputs. Zero-size regions are ignored.

Prompt-level policy decisions fail closed when no matching unexpired approval-file grant is available.
