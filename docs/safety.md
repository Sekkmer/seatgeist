# Safety

Default rules:

- Observe is allowed; control defaults to prompt.
- Destructive semantic actions default to prompt and require a matching approval-file grant or explicit local allow policy.
- Clipboard reads default to prompt.
- Clipboard writes default to allow, but still flow through policy and the action journal.
- Secret/password fields default to deny.
- Full-resolution screenshots default to prompt and require a matching approval-file grant or explicit daemon approval mode before the backend capture path runs.
- Privileged input backends default to prompt.
- Focus guards should be supplied for pointer, keyboard, and semantic control actions whenever the caller has active-window context.
- Daemon requests are journaled as compact JSONL records with restrictive file permissions. Journal entries include safety class and guard/context metadata, but must not store raw request payload text or screenshot contents.
- Daemon requests pass through the policy engine before execution; prompt-level decisions fail closed unless a configured approval file contains a matching unexpired method/safety-class grant.
- Accepted control-class requests are rate-limited by `[safety].control_rate_limit_per_minute`, which defaults to `120` over a rolling 60-second window. Observe/status requests are not counted.
- The daemon reads optional path and policy defaults from `~/.config/plasma-pilot/config.toml` or `--config` / `PLASMA_PILOT_CONFIG`. Explicit CLI/env approval flags override file policy defaults for intentional local sessions.
- Approval files are opt-in through `--approval-file`, `PLASMA_PILOT_APPROVAL_FILE`, or `[daemon].approval_file`; the daemon rejects approval files that are not regular files, not owned by the daemon uid, readable/writable/executable by group or other, or located in a parent directory writable by group or other.
- Configured app deny rules block control-class actions before backend execution. If an app allow list is configured, control-class actions require a matching app id; deny rules take precedence.
- `[policy].destructive_actions` applies to requests explicitly marked `destructive` and to high-level button/menu labels that obviously imply delete, remove, discard, quit, shutdown, restart, or similar state loss.
- `[policy].secret_fields` applies to high-level text-field targets whose names look secret-related, and defaults to deny.
- `[safety].require_focus_guard = true` makes active-window guards mandatory for control-class requests before backend execution.
- `[safety].pause_on_human_input = true` blocks control-class requests when the configured human-input activity file is newer than `human_input_quiet_ms`.
- `[[safety.redact_regions]]` physical-pixel rectangles are mapped through screenshot output transforms and black-filled before screenshots leave the daemon.
- Replay traces are not a policy bypass: `plasma-pilot-cli trace validate` checks trace structure without contacting the daemon, and `plasma-pilot-cli trace replay` resubmits each recorded daemon request through the normal daemon socket, so control and clipboard-read requests remain policy-checked and journaled. Trace steps can assert expected error-message substrings for fail-closed paths and JSON-pointer equality for compact response metadata without exposing request payload contents or full response payloads in the replay summary; validation rejects empty/duplicate labels and error expectations that conflict with a non-error response type or `ok=true`. Use `trace validate --dir` or `make validate-traces` for the daemon-free validation pass over every checked-in trace; empty trace sets fail instead of passing vacuously. Use `examples/traces/status-smoke.json` for a safe status-only validation/replay smoke, `examples/traces/policy-denials-smoke.json` to verify protected full-resolution screenshot, clipboard-read, and focus-control requests still fail closed before backend side effects, and `examples/traces/panic-stop-smoke.json` to verify private-daemon panic-stop state transitions.
- Panic-stop is file-backed and checked inside the daemon after policy approval but before control execution; when active, it blocks control-class requests even if the daemon was started with `--allow-control`.
- Current control requests can carry active-window guards. When supplied or required by config, the daemon checks the active window id, app id, and title substring before control execution and rejects stale guards.
- Keyboard input uses policy-gated `ControlKeyboard` requests, supports active-window guards, and journal/MCP summaries must report text length or key count rather than typed text.
- Pointer input uses policy-gated `ControlPointer` requests, requires explicit coordinate space for move/click/drag tools, validates resolved physical-pixel coordinates against monitor-derived desktop bounds, supports active-window guards, and is blocked by panic-stop. `window_local` pointer coordinates are active-window-relative and always require an active-window guard before backend execution.
- `wait_for_change` is observe-class and stores only the caller-requested bounded screenshot output path plus compact delta metadata in journal/MCP summaries.
- Clipboard journal entries and compact MCP status text must not echo clipboard contents; they should report metadata such as text length.
- Clipboard read tools should be bounded by default and require an explicit full-read option for unbounded content.
- AT-SPI password-text nodes are marked sensitive and excluded from current semantic action candidates.
- AT-SPI invoke is semantic control: it may only run through the policy engine and action journal, and defaults to prompt/deny when no matching approval grant is available.
- AT-SPI set-text is semantic control: it must reject sensitive nodes by default and journal replacement length rather than replacement contents.
- AT-SPI insert-text is semantic control: it must reject sensitive nodes by default and journal inserted-text length plus offset rather than inserted contents.
- AT-SPI delete-text is semantic control: it must reject sensitive nodes by default and journal only the deleted offset range, not deleted contents.
- AT-SPI copy-text and cut-text are semantic control: they must reject sensitive nodes by default, must not read clipboard contents after writing the selected range to the system clipboard, and must journal only offset ranges.
- AT-SPI paste-text is semantic control: it must reject sensitive nodes by default, must not read clipboard contents for the paste operation, and must journal only the paste offset.
- High-level semantic actions, including list/item selection, must refuse ambiguous matches instead of choosing one candidate implicitly; ambiguity errors should include bounded non-sensitive candidate choices so the caller can disambiguate.

The current human-input pause uses a file-backed activity signal so a future KDE/libinput watcher can touch the file when the user takes over.
