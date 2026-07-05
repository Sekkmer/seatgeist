# Threat Model

Primary assets:

- Signed-in browser and desktop sessions.
- Clipboard contents and visible private data.
- Shells, editors, source repositories, and system settings.
- Desktop focus, keyboard input, pointer input, and screenshots.

Primary mitigations:

- Unix socket under `$XDG_RUNTIME_DIR/seatgeist/` with restrictive permissions.
- Policy checks before every control, clipboard, full-resolution screenshot, or privileged backend action.
- Focus/window guards for pointer and keyboard actions.
- Panic-stop support.
- JSONL action journal without storing screenshot payloads by default.
- Downscaled or tiled screenshot defaults for 8K displays.
