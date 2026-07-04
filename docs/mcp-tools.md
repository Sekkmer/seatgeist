# MCP Tools

Initial tool groups:

- Observation: `pilot.health`, `pilot.capabilities`, `pilot.list_monitors`, `pilot.list_windows`, `pilot.active_window`, `pilot.screenshot`, `pilot.screenshot_tile`, `pilot.observe`, `pilot.wait_for_change`.
- Control: `pilot.focus_window`, `pilot.move_pointer`, `pilot.click`, `pilot.double_click`, `pilot.drag`, `pilot.scroll`, `pilot.key`, `pilot.type_text`.
- Clipboard: `pilot.clipboard_get_text`, `pilot.clipboard_set_text`.
- Accessibility: `pilot.a11y_focused_tree`, `pilot.a11y_find`, `pilot.a11y_invoke`, `pilot.a11y_set_text`.
- Safety: `pilot.policy_status`, `pilot.set_panic_stop`, `pilot.journal_recent`.

All coordinate-bearing tools must require an explicit coordinate space. Full-resolution screenshots and clipboard reads are policy-gated.

Current daemon protocol exposes `screenshot`, `screenshot-tile`, window listing, active-window bridge reads, `focus_window`, and `journal_tail` through the CLI. Tile coordinates are physical screenshot pixels. Screenshot responses include full source dimensions, output dimensions, source origin, scale factors, and monitor metadata when KWin responds. Focus is policy-gated control. Journal responses return compact structured records rather than raw log text.
