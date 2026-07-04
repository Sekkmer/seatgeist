# Backends

Preferred KDE Plasma 6 Wayland order:

1. Semantic AT-SPI actions when an accessible node is available.
2. KWin metadata through DBus or KWin scripting for window state, focus, scaling, and geometry.
3. xdg-desktop-portal ScreenCast/Screenshot and RemoteDesktop for supported consented capture/control flows.
4. libei where the compositor exposes a suitable emulated-input server path.
5. Controlled uinput virtual devices for privileged local fallback.
6. Custom KWin plugin, KDE patch, or kernel module only after a measured gap remains.

Every backend must report capabilities and provenance. The daemon should refuse ambiguous fallback behavior.
