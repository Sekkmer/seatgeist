# Mac Computer-Use Research Notes

Last checked: 2026-07-05.

This note captures Mac computer-use patterns that should influence PlasmaPilot live evals on KDE.

## Source Summary

- OpenAI Codex Computer Use runs on macOS and Windows. On macOS it requires Screen Recording for observation and Accessibility for clicking, typing, and navigation. OpenAI's safety guidance emphasizes scoped tasks, app approvals, user takeover, and avoiding sensitive flows unless the user is present.
  Source: https://developers.openai.com/codex/app/computer-use
- Anthropic's computer-use tool is screenshot, mouse, and keyboard oriented. Their docs emphasize that implementers own the actual screenshot capture, action execution, error handling, and coordinate mapping. They specifically warn that oversized screenshots may be downscaled, and that coordinate mappings must account for scale factors and Retina/device-pixel-ratio differences.
  Source: https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool
- macOS-specific agents increasingly prefer the Accessibility API/AX tree for reliable element targeting. MacOS-Use reads the macOS Accessibility API and advertises click/type/scroll/drag, AppleScript/shell, browser AX-tree scraping, window management, and Spaces support without relying on a vision model for every decision.
  Source: https://github.com/CursorTouch/MacOS-Use
- The macos-use project frames the AX tree as the main reliability primitive: element labels drive clicks instead of guessed pixels, and each tool returns an updated accessibility-tree diff so the agent can see what changed.
  Source: https://macos-use.dev/
- Hermes' macOS computer-use skill uses a capture-first workflow with screenshots plus numbered overlays and an AX index, clicks by element index as the preferred targeting mode, and re-captures after state-changing actions. It also scopes captures to an app to reduce noise and avoid leaking unrelated windows.
  Source: https://hermes-agent.nousresearch.com/docs/user-guide/skills/bundled/apple/apple-macos-computer-use
- Mac pyautogui-style demos still exist, but they require Accessibility permission for mouse and keyboard control and carry broad local-control risk.
  Source: https://github.com/PallavAg/claude-computer-use-macos
- Positive Mac reports also emphasize the same direction: Codex's stronger computer-use examples rely on a window hierarchy/accessibility representation, can work in background apps, and are most useful when no API, CLI, or AppleScript path exists.
  Source: https://www.macstories.net/notes/openais-new-codex-app-has-the-best-computer-use-feature-ive-ever-tested/

## Reported Gaps And Complaints

- Permission setup is still painful. Mac users report the Screen Recording plus Accessibility path as a multi-panel system-settings flow, followed by per-app approval prompts.
  Source: https://findskill.ai/blog/openai-codex-mac-48-hours/
- Permissions and approvals are easy to misdiagnose. Users have reported failures after Screen Recording and Accessibility were already granted, including a Codex helper issue where the visible error hid a lower-level macOS permission/entitlement denial, and a separate thread where browser use was blocked by app-approval policy rather than OS permissions.
  Sources: https://github.com/openai/codex/issues/18507 and https://www.reddit.com/r/codex/comments/1snlvgy/the_updated_computer_use_is_absolutely_insane/
- Device, version, and packaging differences matter. A Codex issue reports Locked Computer Use failing on an Intel Mac despite Screen Recording and Accessibility being enabled, with code-signing evidence in the x86_64 app path. Treat this as a reminder that backend diagnostics must report the exact runtime path, not just a broad "permission missing" label.
  Source: https://github.com/openai/codex/issues/28853
- Screenshot-heavy workflows are expensive and fragile. Reported failure modes include OCR errors in dense UI, stale coordinates after window moves, large context/token cost, off-screen UI invisibility, and image limits. AX-first tools argue for screenshots only when visual confirmation is needed.
  Source: https://fazm.ai/alternative/accessibility-tree-vs-screenshots
- Accessibility-first workflows still need fallbacks. AX-style agents degrade on apps with weak or missing accessibility trees, including Electron text trees, Qt apps without the right bridge, OpenGL/Metal/web canvases, and some Python/Tk UIs; equivalent KDE evals should expect AT-SPI gaps and prove the fallback path is explicit.
  Source: https://fazm.ai/t/computer-use-native-app-accessibility-limits
- Multi-step and multi-app workflows are still flaky. One review classifies app opening, document lookup, simple data extraction, and forms as reliable, but multi-app workflows, presentation creation, complex spreadsheets, email/Slack handoff, ambiguous UI, long scrolling, pixel-precise interactions, complex drag/drop, and multi-monitor setups as weak.
  Source: https://findskill.ai/blog/claude-computer-use-honest-review/
- Computer use should not replace better structured paths. In one Codex-on-Mac review, an Asana setup failed through GUI automation but succeeded quickly via CSV import. That reinforces using APIs/imports/plugins before desktop control when available.
  Source: https://findskill.ai/blog/openai-codex-mac-48-hours/
- Background execution can still fail when the host app is unfocused. Cursor users reported agents hanging until a window was refocused; Cursor attributed it to Chromium background throttling and macOS App Nap interfering with output polling.
  Source: https://forum.cursor.com/t/agents-hang-when-window-is-not-in-focus/155444
