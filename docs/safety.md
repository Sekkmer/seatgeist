# Safety

Default rules:

- Observe is allowed; control defaults to prompt.
- Destructive semantic actions default to prompt and fail closed until a trusted approval channel or explicit local allow policy exists.
- Clipboard reads default to prompt.
- Clipboard writes default to allow, but still flow through policy and the action journal.
- Secret/password fields default to deny.
- Full-resolution screenshots default to prompt and require an explicit daemon approval mode before the backend capture path runs.
- Privileged input backends default to prompt.
- Focus guards should be supplied for pointer, keyboard, and semantic control actions whenever the caller has active-window context.
- Daemon requests are journaled as compact JSONL records with restrictive file permissions.
- Daemon requests pass through the policy engine before execution; prompt-level decisions fail closed until a trusted approval channel is implemented.
- The daemon reads optional path and policy defaults from `~/.config/plasma-pilot/config.toml` or `--config` / `PLASMA_PILOT_CONFIG`. Explicit CLI/env approval flags override file policy defaults for intentional local sessions.
- Configured app deny rules block control-class actions before backend execution. If an app allow list is configured, control-class actions require a matching app id; deny rules take precedence.
- `[policy].destructive_actions` applies to requests explicitly marked `destructive` and to high-level button/menu labels that obviously imply delete, remove, discard, quit, shutdown, restart, or similar state loss.
- `[policy].secret_fields` applies to high-level text-field targets whose names look secret-related, and defaults to deny.
- `[safety].require_focus_guard = true` makes active-window guards mandatory for control-class requests before backend execution.
- `[safety].pause_on_human_input = true` blocks control-class requests when the configured human-input activity file is newer than `human_input_quiet_ms`.
- `[[safety.redact_regions]]` physical-pixel rectangles are mapped through screenshot output transforms and black-filled before screenshots leave the daemon.
- Replay traces are not a policy bypass: `plasma-pilot-cli trace replay` resubmits each recorded daemon request through the normal daemon socket, so control and clipboard-read requests remain policy-checked and journaled.
- Panic-stop is file-backed and checked inside the daemon after policy approval but before control execution; when active, it blocks control-class requests even if the daemon was started with `--allow-control`.
- Current control requests can carry active-window guards. When supplied or required by config, the daemon checks the active window id, app id, and title substring before control execution and rejects stale guards.
- Keyboard input uses policy-gated `ControlKeyboard` requests, supports active-window guards, and journal/MCP summaries must report text length or key count rather than typed text.
- Pointer input uses policy-gated `ControlPointer` requests, requires explicit coordinate space for move/click tools, validates current physical-pixel coordinates against monitor-derived desktop bounds, supports active-window guards, and is blocked by panic-stop.
- `wait_for_change` is observe-class and stores only the caller-requested bounded screenshot output path plus compact delta metadata in journal/MCP summaries.
- Clipboard journal entries and compact MCP status text must not echo clipboard contents; they should report metadata such as text length.
- Clipboard read tools should be bounded by default and require an explicit full-read option for unbounded content.
- AT-SPI password-text nodes are marked sensitive and excluded from current semantic action candidates.
- AT-SPI invoke is semantic control: it may only run through the policy engine and action journal, and defaults to prompt/deny when no approval channel is available.
- AT-SPI set-text is semantic control: it must reject sensitive nodes by default and journal replacement length rather than replacement contents.
- High-level semantic actions must refuse ambiguous matches instead of choosing one candidate implicitly; ambiguity errors should include bounded non-sensitive candidate choices so the caller can disambiguate.

The current human-input pause uses a file-backed activity signal so a future KDE/libinput watcher can touch the file when the user takes over.
