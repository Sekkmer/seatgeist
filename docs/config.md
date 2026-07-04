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
```

Path values can use `$XDG_RUNTIME_DIR`, `$XDG_STATE_HOME`, `$XDG_CONFIG_HOME`, and `$HOME`.

Precedence is:

1. CLI arguments and environment-backed daemon flags.
2. Config file values.
3. Built-in defaults.

Explicit local approval flags such as `--allow-control`, `--allow-clipboard-read`, and `--allow-full-resolution-screenshot` override file policy defaults for that daemon run. App allow/deny lists, destructive-action policy, and sensitive-region screenshot redaction are not implemented yet.
