---
name: seatgeist-computer-use
description: Use KDE Plasma desktop computer-use tools through Seatgeist MCP when a task requires seeing, clicking, typing, controlling windows, reading screenshots, or operating GUI applications on Linux/KDE.
---

Use terminal commands, files, APIs, and structured integrations first when they solve the task directly.

When GUI state matters, use Seatgeist tools through MCP:

1. Call `seatgeist.computer_use_readiness` before acting. If it reports blockers, use the suggested diagnostic tools before attempting control.
2. Call `seatgeist.observe` before acting. Include a bounded screenshot only when visual state matters.
3. Prefer `seatgeist.click_button`, `seatgeist.focus_text_field`, `seatgeist.set_text_field`, `seatgeist.select_menu`, `seatgeist.select_item`, `seatgeist.activate_tab`, `seatgeist.activate_link`, `seatgeist.toggle_check`, `seatgeist.set_value`, and `seatgeist.focus_window` over raw coordinates.
4. Use `seatgeist.a11y_focused_tree` or `seatgeist.a11y_find` before semantic actions when the target is not obvious from `seatgeist.observe`.
5. Call `seatgeist.safety_status` before the first control action in a run if readiness did not already include the current safety state. If `focus_guard=true`, include an active-window guard or expect the daemon to reject the action.
6. Before pointer actions, call `seatgeist.pointer_calibration` and use only explicit `physical_pixel` coordinates.
7. Include `expected_active_window`, `expected_active_app`, or `active_title_contains` on every focus, semantic, keyboard, pointer, and scroll action when current window context is known.
8. Observe again after each action unless performing one bounded text-entry sequence.
9. Stop if `seatgeist.active_window` or `seatgeist.observe` reports a different target than expected.
10. Check `seatgeist.panic_stop_status` if control actions are unexpectedly denied or the desktop appears unsafe.
11. Do not interact with password fields, payment flows, account-security settings, or destructive dialogs without explicit user approval.
12. Set `destructive=true` on `seatgeist.click_button`, `seatgeist.select_menu`, or `seatgeist.a11y_invoke` when the action may delete, discard, close, quit, overwrite, or otherwise lose state.

Useful control tools:

- `seatgeist.type_text` and `seatgeist.key_combo` for guarded text entry and shortcuts.
- `seatgeist.focus_text_field` before guarded keyboard entry when AT-SPI exposes a named non-sensitive focusable text field.
- `seatgeist.a11y_text_attributes` when a known non-sensitive text node needs formatting or attribute-run inspection before choosing an edit path.
- `seatgeist.a11y_insert_text` only when a known non-sensitive `EditableText` node needs insertion at a specific character offset and high-level `seatgeist.set_text_field` is not appropriate.
- `seatgeist.a11y_delete_text` only when a known non-sensitive `EditableText` node needs range deletion at specific character offsets.
- `seatgeist.a11y_copy_text` and `seatgeist.a11y_cut_text` only when a known non-sensitive `EditableText` node needs clipboard copy/cut at specific character offsets.
- `seatgeist.a11y_paste_text` only when a known non-sensitive `EditableText` node needs clipboard paste at a specific character offset and the clipboard was intentionally prepared.
- `seatgeist.a11y_set_caret` and `seatgeist.a11y_set_selection` only when a known non-sensitive text node needs caret movement or an existing text-selection range changed at specific character offsets.
- `seatgeist.move_pointer`, `seatgeist.click_pointer`, `seatgeist.drag_pointer`, and `seatgeist.scroll_pointer` only after semantic routes are unavailable.
- `seatgeist.wait_for_change` to confirm bounded visual changes without repeatedly dumping screenshots; omit `output` unless a task-specific artifact path is needed.
- `seatgeist.journal_tail` to inspect compact action history when debugging a run.
