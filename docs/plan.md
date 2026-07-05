# PlasmaPilot Project Plan

Status: fact-checked architecture and implementation plan, scaffold started 2026-07-04  
Target file: `docs/plan.md`  
Primary target: Arch Linux + KDE Plasma 6 + Wayland  
Primary client: Codex CLI via a Codex plugin that bundles MCP, skills, hooks, and local services  
Language preference: Rust-first, with minimal C++/KWin/Qt code only where KDE integration requires it

## 1. One-sentence goal

Build **PlasmaPilot**, a local Linux/KDE desktop-control substrate that lets Codex CLI safely observe and operate the user’s KDE Plasma desktop through a Codex plugin, MCP tools, skills, hooks, a CLI, and a privileged local daemon.

## 2. Why this project exists

Official Codex Computer Use is useful because it lets an agent see, click, type, and use normal applications. Linux/KDE does not yet have an equivalent first-class path in Codex app, but Codex CLI can already use local tools through MCP and reusable workflows through skills. PlasmaPilot fills that gap for KDE Plasma 6 by exposing desktop observation, input, window state, clipboard, and semantic UI information as controlled local tools.

The goal is not to merely emulate a human through pixels. The goal is to expose a **semantic desktop API** where possible, while keeping pixel-level fallbacks for arbitrary applications.

KDE Plasma remains the only implementation target for the current project. If the project is made public later, keep the core protocol, policy, and backend traits neutral enough that a future GNOME, wlroots/Sway, or X11 backend could implement the same contracts without changing the model-facing tool semantics. A broader public name can be revisited then; for now `PlasmaPilot` is the KDE-first product name.

## 2.1 Fact-check notes and source anchors

The plan was checked on 2026-07-04 against current public documentation:

- OpenAI Codex docs: Codex Computer Use is documented for macOS and Windows in the Codex app, not Linux/KDE. This validates PlasmaPilot as a Linux/KDE-specific local integration rather than a duplicate of an existing official Linux Computer Use path. Source: <https://developers.openai.com/codex/app/computer-use>
- OpenAI Codex docs: plugins can bundle skills, MCP server config, app integrations, and hooks; plugin manifest paths should be relative and start with `./`. Source: <https://developers.openai.com/codex/plugins/build>
- OpenAI Codex docs: Codex supports MCP servers in CLI and IDE, including stdio servers with `command`, `args`, `env`, tool allow/deny lists, and approval modes. Source: <https://developers.openai.com/codex/mcp>
- OpenAI Codex docs: skills are the authoring format for reusable workflows and plugins are the installable distribution unit; skill descriptions should front-load trigger terms because Codex uses progressive disclosure. Source: <https://developers.openai.com/codex/skills>
- OpenAI Codex docs: hooks can be bundled with enabled plugins, but non-managed hooks must be reviewed and trusted before they run. Source: <https://developers.openai.com/codex/hooks>
- Mac computer-use field research: current user reports and third-party tools highlight recurring gaps around permission/app-approval confusion, accessibility-tree quality, screenshot cost, dynamic UI identifiers, regional/device availability, background throttling, and security/data-leakage risk. PlasmaPilot tracks these in `docs/mac-computer-use-research.md` and turns them into KDE preflight, policy, semantic-selector, and eval requirements.
- Rust/Cargo docs: virtual workspaces should set the resolver explicitly, and `resolver = "3"` is the Rust 2024 resolver behavior. Sources: <https://doc.rust-lang.org/cargo/reference/workspaces.html> and <https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html>
- KDE developer docs: KWin scripts are installable with `kpackagetool6 --type=KWin/Script`, enabled through `kwriteconfig6`, and can manipulate windows through the KWin scripting API. Sources: <https://develop.kde.org/docs/plasma/kwin/> and <https://develop.kde.org/docs/plasma/kwin/api/>
- KDE developer docs: D-Bus is a common KDE/freedesktop IPC layer and underpins portals and many desktop services. Source: <https://develop.kde.org/docs/features/d-bus/introduction_to_dbus/>
- KDE KWin review history: KWin implemented the `org.kde.KeyboardLayouts` DBus interface so keyboard-layout DBus clients could also work on Wayland. PlasmaPilot treats this as a best-effort KDE surface and falls back to `kxkbrc`/xkbcommon when unavailable. Source: <https://phabricator.kde.org/D4323>
- XDG Desktop Portal docs: ScreenCast and RemoteDesktop portals expose monitor/window capture and keyboard/pointer/touchscreen remote-desktop sessions. Sources: <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html> and <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html>
- XDG Desktop Portal docs: the Screenshot portal exposes `Screenshot(parent_window, options) -> handle`, where completion is delivered through the shared Request `Response(response, results)` signal and successful results include a screenshot `uri`. Sources: <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Screenshot.html> and <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html>
- freedesktop/libei docs: libei is a Wayland-oriented emulated-input protocol with client/server separation over Unix sockets. Source: <https://libinput.pages.freedesktop.org/libei/>
- Linux kernel docs: uinput lets a userspace process create virtual input devices by writing to `/dev/uinput`; this is a viable privileged fallback, but it should not be the only Wayland plan. Source: <https://docs.kernel.org/input/uinput.html>
- AT-SPI2 docs: AT-SPI is a D-Bus protocol used by toolkit widgets to expose content to assistive technologies. Source: <https://www.freedesktop.org/wiki/Accessibility/AT-SPI2/>

Design consequence: the first implementation should keep multiple KDE/Wayland observation and control paths available: xdg-desktop-portal for consented screen/remote-desktop flows, KWin scripting or a KWin plugin for compositor-native window metadata, AT-SPI for semantic UI, libei where KDE exposes an EIS path, and uinput as a controlled privileged fallback. Diagnostics should distinguish policy denial, missing approval, portal/session availability, weak accessibility trees, active-window guard failure, human-input pause, and backend/runtime packaging issues before the agent attempts live control.

## 3. Project name and package names

Project name: **PlasmaPilot**

Preferred crate and binary names:

```text
plasma-pilotd        # local daemon / service
plasma-pilot-mcp     # MCP stdio server used by Codex
plasma-pilot-cli     # diagnostics and manual control CLI
plasma-pilot-kwin    # KWin integration package, script/plugin/backend
plasma-pilot-plugin  # Codex plugin bundle
libplasma-pilot      # shared Rust library crate
```

Repository name suggestion:

```text
plasmapilot
```

The public-facing spelling should be `PlasmaPilot`; package names should use `plasma-pilot-*`.

## 4. Design principles

1. **Local-first**: all privileged actions happen locally. No remote service is required.
2. **KDE/Wayland-first**: target KDE Plasma 6 Wayland as the primary environment.
3. **X11 optional**: support X11 later as a compatibility backend, not as the foundation.
4. **Semantic before pixels**: prefer window metadata, accessibility roles, named buttons, focused text fields, and app/window identifiers over raw coordinates.
5. **uinput for control**: input injection should happen through a controlled virtual keyboard/mouse device so it works under Wayland.
6. **Least privilege**: split high-privilege input/control from low-privilege MCP and CLI surfaces.
7. **Every action is auditable**: log requested action, resolved target, focused window, timestamp, and result.
8. **Agent-safe by default**: potentially destructive actions require confirmation or explicit policy.
9. **Composable**: the daemon should be usable from Codex, shell scripts, tests, other MCP clients, and eventually other assistants.
10. **MVP first**: deliver screenshot + input + clipboard + MCP before attempting full semantic UI automation.
11. **Portal-aware**: use xdg-desktop-portal capture/control paths where they provide a supported Wayland permission model.
12. **Compositor-native when justified**: use KWin scripts first, then a KWin plugin only when script/DBus surfaces are insufficient.
13. **Privilege is a backend choice**: keep privileged uinput, polkit, and any kernel module work behind backend traits and policy checks.
14. **8K-first ergonomics**: design screenshot downscaling, region capture, tiling, and coordinate metadata from the start so model context and bandwidth stay bounded on 7680x4320 displays.

## 5. Non-goals for v1

- Do not implement a remote desktop server.
- Do not bypass OS security for other users’ sessions.
- Do not capture hidden/locked-session content in v1.
- Do not automate password managers, payment pages, banking, or credential entry by default.
- Do not aim for perfect OCR or visual reasoning locally; Codex can reason from screenshots.
- Do not depend on one fragile KDE private API without a fallback.
- Do not require a custom kernel for the MVP. Kernel changes may be explored later only if they provide clear reliability or security benefits.
- Do not assume raw screenshots can be sent inline on high-DPI or 8K displays.
- Do not disable KDE/Wayland security prompts globally as an MVP shortcut.

## 6. High-level architecture

```text
Codex CLI
  |
  | installs/loads Codex plugin
  v
PlasmaPilot Codex Plugin
  |-- skills/                 reusable workflows and safety rules
  |-- hooks/                  audit/safety lifecycle hooks
  |-- .mcp.json               bundled MCP server config
  v
plasma-pilot-mcp              stdio MCP server
  |
  | Unix socket / local RPC
  v
plasma-pilotd                 local daemon
  |-- policy engine
  |-- action journal
  |-- session backend registry
  |-- screenshot backend via portal/KWin/tool fallback
  |-- input backend via portal/libei/uinput
  |-- clipboard backend
  |-- KWin backend
  |-- AT-SPI accessibility backend
  v
KDE Plasma 6 / KWin / Wayland / AT-SPI / uinput
```

## 7. Repository layout

Create this initial layout:

```text
.
├── Cargo.toml               # virtual workspace, resolver = "3"
├── crates/
│   ├── libplasma-pilot/     # shared protocol/types
│   ├── plasma-pilotd/
│   ├── plasma-pilot-cli/
│   ├── plasma-pilot-mcp/
│   ├── plasma-pilot-policy/
│   ├── plasma-pilot-backend/
│   ├── plasma-pilot-atspi/
│   ├── plasma-pilot-kwin/
│   ├── plasma-pilot-portal/
│   └── plasma-pilot-testkit/
├── plugin/
│   ├── .codex-plugin/
│   │   └── plugin.json
│   ├── .mcp.json
│   ├── hooks/
│   │   └── hooks.json
│   ├── skills/
│   │   ├── plasma-computer-use/
│   │   │   └── SKILL.md
│   │   ├── plasma-gui-testing/
│   │   │   └── SKILL.md
│   │   ├── plasma-browser-debugging/
│   │   │   └── SKILL.md
│   │   └── plasma-desktop-triage/
│   │       └── SKILL.md
│   └── assets/
├── systemd/
│   ├── plasma-pilotd.service
│   └── plasma-pilotd.socket
├── udev/
│   └── 99-plasma-pilot-uinput.rules
├── polkit/
│   └── org.plasmapilot.policy
├── docs/
│   ├── plan.md
│   ├── status.md
│   ├── architecture.md
│   ├── threat-model.md
│   ├── mcp-tools.md
│   ├── backends.md
│   ├── plugin.md
│   ├── arch-kde-install.md
│   ├── config.md
│   ├── uinput-setup.md
│   └── safety.md
└── tests/
    ├── fixtures/
    └── integration/
```

