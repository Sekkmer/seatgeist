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
- The daemon and CLI can capture a full-screen PNG through Spectacle when run in the host KDE session. The smoke capture on this workstation returned a 7680x4320 source image.
- Screenshot output now defaults to a bounded preview. On the 8K workstation, the default CLI screenshot produced a 1600x900 PNG with source/output dimensions and scale metadata; `--full-resolution` produced a 7680x4320 PNG with scale `1.0`.
- Spectacle captures are serialized inside the daemon because concurrent Spectacle captures can race.
- `plasma-pilot-cli monitors` now reports KWin monitor metadata from `org.kde.KWin.supportInformation`; on this workstation it reports `HDMI-A-2` as 5120x2880 logical at scale 1.5, matching the 7680x4320 screenshot source.
- Screenshot responses include the same monitor metadata when KWin responds.
- `plasma-pilot-cli screenshot-tile` can crop a physical-pixel region from the full Spectacle capture and optionally downscale the tile. A host smoke captured a 1600x1200 tile at source origin 3200,1600 and wrote an 800x600 PNG with scale factors 0.5.
- `plasma-pilot-cli windows` lists open windows through KWin's `WindowsRunner` and enriches each stable KWin window id through `org.kde.KWin.getWindowInfo` for title, app id, and logical geometry.
- `kwin/plasma-pilot-bridge` packages the PlasmaPilot KWin script. It reads KWin's `workspace.activeWindow`, subscribes to `windowActivated`, and publishes compact active-window JSON to the daemon over the session bus.
- `plasma-pilot-cli active-window` reads the daemon's latest KWin script bridge update. Before the script reports its first update, the command still fails with the documented bridge requirement because KWin's interactive `queryWindowInfo` is not suitable for unattended active-window checks.
- `make smoke-windows` validates window listing in a host KDE session and accepts either a real active-window bridge response or the documented bridge-not-yet-reporting failure.
- `make install-kwin-script` is available as an explicit, opt-in KWin configuration mutation for installing/enabling the script.
- The KWin script was installed on this workstation and a host smoke observed a real active window with app id and logical geometry through the daemon bridge.
- The daemon writes compact JSONL journal entries for every handled request. `plasma-pilot-cli journal tail --limit N` reads recent entries through the daemon, and smoke tests verify restrictive journal file permissions.
- Daemon requests now pass through the policy engine before execution. Current observe/status requests are allowed by default; any future prompt-level request fails closed until an approval channel exists.
- `plasma-pilot-cli focus --window <id>` focuses a listed KWin window through `WindowsRunner.Run`. It is a control action: default policy rejects it until a trusted approval channel exists, while `plasma-pilotd --allow-control` enables explicit local smoke/manual use.
- `make smoke-focus` validates the focus path in a host KDE session with `--allow-control` and confirms the focus request is journaled.
- `plasma-pilot-mcp --stdio` now implements the first MCP server slice: `initialize`, `ping`, `tools/list`, and `tools/call` for current daemon-backed observe/window/screenshot/focus/journal tools.
- `plasma-pilot-cli observe` and MCP `plasma.observe` return monitors, windows, active-window state when available, and optional screenshot metadata for a bounded preview capture.
- `make smoke-mcp` validates MCP initialize, tool discovery, daemon-backed `plasma.health`, and daemon-backed `plasma.observe` tool calls over stdio.
- `plasma-pilot-cli clipboard get|set` and MCP `plasma.clipboard_get_text` / `plasma.clipboard_set_text` now use the Wayland `wl-paste` / `wl-copy` backend for UTF-8 text. Clipboard reads remain prompt-gated by default and require `plasma-pilotd --allow-clipboard-read` until a trusted approval channel exists. Clipboard reads return at most 64 KiB by default, preserve UTF-8 boundaries when truncated, and include truncation metadata. Clipboard journal summaries report text length only.
- `make smoke-clipboard` validates the clipboard path with explicit clipboard-read approval, restores the previous clipboard text when it can read it, and removes temporary clipboard-content files.
- `plasma-pilot-cli atspi tree --focused` and MCP `plasma.a11y_focused_tree` now return a compact AT-SPI subtree rooted at the focused accessibility node. The first backend discovers the accessibility bus via `org.a11y.Bus.GetAddress`, walks application roots, detects focus from the AT-SPI state bitset, and reports role/name/states/bounds/action names/children with depth and node caps.
- AT-SPI node summaries now include bounded text/value summaries for nodes implementing `org.a11y.atspi.Text` or `org.a11y.atspi.Value`. Text reads are capped at 512 characters and report `value_truncated`; password roles are marked sensitive and values are not read.
- `plasma-pilot-cli atspi find` and MCP `plasma.a11y_find` now find compact AT-SPI nodes by role, name substring, application name, and containing window name with result/depth/node caps.
- `plasma-pilot-cli atspi invoke --node <id> --action <press|focus|select>` and MCP `plasma.a11y_invoke` now invoke advertised AT-SPI actions by resolving normalized actions to the node's `GetActions` index and calling `DoAction`. This is semantic control and remains denied by default until the daemon is started with an explicit control approval mode.
- `plasma-pilot-cli atspi set-text --node <id> <text>` and MCP `plasma.a11y_set_text` now replace non-sensitive `EditableText` contents through `SetTextContents`, cap replacement text at 8192 characters, and journal only text length.
- `make smoke-atspi` validates the daemon-backed focused accessibility tree, accessibility find paths, and deny-by-default accessibility invoke/set-text paths, and confirms the requests are journaled.
- Pointer/keyboard input, richer AT-SPI edit operations, MCP wait/change tools, portal/KDE-native clipboard fallback, portal/KWin-native capture backends, persistent active-window bridge installation checks, and richer journal filtering remain future work.
