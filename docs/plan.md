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

## 2.1 Fact-check notes and source anchors

The plan was checked on 2026-07-04 against current public documentation:

- OpenAI Codex docs: Codex Computer Use is documented for macOS and Windows in the Codex app, not Linux/KDE. This validates PlasmaPilot as a Linux/KDE-specific local integration rather than a duplicate of an existing official Linux Computer Use path. Source: <https://developers.openai.com/codex/app/computer-use>
- OpenAI Codex docs: plugins can bundle skills, MCP server config, app integrations, and hooks; plugin manifest paths should be relative and start with `./`. Source: <https://developers.openai.com/codex/plugins/build>
- OpenAI Codex docs: Codex supports MCP servers in CLI and IDE, including stdio servers with `command`, `args`, `env`, tool allow/deny lists, and approval modes. Source: <https://developers.openai.com/codex/mcp>
- OpenAI Codex docs: skills are the authoring format for reusable workflows and plugins are the installable distribution unit; skill descriptions should front-load trigger terms because Codex uses progressive disclosure. Source: <https://developers.openai.com/codex/skills>
- OpenAI Codex docs: hooks can be bundled with enabled plugins, but non-managed hooks must be reviewed and trusted before they run. Source: <https://developers.openai.com/codex/hooks>
- Rust/Cargo docs: virtual workspaces should set the resolver explicitly, and `resolver = "3"` is the Rust 2024 resolver behavior. Sources: <https://doc.rust-lang.org/cargo/reference/workspaces.html> and <https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html>
- KDE developer docs: KWin scripts are installable with `kpackagetool6 --type=KWin/Script`, enabled through `kwriteconfig6`, and can manipulate windows through the KWin scripting API. Sources: <https://develop.kde.org/docs/plasma/kwin/> and <https://develop.kde.org/docs/plasma/kwin/api/>
- KDE developer docs: D-Bus is a common KDE/freedesktop IPC layer and underpins portals and many desktop services. Source: <https://develop.kde.org/docs/features/d-bus/introduction_to_dbus/>
- XDG Desktop Portal docs: ScreenCast and RemoteDesktop portals expose monitor/window capture and keyboard/pointer/touchscreen remote-desktop sessions. Sources: <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html> and <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html>
- freedesktop/libei docs: libei is a Wayland-oriented emulated-input protocol with client/server separation over Unix sockets. Source: <https://libinput.pages.freedesktop.org/libei/>
- Linux kernel docs: uinput lets a userspace process create virtual input devices by writing to `/dev/uinput`; this is a viable privileged fallback, but it should not be the only Wayland plan. Source: <https://docs.kernel.org/input/uinput.html>
- AT-SPI2 docs: AT-SPI is a D-Bus protocol used by toolkit widgets to expose content to assistive technologies. Source: <https://www.freedesktop.org/wiki/Accessibility/AT-SPI2/>

Design consequence: the first implementation should keep multiple KDE/Wayland observation and control paths available: xdg-desktop-portal for consented screen/remote-desktop flows, KWin scripting or a KWin plugin for compositor-native window metadata, AT-SPI for semantic UI, libei where KDE exposes an EIS path, and uinput as a controlled privileged fallback.

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
│   ├── uinput-setup.md
│   ├── kde-wayland-notes.md
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
plasma-pilot-cli screenshot --output /tmp/screen.png
plasma-pilot-cli windows
plasma-pilot-cli active-window
plasma-pilot-cli focus --window <id>
plasma-pilot-cli input click-pointer --x <x> --y <y> --coordinate-space physical-pixel --button left
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

Future crate for xdg-desktop-portal integration if it becomes large enough to split from `plasma-pilot-backend`.

Responsibilities:

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
- [x] Produce screenshot change thresholds for `plasma.wait_for_change`. Current status: the daemon polls bounded screenshots, computes normalized RGB deltas, and returns changed/captures/elapsed/score metadata plus latest screenshot metadata.
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
    fn click(&self, button: MouseButton) -> Result<()>;
    fn drag(&self, from: Point, to: Point, opts: DragOptions) -> Result<()>;
    fn scroll(&self, dx: i32, dy: i32) -> Result<()>;
    fn key_combo(&self, combo: KeyCombo) -> Result<()>;
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

Clipboard reads should be policy-controlled because clipboard content often contains secrets. The current MCP names are `plasma.clipboard_get_text` and `plasma.clipboard_set_text`.

### 11.4 Accessibility tools

