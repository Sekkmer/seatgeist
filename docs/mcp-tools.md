# MCP Tools

Initial tool groups:

- Observation: `pilot.health`, `pilot.capabilities`, `pilot.list_monitors`, `pilot.list_windows`, `pilot.active_window`, `pilot.screenshot`, `pilot.screenshot_tile`, `pilot.observe`, `pilot.wait_for_change`.
- Control: `pilot.focus_window`, `pilot.move_pointer`, `pilot.click`, `pilot.double_click`, `pilot.drag`, `pilot.scroll`, `pilot.key`, `pilot.type_text`.
- Clipboard: `pilot.clipboard_get_text`, `pilot.clipboard_set_text`.
- Accessibility: `pilot.a11y_focused_tree`, `pilot.a11y_find`, `pilot.a11y_invoke`, `pilot.a11y_set_text`.
- Safety: `pilot.policy_status`, `pilot.set_panic_stop`, `pilot.journal_recent`.

All coordinate-bearing tools must require an explicit coordinate space. Full-resolution screenshots and clipboard reads are policy-gated.

Current daemon protocol exposes `observe`, `screenshot`, `screenshot-tile`, window listing, active-window bridge reads, `focus_window`, clipboard text get/set, focused AT-SPI tree reads, and `journal_tail` through the CLI. The MCP stdio server exposes these current daemon-backed tools as `plasma.health`, `plasma.capabilities`, `plasma.policy_status`, `plasma.list_monitors`, `plasma.list_windows`, `plasma.active_window`, `plasma.observe`, `plasma.screenshot`, `plasma.screenshot_tile`, `plasma.focus_window`, `plasma.clipboard_get_text`, `plasma.clipboard_set_text`, `plasma.a11y_focused_tree`, and `plasma.journal_tail`.

`plasma.observe` returns monitors, windows, active-window state when available, and optional bounded screenshot metadata if `screenshot_output` is provided. Tile coordinates are physical screenshot pixels. Screenshot responses include full source dimensions, output dimensions, source origin, scale factors, and monitor metadata when KWin responds. Focus is policy-gated control. Clipboard reads are policy-gated and fail closed by default until the daemon is started with an explicit clipboard-read approval mode. `plasma.clipboard_get_text` defaults to `max_bytes = 65536`, preserves UTF-8 boundaries when truncating, and supports `full = true` for an explicit unbounded read. `plasma.a11y_focused_tree` returns role/name/states/bounds/action names/children with `depth` and `max_nodes` caps. MCP tool responses return compact text plus structured JSON from the daemon; clipboard compact text and journal summaries report lengths rather than echoing clipboard content.

## Installation

Manual Codex config uses a stdio MCP server entry:

```toml
[mcp_servers.plasmapilot]
command = "plasma-pilot-mcp"
args = ["--stdio"]
```

The plugin bundle points at `plugin/.mcp.json` through `.codex-plugin/plugin.json`, so an installed plugin can provide the same MCP server config. The `plasma-pilot-mcp` binary must be on `PATH` for the current initial config.
