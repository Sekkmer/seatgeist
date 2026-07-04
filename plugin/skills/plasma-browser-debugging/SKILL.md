---
name: plasma-browser-debugging
description: Operate Firefox, Chrome, or Chromium on KDE with PlasmaPilot when browser debugging requires the user's live GUI session.
---

Prefer Playwright, HTTP tools, and browser developer protocols when possible. Use PlasmaPilot browser GUI actions only when login state, extensions, native dialogs, or real visual state matter. Keep actions scoped to the requested site or app.

Workflow:

1. Use Playwright, curl, application logs, or browser debugging protocols first when they can see the relevant state.
2. Switch to PlasmaPilot only for real-session browser state such as existing login, extension behavior, native file dialogs, permission prompts, or visual rendering bugs.
3. Use `plasma.observe` and `plasma.list_windows` to identify the browser window. Use `plasma.focus_window` with an active-window guard before control.
4. Prefer `plasma.a11y_find`, `plasma.click_button`, and `plasma.set_text_field` for browser chrome and web UI when AT-SPI exposes the target.
5. Use guarded `plasma.key_combo` shortcuts such as `Ctrl+L`, `Ctrl+R`, and `Ctrl+F` for navigation and page search.
6. Use `plasma.screenshot` or `plasma.screenshot_tile` for visual evidence, keeping 8K captures bounded unless full resolution is explicitly needed.
7. Use `plasma.wait_for_change` after navigation, form submission, or UI actions that should change the page.
8. Mark destructive browser or site actions with `destructive=true`; default destructive policy fails closed unless the daemon is explicitly configured to allow them.

Do not browse unrelated signed-in services. Do not enter credentials, payment details, or account-security settings unless the user explicitly asked for that action.
