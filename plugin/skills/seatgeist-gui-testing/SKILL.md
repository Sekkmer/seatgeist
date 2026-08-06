---
name: seatgeist-gui-testing
description: Test Linux/KDE desktop apps and browser UI visually with Seatgeist when screenshots, window focus, or real GUI interaction are needed.
---

Start the app under test from the terminal when possible. Use Seatgeist screenshots and observations for visual confirmation, record reproduction steps, and convert the understood behavior into deterministic tests whenever practical.

Workflow:

1. Start the application from a shell command when possible so logs and process state are available.
2. Call `seatgeist.computer_use_readiness` and resolve reported blockers before sending control.
3. Use `seatgeist.observe` to capture monitors, windows, active-window state, and optionally a bounded screenshot. Identify the app under test by its exact KWin UUID rather than title or PID alone.
4. Call `seatgeist.safety_status` before the first control action if readiness did not already include the current safety state. For repeated raw actions, open a retained window session pinned to that KWin UUID and reuse its `session_id`; never call `focus_window`, activate, raise, or restack the physical user's window.
5. Prefer `seatgeist.a11y_find`, `seatgeist.a11y_text_attributes`, `seatgeist.click_button`, `seatgeist.focus_text_field`, `seatgeist.set_text_field`, `seatgeist.select_menu`, `seatgeist.select_item`, `seatgeist.activate_tab`, `seatgeist.activate_link`, `seatgeist.toggle_check`, and `seatgeist.set_value` for repeatable UI operations. Bind semantic actions to the exact window with its short-lived semantic handle or correlated target guard. Use `seatgeist.a11y_insert_text`, `seatgeist.a11y_delete_text`, `seatgeist.a11y_copy_text`, `seatgeist.a11y_cut_text`, `seatgeist.a11y_paste_text`, `seatgeist.a11y_set_caret`, or `seatgeist.a11y_set_selection` only for known non-sensitive text-node offset editing.
7. Use `seatgeist.pointer_calibration` plus guarded `seatgeist.click_pointer` or `seatgeist.drag_pointer` only when semantic access is unavailable.
8. Use `seatgeist.wait_for_change` after actions that should visibly update the UI, omitting `output` for the runtime screenshot directory unless a task-specific artifact path is needed.
9. Save repro artifacts under the repo test/evidence path when the task calls for evidence.
10. Convert the reproduction into a deterministic unit, integration, Playwright, or smoke test once the behavior is understood.
11. Mark destructive UI actions with `destructive=true`; default destructive policy fails closed unless the daemon is explicitly configured to allow them.

Do not use Seatgeist control tools against unrelated windows. Stop on ambiguous targets, sensitive fields, policy denial, or unexpected active-window changes.
Never close a retained app window with a keyboard shortcut. Use `seatgeist.close_window` with the owned session and exact KWin UUID so same-process windows cannot receive the close accidentally.
