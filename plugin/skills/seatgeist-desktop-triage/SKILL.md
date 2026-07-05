---
name: seatgeist-desktop-triage
description: Diagnose KDE Plasma desktop, Wayland, display, screenshot, window, input, clipboard, or accessibility issues using Seatgeist diagnostics.
---

Start with `doctor`, `capabilities`, window state, active-window state, journal output, and screenshots. Report exact backend failures and likely causes. Do not change compositor, session, system, udev, polkit, or kernel settings unless the user requested it.

Workflow:

1. Call `seatgeist.health`, `seatgeist.capabilities`, `seatgeist.policy_status`, `seatgeist.safety_status`, `seatgeist.desktop_session_status`, and `seatgeist.computer_use_readiness` first.
2. Use `seatgeist.kwin_bridge_status`, `seatgeist.list_monitors`, `seatgeist.list_windows`, and `seatgeist.active_window` for KWin/window issues.
3. Use `seatgeist.uinput_status`, `seatgeist.input_backend_status`, and `seatgeist.pointer_calibration` for keyboard, pointer, uinput, portal, libei, or 8K coordinate issues.
4. Use `seatgeist.remote_desktop_session_probe` only when the operator explicitly wants to test the portal RemoteDesktop consent path; it is policy-gated, may show a portal dialog, closes the transient session, and sends no input. Use `seatgeist.remote_desktop_eis_probe` only when the operator also wants to test `ConnectToEIS`; it reports compact libei runtime state, immediately closes the returned FD, and still sends no input.
5. Use `seatgeist.capture_backend_status` for display/capture backend issues before requesting any screenshot.
6. Use `seatgeist.clipboard_status` for clipboard backend issues before requesting `seatgeist.clipboard_get_text`; status does not read clipboard contents.
7. Treat `implemented_available_backend` as the backend Seatgeist can execute today; `configured_backend` is the operator request, and `preferred_available_backend` may name a visible portal/libei path that still needs implementation.
8. Use `seatgeist.screenshot` or `seatgeist.screenshot_tile` for display/capture issues, omitting `output` for the runtime screenshot directory unless a task-specific artifact path is needed, and keeping screenshots bounded by default.
9. Use `seatgeist.a11y_focused_tree` and `seatgeist.a11y_find` for accessibility or semantic-action issues.
10. Use `seatgeist.journal_tail` with method/success filters to distinguish policy denials from backend failures.
11. Report exact backend names, command errors, policy decisions, active safety gates, and setup hints. Suggest system changes only after the diagnostics show they are needed.

Do not change KWin config, compositor state, udev rules, groups, polkit files, kernel modules, or session services unless the user explicitly asks for that mutation.