```text
pilot.a11y_focused_tree(depth?)
pilot.a11y_find(role?, name_contains?, app?, window_id?)
pilot.a11y_invoke(node_id, action, guard?)
pilot.a11y_set_text(node_id, text, guard?)
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

Initial `hooks.json` should be conservative and may be a placeholder until exact Codex hook command schema is validated in the current installed Codex version.

Current scaffold note: `plugin/hooks/hooks.json` intentionally contains no active hooks. This avoids shipping untrusted lifecycle commands before the hook behavior and trust flow are tested locally. `plugin/.codex-plugin/plugin.json` does not override the hook path; the bundle relies on default `hooks/hooks.json` discovery while the skeleton is disabled.

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

Current implementation: daemon requests are classified before execution and evaluated by `plasma-pilot-policy`. Observe/status requests are allowed by default. Prompt decisions fail closed because no trusted approval channel exists yet.

`plasma-pilotd` now reads the config file from `~/.config/plasma-pilot/config.toml`, or from `--config` / `PLASMA_PILOT_CONFIG` when provided. Implemented fields are `[daemon].socket`, `[daemon].journal`, `[daemon].panic_stop_file`, these `[policy]` keys: `default_observe`, `default_control`, `destructive_actions`, `secret_fields`, `default_clipboard_read`, `default_clipboard_write`, and `full_resolution_screenshot`, `[apps].allow` and `[apps].deny`, plus `[safety].require_focus_guard`, `[safety].pause_on_human_input`, `[safety].human_input_activity_file`, `[safety].human_input_quiet_ms`, and `[[safety.redact_regions]]`. CLI arguments and environment-backed flags take precedence over file values, so explicit local approval flags such as `--allow-control`, `--allow-clipboard-read`, and `--allow-full-resolution-screenshot` still override prompt/deny defaults for intentional local runs. App deny rules win over allow rules; control fails closed if an app policy is configured and the relevant app id cannot be determined. When `require_focus_guard` is true, every control-class request must include an active-window guard before backend execution. When `pause_on_human_input` is true, a fresh activity signal file blocks control-class requests before backend execution. Configured screenshot redaction regions are physical-pixel source rectangles and are black-filled in output PNGs before screenshot metadata is returned. Destructive semantic requests and obvious destructive button/menu labels are classified separately as `DestructiveAction` and use `[policy].destructive_actions`, which defaults to prompt/fail-closed. High-level text-field targets with secret-looking names are classified as `SecretField` and use `[policy].secret_fields`, which defaults to deny.

## 15. Action journal

Write JSONL records to:

```text
~/.local/state/plasma-pilot/journal.jsonl
```

Current implementation: `plasma-pilotd` appends compact request records containing `sequence`, `unix_time_ms`, `method`, `ok`, and `summary`. `plasma-pilot-cli journal tail --limit N` returns recent records through the daemon and supports `--method <name>` and `--ok <true|false>` filters. Smoke tests pass target-local journal paths and verify `0600` file permissions.

Future journal expansion should preserve the compact tail format while adding the richer action context below for control operations:

Each record:

```json
{
  "ts": "2026-07-04T15:00:00+02:00",
  "client": "plasma-pilot-mcp",
  "tool": "pilot.click",
  "action_id": "uuid",
  "safety_class": "control.pointer.click",
  "requested_target": {"x": 100, "y": 200, "space": "LogicalPixel"},
  "active_window_before": {"app_id": "org.kde.kate", "title": "main.rs"},
  "policy": "allow",
  "backend": "kwin+uinput",
  "result": "ok",
  "active_window_after": {"app_id": "org.kde.kate", "title": "main.rs"}
}
```

Screenshots should not be stored in the journal by default. Store paths/hashes only if enabled.

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
- Save screenshots to `$XDG_RUNTIME_DIR/plasma-pilot/screenshots/`.
- Add `plasma-pilot-cli screenshot`.
- [x] Add default downscaled previews and explicit full-resolution capture. Current status: bounded previews are observe-class, while direct and observe-attached full-resolution screenshot requests are classified separately and prompt by default until the daemon is started with explicit full-resolution screenshot approval.
- Add tiled screenshots for 8K and multi-monitor workflows. Initial implementation supports physical-pixel tile crops with max-edge downscaling.
- Add coordinate transform metadata for preview/crop/full-size mapping. Initial preview/full-size mapping is implemented with scale factors and source/output dimensions.

Acceptance criteria:

- CLI can capture current screen to PNG.
- Screenshot response includes coordinate metadata, source dimensions, output dimensions, source origin, and preview/tile scale factors.
- Multi-monitor metadata is correct or explicitly marked unsupported. Initial implementation reports KWin logical geometry, physical pixel dimensions derived from scale, origin, and scale factor.

### Phase 3: uinput keyboard and pointer control

Goal: Codex can click/type through controlled virtual input.

Tasks:

- [x] Implement initial keyboard input backend using uinput. Current status: `plasma-pilot-uinput` creates a short-lived `/dev/uinput` virtual keyboard with `UI_DEV_SETUP`; daemon/CLI/MCP expose `type_text` and `key_combo` as policy-gated `ControlKeyboard`.
- [x] Create virtual pointer device and absolute/relative pointer mapping. Current status: `plasma-pilot-uinput` creates a short-lived `/dev/uinput` virtual pointer with absolute X/Y axes and relative wheel axes; the daemon maps physical desktop coordinates into the absolute input range.
- [x] Add udev/polkit/systemd instructions. Current status: skeleton files exist, `docs/uinput-setup.md` documents the optional udev rule, user service setup, current polkit placeholder state, and `input status` diagnostics. Uinput access still relies on `/dev/uinput` being readable/writable by the daemon process.
- [x] Implement move, click, double-click, and scroll. Current status: daemon/CLI/MCP expose `move_pointer`, `click_pointer`, and `scroll_pointer` as policy-gated `ControlPointer`; click supports one or two left/middle/right clicks, and scroll supports vertical/horizontal deltas.
- [x] Implement key combo and type text. Current status: US evdev ASCII text plus newline/tab and named key combos such as `Ctrl+L` are supported; non-US text is rejected instead of guessed.
- [x] Implement focus guard checks before actions. Current status: current daemon control requests accept optional active-window guards (`expected_active_window`, `expected_active_app`, and `active_title_contains`) and reject stale guards before execution.
- [x] Add panic-stop flag. Current status: `plasma-pilotd` has a file-backed panic-stop state, `plasma-pilot-cli panic-stop status|enable|disable` journals state changes, and active panic-stop blocks control-class daemon requests before execution.
- [x] Probe whether xdg-desktop-portal RemoteDesktop or libei can satisfy input needs before requiring uinput on the local machine. Current status: `plasma-pilot-cli input backends` and MCP `plasma.input_backend_status` probe the user bus for `org.freedesktop.portal.RemoteDesktop`, KDE portal service visibility, libei client metadata/socket hints, and uinput fallback availability without starting a portal session.
- [x] Add pointer calibration diagnostics. Current status: `plasma-pilot-cli input pointer-calibration`, MCP `plasma.pointer_calibration`, and `make smoke-pointer-calibration` report monitor-derived physical pointer bounds, per-monitor physical origins, and representative physical-pixel sample points without moving the pointer.
- [x] Add host GUI smoke for a known test window before treating pixel-click use as production-ready. Current status: `make smoke-gui-input` opens a disposable KWrite/Kate file, focuses it through KWin, requires an active-window guard, maps a window point to physical pixels through pointer calibration, clicks, types a sentinel through uinput, saves, verifies file content, and checks the journal.

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
- [x] Add active-window bridge installation diagnostics. Current status: `kwin_bridge_status` reports daemon DBus receiver state, bridge update state, and user-local package/config installation state.
- [x] Implement initial window list with stable KWin id, title, app id, and logical geometry through `WindowsRunner` plus `org.kde.KWin.getWindowInfo`.
- [x] Add pid and monitor association if a supported KWin, portal, or script path exposes them. Current status: daemon window and active-window responses derive `monitor_id` from the largest logical overlap between KWin window geometry and KWin monitor geometry; active-window bridge payloads preserve pid when KWin script provides it. The current `WindowsRunner`/`getWindowInfo` list path does not expose pid for all windows, so list pids remain `null` until a supported path is added.
- [x] Implement focus window. Current status: `plasma-pilot-cli focus --window <id>` uses KWin `WindowsRunner.Run` and is policy-gated as `ControlSemantic`; default policy fails closed without an approval channel, while `plasma-pilotd --allow-control` enables explicit local use.
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
- [x] Expose pointer move/click/scroll after the backing daemon capability exists.
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
- [x] Create hook skeleton. Current status: `plugin/hooks/hooks.json` exists with no active hooks and a disabled reason until hook schema and trust flow are validated locally.
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
  Current status: `make validate-plugin` checks the plugin manifest, MCP config, skill frontmatter, required skill set, and disabled hook skeleton; `make verify` runs the plugin validator.

### Phase 7: Clipboard backend

Goal: controlled clipboard integration.

Tasks:

- [x] Implement clipboard get/set for UTF-8 text.
- [x] Use `wl-copy`/`wl-paste` as the first Wayland backend when available.
- [x] Enforce clipboard read policy.
- [x] Add KDE/portal-native fallback and explicit provenance selection if `wl-copy`/`wl-paste` is unavailable. Current status: daemon clipboard reads/writes prefer `wl-clipboard` and fall back to KDE Klipper DBus (`org.kde.klipper`) when the Wayland commands are unavailable; `ClipboardText` responses and clipboard-set action summaries include backend provenance.
- [x] Truncate large clipboard reads by default.

Acceptance criteria:

- CLI can set and get text clipboard.
- MCP can set clipboard text for paste workflows.
- Clipboard reads are logged and policy-checked.
- Clipboard journal summaries and compact MCP status text do not echo clipboard contents. Current summaries include text length, truncation metadata, original byte count, and backend provenance only.

### Phase 8: AT-SPI semantic UI

Goal: Codex can use accessibility metadata instead of only pixels.

Tasks:

- [x] Implement focused accessibility tree.
- [x] Return compact node summaries: role, name, state, bounds, actions.
- [x] Return text/value summaries for supported nodes.
- [x] Implement find by role/name/app/window.
- [x] Implement policy-gated invoke action where `org.a11y.atspi.Action` is supported.
- [x] Implement set text where `org.a11y.atspi.EditableText` is supported.
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
  - [x] `pilot.select_menu(path, app/window guard)` for visible AT-SPI menu paths.
  - [x] `pilot.activate_tab(name, app/window guard)`
- [x] Use AT-SPI first for `click_button`, `set_text_field`, `select_menu`, and `activate_tab`; screenshot+coordinate fallback remains future work and must only happen when safe.
- [x] Add ambiguity refusal for `click_button`, `set_text_field`, `select_menu`, and `activate_tab`. Current status: ambiguous semantic matches fail closed and return bounded candidate choices with node id, role, name, and action metadata so the caller can disambiguate; broader confidence scoring remains future work.

Acceptance criteria:

- Codex can operate common KDE dialogs semantically.
- Ambiguous matches return choices instead of clicking randomly.

### Phase 10: Reliability and evals

Goal: prevent regressions and measure usefulness.

Tasks:

- Add mock backends for unit tests. Current status: `plasma-pilot-testkit` provides deterministic screen, window, input, clipboard, and accessibility mocks with call recording.
- Add integration tests for CLI and daemon protocol. Current status: daemon core protocol and low-risk CLI status commands have Rust integration tests; GUI/desktop CLI coverage remains in smoke targets.
- Add optional local GUI eval scripts. Current status: `scripts/gui-eval.sh` runs current non-control evals for daemon status, observe, default clipboard-read denial, bounded screenshot preview metadata, screenshot preview coordinate mapping, full-resolution screenshot policy denial, and journal output; `scripts/gui-eval.sh control-safety` and `make gui-eval-control-safety` start a private control-approved daemon and verify active-window guard denial plus panic-stop denial before backend control can execute.
- Add replayable action traces. Current status: `ReplayTrace` stores daemon requests with expected response metadata, and `plasma-pilot-cli trace replay --file <path>` replays each step through the daemon so policy checks and journaling still apply.
- Add screenshot/coordinate calibration tests. Current status: protocol tests cover mapping 8K downscaled previews and physical-pixel tiles back to source screenshot coordinates.

Suggested local evals:

```text
Eval 1: open Kate, type text, save file
Eval 2: open KCalc, calculate 2+2 visually
Eval 3: open Firefox, visit localhost, click a button
Eval 4: focus wrong window and verify guard rejection. Current status: `make gui-eval-control-safety` seeds private active-window state through the daemon KWin bridge and verifies an incorrect guard rejects focus before backend execution.
Eval 5: clipboard read denied by policy
Eval 6: panic-stop blocks input/control. Current status: `make gui-eval-control-safety` enables a private panic-stop file and verifies a focus control request is rejected before backend execution.
Eval 7: 8K screenshot preview maps clicks back to source coordinates. Current status: protocol tests cover the transform math, and `scripts/gui-eval.sh screenshot-coordinate-map` validates real screenshot metadata maps a preview center point back inside the source screenshot bounds.
Eval 8: full-resolution screenshot requires explicit policy approval. Current status: `scripts/gui-eval.sh full-resolution-denied` verifies full-resolution capture is rejected by policy before any output file is written.
```

Acceptance criteria:

- `cargo test --workspace` covers protocol/policy/tool validation.
- Manual eval script passes on KDE Plasma 6 Wayland.
- Failures produce useful diagnostics.

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
- [x] Implement Phase 3. Current status: keyboard and pointer command paths, uinput setup diagnostics/docs, portal/libei input backend probes, pointer calibration diagnostics, and guarded KWrite GUI input smoke exist.

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
- Window guards are used by default.
- Plugin hooks provide useful audit summaries.
- Docs explain installation on Arch Linux/KDE Plasma 6.

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
