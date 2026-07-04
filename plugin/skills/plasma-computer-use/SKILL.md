---
name: plasma-computer-use
description: Use KDE Plasma desktop computer-use tools through PlasmaPilot MCP when a task requires seeing, clicking, typing, controlling windows, reading screenshots, or operating GUI applications on Linux/KDE.
---

Use terminal commands, files, APIs, and structured integrations first when they solve the task directly.

When GUI state matters, use PlasmaPilot tools through MCP:

1. Call `plasma.observe` before acting. Include a bounded screenshot only when visual state matters.
2. Prefer `plasma.click_button`, `plasma.set_text_field`, `plasma.select_menu`, `plasma.activate_tab`, `plasma.toggle_check`, `plasma.set_value`, and `plasma.focus_window` over raw coordinates.
3. Use `plasma.a11y_focused_tree` or `plasma.a11y_find` before semantic actions when the target is not obvious from `plasma.observe`.
4. Before pointer actions, call `plasma.pointer_calibration` and use only explicit `physical_pixel` coordinates.
5. Include `expected_active_window`, `expected_active_app`, or `active_title_contains` on every focus, semantic, keyboard, pointer, and scroll action when current window context is known.
6. Observe again after each action unless performing one bounded text-entry sequence.
7. Stop if `plasma.active_window` or `plasma.observe` reports a different target than expected.
8. Check `plasma.panic_stop_status` if control actions are unexpectedly denied or the desktop appears unsafe.
9. Do not interact with password fields, payment flows, account-security settings, or destructive dialogs without explicit user approval.
10. Set `destructive=true` on `plasma.click_button`, `plasma.select_menu`, or `plasma.a11y_invoke` when the action may delete, discard, close, quit, overwrite, or otherwise lose state.

Useful control tools:

- `plasma.type_text` and `plasma.key_combo` for guarded text entry and shortcuts.
- `plasma.a11y_insert_text` only when a known non-sensitive `EditableText` node needs insertion at a specific character offset and high-level `plasma.set_text_field` is not appropriate.
- `plasma.a11y_delete_text` only when a known non-sensitive `EditableText` node needs range deletion at specific character offsets.
- `plasma.move_pointer`, `plasma.click_pointer`, and `plasma.scroll_pointer` only after semantic routes are unavailable.
- `plasma.wait_for_change` to confirm bounded visual changes without repeatedly dumping screenshots.
- `plasma.journal_tail` to inspect compact action history when debugging a run.
