---
name: plasma-computer-use
description: Use KDE Plasma desktop computer-use tools through PlasmaPilot MCP when a task requires seeing, clicking, typing, controlling windows, reading screenshots, or operating GUI applications on Linux/KDE.
---

Use terminal commands, files, APIs, and structured integrations first when they solve the task directly.

When GUI state matters, use PlasmaPilot tools through MCP:

1. Call `pilot.observe()` before acting.
2. Prefer semantic or window-targeted actions over raw coordinates.
3. Include focus/window guards for every click, key, type, drag, or scroll when possible.
4. Observe again after each action unless performing one bounded text-entry sequence.
5. Stop if the active window is not the expected target.
6. Do not interact with password fields, payment flows, account-security settings, or destructive dialogs without explicit user approval.
