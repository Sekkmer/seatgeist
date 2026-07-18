# Computer-Use Modernization Acceptance Ledger

This ledger separates implementation evidence from live KDE behavior. A green
unit, protocol, or mock test does not prove compositor behavior such as hidden
window capture, popup inclusion, background Firefox semantics, or cooperative
focus under concurrent physical input.

Status meanings:

- `PROVEN`: current evidence directly covers the requirement.
- `AUTOMATED`: protocol/policy/mock coverage exists; live application evidence
  is still required where the requirement names desktop behavior.
- `PENDING LIVE`: the implementation exists but the required KDE scenario has
  not been recorded.
- `CONTRADICTED`: current evidence shows the target is not yet met.

## Slice 1: one-call result and observation

| Requirement | Status | Evidence |
| --- | --- | --- |
| Bounded screenshot is native MCP image content in one call | PROVEN | MCP unit/integration tests cover MIME, bounds, and attachment behavior. |
| Semantic action returns a settled observation in the same call | AUTOMATED | Daemon/MCP action observation and settle tests pass; live common-app behavior remains part of the round-trip eval. |
| An action can return a matching bounded retained frame in the same call | AUTOMATED | Protocol/MCP attachment, target/session correlation, bounds, and journal tests pass; live target/revision evidence remains pending. |
| Text-field result does not echo sensitive replacement text | PROVEN | Protocol, MCP, and journal redaction tests cover content absence. |
| Screenshot bytes stay out of structured JSON and journal | PROVEN | MCP attachment and journal artifact tests cover the boundary. |

## Slice 2: bounded model-facing facade

| Requirement | Status | Evidence |
| --- | --- | --- |
| Core profile exposes at most eight tools | PROVEN | Core integration test exposes exactly six tools. |
| One logical `act` maps through existing policy/journal path | PROVEN | Facade mapping, policy denial, and stdio integration tests. |
| `window_session` explicitly opens, inspects, renews, and closes | PROVEN | Protocol, CLI, MCP schema/routing, policy, and interaction-store tests cover id-checked renewal without portal or input side effects. |
| Retained capture and sticky interaction state is bound to a trusted client owner | PROVEN | The daemon derives owners from same-UID Unix peer credentials. MCP sessions are process-scoped; verified CLI lifecycles are tool-scoped across invocations. Renew/snapshot/wait/close, sticky raw actions, and post-action images reject a foreign owner before frame, focus, or input side effects. Store, dispatcher, protocol, and baseline tests cover the boundary. |
| Session state records all selected backend/policy/settle metadata | PROVEN | A dedicated execution store records the capture backend at open, target-policy result, resolved semantic/raw action backend, successful action method/id/safety class/policy result, cooperative-focus policy, trusted activity provenance, focus-lease outcome, and final settle result. Read-only lifecycle calls do not overwrite the last action. Protocol, store, dispatcher, privacy, and compact MCP tests cover the record. |
| Repeated readiness does not create repeated daemon calls | PROVEN | Real-daemon MCP test proves two consecutive status calls create one journal request; another tool invalidates the cache. |
| Routine repeated session actions need no model focus/window polling | PENDING LIVE | Analyzer and harness are ready; post-restart 20-action trace is missing. |
| Expert protocol compatibility remains available | PROVEN | Workspace and stdio expert/all profile tests pass. |

## Slice 3: retained window capture

