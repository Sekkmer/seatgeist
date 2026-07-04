# Architecture

PlasmaPilot is split into a low-privilege Codex-facing MCP server, a local daemon that owns desktop-control backends, shared protocol/types, policy enforcement, and optional KDE/Wayland integration crates.

The initial architecture keeps all real desktop side effects behind backend traits. KWin, xdg-desktop-portal, AT-SPI, libei, uinput, and any future custom KDE or kernel module work must expose capabilities separately so policy can approve or deny each class of action.

High-DPI and 8K displays are first-class: screenshots should carry transform metadata and default to bounded previews or tiles rather than full-screen full-resolution payloads.
