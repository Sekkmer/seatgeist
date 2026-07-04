# MCP Tools

Initial tool groups:

- Observation: `pilot.health`, `pilot.capabilities`, `pilot.list_monitors`, `pilot.list_windows`, `pilot.active_window`, `pilot.screenshot`, `pilot.screenshot_tile`, `pilot.observe`, `pilot.wait_for_change`.
- Control: `plasma.focus_window`, `plasma.click_button`, `plasma.set_text_field`, `plasma.select_menu`, `plasma.activate_tab`; pointer/keyboard tools are planned.
- Clipboard: `pilot.clipboard_get_text`, `pilot.clipboard_set_text`.
- Accessibility: `plasma.a11y_focused_tree`, `plasma.a11y_find`, `plasma.a11y_invoke`, `plasma.a11y_set_text`.
- Safety: `pilot.policy_status`, `pilot.set_panic_stop`, `pilot.journal_recent`.

All coordinate-bearing tools must require an explicit coordinate space. Full-resolution screenshots and clipboard reads are policy-gated.

Current daemon protocol exposes `observe`, `screenshot`, `screenshot-tile`, window listing, active-window bridge reads, `focus_window`, high-level `click_button`, high-level `set_text_field`, high-level `select_menu`, high-level `activate_tab`, clipboard text get/set, focused AT-SPI tree reads, AT-SPI find, AT-SPI invoke, AT-SPI set-text, and `journal_tail` through the CLI. The MCP stdio server exposes these current daemon-backed tools as `plasma.health`, `plasma.capabilities`, `plasma.policy_status`, `plasma.list_monitors`, `plasma.list_windows`, `plasma.active_window`, `plasma.observe`, `plasma.screenshot`, `plasma.screenshot_tile`, `plasma.focus_window`, `plasma.click_button`, `plasma.set_text_field`, `plasma.select_menu`, `plasma.activate_tab`, `plasma.clipboard_get_text`, `plasma.clipboard_set_text`, `plasma.a11y_focused_tree`, `plasma.a11y_find`, `plasma.a11y_invoke`, `plasma.a11y_set_text`, and `plasma.journal_tail`.

`plasma.observe` returns monitors, windows, active-window state when available, and optional bounded screenshot metadata if `screenshot_output` is provided. Tile coordinates are physical screenshot pixels. Screenshot responses include full source dimensions, output dimensions, source origin, scale factors, and monitor metadata when KWin responds. Focus is policy-gated control. `plasma.click_button` finds a named non-sensitive AT-SPI button with optional app/window guards, invokes press only when exactly one viable match remains, and refuses ambiguous matches. `plasma.set_text_field` finds a named non-sensitive AT-SPI text field with optional app/window guards, requires one viable match, uses `EditableText` set-text, and reports replacement length only. `plasma.select_menu` selects a visible AT-SPI menu path, such as `["File", "Open"]`, with optional app/window guards, requires one non-sensitive activatable item, and refuses missing or ambiguous paths. `plasma.activate_tab` finds a named non-sensitive AT-SPI tab with optional app/window guards, requires one viable match, and invokes select or press. Clipboard reads are policy-gated and fail closed by default until the daemon is started with an explicit clipboard-read approval mode. `plasma.clipboard_get_text` defaults to `max_bytes = 65536`, preserves UTF-8 boundaries when truncating, and supports `full = true` for an explicit unbounded read. `plasma.a11y_focused_tree` returns role/name/value/states/bounds/action names/children with `depth` and `max_nodes` caps; AT-SPI text values are capped at 512 characters and password roles suppress values. `plasma.a11y_find` filters by role, accessible-name substring, app name, and containing window name with result/depth/node caps. `plasma.a11y_invoke` invokes a normalized advertised action on an AT-SPI node id and is policy-gated as semantic control. `plasma.a11y_set_text` replaces non-sensitive `EditableText` contents, is capped at 8192 characters, and reports text length only in summaries. MCP tool responses return compact text plus structured JSON from the daemon; clipboard compact text and journal summaries report lengths rather than echoing clipboard content.

## Installation

Manual Codex config uses a stdio MCP server entry:

```toml
[mcp_servers.plasmapilot]
command = "plasma-pilot-mcp"
args = ["--stdio"]
```

The plugin bundle points at `plugin/.mcp.json` through `.codex-plugin/plugin.json`, so an installed plugin can provide the same MCP server config. The `plasma-pilot-mcp` binary must be on `PATH` for the current initial config.
