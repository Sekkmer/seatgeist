# Seatgeist Agent Instructions

This repository builds Seatgeist, a KDE Plasma desktop-control substrate for Codex.

Rules for AI agents:

1. Keep the project compiling after every change.
2. Prefer small vertical slices over large unfinished rewrites.
3. Never add unsafe desktop-control behavior without policy checks.
4. All input actions must flow through the policy engine.
5. All actions must be journaled.
6. Do not hardcode one KDE private API without documenting fallback behavior.
7. Use traits for backends so KDE/Wayland/X11/mock implementations can coexist.
8. Keep MCP tool outputs compact and model-friendly.
9. Add tests for policy and protocol changes.
10. Update docs when architecture or tool contracts change.

Before finishing a task, run:

```bash
cargo fmt --all
cargo test --workspace
cargo check --workspace
```

When changing daemon/CLI protocol behavior, also run:

```bash
make smoke
```