| Requirement | Status | Evidence |
| --- | --- | --- |
| Production retained window ScreenCast/PipeWire lifecycle is trait-backed | PROVEN | The runtime injects `Arc<dyn ScreenBackend>`; the production adapter owns portal/PipeWire construction and restore state, and a recording-backend test proves daemon routing through the trait. |
| Retained monitor and virtual-output source contracts exist | AUTOMATED | Portal options, PipeWire stream validation/metadata, production trait routing, protocol, CLI, and expert MCP tests cover exact source masks without persistence or desktop fallback. Live KDE support remains unproven. |
| Screenshot v3 target and visible-crop compatibility adapters implement `ScreenBackend` | PROVEN | Both use a modular one-shot `CaptureSession` coordinator behind `ScreenBackend`. Screenshot v3 target requests preserve portal authority and never fall back to Spectacle; explicit `visible_window_crop_id` resolves exact KWin geometry, fails closed for uncertain layouts, reports `backend=visible_window_crop` plus `occlusion_possible=true`, and returns bounded window-local output. Protocol, CLI, MCP, routing, geometry, privacy, and mock execution tests pass. |
| Repeated snapshot/wait reuses one retained consent session | AUTOMATED | Retained-session unit/protocol tests pass. |
| A requested-window session can reuse rotated portal consent after daemon restart | LIVE PROVEN | Private token-store and portal option tests cover persistence. On 2026-07-11 the two-phase live runner proved a changed daemon socket, the same opaque target reference, no repeated chooser, a fresh bounded Firefox frame, atomic private-state replacement, and cleanup of both sessions. The private artifact is under `target/seatgeist-capture-restore-eval/restart-20260710T225125Z-2298086/`; the final bundle accepts it only if its worktree fingerprint matches every other artifact. |
| Closing and reopening the target cannot transfer sticky authority to a replacement | LIVE PROVEN | On 2026-07-11 the disposable Firefox target was replaced by a distinct KWin id and PID while Konsole stayed active. The old portal stream ended as `portal_closed`, status cleared the sticky target, and no focus or raw-input call occurred. The private artifact is under `target/seatgeist-target-reopen-eval/target-reopen-20260710T225319Z-2301597/`; the final bundle will regenerate it after remaining edits so every artifact has one worktree fingerprint. |
| Window session never silently returns full desktop | PROVEN | Core snapshot/wait require a retained session id; whole-desktop compatibility tools are expert-only, and routing/schema tests fail closed when the id is absent. |
| Portal cancellation is structured and excluded from reliability defects | PROVEN | Daemon error classification returns `consent_cancelled`; the baseline analyzer categorizes it as an expected user outcome rather than backend failure. |
| Visible, unfocused, occluded, minimized, popup, resize/scale, revocation, and multi-monitor behavior | PENDING LIVE | Daemon restart and close/reopen identity are live-proven separately. The isolated KWin fixture live-proves ScreenCast v4, private bridge/daemon isolation, two outputs with a non-zero origin, host-AT-SPI preservation, and clean teardown. Its first visible matrix found that Seatgeist retained the oldest unread PipeWire frames. After replacing that FIFO behavior with a bounded latest-value mailbox, a focused live rerun passed minimized, Firefox popup, a 583x540 logical resize at 1.5 scale producing 875x810 pixels, and WL-0 to WL-1 movement with fresh target-only frames. Focused partial evidence is diagnostic; the complete eight-scenario and revocation artifacts are still required. Negative-origin math remains automated because current KDE Wayland normalizes live output origins. |

## Slice 4: target-centric background semantics

| Requirement | Status | Evidence |
| --- | --- | --- |
| Resolve exact KWin/AT-SPI target before app policy and side effect | PROVEN | Target correlation and policy-order tests pass. |
| Denied app, secret/destructive action, ambiguity, and reopen mismatch fail closed | PROVEN | Policy and target tests pass. |
| Target-scoped AT-SPI event settle avoids active-window dependence | AUTOMATED | Event filter/subscription and fallback tests pass. |
| Firefox and KDE semantic actions work while the user continues working in non-target windows | LIVE PROVEN | On 2026-07-11 the version-2 live producers passed against the disposable Firefox button and KCalc numeric button while Firestorm remained active. AT-SPI state proved the target effects, and daemon journals proved exact KWin/app/PID guards, `atspi` execution, non-target focus before/after, and no focus/raw calls. The private artifacts are under `target/seatgeist-step12-live/background-firefox-5/` and `background-kde-2/`; the final eight-artifact bundle will regenerate them after remaining Step 12 edits so every workspace fingerprint matches. |

## Slice 5: sticky target and cooperative focus lease

| Requirement | Status | Evidence |
| --- | --- | --- |
| Sticky raw action re-resolves target, focuses, verifies, then injects once | PROVEN | Session, identity, policy, focus verification, and journal correlation tests pass. |
| Trusted physical activity is distinct from exact Seatgeist uinput/EIS sources | AUTOMATED | KWin plugin builds; daemon payload/privacy/provenance tests pass. |
| Unknown provenance fails closed for restoration | PROVEN | Activity tracker tests and restoration guards cover this rule. |
| Twenty deliberate focus changes produce zero wrong-window input and safe restoration | PENDING LIVE | The KWin 6.7.2 activity plugin is loaded with trusted provenance. A live attempt proved the safety boundary: after exact method approvals, WindowsRunner returned success but the bridge never confirmed Firefox active, so `FocusLeaseConflict` stopped before input. Direct KWin runner and experimental internal activation probes also reverted to the user's Konsole within 50 ms; the experimental focus plugin was removed. This is no longer treated as a timeout/retry problem. Same-seat raw focus acceptance remains pending while Step 13 evaluates separate-seat/session designs; background semantic actions remain the preferred non-focus path. |

## Error and round-trip budget

The latest bounded pre-modernization live slice contains six model requests,
two failures, two preflights, three observations, one explicit focus request,
no sticky action, and a 20% reliability-failure rate. Therefore:

| Budget | Status |
| --- | --- |
| Reliability failures below 5% | CONTRADICTED by pre-change evidence; post-change trace pending |
| Wrong-window raw input zero | PENDING LIVE |
| Repeated portal prompts zero | PENDING LIVE |
| Model-driven focus polling zero | CONTRADICTED by pre-change evidence; post-change trace pending |
| All successful sticky actions have daemon focus verification | PENDING LIVE |
| All successful sticky actions pass a final trusted-activity check before injection | PENDING LIVE |

