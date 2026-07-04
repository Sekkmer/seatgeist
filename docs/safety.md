# Safety

Default rules:

- Observe is allowed; control defaults to prompt.
- Clipboard reads default to prompt.
- Clipboard writes default to allow, but still flow through policy and the action journal.
- Secret/password fields default to deny.
- Full-resolution screenshots default to prompt.
- Privileged input backends default to prompt.
- Focus guards should be supplied for pointer, keyboard, and semantic control actions whenever the caller has active-window context.
- Daemon requests are journaled as compact JSONL records with restrictive file permissions.
- Daemon requests pass through the policy engine before execution; prompt-level decisions fail closed until a trusted approval channel is implemented.
- Replay traces are not a policy bypass: `plasma-pilot-cli trace replay` resubmits each recorded daemon request through the normal daemon socket, so control and clipboard-read requests remain policy-checked and journaled.
- Panic-stop is file-backed and checked inside the daemon after policy approval but before control execution; when active, it blocks control-class requests even if the daemon was started with `--allow-control`.
- Current control requests can carry active-window guards. When supplied, the daemon checks the active window id, app id, and title substring before control execution and rejects stale guards.
- Keyboard input uses policy-gated `ControlKeyboard` requests, supports active-window guards, and journal/MCP summaries must report text length or key count rather than typed text.
- Pointer input uses policy-gated `ControlPointer` requests, requires explicit coordinate space for move/click tools, validates current physical-pixel coordinates against monitor-derived desktop bounds, supports active-window guards, and is blocked by panic-stop.
- `wait_for_change` is observe-class and stores only the caller-requested bounded screenshot output path plus compact delta metadata in journal/MCP summaries.
- Clipboard journal entries and compact MCP status text must not echo clipboard contents; they should report metadata such as text length.
- Clipboard read tools should be bounded by default and require an explicit full-read option for unbounded content.
- AT-SPI password-text nodes must be marked sensitive before they are used for any future semantic control action.
- AT-SPI invoke is semantic control: it may only run through the policy engine and action journal, and defaults to prompt/deny when no approval channel is available.
- AT-SPI set-text is semantic control: it must reject sensitive nodes by default and journal replacement length rather than replacement contents.
- High-level semantic actions must refuse ambiguous matches instead of choosing one candidate implicitly.

The daemon should pause automation when configured human-input detection indicates the user has taken over.
