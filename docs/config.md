# Configuration

`plasma-pilotd` reads an optional TOML config from:

```text
~/.config/plasma-pilot/config.toml
```

Use `--config <path>` or `PLASMA_PILOT_CONFIG=<path>` to point at another file.

Implemented fields:

```toml
[daemon]
socket = "$XDG_RUNTIME_DIR/plasma-pilot/plasma-pilotd.sock"
journal = "$XDG_STATE_HOME/plasma-pilot/journal.jsonl"
panic_stop_file = "$XDG_RUNTIME_DIR/plasma-pilot/panic-stop"

[policy]
default_observe = "allow"
default_control = "prompt"
destructive_actions = "prompt"
default_clipboard_read = "prompt"
default_clipboard_write = "allow"
full_resolution_screenshot = "prompt"

[apps]
allow = ["org.kde.kate", "org.mozilla.firefox"]
deny = ["org.keepassxc.KeePassXC"]

[safety]
require_focus_guard = false
pause_on_human_input = false
human_input_activity_file = "$XDG_RUNTIME_DIR/plasma-pilot/human-input-active"
human_input_quiet_ms = 1500

[[safety.redact_regions]]
x = 0
y = 0
width = 640
height = 120
```

Path values can use `$XDG_RUNTIME_DIR`, `$XDG_STATE_HOME`, `$XDG_CONFIG_HOME`, and `$HOME`.

Precedence is:

1. CLI arguments and environment-backed daemon flags.
2. Config file values.
3. Built-in defaults.

Explicit local approval flags such as `--allow-control`, `--allow-clipboard-read`, and `--allow-full-resolution-screenshot` override file policy defaults for that daemon run.

`[policy].destructive_actions` applies after ordinary control policy for requests marked destructive and for obvious destructive labels in high-level semantic controls, such as delete, remove, discard, quit, shutdown, and restart. The default is `prompt`, which fails closed until a trusted approval channel exists; set it to `allow` only for an intentional local session.

`[apps].deny` blocks control-class actions when the relevant app id matches. Deny rules win over allow rules. If `[apps].allow` is non-empty, control-class actions are allowed only for matching app ids. For focus requests, the daemon checks the target window app id; for keyboard, pointer, and semantic control, it checks the active window app id and fails closed if app policy is configured but the app id is unavailable.

When `[safety].require_focus_guard = true`, every control-class request must include an active-window guard before the daemon will run backend control. Observe, status, policy, and journal requests are unaffected. The guard is still checked against the active window after this presence check.

When `[safety].pause_on_human_input = true`, the daemon checks `human_input_activity_file` before control-class requests. If the file exists and its mtime is newer than `human_input_quiet_ms`, control is refused before backend execution. This is a file-backed signal for a future KDE/libinput watcher; observe, status, policy, and journal requests are unaffected.

`[[safety.redact_regions]]` entries define physical-pixel source screenshot rectangles. The daemon maps each rectangle through the screenshot transform and black-fills the matching output pixels before returning screenshot, screenshot-tile, observe screenshot, or wait-for-change outputs. Zero-size regions are ignored.

Prompt-level policy decisions still fail closed until a trusted approval channel is implemented.
