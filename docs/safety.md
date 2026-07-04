# Safety

Default rules:

- Observe is allowed; control defaults to prompt.
- Clipboard reads default to prompt.
- Secret/password fields default to deny.
- Full-resolution screenshots default to prompt.
- Privileged input backends default to prompt.
- Focus guards are required for normal control actions.
- Daemon requests are journaled as compact JSONL records with restrictive file permissions.

The daemon should pause automation when panic-stop is active or when configured human-input detection indicates the user has taken over.
