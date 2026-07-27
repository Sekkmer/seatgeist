# Computer-Use Modernization Plan

Status: active implementation goal
Last updated: 2026-07-11

Implementation progress:

- [x] Step 1: task-level journal baseline analyzer and structured evidence.
- [x] Step 2: bounded screenshot results attach native MCP image content.
- [x] Step 3: default compact post-action observation and bounded settle metadata.
- [x] Step 4: bounded core MCP facade and explicit core/expert/all profiles.
- [x] Step 5: lifecycle capture traits and deterministic retained-session mock.
- [x] Step 6: portal ScreenCast consent/session/PipeWire-FD lifecycle.
- [x] Step 7: bounded PipeWire frames plus daemon-retained snapshot/wait sessions.
- [x] Step 8: resolved-target policy and separate target-window guards.
- [x] Step 9: target-scoped AT-SPI event settling for background semantics.
- [x] Step 10: sticky interaction sessions and daemon-owned focus reacquisition.
- [x] Step 11: trusted human-activity provenance and cooperative focus restoration.
- [x] Full retained-session backend, policy, activity, focus-lease, and settle metadata.
- [x] Screenshot v3 target and explicit visible-window crop `ScreenBackend` adapters.
- [x] Step 12 offline evidence schema and same-worktree aggregate verifier.
- [ ] Step 12 live KDE evidence and budget gate.
- [ ] Step 13 multi-use research in progress; the safe nested capability gate passes.

Requirement-by-requirement proof and remaining live gaps are tracked in
`docs/computer-use-modernization-acceptance.md`.

## 1. Goal

Move Seatgeist from a safe KDE computer-use prototype to a low-round-trip,
target-aware desktop-control substrate that can coexist with a person using the
same Plasma session.

The first milestone is not unrestricted autonomous desktop control. It is a
predictable interaction session in which Codex can be assigned a window, keep
that target across many actions, observe it without capturing the whole
desktop, and perform one policy-checked action plus verification in one model
round trip.

Longer term, Seatgeist should support multiple independent work lanes so that a
person and one or more Codex sessions can operate concurrently. That may require
a nested desktop session or compositor support. It must be investigated only
after the five user-facing modernization slices in this document are complete
and measured.

## 2. Current Evidence

The local KDE session inspected on 2026-07-10 reports:

- xdg-desktop-portal Screenshot interface version 2.
- No Screenshot `AvailableTargets` property and therefore no supported
  `window` or `active_window` target.
- xdg-desktop-portal ScreenCast interface version 5.
- ScreenCast source mask `7`, advertising monitor, window, and virtual sources.
- `xdg-desktop-portal-kde`, Plasma, and KWin version 6.7.2.

Screenshot v2 is usable only for full-desktop observation in this environment.
It is not a viable primary backend for target-window computer use. Seatgeist
must use a retained ScreenCast/PipeWire source for real window capture.

The current local Seatgeist journal contains 150 MCP requests, of which 25
failed. The requests include 22 active-window checks, 16 observations, 18
pointer-click attempts, and 13 focus attempts. Ten of the focus attempts failed.
This is not a controlled benchmark, but it confirms that focus checks,
preflights, portal cancellation, and post-action observation create excessive
round trips and recoverable errors.

Historical code gaps recorded before implementation (the acceptance ledger is
the current source of truth):

- MCP screenshot responses contain a path and metadata but no native MCP image
  content.
- `ActionResult` has an observation field, but current actions leave it empty.
- The plugin skill instructs the model to observe before and after most actions.
- The default MCP server exposes 59 tools, mixing common interaction tools with
  expert diagnostics.
- The daemon calls concrete capture, KWin, and AT-SPI functions in important
  paths instead of composing the existing backend traits.
- Human-input pause is a file-backed policy hook for a future activity watcher;
  it does not yet detect real physical keyboard or pointer activity.

## 3. Operating Modes

Seatgeist should define three explicit operating modes instead of treating every
desktop task as foreground-exclusive input.

### 3.1 Foreground-exclusive

This is close to the current behavior. Raw input targets the active desktop
seat, and the user should not interact with the desktop during the action.

Keep this mode for compatibility and high-risk debugging. Do not make it the
desired steady-state experience.

### 3.2 Cooperative sticky target

A Codex interaction session is pinned to a target window such as a particular
Firefox window. The target remains pinned even when the user focuses another
window between Codex actions.

The model does not repeatedly call `active_window`, `focus_window`, and
`observe` to re-establish the target. On each action the daemon:

1. Resolves the pinned target and validates its identity.
2. Prefers a background AT-SPI action that does not require focus.
3. For raw input only, checks real human activity, acquires a short seat lease,
   focuses the pinned target internally, executes and verifies one action, and
   conditionally restores the user's prior focus.
4. Returns the settled post-action observation in the same response.

Sticky means sticky target ownership, not permanently forcing the window to
remain focused. The user remains free to focus and use other windows between
agent actions.

### 3.3 Isolated work lane

Each human or agent receives an independent capture, focus, pointer, and
keyboard domain. Candidate implementations include a nested compositor, a
separate graphical login session, or future native KWin multi-seat/agent-seat
support.

This is the desired long-term mode for multiple parallel Codex sessions, but it
is a post-modernization research program rather than a hidden requirement of
the first five slices.

## 4. Architectural Direction

```text
Codex
  -> small MCP computer-use facade
  -> interaction-session coordinator
       -> resolved target + target policy
       -> capture session / frame watcher
       -> semantic action backend
       -> cooperative seat lease for raw input
       -> settle + compact post-action observation
  -> backend traits
       -> ScreenCast/PipeWire capture
       -> KWin window metadata/focus
       -> AT-SPI semantic control/events
       -> portal/libei or uinput raw input
       -> mocks
```

The session coordinator is responsible for orchestration. Backends remain
small, testable capability providers. MCP tools must not reproduce focus,
capture, policy, and retry state machines in the model prompt.

## 5. Cross-Cutting Invariants

The following requirements apply to every phase:

- Every externally requested action passes through the policy engine.
- Every internal side effect, including automatic focus and focus restoration,
  is journaled under a common parent action/session identifier.
- A high-level action never becomes an unrestricted batch or scripting bypass.
- App deny rules and secret/destructive classifications apply to the resolved
  target, not merely the currently active window.
- Panic-stop and fresh human input override sticky target ownership.
- A target may not silently rebind to a different window after close/reopen,
  PID change, portal-source change, or ambiguous identity.
- Bounded images may be returned to the model. Full-resolution images remain
  separately policy-gated and should not be embedded in MCP responses by
  default.
- Typed text, clipboard contents, screenshot bytes, and sensitive semantic
  names remain absent from journals.
- Backend failures are explicit. Window capture must not silently degrade to an
  unlabeled full-desktop screenshot.
- New KDE-private APIs require documented fallback and version behavior.

## 6. Slice 1: One-Call Result and Observation

### Objective

Make a screenshot or successful action directly usable by Codex without a
second view call or a separate post-action observation call.

### Work

1. Return bounded PNG results as MCP `image` content alongside compact text and
   structured metadata.
2. Keep image bytes out of `structuredContent` and the action journal.
3. Add explicit result controls such as:

   - `include_image`
   - `max_edge`
   - `observe_after`
   - `settle_timeout_ms`
   - `settle_condition`

4. Populate `ActionResult.observation` after successful actions.
5. Include only useful post-action state:

   - target window identity and revision
   - active window only when relevant
   - bounded target-window image when requested
   - compact accessibility delta or target-node state
   - settle result, elapsed time, and timeout state

6. Prefer event-driven settling. Use frame delta only when an AT-SPI or KWin
   event cannot express the expected change.
7. Change `wait_for_change` to reuse a retained capture stream instead of
   repeatedly opening full-desktop screenshot requests.

### Protocol rules

- `observe_after=false` remains available for low-level or latency-sensitive
  callers.
- `settle_condition=auto` chooses an AT-SPI state/value event for semantic
  actions and a frame change for pixel actions.
- Timeouts are successful watchdog results with `settled=false`, not generic
  backend errors.
- An image returned after an action uses the same target and target revision as
  the executed action.

### Acceptance

- A bounded screenshot is visible to Codex from one MCP call.
- A semantic button click returns the changed state from one MCP call.
- A text-field update returns the final field state or a compact state hash
  without echoing sensitive text.
- Screenshot and action-result MCP tests verify image MIME type, size bounds,
  metadata, and absence from the journal.

## 7. Slice 2: Small Model-Facing Facade

### Objective

Move routine orchestration out of the prompt and daemon client while retaining
the existing expert tools for diagnostics and compatibility.

### Proposed default tools

- `seatgeist.computer_status`
- `seatgeist.window_session`
- `seatgeist.snapshot`
- `seatgeist.act`
- `seatgeist.wait`
- `seatgeist.panic_stop`

Expert status, backend probes, raw AT-SPI operations, raw pointer operations,
portal lifecycle probes, and journal tools remain available through an expert
tool profile or separate expert MCP entrypoint.

### Interaction session

`window_session` opens, inspects, renews, or closes a bounded interaction
session. A session records:

- stable session id and client/agent identity
- requested window selector
- resolved KWin window identity, app id, PID, and generation
- optional user-approved ScreenCast source and restore token reference
- selected semantic, capture, and raw-input backends
- target policy result and expiry
- latest target revision and settle state
- cooperative focus policy
- last activity and lease expiry

Restore tokens and portal session details are sensitive local state. Store them
with restrictive permissions and expose only opaque references through MCP.

Implemented owner boundary: a retained capture records an owner derived from
the daemon socket's same-UID Unix peer PID plus its sanitized client identity.
Long-lived MCP servers are process-scoped, so a different MCP process cannot
renew, snapshot, wait, close, drive sticky raw input, or request a post-action
frame from the session. Verified `seatgeist-cli` requests are tool-scoped so
separate CLI invocations can complete an explicit manual lifecycle. The daemon
checks ownership before capture, focus, settle preparation, or input side
effects and returns the non-oracular `session_owner_mismatch` kind for missing,
wrong-id, missing-owner, and foreign-owner attempts. Status remains read-only
and exposes compact owner tool/PID/scope metadata for coordination. A dedicated
daemon store also retains capture, semantic, and raw-input backend selection;
target and last-action policy results; the successful method/action id/safety
class; cooperative-focus policy; trusted activity provenance; focus-lease
outcome; and the last final settle result. Read-only snapshot, wait, renew, and
status operations do not replace the last control-action record. Protocol and
compact MCP status never include window titles, typed values, or accessibility
content in this record.

### `act` contract

`act` represents one logical UI action:

- click a named control
- set a named text field
- select/toggle/set a semantic value
- send one bounded key sequence
- perform one bounded pointer click, scroll, or drag

It may internally resolve, focus, execute, settle, capture, and restore focus,
but each internal side effect remains individually policy-checked and
journaled. Arbitrary arrays of unrelated actions are out of scope.

### Compatibility

- Do not remove existing MCP tools in the first version.
- Add a tool-profile configuration with `core`, `expert`, and `all` modes.
- Update plugin skills to prefer the facade and use expert tools only after a
  structured error recommends one.

### Acceptance

- A common semantic interaction takes one action call after session creation.
- Repeated actions in one session do not repeat readiness, window-list, active
  window, or pointer-calibration calls.
- Core mode exposes no more than eight tools.
- Existing expert tools and protocol clients continue to work.

## 8. Slice 3: Real Window Capture and ScreenBackend Redesign

### Objective

Replace Screenshot v2 full-desktop capture as the primary observation path with
a retained user-approved ScreenCast/PipeWire window stream.

### Backend contract

The current one-shot `ScreenBackend::screenshot(target)` contract is too narrow
for consent, restore tokens, stream lifetime, frame revisions, damage, and
wait-for-change. Evolve it into lifecycle-oriented traits. Exact naming may
change during implementation, but the boundary should resemble:

```rust
#[async_trait]
pub trait ScreenBackend: Send + Sync {
    async fn capabilities(&self) -> Result<CaptureCapabilities>;
    async fn open_capture(
        &self,
        request: CaptureSessionRequest,
    ) -> Result<Box<dyn CaptureSession>>;
}

#[async_trait]
pub trait CaptureSession: Send + Sync {
    fn metadata(&self) -> CaptureSessionMetadata;
    async fn snapshot(&self, request: FrameRequest) -> Result<CapturedFrame>;
    async fn wait_for_frame(&self, request: FrameWaitRequest) -> Result<CapturedFrame>;
    async fn close(&self) -> Result<()>;
}
```

Required implementations:

- portal ScreenCast/PipeWire window stream
- portal ScreenCast monitor or virtual-output stream where supported
- explicitly labeled Screenshot v3 one-shot target when available
- explicitly labeled visible-window crop compatibility fallback
- deterministic mock capture session

The daemon must depend on these traits rather than directly calling portal or
Spectacle helpers.

### Portal/PipeWire work

1. Extend `seatgeist-portal` with ScreenCast lifecycle support:

   - `CreateSession`
   - `SelectSources` with one `WINDOW` source
   - `Start`
   - `OpenPipeWireRemote`
   - session `Closed` handling
   - restore token and persist mode

2. Add a PipeWire consumer behind a narrow frame-source trait.
3. Retain one capture session per interaction session instead of recreating the
   portal request per screenshot.
4. Capture the latest complete frame, preserve stream dimensions and transform
   metadata, and bound/downscale before returning it.
5. Track stream identity, source type, mapping id, frame sequence, format
   changes, session closure, and restoration failure.
6. Evaluate DMA-BUF and shared-memory formats, but land a reliable shared-memory
   path before optimizing zero-copy behavior.

Implemented non-live follow-up: the production `ScreenBackend` now routes
retained window, monitor, and virtual-output requests through exact portal
source masks and validates the returned PipeWire stream type before creating a
session. Core remains window-only; the other source contracts are expert-only,
non-persistent, and carry no sticky-control authority. Their live KDE behavior
is still part of the paused acceptance work. Screenshot v3 target capture and
visible-window crop compatibility now also use modular one-shot
`ScreenBackend`/`CaptureSession` adapters. A v3 target never falls back to a
full-desktop Spectacle image. Visible crop is expert-only, requires an exact
KWin id on one unrotated monitor, exposes `backend=visible_window_crop` and
`occlusion_possible=true`, and fails closed when desktop/geometry correlation
is uncertain.

### Target and consent behavior

The ScreenCast portal lets the user select a window; it does not authorize
Seatgeist to silently name any arbitrary KWin window. Session creation must make
that distinction clear:

- the requested KWin selector describes intended use
- the portal grants a specific capture source
- Seatgeist binds the approved stream to the interaction session only after
  validation/correlation
- ambiguous correlation requires user confirmation or fails closed

### Fallback behavior

- Screenshot v2 is full-desktop only and must never claim window capture.
- Screenshot v3 window/active-window requests are optional one-shot backends,
  not a substitute for a retained stream.
- Cropping a full desktop through KWin geometry is permitted only as an explicit
  compatibility mode with `occlusion_possible=true` and
  `backend=visible_window_crop` provenance.
- Portal cancellation is a structured `consent_cancelled` result. It should not
  trigger repeated automatic prompts.

### Live evals

- selected window visible and focused
- selected window visible but unfocused
- selected window partially and fully occluded
- selected window minimized
- popups, context menus, dialogs, and browser menus
- window move, resize, scale change, monitor move, and close/reopen
- restored session after daemon restart
- portal revocation and session closure
- multi-monitor layouts with non-zero logical origins, plus deterministic
  negative-origin coordinate coverage

The opt-in `scripts/retained-capture-eval.py` runner records the first eight
interactive source-state scenarios with one retained portal session. It
requires a fresh frame for each state, stores only bounded private PNGs plus a
hashed target identifier, makes no explicit focus call, and closes only the
session it opened. Run it through
`WINDOW_ID=<approved-kwin-id> make gui-eval-retained-capture` only when portal
consent and live screenshots are allowed. Restart, revocation, and close/reopen
remain separate scenarios; deterministic fake-CLI tests do not prove
compositor behavior.

For the multi-output coordinate variant, run
`REQUIRE_MULTI_OUTPUT_NONZERO_ORIGIN=1 WINDOW_ID=<approved-kwin-id> make
gui-eval-retained-capture`. The preflight records sanitized monitor geometry
and refuses portal consent unless at least two outputs and one non-zero logical
X or Y origin are present;
the `monitor_or_scale_change` scenario still requires the operator to move the
approved window and confirm a fresh target-only frame.

The separate `scripts/capture-restore-eval.py` runner records restart behavior
in two phases. `prepare` captures and closes one requested-window session, then
the operator restarts `seatgeistd`; `resume` refuses an unchanged daemon socket
and opens the same target again. Acceptance requires the chooser not to
reappear, the opaque target reference to remain stable, the private restore
state to be atomically replaced, a fresh bounded frame, and both agent-opened
sessions to close. The harness never restarts the daemon itself and never puts
the raw target, portal token, or capture session id into evidence.

Portal-driven closure is now tracked independently of explicit client close.
The daemon polls the pre-subscribed `Session.Closed` stream before retained
session operations, reaps the portal-ended slot, invalidates sticky control,
and reports the compact reason without retaining portal handles. The opt-in
`WINDOW_ID=<approved-kwin-id> make gui-eval-capture-revocation` runner records
one bounded frame, operator revocation, inactive `portal_closed` status, and
rejection of the old session id. Its deterministic tests do not prove KDE
revocation behavior.

Sticky status now also re-resolves the pinned KWin id/app/PID against the live
window list. A closed or replaced target clears the interaction binding while
leaving any still-valid capture stream explicitly manageable; raw sticky input
rechecks capture and target identity before its focus lease. The opt-in
`WINDOW_ID=<original-kwin-id> make gui-eval-target-reopen` runner requires a
distinct same-application replacement, proves status did not rebind it, and
performs no focus or raw-input action. On 2026-07-11 the live runner passed
against the disposable Firefox fixture: the PID and KWin id changed, the
portal ended the old stream, sticky authority was cleared, and the user's
Konsole remained active throughout.