## Step 13: multi-use research

| Requirement | Status | Evidence |
| --- | --- | --- |
| Nested compositor has private session-bus and Wayland namespaces | LIVE PROVEN | The headless `probe-nested-remote-desktop` fixture validates both isolation markers before probing. |
| Nested portal exposes a complete RemoteDesktop/EIS capability path | LIVE PROVEN | KDE reports RemoteDesktop v2, device mask `7` for keyboard/pointer/touch, and `CreateSession`, `SelectDevices`, `Start`, and `ConnectToEIS`. The probe creates no session and sends no input. |
| Agent input changes only the nested lane while host focus remains unchanged | LIVE PROVEN | On 2026-07-11 a visible nested KWin session retained its portal D-Bus owner, completed the libei handshake, bound keyboard/pointer capabilities, and exposed one resumed virtual device. One policy-approved F12 was journaled through `portal_remote_desktop`; the private nested Konsole received it, while the host active window remained the same Codex Konsole ID before and after. Artifacts: `target/seatgeist-gui-eval/remote-desktop-eis-session-20260711T005331Z-2672083/` and `target/seatgeist-nested-kde/fixture-20260711T005325Z-2671235/`. |
| Two nested lanes can run concurrently with independent focus and attribution | PENDING | Begin only after the single-lane isolation proof. |

Run the remaining acceptance sequence when live desktop evaluation resumes:

```bash
make kwin-activity-preflight
seatgeist-cli windows
WINDOW_ID=<approved-kwin-id> make gui-eval-retained-capture
# On a real or isolated multi-output layout with a non-zero origin:
REQUIRE_MULTI_OUTPUT_NONZERO_ORIGIN=1 WINDOW_ID=<approved-kwin-id> \
  make gui-eval-retained-capture
# Preferred isolated two-output path; requires the operator to be present:
I_AM_PRESENT=1 make gui-eval-nested-retained-capture
# Prepare writes an artifact directory, then restart seatgeistd outside the harness.
WINDOW_ID=<approved-kwin-id> make gui-eval-capture-restore-prepare
WINDOW_ID=<same-kwin-id> EVIDENCE_DIR=<prepare-artifact-dir> \
  make gui-eval-capture-restore-resume
WINDOW_ID=<approved-kwin-id> make gui-eval-capture-revocation
WINDOW_ID=<original-kwin-id> make gui-eval-target-reopen
# Run once with SCENARIO=firefox and once with SCENARIO=kde.
SCENARIO=<firefox-or-kde> TARGET_WINDOW_ID=<background-target-id> \
  USER_WINDOW_ID=<active-work-window-id> BUTTON_NAME='<safe accessible button>' \
  make gui-eval-background-semantic
WINDOW_ID=<firefox-kwin-id> make gui-eval-cooperative-sticky
```

Keep the worktree unchanged while recording those runs. Each runner embeds a
fingerprint containing the Git HEAD plus the complete tracked and untracked,
non-ignored worktree content. After all eight private JSON artifacts pass, run
the single offline gate below with the exact `evidence.json` paths printed by
the runners:

```bash
RETAINED_CAPTURE_EVIDENCE=<retained-capture-evidence.json> \
MULTI_OUTPUT_EVIDENCE=<multi-output-evidence.json> \
CAPTURE_RESTORE_EVIDENCE=<restart-evidence.json> \
CAPTURE_REVOCATION_EVIDENCE=<revocation-evidence.json> \
TARGET_REOPEN_EVIDENCE=<target-reopen-evidence.json> \
BACKGROUND_FIREFOX_EVIDENCE=<firefox-background-evidence.json> \
BACKGROUND_KDE_EVIDENCE=<kde-background-evidence.json> \
COOPERATIVE_STICKY_EVIDENCE=<firefox-sticky-live.json> \
make verify-cooperative-use-acceptance
```

This command only reads evidence files; it does not call the daemon, capture
pixels, focus windows, or send input. It requires owner-only regular files,
the exact eight artifact types and versions, passing scenario/budget fields,
one identical worktree fingerprint, and timestamps no more than 24 hours old
and spanning no more than 24 hours by default. The limits can be narrowed with
`ACCEPTANCE_MAX_AGE_HOURS` and `ACCEPTANCE_MAX_SPAN_HOURS`. A successful run
writes an owner-only, path-free aggregate bundle to
`target/seatgeist-cooperative-acceptance/bundle.json`; set
`ACCEPTANCE_OUTPUT` to choose another path.

Any source change invalidates the set and requires all live artifacts to be
recorded again. Only after this aggregate gate passes may step 12 be marked
complete and the post-slice multi-use research matrix begin.
