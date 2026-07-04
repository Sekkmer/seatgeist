---
name: plasma-desktop-triage
description: Diagnose KDE Plasma desktop, Wayland, display, screenshot, window, input, clipboard, or accessibility issues using PlasmaPilot diagnostics.
---

Start with `doctor`, `capabilities`, window state, active-window state, journal output, and screenshots. Report exact backend failures and likely causes. Do not change compositor, session, system, udev, polkit, or kernel settings unless the user requested it.

Workflow:

1. Call `plasma.health`, `plasma.capabilities`, and `plasma.policy_status` first.
2. Use `plasma.kwin_bridge_status`, `plasma.list_monitors`, `plasma.list_windows`, and `plasma.active_window` for KWin/window issues.
3. Use `plasma.uinput_status`, `plasma.input_backend_status`, and `plasma.pointer_calibration` for keyboard, pointer, uinput, portal, libei, or 8K coordinate issues.
4. Use `plasma.capture_backend_status` for display/capture backend issues before requesting any screenshot.
5. Use `plasma.screenshot` or `plasma.screenshot_tile` for display/capture issues, keeping screenshots bounded by default.
6. Use `plasma.a11y_focused_tree` and `plasma.a11y_find` for accessibility or semantic-action issues.
7. Use `plasma.journal_tail` with method/success filters to distinguish policy denials from backend failures.
8. Report exact backend names, command errors, policy decisions, and setup hints. Suggest system changes only after the diagnostics show they are needed.

Do not change KWin config, compositor state, udev rules, groups, polkit files, kernel modules, or session services unless the user explicitly asks for that mutation.
