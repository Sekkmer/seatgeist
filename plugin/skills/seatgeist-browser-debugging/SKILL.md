---
name: seatgeist-browser-debugging
description: Operate Firefox, Chrome, or Chromium on KDE with Seatgeist when browser debugging requires the user's live GUI session.
---

Prefer Playwright, HTTP tools, and browser developer protocols when possible. Use Seatgeist browser GUI actions only when login state, extensions, native dialogs, or real visual state matter. Keep actions scoped to the requested site or app.

Workflow:

1. Use Playwright, curl, application logs, or browser debugging protocols first when they can see the relevant state.
2. Switch to Seatgeist only for real-session browser state such as existing login, extension behavior, native file dialogs, permission prompts, or visual rendering bugs.
3. Call `seatgeist.computer_use_readiness` before browser GUI control and resolve reported blockers.
4. Use `seatgeist.observe` and `seatgeist.list_windows` to identify the browser window. Use `seatgeist.focus_window` with an active-window guard before control.
5. Prefer `seatgeist.a11y_find`, `seatgeist.click_button`, `seatgeist.focus_text_field`, `seatgeist.set_text_field`, `seatgeist.select_item`, and `seatgeist.toggle_check` for browser chrome and web UI when AT-SPI exposes the target.
6. Use guarded `seatgeist.key_combo` shortcuts such as `Ctrl+L`, `Ctrl+R`, and `Ctrl+F` for navigation and page search.
7. Use `seatgeist.screenshot` or `seatgeist.screenshot_tile` for visual evidence, omitting `output` for the runtime screenshot directory unless a task-specific artifact path is needed, and keeping 8K captures bounded unless full resolution is explicitly needed.
8. Use `seatgeist.wait_for_change` after navigation, form submission, or UI actions that should change the page; omit `output` unless a task-specific artifact path is needed.
9. Mark destructive browser or site actions with `destructive=true`; default destructive policy fails closed unless the daemon is explicitly configured to allow them.

Do not browse unrelated signed-in services. Do not enter credentials, payment details, or account-security settings unless the user explicitly asked for that action.
