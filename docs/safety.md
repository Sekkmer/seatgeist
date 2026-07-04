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
- Clipboard journal entries and compact MCP status text must not echo clipboard contents; they should report metadata such as text length.
- Clipboard read tools should be bounded by default and require an explicit full-read option for unbounded content.
- AT-SPI password-text nodes must be marked sensitive before they are used for any future semantic control action.

The daemon should pause automation when panic-stop is active or when configured human-input detection indicates the user has taken over.