Do not claim minimized, occluded, popup, or restoration behavior until the KDE
live eval records it.

### Acceptance

- `snapshot` returns only the approved window stream by default.
- Repeated snapshots do not reopen portal consent.
- `wait` observes frame revisions from the retained stream.
- Capture backend choice and limitations are visible in compact metadata.
- No full-desktop image is silently returned for a window-session request.

## 9. Slice 4: Target-Centric Background Semantic Control

### Objective

Operate accessible background windows without changing desktop focus whenever
the application/toolkit supports it.

### Resolve, authorize, execute

Current semantic tools authorize before the final AT-SPI target has been
resolved and commonly validate app policy against the active window. Replace
that flow with a prepared action:

```text
request
  -> resolve window and AT-SPI target without side effects
  -> produce ResolvedActionTarget
  -> classify safety from the resolved target and requested operation
  -> apply app/target/session policy
  -> execute the prepared action
  -> settle and return target-scoped observation
```

`ResolvedActionTarget` should include:

- interaction session id
- KWin window identity and revision
- app id and PID when available
- AT-SPI application, containing window, node id, role, actions, and sensitivity
- deterministic candidate id
- requested action and safety classification
- whether focus is required or merely preferred

### Target guard

Add a target-window guard separate from the active-window guard. It validates
the resolved destination even if another user-controlled window is active.

The active-window guard remains relevant to raw seat input and focus leases. It
must not be required for a proven background semantic operation.

Implemented in step 8: the nine high-level semantic actions accept a distinct
target-window guard. The daemon first resolves an AT-SPI candidate with
application/window/PID provenance, correlates it to the exact KWin window, and
then enforces app policy against that resolved destination before the side
effect. A stale/reopened window or identity mismatch fails closed. Target
resolution lives in a dedicated daemon module; CLI target argument handling is
also isolated from the command entry point. Target-scoped event settling and
live background-application evidence remain step 9 work.

### Backend behavior

- Prefer `DoAction`, `EditableText`, `Text`, and `Value` operations that work
  without focus.
- Record whether the operation actually changed focus, raised a window, opened a
  child window, or required fallback.
- Never silently retry a failed semantic action through raw pointer or keyboard
  input. Return a structured fallback recommendation to the coordinator.
- Add AT-SPI event subscriptions so semantic settling can use state, property,
  text, focus, and child-window changes instead of rescanning whole trees.

Implemented in step 9: event subscriptions use the public AT-SPI Registry
contract through an accessibility-event backend trait. A subscription is
created after resolved-target policy succeeds and before the semantic side
effect. Object, window, and focus signals are filtered to the correlated
application bus name and exact target/containing-window source paths. The
post-action result carries target-scoped window/node state plus settle backend
and event provenance; event failure uses a bounded target-node fallback and
never raw input. The event transport and daemon settle coordinator live in
separate modules, with mock subscription tests. Live Firefox/KDE coverage still
has to be recorded on a desktop session before claiming application-wide
background support.

### Acceptance

- A supported Firefox/KDE semantic action can complete while a different user
  window remains focused.
- App deny and secret/destructive rules are enforced against the resolved
  semantic target.
- Target close/reopen and ambiguous node changes fail safely.
- Tests prove policy runs after resolution and before side effects.
- Live evals record which common applications/actions genuinely work in the
  background.

The opt-in `scripts/background-semantic-eval.py` runner covers that live gap
one safe button at a time. It resolves the target and designated user work
window from KWin, requires any non-target window before and after one exact
target-guarded `click_button`. This permits
the physical user to switch work windows during the action while still failing
closed if the semantic target becomes active. It creates only a short-lived
method-scoped policy grant and verifies the successful journal record. It makes no focus,
raw-input, or screenshot call and keeps raw window ids, the button name, action
id, and titles out of evidence. A passing Firefox run and a separate passing KDE
run are both required; deterministic tests do not prove application behavior.
Both version-2 live runs passed on 2026-07-11 against the disposable Firefox
button and KCalc while the user worked in Firestorm. Their private evidence
shares one workspace fingerprint. The final aggregate will regenerate them
after the remaining Step 12 edits so all eight artifacts share the final tree.

## 10. Slice 5: Sticky Target and Cooperative Focus Lease

### Objective

Make repeated raw-input actions reliable when the user changes focus between
Codex actions, without requiring the model to re-check or re-focus the target.

### Sticky target semantics

- A session remains pinned to its target until close, expiry, explicit rebind,
  or target invalidation.
- User focus changes do not invalidate the session.
- The daemon re-resolves the target immediately before each action.
- The daemon automatically focuses the pinned target only when the selected
  action backend requires the desktop seat.
- The model receives a structured target-lost result instead of being asked to
  rediscover windows repeatedly.

Implemented in step 10: an opened retained-capture session can pin an exact
KWin window id, app id, and PID for 30 minutes. Raw keyboard and pointer
requests accept that opaque session id instead of an active-window guard. The
daemon-owned `interaction` module re-resolves and authorizes the target, takes
a bounded seat mutex, applies the ordinary focus policy, calls the trait-backed
KWin focus implementation only when necessary, confirms the active target, and
then executes exactly one input action. Focus policy, focus request, focus
verification, and input records share one action id in the journal. Session
status exposes only compact target identity and expiry metadata, while closed,
expired, or identity-changed targets and lease contention return structured
`target_lost` and `focus_lease_conflict` errors. This slice intentionally does
not restore prior user focus; trusted physical-activity provenance and
cooperative restoration are implemented in the next step.

Ordinary window reads and direct focus now use the shared injected
`WindowBackend`. Its production KWin adapter owns runner/bridge merging and
monitor correlation and delegates focus to the internal KWin executor. Direct
focus and sticky leases both consume this shared backend, so neither can bypass
the executor boundary while dispatcher policy and action journaling remain
unchanged. Sticky target binding, renewal and status identity checks,
pre-action re-resolution, bounded focus verification, cooperative restoration,
and post-action target validation use the same backend and record its declared
name rather than hardcoding KWin provenance.

