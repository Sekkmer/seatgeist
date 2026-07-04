# Safety

Default rules:

- Observe is allowed; control defaults to prompt.
- Clipboard reads default to prompt.
- Clipboard writes default to allow, but still flow through policy and the action journal.
- Secret/password fields default to deny.
- Full-resolution screenshots default to prompt.
- Privileged input backends default to prompt.
- Focus guards are required for normal control actions.
- Daemon requests are journaled as compact JSONL records with restrictive file permissions.
- Daemon requests pass through the policy engine before execution; prompt-level decisions fail closed until a trusted approval channel is implemented.
- Replay traces are not a policy bypass: `plasma-pilot-cli trace replay` resubmits each recorded daemon request through the normal daemon socket, so control and clipboard-read requests remain policy-checked and journaled.
- Clipboard journal entries and compact MCP status text must not echo clipboard contents; they should report metadata such as text length.
- Clipboard read tools should be bounded by default and require an explicit full-read option for unbounded content.
- AT-SPI password-text nodes must be marked sensitive before they are used for any future semantic control action.
- AT-SPI invoke is semantic control: it may only run through the policy engine and action journal, and defaults to prompt/deny when no approval channel is available.
- AT-SPI set-text is semantic control: it must reject sensitive nodes by default and journal replacement length rather than replacement contents.
- High-level semantic actions must refuse ambiguous matches instead of choosing one candidate implicitly.

The daemon should pause automation when panic-stop is active or when configured human-input detection indicates the user has taken over.
