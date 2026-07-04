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
default_clipboard_read = "prompt"
default_clipboard_write = "allow"
full_resolution_screenshot = "prompt"

[apps]
allow = ["org.kde.kate", "org.mozilla.firefox"]
deny = ["org.keepassxc.KeePassXC"]

[safety]
require_focus_guard = false
```

Path values can use `$XDG_RUNTIME_DIR`, `$XDG_STATE_HOME`, `$XDG_CONFIG_HOME`, and `$HOME`.

Precedence is:

1. CLI arguments and environment-backed daemon flags.
2. Config file values.
3. Built-in defaults.

Explicit local approval flags such as `--allow-control`, `--allow-clipboard-read`, and `--allow-full-resolution-screenshot` override file policy defaults for that daemon run.

`[apps].deny` blocks control-class actions when the relevant app id matches. Deny rules win over allow rules. If `[apps].allow` is non-empty, control-class actions are allowed only for matching app ids. For focus requests, the daemon checks the target window app id; for keyboard, pointer, and semantic control, it checks the active window app id and fails closed if app policy is configured but the app id is unavailable.

When `[safety].require_focus_guard = true`, every control-class request must include an active-window guard before the daemon will run backend control. Observe, status, policy, and journal requests are unaffected. The guard is still checked against the active window after this presence check.

Destructive-action policy and sensitive-region screenshot redaction are not implemented yet.