Model-facing desktop observations and polling post-action observations now
consume the injected window and screen backends as well. Their assembly,
compact accessibility projection, and revision generation live in a dedicated
daemon observation module with mock-backed tests. Pre-execution active-window
guards and app-policy active/focus-target reads now use the injected window
backend too, isolated in a dedicated `window_safety` module. The original
fail-closed ordering remains: policy, panic stop, trusted human-input pause,
ownership, and required-guard validation precede these reads, while rate-limit
acceptance and every action remain after them. Journal before/after context is
still a best-effort synchronous bridge snapshot and is not used to authorize
control.

All nine guarded semantic actions now obtain their KWin correlation candidates
through the shared injected `WindowBackend` before PID, title, application,
target-window, and app-policy checks. Async authorization is co-located with
the pure identity resolver in the daemon target module. Unguarded semantic
actions preserve their explicit uncorrelated behavior and avoid an unnecessary
window lookup.

Raw-action execution now supports an async action-preparation stage so guarded
window-local pointer mapping happens after sticky focus verification. The
dedicated pointer-coordinate module loads monitor metadata through
`ScreenBackend` and the active window through `WindowBackend`; one action
context resolves both drag endpoints against one snapshot. Deterministic tests
assert that multiple window-local points require exactly one active-window
read, reducing round-trips and avoiding mid-drag geometry races.

### Seat lease transaction

For one raw-input action:

1. Resolve and authorize the pinned target.
2. Reject if panic-stop is active.
3. Reject or wait if recent physical user activity is detected.
4. Acquire the per-seat action mutex with a short deadline.
5. Snapshot current user focus and target revision.
6. Focus the pinned target internally if needed.
7. Confirm that the target actually became active inside the daemon.
8. Execute exactly one bounded raw-input action.
9. Settle and verify against the pinned target.
10. Restore the previous focus only when:

    - no physical user activity occurred during the lease
    - focus is still on the agent target
    - the previous window still exists
    - focus restoration is allowed by session policy

11. Release the seat mutex and return the post-action result.

If the user changes focus or provides input during the lease, user activity
wins. Seatgeist must stop injecting input and must not pull focus back.

### Real human-activity detection

Replace the current manually touched activity file with an implemented watcher.
The preferred design is a KWin/compositor integration that reports trusted
physical input activity and distinguishes it from Seatgeist's own EIS/uinput
devices. A userspace libinput watcher may be evaluated, but broad `/dev/input`
access and event confidentiality must be treated as security costs.

The watcher reports only activity metadata:

- seat id
- monotonic timestamp
- event class such as keyboard, pointer, or touch
- trusted physical versus Seatgeist-injected provenance

It must not log keys, coordinates, text, or device details unnecessary for the
pause decision.

Implemented in step 11: `kwin/seatgeist-activity` is a separately compiled
KWin binary plugin using `InputEventSpy`. It sends only seat, monotonic time,
event class, and conservative provenance to the daemon's existing local D-Bus
bridge. The daemon keeps this state in a dedicated `activity` module; exact
Seatgeist uinput/EIS sources do not count as user interference, while physical
and unknown sources do. Sticky raw actions snapshot this generation and restore
the prior window only when provenance is trusted, no interference occurred,
focus remains on the pinned target, the prior KWin/app/PID identity still
exists, both app and focus policy allow restoration, and KWin confirms the
result. Every decision shares the raw action's lease id. If the ABI-specific
plugin is absent, stale, or unconfirmed, restoration is skipped rather than
falling back to focus/file/idle heuristics. The legacy file remains a pause-only
compatibility input.

### Focus behavior and errors

- Automatic focus is an internal sub-action, not another model round trip.
- Focus approval can be scoped to the interaction session and target.
- A session-scoped grant must not authorize focusing arbitrary windows.
- Failure to focus is a single structured action result with backend evidence;
  the coordinator may retry once only for a documented transient KWin race.
- Repeated portal prompts, readiness loops, or open-ended focus retries are
  forbidden.

### Acceptance

- Codex can pin one Firefox window, the user can work in another window, and the
  next Codex raw action automatically reacquires Firefox without a model-driven
  active-window check.
- A raw action plus focus, verification, and optional restoration takes one MCP
  call.
- Semantic background actions cause zero focus changes.
- User activity during a focus lease aborts further injection and prevents
  focus tug-of-war.
- Every internal focus/action/restore step is correlated and journaled.
- Twenty repeated actions with deliberate user focus changes produce zero
  wrong-window inputs in the live cooperative-use eval.

## 11. Error and Round-Trip Budget

Add task-level metrics rather than relying only on unit and protocol coverage.

`scripts/computer-use-baseline.py` writes privacy-preserving evidence from a
bounded journal selection. Use `--client-pid` and/or inclusive
`--start-unix-ms` / `--end-unix-ms` filters to isolate one task. The report
contains method, safety-class, guard, focus-change, and structured error-kind
counts but does not copy journal summaries, window titles, or UI content.

Example:

```bash
scripts/computer-use-baseline.py \
  --journal "$XDG_STATE_HOME/seatgeist/journal.jsonl" \
  --scenario firefox-sticky-target \
  --client-pid 12345 \
  --output target/seatgeist-computer-use-baseline/firefox.json
```

Initial targets:

- bounded screenshot visible to the model: one MCP call
- repeated semantic action in an open session: one MCP call
- repeated raw action with sticky target: one MCP call
- successful action verification: included in that same call
- readiness: once per MCP/daemon generation, then cached until a relevant
  capability changes
- expected operational failure rate in controlled common-app evals: below 5%
- wrong-window raw input: zero
- repeated automatic portal consent prompts: zero
- model-driven focus polling during an open sticky session: zero

Classify failures separately:

- policy/approval required
- user consent cancelled
- target lost or ambiguous
- parallel session-owner conflict
- user activity conflict
- focus lease conflict
- semantic capability missing
- capture stream ended or format changed
- backend unavailable
- settle timeout
- implementation defect

Policy denials and deliberate user cancellation are outcomes, not backend
reliability failures. They should still terminate the current action cleanly
without causing a blind retry loop.

Implemented in step 12: the core MCP facade now rejects contradictory sticky
session plus active-window guards before contacting the daemon, and its startup
guidance accurately describes all six bounded tools. Core `snapshot` and
`wait` now require a retained window-session id and cannot fall back to
Screenshot v2 or whole-desktop polling; those compatibility paths remain
explicit expert tools. The version 2 baseline
analyzer correlates daemon-internal focus/restore records back to the one model
action id, separates deliberate policy/user-stop outcomes from reliability
failures, classifies failure categories, counts explicit focus polling and
portal opens, and evaluates the measurable budget invariants. Sticky-only
checks are reported as not applicable instead of passing when a trace has no
sticky action. Successful sticky actions must now also correlate an
`interaction_input_activity` check after focus verification and immediately
before injection; physical or unknown activity records a failed internal step
and aborts the action without calling the input backend.

The latest bounded live journal slice available before deploying this build is
preserved as generated evidence under
`target/seatgeist-computer-use-baseline/pre-sticky-latest-live-task.json`. It
contains six model requests, two failures, two preflights, three observations,
one explicit focus call, no sticky raw action, and a 20% reliability-failure
rate. This is a pre-modernization baseline, not acceptance evidence. A valid
post-change Firefox score requires installing the exact-KWin activity plugin,
restarting the normal Plasma session, and running the 20-action cooperative
eval; the analyzer explicitly marks its sticky budget as not applicable until
then.

The deployment boundary is now machine-checkable with
`make kwin-activity-preflight`. After the 2026-07-10 upgrade and reboot, the
running, installed, and plugin-factory ABIs are all KWin 6.7.2. KWin reports the
plugin available and loaded, and the daemon reports the trusted
`kwin_input_spy_v1` activity backend. The rootless installer also enables a
user-systemd ABI watcher that checks at Plasma startup and watches KWin's
installed ABI header during the session; it notifies on a mismatch without
building as root or restarting the compositor. `WINDOW_ID=<firefox-id> make
gui-eval-cooperative-sticky` remains the pending acceptance harness: it opens
one portal session, performs 20
harmless sticky raw actions while the terminal remains the user's work window,
requires target reacquisition and cooperative restoration each time, and
scores the exact journal interval. It writes short-lived grants for only
`focus_window` and `key_combo`; wildcard/class-wide approval is not used. The
helper also re-registers after daemon restarts through a D-Bus service watcher.

The first live execution reached the focus lease after those exact approvals.
KWin's WindowsRunner returned success, but the active-window bridge remained on
the user's Konsole for the entire confirmation interval, so Seatgeist emitted
`FocusLeaseConflict` and sent no keyboard input. Direct runner calls for
Firefox, KCalc, and Firestorm behaved the same. A bounded experiment using a
separate KWin focus plugin, including forced and next-event-loop activation,
also reverted to Konsole within 50 ms and was removed rather than retained as
an unproven privileged path. Do not turn this into open-ended focus retries.
Step 13 must compare separate seat/session/virtual-output designs for raw
parallel use; target-native AT-SPI actions remain the proven same-session path
when focus is unnecessary.

The current host has one physical KScreen output at logical origin `0,0`, and
the live `org.kde.KWin` object exposes no virtual-output mutation method. Do not
disturb the operator's real display merely to satisfy the negative-origin gate.
This KWin build does expose `kwin_wayland --virtual --output-count <n>`, so the
safe follow-up is an isolated nested KDE/portal fixture with at least two
virtual outputs. A live probe confirmed KWin reports them at `0,0` and
`1280,0`, and normalizes a requested `-1280,0` position back to that
non-negative coordinate space. The fixture must prove the same retained
PipeWire window contract with real multi-output/non-zero-origin metadata;
synthetic evidence or merely rewriting monitor JSON is not accepted.

The fixture launcher is split into the pure
`scripts/nested_kde_contract.py` contract/environment module,
`scripts/nested_kde_fixture.py` process supervisor, and the small
`scripts/nested-kde-fixture.py` entry point. Its default headless probe
creates a private D-Bus session and isolated HOME/XDG tree, supervises KWin,
validates live KScreen topology, records only sanitized geometry, and always
tears KWin down. Isolation is applied before D-Bus starts, keeping nested
portal/accessibility activation out of the host runtime; a live regression
probe confirmed the host AT-SPI bus and Firefox registrations survive. The
explicit visible mode is reserved for the operator-present portal test.
The first visible run exposed an oldest-frame consumer queue: unread PipeWire
frames filled a bounded FIFO and newer frames were discarded, so later samples
replayed earlier popup and geometry states. The consumer now uses a bounded
latest-value mailbox. A focused rerun live-passed minimized, popup, resize, and
cross-output states with current revisions and scale-correct dimensions;
completing one same-fingerprint eight-scenario artifact remains required.

The visible run no longer needs manual nested-session plumbing. The modular
`nested_seatgeist_probe.py` and `nested_retained_capture.py` workload start a
private daemon, verify its bridge heartbeat and two-output view, launch one
exact disposable Firefox target plus helper Konsole, and pass the target id,
short daemon socket, and multi-output requirement directly to the standard
retained-capture runner. Its headless application probe passes with sanitized
evidence and clean teardown. The process environment is allowlisted, so host
credentials are not inherited. Only the chooser and eight visual scenario
steps remain operator-present work. The disposable Firefox profile exposes a
normal KDE title bar, and the visible runner accepts a named scenario for
focused regression reruns.

All Step 12 live runners now embed the same content-addressed worktree
fingerprint. `make verify-cooperative-use-acceptance` is the offline final gate:
it requires the exact eight private artifacts, independently checks their core
scenario and budget fields, rejects stale, missing, failed, cross-revision, or
overlong evidence sets, and writes a compact path-free bundle. The verifier
does not contact the daemon or desktop. Its deterministic valid/failure and
producer-contract tests pass; the bundle cannot exist until the deliberately
paused live KDE runs are completed.