## 8. Component responsibilities

### 8.1 `libplasma-pilot`

Shared types and protocol definitions.

Must define:

- `ActionRequest`
- `ActionResult`
- `Observation`
- `WindowInfo`
- `MonitorInfo`
- `CoordinateSpace`
- `PolicyDecision`
- `ToolApprovalLevel`
- `SafetyClass`
- `BackendCapability`
- `PilotError`

Keep this crate mostly dependency-light.

### 8.2 `plasma-pilotd`

Long-running local daemon.

Responsibilities:

- Own the uinput virtual devices.
- Enforce local policy.
- Expose a Unix socket API to local clients.
- Maintain action journal.
- Coordinate screenshot/window/input/clipboard/accessibility backends.
- Validate focus/window guard before actions.
- Provide health and capabilities queries.

Daemon should run as the user where possible, with only the minimal additional access needed for uinput. If root is required initially, design the code so privilege separation can be improved later.

### 8.3 `plasma-pilot-mcp`

MCP server that Codex talks to over stdio.

Responsibilities:

- Translate MCP tool calls into daemon RPC requests.
- Expose only safe, documented, versioned tool schemas.
- Return compact, model-friendly observations.
- Avoid returning huge images inline unless explicitly requested. Prefer paths or structured metadata where practical.
- Apply MCP-side argument validation before touching the daemon.

### 8.4 `plasma-pilot-cli`

Manual diagnostics and development tool.

Required subcommands:

```bash
plasma-pilot-cli doctor
plasma-pilot-cli capabilities
plasma-pilot-cli screenshot [--output /tmp/screen.png]
plasma-pilot-cli windows
plasma-pilot-cli active-window
plasma-pilot-cli focus --window <id>
plasma-pilot-cli input click-pointer --x <x> --y <y> --coordinate-space physical-pixel --button left
plasma-pilot-cli input drag-pointer --from-x <x> --from-y <y> --to-x <x> --to-y <y> --coordinate-space physical-pixel
plasma-pilot-cli input type-text "hello"
plasma-pilot-cli input key-combo Ctrl+L
plasma-pilot-cli clipboard get
plasma-pilot-cli clipboard set "text"
plasma-pilot-cli atspi tree --focused
plasma-pilot-cli journal tail
```

### 8.5 `plasma-pilot-kwin`

KDE/KWin integration package.

Responsibilities:

- Enumerate windows.
- Report active window.
- Report monitor geometry and scaling.
- Focus windows.
- Get window geometry.
- Eventually expose KWin-native screenshots or region captures if available and stable.

Implementation strategy:

1. First probe existing DBus/KWin interfaces from Rust.
2. Use KWin's `WindowsRunner` plus `org.kde.KWin.getWindowInfo` for initial stable window listing when available.
3. Use the packaged `kwin/plasma-pilot-bridge` script for active-window updates; it reads `workspace.activeWindow` and calls the daemon's `org.plasmapilot.KWinBridge1.UpdateActiveWindow` method over DBus.
4. Use a small KWin plugin only when script/DBus APIs cannot provide stable geometry, focus, scaling, or capture semantics.
5. Document install/enable commands using `kpackagetool6`, `kwriteconfig6`, and KWin reconfigure.

### 8.6 `plasma-pilot-atspi`

Accessibility backend.

Responsibilities:

- Query focused application and focused element.
- Return role/name/state/value/action metadata.
- Expose semantic actions where possible, such as `press`, `set_text`, `select_menu_item`.
- Provide a reduced tree suitable for Codex context.

### 8.7 `plasma-pilot-policy`

Policy engine.

Responsibilities:

- Classify actions by risk.
- Enforce allow/deny/prompt policy.
- Enforce app/window allowlists and denylists.
- Enforce destructive-action confirmations.
- Enforce secret-field handling.
- Enforce rate limits and panic-stop state.

### 8.8 `plasma-pilot-portal`

Crate for xdg-desktop-portal integration contracts and future portal execution backends.

Responsibilities:

- Screenshot portal request contracts: bus name, object path, method/interface names, handle-token validation, expected and returned Request object paths, Response signal match rules, response-code handling, screenshot URI extraction, file URI decoding, and a transport trait for the request lifecycle.
- ScreenCast portal session setup and monitor/window stream metadata.
- RemoteDesktop portal keyboard/pointer/touchscreen session setup.
- Permission/session lifecycle reporting.
- PipeWire stream handling when capture requires it.
- Clear diagnostics when KDE portal support or user consent is unavailable.

### 8.9 `plasma-pilot-image`

Future crate for screenshot processing if needed.

Responsibilities:

- Downscale 8K screenshots to model-safe dimensions.
- Generate region crops and tiled screenshots.
- Preserve coordinate transforms between physical pixels, logical pixels, and downscaled images.
- [x] Produce screenshot change thresholds for `plasma.wait_for_change`. Current status: the daemon polls bounded screenshots, computes normalized RGB deltas, and returns changed/timed-out/captures/elapsed/timeout/interval/score metadata plus latest screenshot metadata, so a no-change watchdog timeout is distinguishable from a failed command or backend error.
- [x] Redact configured sensitive regions before screenshots leave the daemon. Current status: `[[safety.redact_regions]]` physical-pixel rectangles are mapped through full, preview, tile, observe, and wait-for-change screenshot transforms before output PNGs are returned.

## 9. Backend model

Define backend traits rather than hardcoding one implementation.

```rust
trait ScreenBackend {
    fn list_monitors(&self) -> Result<Vec<MonitorInfo>>;
    fn screenshot(&self, target: ScreenshotTarget) -> Result<Screenshot>;
    fn screenshot_scaled(&self, target: ScreenshotTarget, max_edge: u32) -> Result<Screenshot>;
}

trait WindowBackend {
    fn list_windows(&self) -> Result<Vec<WindowInfo>>;
    fn active_window(&self) -> Result<Option<WindowInfo>>;
    fn focus_window(&self, id: WindowId) -> Result<()>;
}

trait InputBackend {
    fn move_pointer(&self, point: Point) -> Result<()>;
    fn click(&self, point: Point, button: PointerButton, clicks: u8) -> Result<()>;
    fn drag(&self, from: Point, to: Point, button: PointerButton, duration_ms: u64) -> Result<()>;
    fn scroll(&self, dx: i32, dy: i32) -> Result<()>;
    fn key_combo(&self, combo: &str) -> Result<()>;
    fn type_text(&self, text: &str) -> Result<()>;
}

trait ClipboardBackend {
    fn get_text(&self) -> Result<Option<String>>;
    fn set_text(&self, text: &str) -> Result<()>;
}

trait AccessibilityBackend {
    fn focused_tree(&self, depth: usize) -> Result<AccessibilityNode>;
    fn find(&self, query: AccessibilityQuery) -> Result<Vec<AccessibilityNode>>;
    fn invoke(&self, node: NodeId, action: AccessibilityAction) -> Result<()>;
    fn set_text(&self, node: NodeId, text: &str) -> Result<()>;
    fn insert_text(&self, node: NodeId, offset: i32, text: &str) -> Result<()>;
    fn delete_text(&self, node: NodeId, start_offset: i32, end_offset: i32) -> Result<()>;
    fn copy_text(&self, node: NodeId, start_offset: i32, end_offset: i32) -> Result<()>;
    fn cut_text(&self, node: NodeId, start_offset: i32, end_offset: i32) -> Result<()>;
    fn paste_text(&self, node: NodeId, offset: i32) -> Result<()>;
}
```

Backend priority for KDE Plasma 6 Wayland:

```text
Observation:
  KWin metadata / portal metadata
  -> xdg-desktop-portal ScreenCast/Screenshot
  -> KDE tools such as Spectacle only as a diagnostic fallback
  -> custom KWin plugin if supported paths are insufficient

Input:
  semantic AT-SPI action when available
  -> xdg-desktop-portal RemoteDesktop or libei where available and consented
  -> controlled uinput virtual devices
  -> custom KWin/plugin/kernel work only for validated gaps
```

## 10. Coordinate model

This is critical under Wayland/KDE because scaling and multi-monitor layouts can make raw coordinates unreliable.

Implement these coordinate spaces explicitly:

```text
PhysicalPixel       actual screenshot pixel coordinate
LogicalPixel        compositor logical coordinate
WindowLocal         coordinate relative to one window
AccessibilityNode   semantic node target, preferred when available
```

Every screenshot response must include:

- monitor id
- physical width/height
- logical width/height
- scale factor
- transform/rotation if available
- origin in global logical coordinates
- active window id and geometry if available
- source image width/height when a screenshot is downscaled
- downscale factor and crop origin when returning model-sized images

Never let MCP tools accept ambiguous coordinates without declaring their coordinate space.

For an 8K monitor, default screenshots should not blindly return a full-resolution PNG to the model. The default `pilot.observe()` response should return a bounded preview plus metadata and make full-resolution capture explicit:

```text
preview_max_edge default: 1600
tile_max_edge default: 1600
full_resolution requires: explicit_full_resolution = true
full_resolution policy default: prompt
```

## 11. MCP tool surface

Start with a small, strong tool surface. Add semantic tools later.

### 11.1 Observation tools

```text
pilot.health()
pilot.capabilities()
pilot.list_monitors()
pilot.list_windows()
pilot.active_window()
pilot.screenshot(target?, include_cursor?, output_format?)
pilot.screenshot_tile(target, row, col, tile_size?)
pilot.observe(target?)
pilot.wait_for_change(target?, timeout_ms?, threshold?)
```

`pilot.observe()` should be the preferred high-level tool. It returns:

- active window
- window list summary
- monitor summary
- screenshot path or image payload
- preview dimensions and transform metadata if downscaled
- focused accessibility summary if available
- clipboard metadata, not clipboard content unless requested

### 11.2 Control tools

```text
pilot.focus_window(window_id)
pilot.move_pointer(point, coordinate_space)
pilot.click(point, button, coordinate_space, guard?)
pilot.double_click(point, button, coordinate_space, guard?)
pilot.drag(from, to, coordinate_space, duration_ms?, guard?)
pilot.scroll(dx, dy, guard?)
pilot.key(combo, guard?)
pilot.type_text(text, guard?)
```

`guard` should optionally include:

```json
{
  "expected_active_window": "...",
  "expected_app": "...",
  "expected_title_contains": "..."
}
```

The daemon must reject the action if the guard does not match.

### 11.3 Clipboard tools

```text
pilot.clipboard_get_text(max_bytes?)
pilot.clipboard_set_text(text)
```

Clipboard reads should be policy-controlled because clipboard content often contains secrets. The current MCP names are `plasma.clipboard_status`, `plasma.clipboard_get_text`, and `plasma.clipboard_set_text`; `plasma.clipboard_status` reports backend availability without reading clipboard contents.

### 11.4 Accessibility tools

```text
pilot.a11y_focused_tree(depth?)
pilot.a11y_find(role?, name_contains?, app?, window_id?)
pilot.a11y_text_attributes(node_id, offset, include_defaults?)
pilot.a11y_invoke(node_id, action, guard?)
pilot.a11y_set_text(node_id, text, guard?)
pilot.a11y_insert_text(node_id, offset, text, guard?)
pilot.a11y_delete_text(node_id, start_offset, end_offset, guard?)
pilot.a11y_copy_text(node_id, start_offset, end_offset, guard?)
pilot.a11y_cut_text(node_id, start_offset, end_offset, guard?)
pilot.a11y_paste_text(node_id, offset, guard?)
pilot.a11y_set_caret(node_id, offset, guard?)
pilot.a11y_set_selection(node_id, selection_num?, start_offset, end_offset, guard?)
```

These should be added after the pixel/control MVP is stable.

### 11.5 Safety tools

```text
pilot.policy_status()
pilot.set_panic_stop(enabled)
pilot.journal_recent(limit?)
```

## 12. Codex plugin bundle

Create a plugin under `plugin/`.

### 12.1 `plugin/.codex-plugin/plugin.json`

Initial manifest skeleton:

```json
{
  "name": "plasmapilot",
  "version": "0.1.0",
  "description": "KDE Plasma desktop control substrate for Codex CLI via MCP, skills, hooks, and a local daemon.",
  "author": {
    "name": "Sekkmer",
    "email": "sekkmer@gmail.com"
  },
  "license": "MIT OR Apache-2.0",
  "keywords": ["codex", "mcp", "kde", "plasma", "wayland", "computer-use", "desktop-automation"],
  "skills": "./skills",
  "mcpServers": "./.mcp.json",
  "interface": {
    "displayName": "PlasmaPilot",
    "shortDescription": "Use KDE Plasma desktop apps from Codex through local MCP tools.",
    "longDescription": "PlasmaPilot exposes a local KDE Plasma desktop-control runtime to Codex. It provides screenshots, window state, input, clipboard, accessibility metadata, safety policy, and audit logging through MCP tools and reusable skills.",
    "developerName": "Sekkmer",
    "category": "Developer Tools",
    "capabilities": ["MCP", "Skills", "Hooks", "Desktop Automation", "KDE", "Wayland"]
  }
}
```

### 12.2 `plugin/.mcp.json`

Initial MCP server config:

```json
{
  "mcp_servers": {
    "plasmapilot": {
      "command": "plasma-pilot-mcp",
      "args": ["--stdio"],
      "env": {
        "PLASMA_PILOT_SOCKET": "${XDG_RUNTIME_DIR}/plasma-pilot/plasma-pilotd.sock"
      }
    }
  }
}
```

### 12.3 Skills

Create four skills.

#### `plasma-computer-use`

Purpose: generic KDE desktop operation.

Description should front-load trigger terms:

```text
Use KDE Plasma desktop computer-use tools through PlasmaPilot MCP when a task requires seeing, clicking, typing, controlling windows, reading screenshots, or operating GUI applications on Linux/KDE.
```

Rules:

- Prefer terminal, files, APIs, and structured integrations when they solve the task directly.
- Use PlasmaPilot when GUI state matters or no API exists.
- Always call `pilot.observe()` before acting.
- After every click/type/key/drag/scroll, observe again unless performing a tightly bounded text entry.
- Use focus/window guards for every action when possible.
- Never interact with password fields, payment flows, account-security settings, or destructive dialogs without explicit user approval.
- Stop if the active window is not the expected target.

#### `plasma-gui-testing`

Purpose: test desktop apps and web apps visually.

Rules:

- Start app under test from terminal when possible.
- Use screenshots for visual confirmation.
- Record reproduction steps.
- Prefer deterministic test code once a GUI bug is understood.

#### `plasma-browser-debugging`

Purpose: operate Firefox/Chrome/Chromium for debugging.

Rules:

- Use Playwright or HTTP tools when possible.
- Use browser GUI when login/session/UI state matters.
- Keep browser actions scoped to the requested site/app.
- Do not browse unrelated signed-in services.

#### `plasma-desktop-triage`

Purpose: diagnose KDE/session/display/input issues.

Rules:

- Use `doctor`, `capabilities`, `windows`, `active-window`, journal, and screenshot tools.
- Do not change compositor/system settings unless requested.
- Report exact backend failures and likely causes.

### 12.4 Hooks

Use hooks for audit and safety, not for hidden magic.

Initial hook goals:

- Before risky MCP tools, emit a concise warning or require policy confirmation if supported.
- After PlasmaPilot actions, append a local audit record.
- On `Stop`, summarize actions taken and any denied actions.

Current hook implementation: `plugin/hooks/hooks.json` enables one conservative Codex `Stop` command hook. After Codex's normal `/hooks` trust review, it runs `plugin/hooks/plasma_audit_summary.py` from the git root and writes `target/plasma-pilot-hook-audit/latest.json`. The hook is fail-open, ignores prompt/hook stdin, and records only repo status, HEAD/branch, recent compact PlasmaPilot journal metadata, failure/control/unguarded-control counts, method, safety-class, and client counts, and compact active-window context. `make validate-plugin` verifies the hook command, timeout, script import, and audit aggregation behavior.

## 13. Safety and threat model

Create `docs/threat-model.md` with these assumptions.

### 13.1 Assets to protect

- User’s signed-in browser sessions.
- Password manager contents.
- Clipboard secrets.
- Private files visible in apps.
- Shell sessions with production credentials.
- Desktop input focus.
- Git repositories and source code.
- System settings.

### 13.2 Main risks

1. Agent clicks the wrong window.
2. Web page prompt-injects the agent into taking account actions.
3. Clipboard contains secrets and is read unnecessarily.
4. Screenshot captures sensitive content.
5. Input injection continues after the user takes over.
6. A local untrusted process connects to the daemon socket.
7. Coordinates are wrong due to scaling or monitor transform.
8. Destructive action is performed without approval.
9. Accessibility tree exposes more information than needed.
10. MCP tool is too powerful and lacks policy checks.
11. Full-screen screenshots expose unrelated private content on large displays.
12. A portal or KWin permission grant persists longer than the user expects.
13. A privileged backend can be called by a local process that is not Codex.
14. Downscaled images cause coordinate drift if transforms are not returned.

### 13.3 Required mitigations

- Unix socket must be under `$XDG_RUNTIME_DIR/plasma-pilot/` with mode `0600` by default.
- Daemon must reject requests from other users.
- Every control action should support focus/window guards.
- Dangerous apps can be denied by policy.
- Sensitive action classes require explicit approval mode.
- Provide a panic-stop command and optional global hotkey.
- After local human keyboard/mouse input is detected, optionally pause automation.
- Never auto-type into fields marked password/secret by AT-SPI.
- Never read clipboard automatically; only on explicit tool call.
- Log all actions to JSONL.
- Prefer region capture and downscaled previews over full-screen 8K capture by default.
- Include coordinate transform metadata with every image returned to MCP.
- Track backend provenance in every observation and action journal entry.
- Treat portal/libei/uinput/KWin/custom-kernel control paths as separate capabilities with separate policy gates.

## 14. Policy model

Create a TOML config at:

```text
~/.config/plasma-pilot/config.toml
```

Initial example:

```toml
[daemon]
socket = "$XDG_RUNTIME_DIR/plasma-pilot/plasma-pilotd.sock"
journal = "$XDG_STATE_HOME/plasma-pilot/journal.jsonl"

[policy]
default_control = "prompt"
default_observe = "allow"
default_clipboard_read = "prompt"
default_clipboard_write = "allow"
destructive_actions = "prompt"
secret_fields = "deny"
full_resolution_screenshot = "prompt"
privileged_input = "prompt"

[apps]
allow = ["org.kde.kate", "org.mozilla.firefox", "code", "konsole", "dolphin"]
deny = ["org.keepassxc.KeePassXC"]

[safety]
require_focus_guard = true
pause_on_human_input = true
human_input_activity_file = "$XDG_RUNTIME_DIR/plasma-pilot/human-input-active"
human_input_quiet_ms = 1500
panic_stop_file = "$XDG_RUNTIME_DIR/plasma-pilot/panic-stop"
control_rate_limit_per_minute = 120
preview_max_edge = 1600
tile_max_edge = 1600

[[safety.redact_regions]]
x = 0
y = 0
width = 640
height = 120
```

Policy values:

```text
allow
prompt
deny
```

If prompt support is not available through the current client, `prompt` should resolve to `deny` for high-risk actions and `allow` only for low-risk actions explicitly configured that way.

Current implementation: daemon requests are classified before execution and evaluated by `plasma-pilot-policy`. Observe/status requests are allowed by default. Prompt decisions fail closed unless the daemon is configured with an approval file and that file contains a matching unexpired method/safety-class grant. The CLI can append grants with `plasma-pilot-cli approve`; the daemon rejects approval files that are not regular files, not owned by the daemon uid, readable/writable/executable by group or other, or located in a parent directory writable by group or other.