- Accessibility IDs are not always stable. Users report enterprise apps with dynamic automation IDs; robust selectors combine role, name, and parent/ancestor context instead of using IDs alone.
  Source: https://www.reddit.com/r/AI_Agents/comments/1sw7akp/when_your_computer_use_agent_should_look_at/
- Users worry about security and data leakage when screenshots can capture unrelated sensitive tabs, files, or banking sessions, especially when prompt-injected web content can influence a local agent with desktop control.
  Source: https://www.reddit.com/r/ArtificialInteligence/comments/1s24t3w/claudes_computer_use_is_great_but_security_risks/
- Users report rough edges around regional/device availability, rate limits, occasional screen takeover, and non-Latin text input. Treat these as anecdotal but useful test prompts for PlasmaPilot: backend availability must be explicit, throttling must be visible, foreground takeover must be avoidable or documented, and non-US keyboard paths need dedicated coverage.
  Source: https://findskill.ai/blog/openai-codex-mac-48-hours/

## Implications For PlasmaPilot

- Keep KDE preflight explicit. `desktop-session-status`, `safety-status`, `kwin-bridge-status`, `input_backend_status`, `capture-backends`, and `pointer-calibration` should remain first-class eval steps before live control.
- Prefer semantic/AT-SPI actions before pixels. Our KDE equivalent of the Mac AX pattern is AT-SPI: `a11y_find`, semantic actions, ambiguity refusal, and candidate summaries should be tested before raw pointer fallback.
- Keep pixel tests focused on mapping, not blind clicking. Large 8K screenshots must be downscaled or tiled with transform metadata, and every visual eval should verify source/output dimensions and coordinate transforms.
- Re-observe after state changes. Live evals should capture fresh active-window, journal, and where useful screenshot/AT-SPI state after every control action.
- Scope observation. When possible, prefer active-window or tile captures over full desktop captures, and keep full-resolution requests policy-gated.
- Treat local user input as a hard stop. Mac guidance repeatedly assumes user takeover is possible; PlasmaPilot's KDE equivalent is `pause_on_human_input`, panic-stop, and active-window guards. These should be verified in deterministic smokes as well as opt-in GUI evals.
- Make backend limitations observable. Portal visibility, libei readiness, KWin bridge state, AT-SPI availability, app policy, active-window guard state, and rate limits should be visible before a live action starts.
- Separate OS/session/backend failures from policy failures. Diagnostic outputs should tell the operator whether a request was blocked by app policy, missing approval, portal/session availability, AT-SPI tree quality, code/runtime packaging, or a safety guard.
- Avoid ID-only semantic targeting. Long-term semantic selectors should prefer role/name/path/ancestor context, with volatile ids treated as hints.
- Prefer structured integration over GUI automation. If a plugin, MCP server, import file, or app API can perform a task deterministically, PlasmaPilot should make the desktop path an explicit fallback.

## Added KDE Test Coverage

- `make smoke-human-input-pause` starts a private daemon with `[safety].pause_on_human_input = true`, writes a fresh activity signal, verifies `safety-status` reports the signal as fresh, then confirms an approved focus-control request is denied and journaled before backend execution. This is included in `make verify`.
- `semantic_resolvers_do_not_depend_on_stable_node_ids` verifies that high-level button and visible menu-path resolution still works when AT-SPI node IDs change between sessions but role/name/path/action context stays stable. Ambiguity summaries now include stable semantic candidate IDs and 1-based choice indexes alongside raw node IDs, so replay/eval diagnostics can refer to candidates without depending on volatile AT-SPI identifiers.
- `plasma-pilot-cli atspi quality-status`, daemon `accessibility_quality_status`, and MCP `plasma.a11y_quality_status` report whether a bounded focused AT-SPI tree looks useful for semantic targeting, including flat/generic/empty-tree signals and a fallback recommendation. The status replay trace and MCP smoke cover this diagnostic.
- Unit fixture coverage now exercises flat and mostly generic weak accessibility trees through the same quality-status response and compact operator summary emitted by the daemon.
- Daemon error responses now carry structured `kind` metadata for missing approval, explicit policy denial, app denial, focus guard, human-input pause, panic-stop, rate limit, portal/backend availability, backend failure, accessibility availability/weak-tree, validation, and unknown failures. MCP compact text includes the kind; policy/input denial replay traces assert `/data/kind = policy_prompt_required`; trace replay reports `error_kind`; configured-daemon CLI replay coverage proves app-policy, focus-guard, human-input-pause, forced portal-unavailable, and accessibility-unavailable kinds without unsafe desktop control or portal consent UI. MCP stdio integration now covers the same configured-denial categories through private daemons and asserts structured `data.kind` plus compact error text.
- `wait_for_change` now reports explicit watchdog metadata, including `timed_out`, requested timeout, and poll interval, so a stalled/no-change visual poll can be distinguished from command failure or screenshot backend failure.

## Future Eval Candidates

- Add an opt-in app-scoped screenshot eval that compares active-window capture/tile behavior against full-screen preview metadata.
- Add a post-action observation contract for opt-in GUI smokes: every live control eval should require a follow-up active-window or AT-SPI read plus journal evidence before considering the action complete.
- Add a long-scroll semantic eval that proves the agent can inspect more than one viewport through AT-SPI before falling back to screenshots.
- Add non-US keyboard and IME coverage for explicit EIS/keymap paths before claiming broad text-entry reliability.