Core readiness now has a separate MCP `readiness` cache module. A successful
`computer_status` is reused only for the same daemon socket identity, for at
most 30 seconds, and only across consecutive readiness calls. Any other tool
call or daemon socket replacement invalidates it. An MCP/real-daemon integration
test proves two consecutive calls create one readiness journal request and a
subsequent tool call forces a fresh request. Control actions themselves are
never cached and continue to rerun all daemon safety gates.

Retained requested-window capture now persists the portal's rotated restore
token in a separate owner-only daemon module. The token file contains one
hashed target key, is atomically replaced with mode `0600`, and fails closed on
unsafe ownership, permissions, symlinks, malformed data, or an unsupported
format. Reopening the same requested target after a daemon restart supplies the
token with explicitly-revoked portal persistence; status and MCP output expose
only an opaque reference. This removes the implementation-level repeated
chooser requirement. On 2026-07-11 the two-phase live runner passed across a
real `seatgeistd` service restart: the daemon socket changed, the private token
state rotated, no chooser appeared, a fresh retained Firefox frame arrived,
and both sessions closed. KDE portal revocation remains pending live evidence.

## 12. Eval Scenarios for the Five Slices

### Firefox sticky-target scenario

1. User approves one Firefox window and opens a Seatgeist session.
2. Codex performs semantic navigation and form edits when available.
3. User repeatedly switches to Kate/Konsole and continues working.
4. Codex performs bounded raw input when semantic control is unavailable.
5. Verify no input reaches the user's active window and focus restoration never
   overrides user activity.

### Screenshot scenario

1. Open a retained window stream.
2. Capture and return a bounded image in one MCP response.
3. Cover and uncover the target window.
4. Exercise browser menus and dialogs.
5. Record exactly what the stream includes on the current KDE version.

### Round-trip scenario

Measure the complete tool-call sequence for:

- click a named button and verify the result
- set a named field and verify the result
- click a visual-only control and verify the result
- wait for a browser page change
- recover from a closed target window

Store calls, failures, focus changes, captures, portal prompts, settle time, and
user-activity conflicts in structured evidence.

### Safety scenario

- denied app target
- secret field
- destructive action
- panic-stop during settle
- user input before and during a seat lease
- target identity change after resolution but before execution
- two agents competing for the same seat or target

## 13. Post-Slice Research: True Multi-Use and Parallel Agents

Do not choose a kernel or KDE modification before testing the supported
userspace designs. Linux input and Wayland focus are compositor concepts as well
as kernel-device concepts: adding more virtual input devices alone does not give
each agent an independent surface focus.

### Option A: Cooperative sessions in the current Plasma desktop

Use sticky targets, background semantics, retained window capture, and short raw
seat leases.

Advantages:

- smallest change from the current system
- uses the user's existing signed-in applications and browser sessions
- should satisfy many single-agent tasks

Limits:

- one compositor seat still has one keyboard/pointer focus domain
- raw-input actions can briefly affect user focus
- multiple raw-input agents must serialize

This is the baseline delivered by the first five slices.

### Option B: Portal RemoteDesktop plus virtual ScreenCast output

Create a portal-approved virtual output and retained RemoteDesktop/EIS session,
then place agent windows on the virtual output.

Questions to answer experimentally:

- Does the current KWin/portal stack provide an EIS region mapped only to the
  virtual output?
- Is keyboard focus still shared with the physical desktop seat?
- Can windows be reliably launched or moved to the virtual output?
- Can the user inspect/take over the lane without disrupting it?
- Do restore tokens survive daemon/session restart reliably?

This may provide an excellent invisible capture surface, but it must not be
called multi-user until independent focus and input routing are proven.

### Option C: Nested compositor agent desktop

Run an isolated Wayland compositor/Plasma-compatible session inside the user's
login, with its own Wayland display, DBus session, accessibility bus, portal
services, virtual seat, and capture stream.

Advantages:

- independent focus, pointer, keyboard, clipboard policy, and window namespace
- one nested session per Codex lane is conceptually straightforward
- no kernel module required

Costs and questions:

- GPU and memory overhead
- application launch and desktop-service completeness
- browser profile and credential isolation
- audio, clipboard, file chooser, notification, and secret-service behavior
- reliable headless/nested KWin support and lifecycle supervision

This is the leading candidate for parallel agents if the virtual-output portal
experiment still shares focus.

### Option D: Separate graphical login or remote session

Use a separate systemd-logind/PAM graphical session, potentially a separate Unix
user, and access it through an RDP or ScreenCast/RemoteDesktop boundary.

Advantages:

- strongest process, session-bus, clipboard, focus, and credential separation
- mature operating-system ownership boundary

Costs and questions:

- whether current KDE remote desktop serves the existing session or can create
  an independent headless session
- GPU/session resource ownership
- sharing repositories, browser credentials, files, and user services safely
- session startup, lock, suspend, logout, and crash recovery

Do not implement a new remote desktop server as part of Seatgeist merely to test
this option. Reuse an existing KDE/RDP or portal boundary where possible.

### Option E: Native KWin agent seats or per-window input routing

The first experimental vertical slice is implemented in
`kwin/seatgeist-agent-seat`. It creates a second `wl_seat` in KWin and routes
daemon-approved keyboard and window-local pointer actions to the native
Wayland surface pinned by a retained capture session. The daemon owns the pull
queue, policy, app/session identity checks, and journal correlation; the plugin
never activates or raises the target. See `docs/agent-seat.md`.

Potential capability:

- independent agent focus without changing the user's seat focus
- compositor-authoritative window capture, including occlusion behavior
- trusted physical-versus-agent input provenance
- one seat or focus domain per parallel Codex lane

Risks:

- deep compositor security boundary
- private API/version maintenance if not upstreamed
- applications and toolkits may assume the primary seat
- clipboard, drag-and-drop, popups, activation, shortcuts, and accessibility
  semantics need explicit design

