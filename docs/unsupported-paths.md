# Unsupported Paths

This document names the current boundaries for a public PlasmaPilot release. The project is intentionally KDE Plasma 6 Wayland first. Backend traits and protocol shapes should stay general, but unsupported desktops or control paths must not be implied as working until they have implementation, policy coverage, journaling, and eval evidence.

## Supported Baseline

The supported baseline is:

- Arch Linux or a comparable rolling Linux desktop.
- KDE Plasma 6 Wayland user session.
- User-scoped `plasma-pilotd`.
- Codex CLI connection through the bundled MCP server and plugin assets.
- Observation through daemon status, KWin metadata, bounded screenshots, clipboard status, AT-SPI diagnostics, and journal tools.
- Control only after policy checks, active-window guards where required, panic-stop checks, backend readiness checks, and journaling.

## Not Supported Yet

GNOME is not implemented. A future GNOME backend must provide its own window metadata, active-window guard source, capture behavior, accessibility quality evidence, input readiness diagnostics, install docs, and evals before it can share the public support claim.

wlroots and Sway are not implemented. They should be treated as future backend targets, not as KDE-compatible variants. A future implementation needs compositor-specific window/focus metadata, screenshot/capture support, and input routes that still pass through the common policy and journal model.

X11 is not a supported baseline. The plan allows an optional X11 backend later, but the current support target is KDE Wayland. Any X11 backend must still preserve policy checks, active-window guards, input journaling, and coordinate provenance instead of relying on unconstrained legacy automation.

Kernel modules are not implemented. uinput is the current privileged fallback, and portal/libei paths are preferred where available. A custom kernel module should only be considered after supported portal, KWin, AT-SPI, libei, and uinput paths show a measured gap that cannot be solved safely in userspace.

Custom KDE patches or native KWin plugins are not part of the current baseline. The packaged KWin script bridge is the supported KDE integration point today. A KWin plugin or KDE patch needs a specific measured gap, fallback behavior, policy gating, journaling, install docs, and rollback guidance.

Screenshot/OCR fallback for semantic actions is not implemented. High-level semantic actions currently use AT-SPI. Pixel or OCR fallback must not bypass semantic-control policy, active-window guards, candidate ambiguity rules, or screenshot redaction and sizing constraints.

Native desktop approval UX is not implemented. Approval files and explicit local daemon flags exist for operator-controlled runs. A future KDE approval UI must still create auditable, scoped, expiring grants instead of directly executing input or semantic control.

Public backend-neutral packaging is not finalized. `PlasmaPilot` remains the KDE-first working name until the project chooses a backend-neutral public name and preserves compatibility for existing crate, binary, MCP, and plugin identifiers.

## Release Rule

Do not mark an unsupported path as available in the tracker, release notes, plugin skills, MCP tool descriptions, or install docs unless the same slice includes implementation, policy classification, journal evidence, focused tests or evals, and updated troubleshooting documentation.
