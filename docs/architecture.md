# Architecture

Seatgeist is split into a low-privilege Codex-facing MCP server, a local daemon that owns desktop-control backends, shared protocol/types, policy enforcement, and optional KDE/Wayland integration crates.

The project is KDE-first, not KDE-hardcoded. Binaries, crates, service names, plugin config, and MCP tools now use the `seatgeist` / `seatgeist.*` identity. Public protocol fields, policy decisions, journal records, and backend traits should use generic desktop concepts such as screen, window, input, clipboard, accessibility, and session so a future GNOME, wlroots/Sway, or X11 backend can implement the same contracts without changing model-facing semantics.

The public top-level product name is `Seatgeist`. KDE Plasma 6 Wayland is the first supported backend; future desktop backends should attach to the same product identity instead of creating desktop-specific product names.

The initial architecture keeps all real desktop side effects behind backend traits. KWin, xdg-desktop-portal, AT-SPI, libei, uinput, and any future custom KDE or kernel module work must expose capabilities separately so policy can approve or deny each class of action.

High-DPI and 8K displays are first-class: screenshots should carry transform metadata and default to bounded previews or tiles rather than full-screen full-resolution payloads.