The initial slice deliberately excludes XWayland, input methods, clipboard,
drag-and-drop, and popup/grab routing. Live Firefox/Chromium/KDE compatibility
and dynamic second-seat binding remain acceptance gates; the nested compositor
lane remains the supported fallback.

### Option F: Kernel/input changes

A kernel module could provide virtual devices or seat tagging, but it cannot by
itself decide which Wayland surface receives keyboard focus. KWin or another
compositor must still route the seat and focus.

Kernel work is therefore last, and only justified by a specific missing kernel
primitive discovered during a compositor-level prototype. Prefer standard
uinput, libei, logind seat assignment, and compositor changes first.

### Multi-use decision gate

Select the long-term architecture using measured evidence for:

- independent focus and pointer domains
- capture correctness for hidden and popup surfaces
- user interruption/takeover behavior
- multiple simultaneous agent lanes
- application compatibility, especially Firefox/Chromium and KDE applications
- GPU/CPU/memory overhead
- crash containment and cleanup
- clipboard, secret, notification, and file-access isolation
- policy and journal attribution per agent/session/seat
- install, upgrade, rollback, and upstream maintenance burden

The nested compositor/session and native KWin agent-seat slices can now be
measured side by side. The native lane is the lower-overhead integration
candidate; the nested lane remains the stronger compatibility and isolation
fallback. Kernel changes remain the final fallback.

### Measured matrix (2026-07-27)

| Design | Focus/input isolation | Capture | Current evidence | Decision |
| --- | --- | --- | --- | --- |
| A. Current Plasma seat | Shared | Retained window capture works | Background semantics pass; same-seat raw focus does not remain on the requested window | Keep for semantic-only access and serialized fallback |
| B. Portal virtual output | Unknown | Protocol support exists | No independent keyboard-focus proof yet | Defer until the nested lane proves the reusable portal/EIS path |
| C. Nested KWin session | Separate namespaces and live input routing proven | Retained ScreenCast works | Private D-Bus and Wayland namespaces, two outputs, ScreenCast v4, RemoteDesktop v2, keyboard/pointer/touch device mask `7`, `ConnectToEIS`, and one nested-only F12 with unchanged host focus are live-proven | Selected for the first persistent multi-use lane |
| D. Separate login/RDP | Strong by design | Expected through RDP | Not locally measured | Reserve for stronger credential/process isolation |
| E. Native KWin agent seats | Separate `wl_seat`; no activation/stacking calls | Existing exact KWin window capture is compositor-authoritative | Experimental native-Wayland keyboard/pointer slice builds with policy-owned pull queue and journal tests; live client compatibility is not yet proven | Run operator-approved Firefox and KDE acceptance tests; retain C as fallback |
| F. Kernel/input changes | Insufficient alone | Not applicable | Standard EIS/uinput already exists | Do not pursue without a specific missing kernel primitive |

The capability probe is `make probe-nested-remote-desktop`. It runs headless,
opens no consent dialog, creates no RemoteDesktop session, sends no input, and
writes private structured evidence inside the fixture state directory. Its next
gate was an operator-approved visible test that created one retained
RemoteDesktop/EIS session, sent F12 to a known nested Konsole, and proved the
host active window did not change. That gate passed. The next slice is a
persistent agent-lane lifecycle with explicit start, inspect, attach, and stop
operations plus per-lane policy/journal attribution.

## 14. Implementation Order

Land small vertical slices that compile and remain independently useful:

1. Add task-level baseline measurement and structured eval evidence.
2. Land MCP image content for existing bounded screenshots.
3. Populate post-action observations and add settle metadata.
4. Add the core facade without removing expert tools.
5. Introduce lifecycle capture traits and mock sessions.
6. Implement portal ScreenCast lifecycle without frame consumption.
7. Implement bounded PipeWire frame capture and retained `snapshot`/`wait`.
8. Add resolved target policy and target-window guards.
9. Add AT-SPI event-driven background semantic actions.
10. Add sticky interaction sessions and daemon-owned focus reacquisition.
11. Implement real human-activity provenance and cooperative focus restoration.
12. Run the complete cooperative-use eval, aggregate the same-worktree evidence
    with `make verify-cooperative-use-acceptance`, and meet the error/round-trip
    budget.
13. Begin the virtual-output, nested-session, remote-session, and KWin-agent-seat
    research matrix.

Every protocol-changing slice must update protocol tests, MCP integration tests,
replay traces, tool documentation, plugin skills, and journal assertions.

## 15. Verification Gates

Before finishing each implementation slice:

```bash
cargo fmt --all
cargo test --workspace
cargo check --workspace
```

For daemon, CLI, MCP, journal, or protocol behavior changes:

```bash
make smoke
```

Additional gates by phase:

- MCP image response tests with bounded fixture images.
- Mock capture-session lifecycle and session-closure tests.
- Portal contract tests that do not open live consent UI.
- Policy tests proving resolved-target authorization precedes side effects.
- Journal tests for correlated internal focus/action/restore records.
- Opt-in live KDE evals for ScreenCast/PipeWire and cooperative focus.
- Repeated task-level evidence meeting the error and round-trip budget.

## 16. Source Anchors

- XDG ScreenCast portal:
  https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
- XDG Screenshot backend interface and version 3 targets:
  https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Screenshot.html
- XDG RemoteDesktop and its ScreenCast integration:
  https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html
- Chromium/WebRTC Wayland PipeWire capture implementation:
  https://chromium.googlesource.com/external/webrtc/+/master/modules/desktop_capture/linux/wayland/
- Wayland seat, keyboard focus, and pointer focus model:
  https://wayland.freedesktop.org/docs/book/Protocol.html
- systemd-logind session and multi-seat model:
  https://www.freedesktop.org/software/systemd/man/org.freedesktop.login1.html
- KDE portal window/region ScreenCast behavior and known popup limitations:
  https://invent.kde.org/plasma/xdg-desktop-portal-kde/-/merge_requests/161
- KDE KRdp project:
  https://invent.kde.org/plasma/krdp
