---
name: plasma-gui-testing
description: Test Linux/KDE desktop apps and browser UI visually with PlasmaPilot when screenshots, window focus, or real GUI interaction are needed.
---

Start the app under test from the terminal when possible. Use PlasmaPilot screenshots and observations for visual confirmation, record reproduction steps, and convert the understood behavior into deterministic tests whenever practical.

Workflow:

1. Start the application from a shell command when possible so logs and process state are available.
2. Use `plasma.observe` to capture monitors, windows, active-window state, and optionally a bounded screenshot.
3. Use `plasma.focus_window` with an active-window guard before interacting with the app under test.
4. Prefer `plasma.a11y_find`, `plasma.click_button`, `plasma.set_text_field`, `plasma.select_menu`, `plasma.select_item`, `plasma.activate_tab`, `plasma.activate_link`, `plasma.toggle_check`, and `plasma.set_value` for repeatable UI operations; use `plasma.a11y_insert_text`, `plasma.a11y_delete_text`, `plasma.a11y_copy_text`, `plasma.a11y_cut_text`, or `plasma.a11y_paste_text` only for known non-sensitive `EditableText` offset editing.
5. Use `plasma.pointer_calibration` plus guarded `plasma.click_pointer` or `plasma.drag_pointer` only when semantic access is unavailable.
6. Use `plasma.wait_for_change` after actions that should visibly update the UI.
7. Save repro artifacts under the repo test/evidence path when the task calls for evidence.
8. Convert the reproduction into a deterministic unit, integration, Playwright, or smoke test once the behavior is understood.
9. Mark destructive UI actions with `destructive=true`; default destructive policy fails closed unless the daemon is explicitly configured to allow them.

Do not use PlasmaPilot control tools against unrelated windows. Stop on ambiguous targets, sensitive fields, policy denial, or unexpected active-window changes.
