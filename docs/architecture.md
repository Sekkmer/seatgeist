# Architecture

PlasmaPilot is split into a low-privilege Codex-facing MCP server, a local daemon that owns desktop-control backends, shared protocol/types, policy enforcement, and optional KDE/Wayland integration crates.

The project is KDE-first, not KDE-hardcoded. Current binaries, crates, and MCP tool names keep the `plasma-pilot` / `plasma.*` identity because KDE Plasma is the only supported implementation target today. Public protocol fields, policy decisions, journal records, and backend traits should use generic desktop concepts such as screen, window, input, clipboard, accessibility, and session so a future GNOME, wlroots/Sway, or X11 backend can implement the same contracts without changing model-facing semantics.

If the project is prepared for a broader public release, treat naming as a packaging decision rather than a protocol rewrite. A neutral top-level product name can wrap the existing Plasma backend, while `plasma-pilot-*` remains the KDE/Plasma reference backend family until a migration plan and compatibility aliases exist.

The initial architecture keeps all real desktop side effects behind backend traits. KWin, xdg-desktop-portal, AT-SPI, libei, uinput, and any future custom KDE or kernel module work must expose capabilities separately so policy can approve or deny each class of action.

High-DPI and 8K displays are first-class: screenshots should carry transform metadata and default to bounded previews or tiles rather than full-screen full-resolution payloads.
