---
name: plasma-computer-use
description: Use KDE Plasma desktop computer-use tools through PlasmaPilot MCP when a task requires seeing, clicking, typing, controlling windows, reading screenshots, or operating GUI applications on Linux/KDE.
---

Use terminal commands, files, APIs, and structured integrations first when they solve the task directly.

When GUI state matters, use PlasmaPilot tools through MCP:

1. Call `plasma.computer_use_readiness` before acting. If it reports blockers, use the suggested diagnostic tools before attempting control.
2. Call `plasma.observe` before acting. Include a bounded screenshot only when visual state matters.
3. Prefer `plasma.click_button`, `plasma.focus_text_field`, `plasma.set_text_field`, `plasma.select_menu`, `plasma.select_item`, `plasma.activate_tab`, `plasma.activate_link`, `plasma.toggle_check`, `plasma.set_value`, and `plasma.focus_window` over raw coordinates.
4. Use `plasma.a11y_focused_tree` or `plasma.a11y_find` before semantic actions when the target is not obvious from `plasma.observe`.
5. Call `plasma.safety_status` before the first control action in a run if readiness did not already include the current safety state. If `focus_guard=true`, include an active-window guard or expect the daemon to reject the action.
6. Before pointer actions, call `plasma.pointer_calibration` and use only explicit `physical_pixel` coordinates.
7. Include `expected_active_window`, `expected_active_app`, or `active_title_contains` on every focus, semantic, keyboard, pointer, and scroll action when current window context is known.
8. Observe again after each action unless performing one bounded text-entry sequence.
9. Stop if `plasma.active_window` or `plasma.observe` reports a different target than expected.
10. Check `plasma.panic_stop_status` if control actions are unexpectedly denied or the desktop appears unsafe.
11. Do not interact with password fields, payment flows, account-security settings, or destructive dialogs without explicit user approval.
12. Set `destructive=true` on `plasma.click_button`, `plasma.select_menu`, or `plasma.a11y_invoke` when the action may delete, discard, close, quit, overwrite, or otherwise lose state.

Useful control tools:

- `plasma.type_text` and `plasma.key_combo` for guarded text entry and shortcuts.
- `plasma.focus_text_field` before guarded keyboard entry when AT-SPI exposes a named non-sensitive focusable text field.
- `plasma.a11y_text_attributes` when a known non-sensitive text node needs formatting or attribute-run inspection before choosing an edit path.
- `plasma.a11y_insert_text` only when a known non-sensitive `EditableText` node needs insertion at a specific character offset and high-level `plasma.set_text_field` is not appropriate.
- `plasma.a11y_delete_text` only when a known non-sensitive `EditableText` node needs range deletion at specific character offsets.
- `plasma.a11y_copy_text` and `plasma.a11y_cut_text` only when a known non-sensitive `EditableText` node needs clipboard copy/cut at specific character offsets.
- `plasma.a11y_paste_text` only when a known non-sensitive `EditableText` node needs clipboard paste at a specific character offset and the clipboard was intentionally prepared.
- `plasma.a11y_set_caret` and `plasma.a11y_set_selection` only when a known non-sensitive text node needs caret movement or an existing text-selection range changed at specific character offsets.
- `plasma.move_pointer`, `plasma.click_pointer`, `plasma.drag_pointer`, and `plasma.scroll_pointer` only after semantic routes are unavailable.
- `plasma.wait_for_change` to confirm bounded visual changes without repeatedly dumping screenshots; omit `output` unless a task-specific artifact path is needed.
- `plasma.journal_tail` to inspect compact action history when debugging a run.
