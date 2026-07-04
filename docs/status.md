# PlasmaPilot Status

## 2026-07-04

Phase 0 scaffold is present:

- Cargo virtual workspace with resolver 3.
- Rust 2024 member crates for shared types, policy, backend traits, daemon, CLI, MCP, KWin, AT-SPI, and testkit.
- Conservative plugin, MCP, hook, skill, systemd, udev, and polkit skeletons.
- CLI and daemon are stubs; real socket RPC, screenshot, input, KWin, AT-SPI, and MCP protocol implementation remain future work.

Phase 1 first slice is implemented:

- `plasma-pilotd` binds a Unix socket, enforces restrictive socket directory/socket permissions, and rejects clients from another UID using Unix peer credentials.
- The daemon serves newline-delimited JSON requests for `health`, `capabilities`, and `policy_status`.
- `plasma-pilot-cli doctor`, `capabilities`, and `policy-status` call the daemon over the Unix socket.
- `make smoke` starts a temporary daemon, calls the CLI health/capability/policy commands, and verifies socket directory/socket modes.
- Screenshot, input, KWin, AT-SPI, real MCP tools, and journaling remain future work.