`plasma-pilotd` now reads the config file from `~/.config/plasma-pilot/config.toml`, or from `--config` / `PLASMA_PILOT_CONFIG` when provided. Implemented fields are `[daemon].socket`, `[daemon].journal`, `[daemon].panic_stop_file`, `[daemon].approval_file`, `[journal].include_artifact_metadata`, `[backends].input`, these `[policy]` keys: `default_observe`, `default_control`, `destructive_actions`, `secret_fields`, `default_clipboard_read`, `default_clipboard_write`, and `full_resolution_screenshot`, `[apps].allow` and `[apps].deny`, plus `[safety].require_focus_guard`, `[safety].pause_on_human_input`, `[safety].human_input_activity_file`, `[safety].human_input_quiet_ms`, `[safety].control_rate_limit_per_minute`, `[safety].preview_max_edge`, `[safety].tile_max_edge`, and `[[safety.redact_regions]]`. CLI arguments and environment-backed flags take precedence over file values, so explicit local approval flags such as `--allow-control`, `--allow-clipboard-read`, and `--allow-full-resolution-screenshot` still override prompt/deny defaults for intentional local runs, and `--input-backend` / `PLASMA_PILOT_INPUT_BACKEND` override `[backends].input`. App deny rules win over allow rules; control fails closed if an app policy is configured and the relevant app id cannot be determined. Focus guards are required by default: every control-class request must include an active-window guard before backend execution unless `[safety].require_focus_guard = false` is set for a scoped local development daemon. When `pause_on_human_input` is true, a fresh activity signal file blocks control-class requests before backend execution. `make smoke-human-input-pause` verifies that behavior through a private configured daemon, a fresh activity signal, an approved focus-control request, safety-status freshness metadata, and a failed journaled control attempt without sending input; it is included in `make verify`. Control-class requests are also rate-limited to 120 accepted requests per rolling minute by default, with `0` disabling the limiter only for scoped local development. Default screenshot previews and tiles are bounded to the configured positive max-edge defaults, currently 1600 pixels for each. Configured screenshot redaction regions are physical-pixel source rectangles and are black-filled in output PNGs before screenshot metadata is returned. `plasma-pilot-cli safety-status` and MCP `plasma.safety_status` expose a compact read-only preflight for these safety gates, including focus-guard enforcement, human-input pause freshness, quiet interval, signal path, control rate limit, screenshot preview/tile max-edge defaults, redaction count, and whether opt-in journal artifact metadata is enabled. Destructive semantic requests and obvious destructive button/menu labels are classified separately as `DestructiveAction` and use `[policy].destructive_actions`, which defaults to prompt/fail-closed unless an approval grant matches. High-level text-field targets with secret-looking names are classified as `SecretField` and use `[policy].secret_fields`, which defaults to deny.

## 15. Action journal

Write JSONL records to:

```text
~/.local/state/plasma-pilot/journal.jsonl
```

Current implementation: `plasma-pilotd` appends compact request records containing `sequence`, `unix_time_ms`, `method`, optional `client` metadata, `safety_class`, `guard_present`, best-effort `active_window_before` and `active_window_after` for control-class requests, optional structured `control` metadata, optional `artifacts`, `ok`, and `summary`. Client metadata may include an explicit protocol-level `tool` identity from the request envelope, currently stamped by `plasma-pilot-cli` and `plasma-pilot-mcp`, plus best-effort same-UID Unix peer pid and sanitized `/proc/<pid>/comm` process name, which Linux may truncate. Callers cannot self-report pid/process metadata; the daemon derives those fields from peer credentials. Control metadata includes the action id when an action result exists, policy outcome, backend provenance, and a compact `requested_target` object with non-content fields such as coordinates, node ids, offsets, text length, key count, semantic-name length, app filters, and requested backend/device hints. It does not store typed text, replacement text, clipboard contents, screenshots, or semantic target names. Screenshot and wait-for-change summaries include capture backend provenance while still storing only metadata and output paths, not image payloads. When `[journal].include_artifact_metadata = true`, screenshot-bearing journal entries include artifact path, byte count, and SHA-256 metadata for the written output; this remains off by default because paths can reveal local context. `plasma-pilot-cli journal tail --limit N` returns recent records through the daemon and supports `--method <name>` and `--ok <true|false>` filters. Existing raw request lines and journal lines without the context fields remain parseable. Smoke tests pass target-local journal paths and verify `0600` file permissions.

The remaining future journal work should preserve the compact tail format while broadening opt-in artifact metadata only when the operator enables it:

Each record:

```json
{
  "ts": "2026-07-04T15:00:00+02:00",
  "client": "plasma-pilot-mcp",
  "tool": "pilot.click",
  "action_id": "uuid",
  "safety_class": "control.pointer.click",
  "requested_target": {"x": 100, "y": 200, "space": "LogicalPixel"},
  "active_window_before": {"id": "window-id", "app_id": "org.kde.kate", "title": "main.rs"},
  "policy": "allow",
  "backend": "kwin+uinput",
  "result": "ok",
  "active_window_after": {"app_id": "org.kde.kate", "title": "main.rs"}
}
```

Screenshots are not stored in the journal by default. Opt-in artifact metadata stores paths and hashes only, not image payloads.

## 16. Implementation phases

### Phase 0: Scaffolding

Goal: create project structure and docs.

Tasks:

- [x] Create Cargo workspace.
- [x] Create crates listed in the repository layout.
- [x] Create `docs/architecture.md`, `docs/threat-model.md`, `docs/mcp-tools.md`, `docs/backends.md`, `docs/safety.md`.
- [x] Create plugin skeleton.
- [x] Add `justfile` or `Makefile` with common commands.
- [x] Add `AGENTS.md` with coding instructions for Codex.

Acceptance criteria:

- `cargo check --workspace` passes.
- `cargo test --workspace` passes with stub tests.
- `plasma-pilot-cli --help` works.
- `plasma-pilot-mcp --help` works.

### Phase 1: Daemon + socket + health

Goal: working local daemon with Unix socket RPC.

Tasks:

- Implement `plasma-pilotd` with async Unix socket server.
- Implement request/response protocol using JSON or MessagePack. Use JSON first for debuggability.
- Implement `health`, `capabilities`, `policy_status`.
- Implement CLI commands for health/capabilities.
- Add systemd user service/socket files.
- Add a focused daemon/CLI smoke command using a temporary socket before enabling the user service.

Acceptance criteria:

- `systemctl --user start plasma-pilotd` works.
- `plasma-pilot-cli doctor` reports daemon status.
- Daemon refuses clients from wrong UID or unsafe socket permissions.
- `plasma-pilot-cli capabilities` and `plasma-pilot-cli policy-status` return daemon responses.

### Phase 2: Screenshot and monitor observation

Goal: Codex can see the desktop.

Tasks:

- Implement `ScreenBackend` trait.
- Implement a KDE/Wayland screenshot backend. Initial implementation uses Spectacle as a command backend.
- Prefer a backend that works on Plasma 6 without manual prompts once trusted/configured.
- Add fallback backend using command-line tools if needed.
- Return monitor geometry and scale info. Initial implementation parses KWin support information.
- [x] Save screenshots to `$XDG_RUNTIME_DIR/plasma-pilot/screenshots/` by default. Current status: direct CLI `screenshot`, `screenshot-tile`, and `wait-for-change` commands plus MCP `plasma.screenshot`, `plasma.screenshot_tile`, and `plasma.wait_for_change` accept explicit output paths, but when omitted they write timestamped PNGs under the PlasmaPilot runtime screenshot directory.
- Add `plasma-pilot-cli screenshot`.
- [x] Add default downscaled previews and explicit full-resolution capture. Current status: bounded previews are observe-class, while direct and observe-attached full-resolution screenshot requests are classified separately and prompt by default until the daemon is started with explicit full-resolution screenshot approval.
- Add tiled screenshots for 8K and multi-monitor workflows. Initial implementation supports physical-pixel tile crops with max-edge downscaling.
- Add coordinate transform metadata for preview/crop/full-size mapping. Initial preview/full-size mapping is implemented with scale factors and source/output dimensions.
- [x] Add safe capture backend diagnostics before implementing portal/KWin-native capture. Current status: `plasma-pilot-cli capture-backends`, MCP `plasma.capture_backend_status`, and `make smoke-capture-backends` probe xdg-desktop-portal Screenshot/ScreenCast interface visibility, KWin supportInformation metadata availability, and Spectacle fallback availability without starting a portal session or capturing pixels. Status responses distinguish the preferred visible backend from the currently implemented capture backend, and the smoke accepts either `portal_screenshot` or `spectacle` as the implemented backend. When portal Screenshot is visible, full-screen screenshot execution uses the portal, follows returned Request handles, bounds the Response wait, copies the returned screenshot URI into the requested PNG output, and then applies the same downscaling, 8K transform metadata, monitor metadata, redaction, and backend provenance as the Spectacle path. `screenshot-tile` also prefers the portal Screenshot source when visible, crops/downscales only the requested physical-pixel tile, and falls back to Spectacle only when the portal backend fails before user cancellation. Spectacle remains the compatibility fallback when portal Screenshot is unavailable or fails before a user response. `plasma-pilot-portal` codifies the official Screenshot method contract: `org.freedesktop.portal.Desktop` at `/org/freedesktop/portal/desktop`, the `org.freedesktop.portal.Screenshot.Screenshot(parent_window, options) -> handle` call, Request handle-token/path rules, Request `Response(response, results)` completion, and the screenshot `uri` result. This safe smoke is included in `make verify`.

Acceptance criteria:

- CLI can capture current screen to PNG.
- Screenshot response includes coordinate metadata, source dimensions, output dimensions, source origin, and preview/tile scale factors.
- Multi-monitor metadata is correct or explicitly marked unsupported. Initial implementation reports KWin logical geometry, physical pixel dimensions derived from scale, origin, and scale factor.

### Phase 3: uinput keyboard and pointer control

Goal: Codex can click/type through controlled virtual input.

Tasks:

- [x] Implement initial keyboard input backend using uinput. Current status: `plasma-pilot-uinput` creates a short-lived `/dev/uinput` virtual keyboard with `UI_DEV_SETUP`; daemon/CLI/MCP expose `type_text` and `key_combo` as policy-gated `ControlKeyboard`.
- [x] Create virtual pointer device and absolute/relative pointer mapping. Current status: `plasma-pilot-uinput` creates a short-lived `/dev/uinput` virtual pointer with absolute X/Y axes and relative wheel axes; the daemon maps physical desktop coordinates into the absolute input range.
- [x] Add udev/polkit/systemd instructions. Current status: skeleton files exist, `docs/uinput-setup.md` documents the optional udev rule, user service setup, current polkit placeholder state, and `input status` diagnostics. `make validate-install-assets` checks that the shipped user service remains user-scoped, the socket permissions stay private, the udev rule stays narrow, and the current polkit placeholder keeps denied defaults. `make smoke-uinput-status` verifies safe uinput/input-backend diagnostics and is included in `make verify`. Uinput access still relies on `/dev/uinput` being readable/writable by the daemon process.
- [x] Implement move, click, double-click, drag, and scroll. Current status: daemon/CLI/MCP expose `move_pointer`, `click_pointer`, `drag_pointer`, and `scroll_pointer` as policy-gated `ControlPointer`; click supports one or two left/middle/right clicks, drag supports bounded press-move-release with left/middle/right buttons, and scroll supports vertical/horizontal deltas. Move/click/drag accept `physical_pixel`, global `logical_pixel`, and guarded active-window `window_local` coordinates.
- [x] Implement key combo and type text. Current status: `auto`/`uinput` supports US evdev ASCII text plus newline/tab and named key combos such as `Ctrl+L`; unsupported uinput text is rejected instead of guessed. Explicit stored EIS backends can use the text capability for UTF-8 text plans and model XKB-compatible text-keysym plans. Explicit EIS `key_combo` planning now keeps the named evdev parser first, then uses configured `[backends.keymap]` RMLVO names, KDE current-layout DBus metadata, KDE `kxkbrc` config via `kreadconfig6`, or xkbcommon defaults for unsupported single-character symbol parts such as `Ctrl+;`.
- [x] Implement focus guard checks before actions. Current status: current daemon control requests accept optional active-window guards (`expected_active_window`, `expected_active_app`, and `active_title_contains`) and reject stale guards before execution.
- [x] Add panic-stop flag. Current status: `plasma-pilotd` has a file-backed panic-stop state, `plasma-pilot-cli panic-stop status|enable|disable` journals state changes, and active panic-stop blocks control-class daemon requests before execution. `scripts/plasma-pilot-panic-stop-hotkey` is a KDE global-shortcut friendly wrapper that defaults to `panic-stop enable` through the CLI, preserving daemon journaling and socket overrides; the Arch/KDE install runbook documents installing it into `~/.local/bin` and binding it as a custom shortcut, and install-asset validation checks the helper.
- [x] Probe whether xdg-desktop-portal RemoteDesktop or libei can satisfy input needs before requiring uinput on the local machine. Current status: `plasma-pilot-cli input backends` and MCP `plasma.input_backend_status` probe the user bus for `org.freedesktop.portal.RemoteDesktop`, KDE portal service visibility, libei client metadata/socket hints, and uinput fallback availability without starting a portal session. Status responses distinguish the configured backend request, the preferred visible backend, and the currently implemented input backend. `[backends].input`, `--input-backend`, and `PLASMA_PILOT_INPUT_BACKEND` can request `auto`, `uinput`, `portal_remote_desktop`, or `libei`; raw keyboard/pointer commands route through a daemon input-executor trait. `auto` and `uinput` use the uinput executor today. Explicit `portal_remote_desktop` and `libei` selections build EIS action plans for text, named evdev key combos, and pointer requests, then execute through the stored daemon EIS session only after the per-plan readiness gate passes; without a stored session or ready selected device they fail closed before side effects.
- [x] Model the xdg-desktop-portal RemoteDesktop request/session contract before enabling execution. Current status: `plasma-pilot-portal` has tested constants and builders for `CreateSession`, `SelectDevices`, `Start`, and `ConnectToEIS`, device-type and persist-mode validation, Request and Session handle-path derivation/validation, parsers for session/start response metadata, EIS FD return validation, a mockable lifecycle/EIS transport, a zbus lifecycle that can drive `CreateSession -> SelectDevices -> Start` while pre-subscribing to expected Request responses, and same-connection zbus helpers that return an owned EIS FD. The daemon can now wrap a portal-returned EIS FD in transient probe runtimes or retained daemon-owned EIS sessions; explicit portal/libei raw input uses the retained session only after policy, active-window, panic-stop, and readiness gates pass.
- [x] Add an explicit transient RemoteDesktop lifecycle probe before raw input integration. Current status: `plasma-pilot-cli input remote-desktop-probe` and MCP `plasma.remote_desktop_session_probe` are policy-gated control-class requests that can request keyboard/pointer/touchscreen devices through `CreateSession -> SelectDevices -> Start`, report selected devices, clipboard state, restore token, request/session handles, and then close the transient session without calling `ConnectToEIS` or sending Notify*/EIS input.
- [x] Add an explicit transient RemoteDesktop EIS probe before raw input integration. Current status: `plasma-pilot-cli input remote-desktop-eis-probe` and MCP `plasma.remote_desktop_eis_probe` run the same consented lifecycle, call `ConnectToEIS` after `Start` on the same DBus connection, initialize a transient daemon EIS runtime from the returned FD, poll pending events, close the runtime, report compact metadata plus runtime connected/event/bound-capability/resumed-device counts, and send no EIS or Notify* input.
- [x] Put raw input execution behind a backend trait before adding portal/libei executors. Current status: daemon keyboard and pointer handlers resolve an `InputExecutionBackend`; `auto` and `uinput` use the uinput adapter, explicit portal/libei selections build libei text, named evdev key-combo, and pointer action plans and execute them through the stored daemon EIS session only after readiness passes, and successful action summaries include backend provenance.
- [x] Model libei sender event sequences before wiring a live EIS executor. Current status: `plasma-pilot-eis` builds tested action plans for UTF-8 text, XKB-compatible text keysyms converted through xkbcommon, named evdev key combos, absolute pointer move, click, drag, and discrete scroll. The model follows the libei sender contract: start emulating before events, frame after generated events, stop emulating after releasing key/button state, Linux input-event-codes key/button values, XKB keysym values for text-keysym events, and 120-unit discrete scroll steps. It also exposes an `EisEventSink` boundary plus a guarded `LibeiDeviceSink` that translates validated plans into libei sender calls for a caller-owned resumed device. The crate now models selection of a resumed device with required capabilities and a virtual absolute-pointer region covering every planned target coordinate, rejecting paused devices, missing capabilities, out-of-region targets, cross-region drags, and physical absolute devices until explicit physical-unit mapping exists. `LibeiSenderContext` can take ownership of a portal-returned EIS FD, configure the libei sender name, expose the libei event FD for polling, dispatch pending events, bind the intersection of plan-required and seat-available capabilities on `SeatAdded`, snapshot connect/seat/device/resume/pause/remove events into compact device metadata, retain refcounted resumed libei device handles until pause/remove/seat removal/disconnect, and is marked `Send` for daemon mutex-serialized ownership transfer. `EisRuntimeState` consumes those snapshots, tracks connection/seat/bound-capability state plus the current device list, applies pause/remove/disconnect events, selects a resumed device for a plan from live-style state, and now has an execution-readiness gate that additionally requires a connected session and every plan-required capability to be bound before a selected device can be treated as executable. `EisSessionRuntime` wraps an event source plus runtime state so a daemon-owned EIS session can poll pending events, poll plan-aware seat bindings, report both planning readiness and stricter execution readiness, and hand a ready plan to a selected-device executor only after readiness passes. The live `LibeiSenderContext` selected-device executor applies validated plans only to retained selected devices and reports a fail-closed error if the runtime-selected device is no longer retained. This executor boundary is tested with mocks and is now used by explicit portal/libei raw input when a stored EIS session is active and ready. The daemon now has a `DaemonPortalEisSession` wrapper that preserves portal session metadata while owning an EIS runtime, a mutex-backed single-session store with start/status/stop daemon protocol, plus a tested session-backed input executor that converts daemon text/keyboard/pointer requests into EIS plans and calls the ready selected-device executor only after readiness passes. The transient EIS probe initializes a session wrapper, polls, drops it, and still sends no input. Explicit portal/libei raw-input selections now use the stored daemon EIS session after policy, panic-stop, active-window guard, and per-plan readiness checks. CLI/MCP wrappers expose stored-session start, status, and stop. A tested xkbcommon wrapper now builds explicit RMLVO keymaps, finds level-0 keysyms, converts XKB keycodes to evdev codes using the documented 8-code offset, and is wired into explicit EIS key-combo fallback for single-character symbols using configured, KDE-discovered, or default keymap names.
- [x] Add pointer calibration diagnostics. Current status: `plasma-pilot-cli input pointer-calibration`, MCP `plasma.pointer_calibration`, and `make smoke-pointer-calibration` report monitor-derived physical pointer bounds, per-monitor physical origins, and representative physical-pixel sample points without moving the pointer. This safe smoke is included in `make verify`.
- [x] Add host GUI smoke for a known test window before treating pixel-click use as production-ready. Current status: `make smoke-gui-input` starts a private daemon with an approval file, grants only the focus, click, type, and save methods it uses, opens a disposable KWrite/Kate file, focuses it through KWin, requires an active-window guard, maps a window point to physical pixels through pointer calibration, clicks, types a sentinel through uinput, saves, verifies file content, and checks the journal.

Acceptance criteria:

- CLI can type into Kate/KWrite. Current implementation provides the daemon-backed command path and `make smoke-gui-input` verified KWrite typing on this workstation.
- CLI can click a known point in a test window. Current implementation provides the daemon-backed command path, physical bounds validation, pointer calibration diagnostics, and `make smoke-gui-input` verified a guarded physical-pixel click into a disposable KWrite window on this workstation.
- Panic-stop prevents further input actions.
- Focus guard rejects action if active window changed. Current implementation covers current daemon control requests when a guard is supplied.

### Phase 4: Window backend through KDE/KWin

Goal: Codex knows what windows exist and can focus target windows.

Tasks:

- [x] Probe available KWin/Plasma DBus interfaces.
- [x] Implement active window query bridge. Current status: daemon DBus receiver and packaged KWin script exist; installation is explicit through `make install-kwin-script`.
- [x] Add active-window bridge installation diagnostics. Current status: `kwin_bridge_status` reports daemon DBus receiver state, active-window update state, window-list update state/count, and user-local package/config installation state.
- [x] Implement initial window list with stable KWin id, title, app id, and logical geometry through `WindowsRunner` plus `org.kde.KWin.getWindowInfo`. Current status: when the KWin script bridge has reported `workspace.stackingOrder`, the daemon also merges bridge-published pid/app/geometry metadata into list responses and can use the bridge list as fallback if the runner path fails.
- [x] Add pid and monitor association if a supported KWin, portal, or script path exposes them. Current status: daemon window and active-window responses derive `monitor_id` from the largest logical overlap between KWin window geometry and KWin monitor geometry; active-window and window-list bridge payloads preserve pid when KWin script provides it.
- [x] Implement focus window. Current status: `plasma-pilot-cli focus --window <id>` uses KWin `WindowsRunner.Run` and is policy-gated as `ControlSemantic`; default policy fails closed without a matching approval grant, while `plasma-pilot-cli approve` plus daemon `--approval-file` enables method-scoped local use and `plasma-pilotd --allow-control` remains a broad explicit local mode.
- [x] Create a KWin script exposing active-window metadata through DBus.

Acceptance criteria:

- CLI lists open windows with stable ids. Initial implementation is present through `plasma-pilot-cli windows`.
- CLI reports active window after the KWin script bridge is installed and has published its first update.
- CLI can focus a listed KWin window by id when control policy is explicitly allowed.
- Tool output is compact enough for model context.

### Phase 5: MCP server MVP

Goal: Codex CLI can use PlasmaPilot tools.

Tasks:

- [x] Implement MCP stdio server. Current status: line-delimited JSON-RPC over stdio with `initialize`, `ping`, `tools/list`, and `tools/call`.
- [x] Expose current daemon tools: health, capabilities, policy status, monitor/window listing, active-window, observe, screenshot, screenshot tile, focus window, and journal tail.
- [x] Expose clipboard_set/get after the backing daemon capability exists.
- [x] Expose wait_for_change after the backing daemon capability exists.
- [x] Expose key and type_text after the backing daemon capabilities exist.
- [x] Expose pointer move/click/drag/scroll after the backing daemon capability exists.
- [x] Add MCP-side argument validation for exposed tools.
- [x] Add docs for installing MCP manually and through plugin.
- [x] Ensure outputs are model-friendly for exposed tools: tool results include compact text plus structured JSON.

Acceptance criteria:

- `plasma-pilot-mcp --stdio` starts and responds to tool calls. `make smoke-mcp` validates initialize, tools/list, and a daemon-backed health tool call.
- Codex can call the MCP server from config.
- Codex can observe screen, focus window, and type into a test app.

### Phase 6: Codex plugin + skills

Goal: installable Codex plugin bundle.

Tasks:

- [x] Finalize `plugin/.codex-plugin/plugin.json`. Current status: manifest has real author, license, keyword, interface, skills, and MCP metadata with relative paths.
- [x] Finalize `plugin/.mcp.json`. Current status: bundled MCP config points at `plasma-pilot-mcp --stdio`, with daemon socket resolution handled by the MCP/daemon defaults or `PLASMA_PILOT_SOCKET`.
- [x] Write the four skills. Current status: the skills describe current `plasma.*` MCP tools, safety guards, observation flow, browser debugging, GUI testing, and desktop triage.
- [x] Create hook skeleton. Current status: `plugin/hooks/hooks.json` now enables one Codex `Stop` command hook that writes a fail-open local audit summary under `target/plasma-pilot-hook-audit/latest.json`; the summary includes repo status, recent compact journal entries, method/safety-class/client counts, failure examples, unguarded-control examples, and last active-window context. Codex still requires the normal `/hooks` trust review before non-managed plugin hooks run.
- [x] Add plugin install instructions. Current status: `docs/plugin.md` documents bundle contents, preconditions, validation, and local-use examples.
- [x] Add examples:
  - “Open Kate and type hello.”
  - “Use Firefox to verify localhost UI.”
  - “Reproduce this GUI bug and write a failing test.”

Acceptance criteria:

- Codex sees the skills.
- Codex can select the generic Plasma computer-use skill implicitly from a GUI task.
- Codex can use the bundled MCP server.
- Hooks either run safely or are disabled with a clear TODO if schema verification is pending.
  Current status: `make validate-plugin` checks the plugin manifest, MCP config, skill frontmatter, required skill set, and the bundled Stop audit hook; `make verify` runs the plugin validator.

### Phase 7: Clipboard backend

Goal: controlled clipboard integration.

Tasks:

- [x] Implement clipboard get/set for UTF-8 text.
- [x] Use `wl-copy`/`wl-paste` as the first Wayland backend when available.
- [x] Enforce clipboard read policy.
- [x] Add KDE/portal-native fallback and explicit provenance selection if `wl-copy`/`wl-paste` is unavailable. Current status: daemon clipboard reads/writes prefer `wl-clipboard` and fall back to KDE Klipper DBus (`org.kde.klipper`) when the Wayland commands are unavailable; `ClipboardText` responses and clipboard-set action summaries include backend provenance. `plasma-pilot-cli clipboard status` and MCP `plasma.clipboard_status` report `wl-paste`, `wl-copy`, and KDE Klipper DBus availability plus selected read/write backend names without reading clipboard contents.
- [x] Truncate large clipboard reads by default.

Acceptance criteria:

- CLI can set and get text clipboard.
- MCP can set clipboard text for paste workflows.
- Clipboard reads are logged and policy-checked.
- Clipboard journal summaries and compact MCP status text do not echo clipboard contents. Current summaries include backend diagnostics without content for `clipboard_backend_status`, and text length, truncation metadata, original byte count, and backend provenance only for explicit clipboard reads.

### Phase 8: AT-SPI semantic UI

Goal: Codex can use accessibility metadata instead of only pixels.

Tasks:

- [x] Implement focused accessibility tree.
- [x] Return compact node summaries: role, name, state, bounds, actions.
- [x] Return text/value summaries for supported nodes.
- [x] Implement find by role/name/app/window.
- [x] Implement text attribute inspection where `org.a11y.atspi.Text` is supported. Current status: CLI/MCP expose observe-class `a11y_text_attributes` for `GetAttributeRun(offset, includeDefaults)` on non-sensitive text nodes, with range/count-only summaries.
- [x] Implement policy-gated invoke action where `org.a11y.atspi.Action` is supported.
- [x] Implement set text where `org.a11y.atspi.EditableText` is supported.
- [x] Implement insert text where `org.a11y.atspi.EditableText` is supported. Current status: CLI/MCP expose policy-gated `a11y_insert_text` with active-window guards, offset validation, an 8192-character text cap, and content-free summaries.
- [x] Implement delete text where `org.a11y.atspi.EditableText` is supported. Current status: CLI/MCP expose policy-gated `a11y_delete_text` with active-window guards, range validation, and offset-only summaries.
- [x] Implement copy text where `org.a11y.atspi.EditableText` is supported. Current status: CLI/MCP expose policy-gated `a11y_copy_text` with active-window guards, range validation, and offset-only summaries; PlasmaPilot does not read copied clipboard contents.
- [x] Implement cut text where `org.a11y.atspi.EditableText` is supported. Current status: CLI/MCP expose policy-gated `a11y_cut_text` with active-window guards, range validation, and offset-only summaries; PlasmaPilot does not read cut clipboard contents.
- [x] Implement paste text where `org.a11y.atspi.EditableText` is supported. Current status: CLI/MCP expose policy-gated `a11y_paste_text` with active-window guards, offset validation, and offset-only summaries; PlasmaPilot does not read clipboard contents for this operation.
- [x] Implement caret movement where `org.a11y.atspi.Text` is supported. Current status: CLI/MCP expose policy-gated `a11y_set_caret` with active-window guards, offset validation, and offset-only summaries.
- [x] Implement text selection updates where `org.a11y.atspi.Text` is supported. Current status: CLI/MCP expose policy-gated `a11y_set_selection` for an existing selection index with active-window guards, range validation, and selection-index/offset-only summaries.
- [x] Add secret/password-field detection.

Acceptance criteria:

- CLI can print focused tree for Kate/Firefox/KDE dialogs.
- Codex can find a button by name in a simple dialog.
- Password fields are marked sensitive and default-denied.

### Phase 9: Semantic action layer

Goal: robust high-level UI operations.

Tasks:

- Implement high-level tools:
  - [x] `pilot.click_button(name, app/window guard)`
  - [x] `pilot.set_text_field(name, text, app/window guard)`
  - [x] `pilot.focus_text_field(name, app/window guard)` for focusing a non-sensitive text field before keyboard input.
  - [x] `pilot.select_menu(path, app/window guard)` for visible AT-SPI menu paths.
  - [x] `pilot.activate_tab(name, app/window guard)`
  - [x] `pilot.activate_link(name, app/window guard)` for AT-SPI links.
  - [x] `pilot.toggle_check(name, checked?, app/window guard)` for checkboxes, radio buttons, and checkable menu items.
  - [x] `pilot.set_value(name, value, app/window guard)` for sliders, spin buttons, scrollbars, and dials exposing AT-SPI Value.
  - [x] `pilot.select_item(name, app/window guard)` for list items, tree items, table rows, combo boxes, options, and menu-item-like choices exposing select or press.
- [x] Use AT-SPI first for `click_button`, `set_text_field`, `focus_text_field`, `select_menu`, `activate_tab`, `activate_link`, `toggle_check`, `set_value`, and `select_item`; screenshot+coordinate fallback remains future work and must only happen when safe.
- [x] Add ambiguity refusal for `click_button`, `set_text_field`, `focus_text_field`, `select_menu`, `activate_tab`, `activate_link`, `toggle_check`, `set_value`, and `select_item`. Current status: ambiguous semantic matches fail closed and return bounded candidate choices with a 1-based choice index, deterministic candidate id that is stable across raw AT-SPI node-id churn, raw node id, role, name, deterministic name-match score, and action metadata so the caller can disambiguate.

Acceptance criteria:

- Codex can operate common KDE dialogs semantically.
- Ambiguous matches return choices instead of clicking randomly.

### Phase 10: Reliability and evals

Goal: prevent regressions and measure usefulness.

Tasks:

- Add mock backends for unit tests. Current status: `plasma-pilot-testkit` provides deterministic screen, window, input, clipboard, and accessibility mocks with call recording; the screen mock covers raw and scaled screenshots, the input mock covers move, click point/button/count, drag, scroll, text, and key-combo events, and the accessibility mock covers focused-tree reads, find requests, text-attribute requests, invoke calls, set-text calls, insert-text calls, delete-text calls, copy-text calls, cut-text calls, paste-text calls, caret-set calls, selection-set calls, and numeric set-value calls.
- Add integration tests for CLI, MCP, and daemon protocol. Current status: daemon core protocol, low-risk CLI status commands, approval writing, trace validation/replay, panic-stop toggling, desktop-independent AT-SPI text-attribute CLI validation, and real MCP stdio initialize/tool-call/journal behavior have Rust integration tests; GUI/desktop CLI coverage remains in smoke targets. `make validate-traces` uses `plasma-pilot-cli trace validate --dir examples/traces` to validate every checked-in replay trace without a daemon. `make validate-install-assets` validates the shipped systemd, udev, and polkit install artifacts without mutating the host. `make smoke` starts a private daemon, exercises health/capabilities/policy/session/readiness/journal CLI paths, and verifies socket directory, socket, and journal permissions; it is part of `make verify`. `make smoke-trace-replay` validates the checked-in status-only, journal-tail, deny-by-default policy, input-denial, and panic-stop traces without a daemon, then replays the trace directory against a private daemon and verifies aggregate trace output plus journal evidence; it is part of `make verify`. `make smoke-mcp` validates the current MCP tool list, including RemoteDesktop EIS, AT-SPI text attributes, and offset-edit operations, and checks daemon-backed MCP health/observe calls plus tool error propagation for text-attribute validation; it is part of `make verify`.
- Add session preflight diagnostics for real KDE troubleshooting. Current status: `plasma-pilot-cli desktop-session-status` and MCP `plasma.desktop_session_status` report sanitized KDE/Wayland/session environment facts, including DBus and runtime-directory presence as booleans, so portal, KWin, AT-SPI, and daemon environment issues can be diagnosed before attempting control. `plasma-pilot-cli readiness` and MCP `plasma.computer_use_readiness` aggregate safe preflight state without screenshots, portal sessions, clipboard reads, or input; the response reports readiness booleans for observe, screenshot, window control, keyboard, pointer, semantic actions, clipboard read/write, active safety blockers, selected backend names, issue lists, and next diagnostic tools. Daemon capabilities advertise these as `daemon_desktop_session_status` and `daemon_computer_use_readiness`.
- Add denial-source and accessibility-quality diagnostics. Current status: safety, policy, backend, and session status are visible through compact CLI/MCP outputs. Daemon error responses now include structured `kind` metadata for policy prompt, policy deny, app deny, focus guard, human-input pause, panic-stop, rate limit, portal/backend availability, backend failure, accessibility availability/weak-tree, validation, and unknown failures; CLI and MCP compact text include the kind, trace replay reports `error_kind` for error steps, and policy/input denial replay traces assert `policy_prompt_required` through `/data/kind`. A configured-daemon CLI replay test now proves focus-guard, human-input-pause, app-policy, forced portal-unavailable, and accessibility-unavailable kinds through the real daemon protocol without invoking unsafe desktop control or portal consent UI. MCP stdio integration covers the same configured-denial categories through private daemons and asserts structured `data.kind` plus compact error text. `plasma-pilot-cli atspi quality-status`, daemon `accessibility_quality_status`, and MCP `plasma.a11y_quality_status` now sample a bounded focused AT-SPI tree and report AT-SPI availability, flat/generic/empty-tree signals, semantic targeting reliability, and the recommended fallback path; flat and mostly generic weak-tree fixtures exercise the same quality-status response and compact daemon summary. Dynamic semantic candidate-id fixtures prove ambiguity summaries retain stable candidate handles when raw AT-SPI node ids change. `plasma-pilot-cli wait-for-change`, daemon `wait_for_change`, and MCP `plasma.wait_for_change` now report explicit `timed_out`, `timeout_ms`, and `interval_ms` metadata in structured and compact outputs, so stalled/no-change polling is distinguishable from failed desktop actions or backend errors. `examples/traces/status-smoke.json`, `examples/traces/policy-denials-smoke.json`, `examples/traces/input-denials-smoke.json`, `make validate-traces`, `make smoke-trace-replay`, and `make smoke-mcp` cover the diagnostic.
- Add optional local GUI eval scripts. Current status: `scripts/gui-eval.sh` runs current non-control evals for daemon status, safety/session preflight metadata, observe, clipboard backend status without reading contents, default clipboard-read denial, KWin bridge status metadata, EIS keymap status metadata, bounded screenshot preview metadata, screenshot preview coordinate mapping, configured screenshot preview/tile safety bounds, full-resolution screenshot policy denial, and journal output through a private per-invocation daemon socket; focused wrappers exist for each safe case, including `make gui-eval-status`, `make gui-eval-session-preflight`, `make gui-eval-observe`, `make gui-eval-clipboard-status`, `make gui-eval-clipboard-denied`, `make gui-eval-screenshot-preview`, `make gui-eval-screenshot-coordinate-map`, `make gui-eval-screenshot-config-bounds`, and `make gui-eval-full-resolution-denied`; screenshot evals skip portal cancellation unless `PLASMA_PILOT_PORTAL_SCREENSHOT_STRICT=1` is set; failures print the per-run artifact directory, daemon log tail, journal tail, and artifact file list before daemon cleanup. GUI evals write artifacts under unique `target/plasma-pilot-gui-eval/<case>-<timestamp>-<pid>/` directories and update `target/plasma-pilot-gui-eval/latest`, so parallel focused evals no longer delete each other's socket or artifact paths; KWin-bridge DBus evals also take a local lock before starting their private daemon because the bridge service name is session-global. `make gui-eval-status` verifies health, capabilities, policy status, and journaling without touching desktop backends; this safe status eval is part of `make verify`. `scripts/gui-eval.sh session-preflight` and `make gui-eval-session-preflight` verify compact `safety_status`, including the default-disabled journal artifact metadata flag, sanitized `desktop_session_status` metadata, aggregated `computer_use_readiness` metadata, and journaling without invoking KDE control, portal prompts, screenshots, clipboard reads, or input; this safe preflight eval is part of `make verify`. `make gui-eval-observe` verifies daemon-backed observation shape and journaling without screenshots, portal prompts, clipboard reads, or input; this safe observe eval is part of `make verify`. `make gui-eval-clipboard-status` and `make gui-eval-clipboard-denied` verify clipboard backend diagnostics, journaling, default clipboard-read policy, and denial journaling without reading clipboard contents; both safe clipboard evals are part of `make verify`. `make gui-eval-full-resolution-denied` verifies full-resolution screenshot policy denial, output non-creation, and denial journaling before any capture backend can run; this safe denial eval is part of `make verify`. `make smoke-human-input-pause` now verifies the file-backed human-input pause through the actual daemon protocol and is part of `make verify`. `scripts/gui-eval.sh kwin-bridge-status` and `make gui-eval-kwin-bridge-status` verify compact `kwin_bridge_status` metadata, including active-window and window-list update state, plus journaling without installing the KWin script or sending input; when the daemon DBus receiver is reachable through `qdbus6`, the eval seeds both bridge update methods and requires the reported active-window and window-list state to match the seeded payload; this safe DBus status eval is part of `make verify`. `scripts/gui-eval.sh keymap-status` and `make gui-eval-keymap-status` verify `input_backend_status.eis_keymap` source/setup metadata and, when KDE exposes current-layout DBus or `kxkbrc` data, check that the reported source tracks that live evidence; this safe keymap metadata eval is part of `make verify`. `scripts/gui-eval.sh journal-artifacts` and `make gui-eval-journal-artifacts` start a private daemon with opt-in journal artifact metadata enabled, verify `safety_status` reports that opt-in flag, capture one bounded screenshot when a screenshot backend is available, and verify the journal artifact path, byte count, and SHA-256 against the written PNG. `scripts/gui-input-smoke.sh text-editor`, `make smoke-gui-input`, and `make gui-eval-text-editor-input` run the opt-in KWrite/Kate text-editor eval with method-scoped approval grants, active-window guards, physical-pixel click mapping, short chunked sentinel typing, save verification, and journal evidence. `scripts/gui-calculator-smoke.sh kcalc` and `make gui-eval-kcalc-visual` run the opt-in KCalc visual calculation eval with method-scoped approval grants, active-window guards, short chunked `2+2=` text input, journal evidence, and a KCalc active-window screenshot artifact when Spectacle is available. `scripts/gui-browser-smoke.sh firefox-localhost-button` and `make gui-eval-firefox-localhost-button` run the opt-in Firefox localhost button eval with a temporary localhost server, disposable Firefox profile, method-scoped approval grants, active-window guards, guarded window-local button click, local-server POST verification, journal evidence, and a Firefox active-window screenshot artifact when Spectacle is available. `scripts/gui-eval.sh portal-screenshot` and `make gui-eval-portal-screenshot` explicitly validate live portal Screenshot execution when the interface is visible, including `backend=portal_screenshot` metadata and journal provenance for bounded screenshot and screenshot-tile outputs when the portal returns screenshots; cancellation is treated as a skip unless `PLASMA_PILOT_PORTAL_SCREENSHOT_STRICT=1` is set. `scripts/gui-eval.sh remote-desktop-probe` and `make gui-eval-remote-desktop-probe` explicitly validate the live RemoteDesktop consent path when the interface and active-window guard metadata are visible, with strict started-session enforcement behind `PLASMA_PILOT_REMOTE_DESKTOP_STRICT=1`. `scripts/gui-eval.sh remote-desktop-eis-session` and `make gui-eval-remote-desktop-eis-session` explicitly validate the retained RemoteDesktop EIS session lifecycle plus minimal explicit-backend scroll and `Shift` key-combo attempts after method approval and readiness checks; cancelled/ended sessions and input readiness failures are skips unless `PLASMA_PILOT_REMOTE_DESKTOP_EIS_STRICT=1` or `PLASMA_PILOT_REMOTE_DESKTOP_EIS_INPUT_STRICT=1` is set. These screenshot, portal, and local-input evals remain opt-in because they may show consent dialogs or send real input. `scripts/gui-eval.sh control-safety` and `make gui-eval-control-safety` start a private daemon with a method-scoped approval-file grant and verify active-window guard denial plus panic-stop denial before backend control can execute; the control-safety eval is part of `make verify`.
- Add replayable action traces. Current status: `ReplayTrace` stores daemon requests with expected response metadata, including optional error-message substring expectations for fail-closed paths and JSON-pointer equality, single-type, type-list, and existence checks for compact response-field assertions. `plasma-pilot-cli trace validate --file <path>` checks one trace, and `plasma-pilot-cli trace validate --dir <path>` validates a directory of `.json` traces with a compact aggregate report and an empty-set failure. Validation checks trace structure, version, non-empty/unique labels, expected response types, JSON-pointer syntax, supported JSON value types, and contradictory error expectations without contacting the daemon. `plasma-pilot-cli trace replay --file <path>` replays one trace, and `plasma-pilot-cli trace replay --dir <path>` replays a directory with a compact aggregate report; replayed requests still go through daemon policy checks and journaling, and error steps include `error_kind` in the compact replay report. Replay mismatches report the step index, label, method, and mismatch detail without echoing full response payloads. `examples/traces/status-smoke.json`, `examples/traces/journal-tail-smoke.json`, `examples/traces/policy-denials-smoke.json`, `examples/traces/input-denials-smoke.json`, and `examples/traces/panic-stop-smoke.json` provide checked-in safe replay fixtures covered by real CLI integration tests and `make smoke-trace-replay`; the status fixture covers computer-use readiness, AT-SPI quality diagnostics, KWin bridge diagnostics, uinput access diagnostics, capture backend diagnostics, clipboard backend diagnostics including nullable selected read/write backend fields, `input_backend_status.eis_keymap` metadata, and inactive retained EIS session status/stop responses, the journal-tail fixture covers method/success filters and default compact journal records without control/artifact payload fields, the policy-denial fixture covers full-resolution screenshot, clipboard-read, focus-control, and AT-SPI caret/selection control fail-closed paths before backend side effects with `policy_prompt_required` error-kind assertions, and the input-denial fixture covers raw keyboard, pointer, RemoteDesktop session probe, RemoteDesktop EIS probe, and RemoteDesktop EIS start policy checks before backend side effects with the same structured error-kind assertion.
- Add screenshot/coordinate calibration tests. Current status: protocol tests cover mapping 8K downscaled previews and physical-pixel tiles back to source screenshot coordinates.

