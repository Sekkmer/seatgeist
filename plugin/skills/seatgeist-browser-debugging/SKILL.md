---
name: seatgeist-browser-debugging
description: Operate Firefox, Chrome, or Chromium on KDE with Seatgeist when browser debugging requires the user's live GUI session.
---

Prefer Playwright, HTTP tools, and browser developer protocols when possible. Use Seatgeist browser GUI actions only when login state, extensions, native dialogs, or real visual state matter. Keep actions scoped to the requested site or app.

Workflow:

1. Use Playwright, curl, application logs, or browser debugging protocols first when they can see the relevant state.
2. Switch to Seatgeist only for real-session browser state such as existing login, extension behavior, native file dialogs, permission prompts, or visual rendering bugs.
3. Call `seatgeist.computer_use_readiness` before browser GUI control and resolve reported blockers.
4. Use `seatgeist.observe` and `seatgeist.list_windows` to identify the exact browser window. For repeated raw actions, open a retained window session pinned to that KWin UUID and reuse its `session_id`; never call `focus_window`, activate, raise, or restack the physical user's window.
5. Prefer `seatgeist.a11y_find`, `seatgeist.click_button`, `seatgeist.focus_text_field`, `seatgeist.set_text_field`, `seatgeist.select_item`, `seatgeist.activate_tab`, and `seatgeist.toggle_check` for browser chrome and web UI when AT-SPI exposes the target. Bind semantic actions to the exact window with its short-lived semantic handle or correlated target guard.
6. Use retained agent-seat `seatgeist.key_combo` shortcuts such as `Ctrl+L`, `Ctrl+R`, and `Ctrl+F` for navigation and page search. Do not combine a retained `session_id` with active-window guards.
7. Use `seatgeist.screenshot` or `seatgeist.screenshot_tile` for visual evidence, omitting `output` for the runtime screenshot directory unless a task-specific artifact path is needed, and keeping 8K captures bounded unless full resolution is explicitly needed.
8. Use `seatgeist.wait_for_change` after navigation, form submission, or UI actions that should change the page; omit `output` unless a task-specific artifact path is needed.
9. Mark destructive browser or site actions with `destructive=true`; default destructive policy fails closed unless the daemon is explicitly configured to allow them.

Do not browse unrelated signed-in services. Do not enter credentials, payment details, or account-security settings unless the user explicitly asked for that action.
Never close Firefox with `Ctrl+W`, `Ctrl+Shift+W`, `Ctrl+Q`, or `Alt+F4`; same-process Firefox windows can route those shortcuts to the wrong window. Close only the exact retained KWin UUID with `seatgeist.close_window`.