Suggested local evals:

```text
Eval 1: open Kate, type text, save file. Current status: `make gui-eval-text-editor-input` opens KWrite or Kate on a disposable file, grants only the required focus/click/type/key-combo methods, requires active-window guard metadata, maps a window-local target to physical pixels, types a sentinel in short chunks, saves it, verifies file contents, and checks journal evidence.
Eval 2: open KCalc, calculate 2+2 visually. Current status: `make gui-eval-kcalc-visual` opens KCalc, grants only the required focus/type/key-combo methods, requires active-window guard metadata, sends `2+2=` as short guarded text chunks, verifies journal evidence, and writes a KCalc active-window screenshot artifact showing the result when Spectacle is available.
Eval 3: open Firefox, visit localhost, click a button. Current status: `make gui-eval-firefox-localhost-button` starts a temporary localhost server, launches Firefox with a disposable profile, grants only the required focus/click/key-combo methods, requires active-window guard metadata, clicks a large localhost button through guarded window-local pointer coordinates, verifies the local server received the button POST, checks journal evidence, and writes a Firefox active-window screenshot artifact when Spectacle is available.
Eval 4: focus wrong window and verify guard rejection. Current status: `make gui-eval-control-safety` writes a short-lived `focus_window` approval grant, seeds private active-window state through the daemon KWin bridge, and verifies an incorrect guard rejects focus before backend execution.
Eval 5: clipboard status and read policy. Current status: `make gui-eval-clipboard-status` verifies backend diagnostics and journaling without reading clipboard contents, and `make gui-eval-clipboard-denied` verifies default clipboard reads fail closed and are journaled without clipboard-read approval; both are included in `make verify`.
Eval 6: panic-stop blocks input/control. Current status: `make gui-eval-control-safety` enables a private panic-stop file and verifies an approved focus control request is rejected before backend execution.
Eval 7: 8K screenshot preview maps clicks back to source coordinates. Current status: protocol tests cover the transform math, and `make gui-eval-screenshot-coordinate-map` validates real screenshot metadata maps a preview center point back inside the source screenshot bounds.
Eval 8: full-resolution screenshot requires explicit policy approval. Current status: `make gui-eval-full-resolution-denied` verifies full-resolution capture is rejected by policy before any output file is written and the denial is journaled; it is included in `make verify`.
Eval 9: configured screenshot bounds control default preview and tile output sizes. Current status: `make gui-eval-screenshot-config-bounds` starts a private daemon from config, verifies `safety-status` reports the configured bounds, and checks default screenshot and tile outputs stay within them.
Eval 10: portal Screenshot executes when available. Current status: `make gui-eval-portal-screenshot` starts a private daemon, verifies `capture-backends` selects `portal_screenshot` when the interface is visible, captures a bounded screenshot and a physical-pixel screenshot tile when the portal returns screenshots, checks compact metadata, and verifies journal provenance for both methods. Portal cancellation is treated as a skip unless `PLASMA_PILOT_PORTAL_SCREENSHOT_STRICT=1` is set. It is intentionally opt-in because the portal may request user consent.
Eval 11: portal RemoteDesktop reaches consent/session Start when available. Current status: `make gui-eval-remote-desktop-probe` starts a private daemon, verifies the RemoteDesktop portal interface is visible, requires active-window guard metadata, writes a short-lived `remote_desktop_session_probe` approval grant, requests keyboard and pointer devices, checks compact metadata and journal provenance, and requires `started=true` only when `PLASMA_PILOT_REMOTE_DESKTOP_STRICT=1` is set. It is intentionally opt-in because the portal may request user consent.
Eval 12: retained portal EIS session can back explicit input when available. Current status: `make gui-eval-remote-desktop-eis-session` starts a private daemon configured for `portal_remote_desktop`, verifies RemoteDesktop portal visibility and active-window guard metadata, writes short-lived grants for `remote_desktop_eis_start`, `scroll_pointer`, and `key_combo`, starts a retained EIS session, checks active session/backend metadata, attempts one minimal scroll and one minimal `Shift` key-combo only after readiness gates pass, stops the session, and checks journal evidence. Portal cancellation and non-strict readiness failures are skips unless `PLASMA_PILOT_REMOTE_DESKTOP_EIS_STRICT=1` or `PLASMA_PILOT_REMOTE_DESKTOP_EIS_INPUT_STRICT=1` is set. It is intentionally opt-in because the portal may request user consent and the eval can send real input.
Eval 13: EIS keymap status follows KDE layout evidence. Current status: `make gui-eval-keymap-status` starts a private daemon, checks compact `eis_keymap` metadata from `input_backend_status`, verifies journal evidence, and, when KDE current-layout DBus or `kxkbrc` data is available, checks that the status source/layout metadata matches the live KDE evidence. This eval is included in `make verify`.
Eval 14: KWin bridge status reports live installation/update diagnostics. Current status: `make gui-eval-kwin-bridge-status` starts a private daemon, checks compact `kwin_bridge_status` metadata including active-window and window-list update state, verifies journal evidence without installing the KWin script or sending input, and, when local DBus seeding is available, requires the status response to reflect seeded `UpdateActiveWindow` and `UpdateWindows` payloads. This eval is included in `make verify`.
Eval 15: opt-in screenshot artifact metadata is correct. Current status: `make gui-eval-journal-artifacts` starts a private daemon with `[journal].include_artifact_metadata = true`, verifies `safety-status` reports the opt-in flag, captures a bounded screenshot when a backend is available, and verifies the journal artifact path, byte count, and SHA-256 against the written PNG. It is intentionally opt-in because screenshot capture may request user consent.
Eval 16: accessibility quality diagnostics remain model-friendly. Current status: `make gui-eval-a11y-quality-status` starts a private daemon, checks compact `accessibility_quality_status` metadata, and verifies journal evidence without invoking accessibility control, screenshots, portal prompts, clipboard reads, or input. This eval is included in `make verify`.
```

Acceptance criteria:

- `cargo test --workspace` covers protocol/policy/tool validation.
- Manual eval script passes on KDE Plasma 6 Wayland.
- Failures produce useful diagnostics. Current status: GUI eval failures report the artifact directory plus daemon log, journal, and artifact-file tails/lists.

## 17. First Codex implementation prompt

Use this as the first task prompt for Codex:

```text
Implement Phase 0 and Phase 1 of docs/plan.md for PlasmaPilot.

Constraints:
- Rust workspace only for now.
- Do not implement real uinput, screenshots, KWin, or AT-SPI yet.
- Create clean traits, types, CLI stubs, daemon health endpoint, and docs.
- The daemon should expose a Unix socket JSON protocol with health/capabilities.
- The CLI should have doctor/capabilities commands that call the daemon.
- Add systemd user unit skeletons.
- Add plugin skeleton files but do not claim they are fully functional yet.
- Keep all code compiling and tested.
- End by updating docs/plan.md checkboxes or adding docs/status.md with what was completed.
```

## 18. Suggested `AGENTS.md`

Create `AGENTS.md` in the repo root:

```markdown
# PlasmaPilot agent instructions

This repository builds PlasmaPilot, a KDE Plasma desktop-control substrate for Codex.

Rules for AI agents:

1. Keep the project compiling after every change.
2. Prefer small vertical slices over large unfinished rewrites.
3. Never add unsafe desktop-control behavior without policy checks.
4. All input actions must flow through the policy engine.
5. All actions must be journaled.
6. Do not hardcode one KDE private API without documenting fallback behavior.
7. Use traits for backends so KDE/Wayland/X11/mock implementations can coexist.
8. Keep MCP tool outputs compact and model-friendly.
9. Add tests for policy and protocol changes.
10. Update docs when architecture or tool contracts change.

Before finishing a task, run:

```bash
cargo fmt --all
cargo test --workspace
cargo check --workspace
```
```

## 19. Immediate TODO checklist

- [x] Create repository structure.
- [x] Add this file as `docs/plan.md`.
- [x] Add `AGENTS.md`.
- [x] Create Cargo workspace.
- [x] Add shared type crate.
- [x] Add daemon crate with health endpoint stub.
- [x] Add CLI crate with `doctor` and `capabilities` stubs.
- [x] Add MCP crate stub.
- [x] Add plugin skeleton.
- [x] Add systemd user service/socket skeleton.
- [x] Add threat model doc.
- [x] Add backend notes doc.
- [x] Implement Phase 1.
- [x] Implement Phase 2.
- [x] Implement Phase 3. Current status: keyboard and pointer command paths including bounded pointer drag, uinput setup diagnostics/docs, portal/libei input backend probes, pointer calibration diagnostics, and guarded KWrite GUI input smoke exist.

## 20. Definition of done for v0.1

v0.1 is complete when:

- PlasmaPilot daemon runs under the user session.
- Codex CLI can connect through MCP.
- Codex can observe the screen.
- Codex can list and focus windows.
- Codex can click/type with uinput.
- Actions are policy-checked and journaled.
- Panic-stop works.
- Clipboard get/set exists with policy checks.
- The Codex plugin bundle contains working MCP config and at least one useful skill.
- Basic KDE Plasma 6 Wayland manual evals pass.

## 21. Definition of done for v0.2

v0.2 is complete when:

- AT-SPI focused tree works.
- Semantic button/text/menu actions work for common KDE apps.
- Coordinate mapping is reliable with scaling and multiple monitors.
- Window guards are used by default. Current status: `[safety].require_focus_guard` defaults to true, so every control-class request must include an active-window guard before backend execution unless a scoped local development config explicitly opts out.
- Plugin hooks provide useful audit summaries. Current status: the Stop hook writes compact repo/journal metadata plus aggregate counts for failures, control actions, unguarded control actions, methods, safety classes, clients, and last active-window context; `make validate-plugin` verifies the aggregation behavior.
- Docs explain installation on Arch Linux/KDE Plasma 6. Current status: `docs/arch-kde-install.md` covers package prerequisites, binary install, config, user service, KWin bridge, safe diagnostics, optional uinput, Codex plugin validation, approval flow, and troubleshooting.

## 22. Development priority

The correct order is:

```text
health daemon
  -> screenshot
  -> uinput
  -> window list/focus
  -> MCP
  -> skills/plugin
  -> clipboard
  -> AT-SPI
  -> semantic actions
  -> KWin plugin/custom KDE improvements
```

Do not start with kernel or custom KDE changes. Those can make the project stronger later, but the first win is a working local daemon + MCP + portal/KWin/uinput screenshot and input path that Codex can use today. Custom KDE or kernel work becomes appropriate only after the supported portal, KWin script, AT-SPI, libei, and uinput paths are measured and a specific gap remains.
