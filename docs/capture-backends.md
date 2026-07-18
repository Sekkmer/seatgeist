# Capture Backend Architecture

Seatgeist's capture boundary is lifecycle-oriented. `ScreenBackend` reports
capabilities and opens a `CaptureSession`; a retained session snapshots the
latest complete frame, waits for a revision after a known frame, exposes
sanitized metadata, and closes idempotently.

The backend contract distinguishes four source types:

- `window`: either an exact policy-checked KWin window screenshot session when a
  UUID is supplied, or a user-approved portal stream when it is omitted.
- `monitor`: a user-approved monitor stream.
- `virtual_output`: an isolated compositor output when supported.
- `desktop_compatibility`: an explicitly labeled full-desktop or visible crop
  fallback. It must report `occlusion_possible=true` when window pixels are
  derived by cropping a composed desktop.

Session metadata exposes only an opaque restore-token reference. Raw portal
restore tokens, PipeWire file descriptors, node credentials, and session
handles remain backend-private. Frame results carry a monotonic session
sequence, opaque revision, completeness state, damage availability, bounded
output metadata, and backend provenance.

The deterministic test backend implements this contract now. It records open,
snapshot, wait, and close operations, advances through a fixed frame sequence,
reports no-change as an explicit timed-out watchdog result, rejects operations
after close, and never opens a portal or sends desktop input.

`seatgeist-portal` now implements the ScreenCast lifecycle independently of
frame decoding: `CreateSession`, a single `SelectSources`, `Start`,
`OpenPipeWireRemote`, explicit `Session.Close`, and early subscription to
`Session.Closed`. The default constructor requests exactly one window source.
Source type, cursor mode, persistence, single-use restore-token rotation,
stream-local id, mapping id, compositor position/size, legacy PipeWire node id,
and the optional version-6 PipeWire serial are modeled explicitly. Cancellation
or failure after session creation closes the session best-effort, and an invalid
PipeWire descriptor is never returned as a usable connection.

The production daemon owns one retained window capture session at a time. Its
dedicated `capture_backend` adapter implements `ScreenBackend`, owns private
restore-token state, and is injected into the daemon runtime as a trait object;
the `capture` session store no longer calls portal or PipeWire constructors.
The adapter implements retained `window`, `monitor`, and `virtual_output`
source contracts. Exact windows route to KWin ScreenShot2; generic windows,
monitors, and virtual outputs route to the portal. Core remains exact-window
only; expert protocol/MCP and
`seatgeist-cli capture open --source ...` expose monitor and virtual-output
requests without persistence or sticky control. Portal selection and
compositor support remain authoritative, unsupported live requests fail
explicitly, and no source silently falls back to desktop capture. The store keeps sanitized
session metadata, enforces the configured bounded-preview limit, and routes
snapshot, revision wait, status, and id-checked close requests. A failed close
using the wrong session id leaves the active session intact.

The retained PipeWire session now polls the pre-subscribed portal
`Session.Closed` stream as part of lifecycle checks. A portal-ended or failed
closure monitor is reaped before status, snapshot, wait, reopen, or sticky raw
input can proceed; status becomes inactive, exposes only a compact
`last_end_reason`, and the old slot is freed. Portal revocation also invalidates
the sticky interaction binding, and sticky raw input rechecks the capture
lifecycle before and after acquiring its seat lease. Explicit client close
remains id-checked and reports `client_closed`.

Each opening session is also bound to a trusted daemon-client owner before the
backend can open the portal. MCP ownership is scoped to the exact Unix peer
process; verified `seatgeist-cli` ownership is scoped to the CLI tool so later
invocations can complete a manual lifecycle. Renew, snapshot, wait, close,
sticky raw actions, and post-action images all pass the same centralized owner
gate before capture, focus, or input side effects. Wrong ids and foreign
clients receive the same `session_owner_mismatch` result so the check is not an
active-session oracle. Status is deliberately read-only and reports only
compact owner tool, PID, and scope metadata.

The daemon keeps execution metadata beside, rather than inside, the backend
session object. Opening registers the sanitized capture backend and target
policy result. Successful raw, semantic, or linked focus actions then record
the resolved action backend, method/id/safety class, policy result, cooperative
focus policy, trusted activity provenance, focus-lease outcome, and final
settle result. Status merges this record into the capture response. Observation
lifecycle calls do not overwrite the last action, and closing or reaping the
capture clears the execution record. The record contains no window title,
typed value, accessibility value, portal credential, or image content.

`seatgeist-pipewire` now supplies the native frame side of the retained
session. A dedicated PipeWire thread owns the portal remote FD, context, core,
stream, main loop, callbacks, and shutdown channel. It negotiates only bounded
raw RGB/BGR formats and mapped shared-memory buffers; the realtime callback
copies validated buffer bytes into a single latest-value mailbox and never
performs PNG encoding. A newer frame atomically replaces an unread frame, so
interactive snapshots cannot accumulate latency or replay an old FIFO backlog.
DMA-BUF-only frames fail explicitly until a dedicated importer is
implemented. Outside the callback, the capture session handles row padding,
positive or negative stride, channel conversion, source bounds, downscaling,
private PNG output, content revisions, retained snapshot/wait state, timeout
watchdogs, and idempotent shutdown coupled to portal `Session.Close`.

The daemon protocol, CLI, and MCP server expose this lifecycle. Core MCP uses
`window_session` for open/status/renew/close and requires `session_id` on `snapshot`
and `wait`; it cannot silently select the older compatibility desktop
Screenshot path. Expert tools expose the individual lifecycle and explicit
compatibility calls. Bounded retained frames use the same native MCP image
attachment limits and opt-in journal artifact metadata as compatibility
screenshots.

Renewal extends only a still-live pinned interaction target. Before changing
its expiry, the daemon checks that the capture session id is active, resolves
the original KWin window id/app/PID again, and reapplies app policy. It opens no
portal dialog, sends no input, and returns `target_lost` rather than binding a
replacement window.

Portal window sessions may ask for explicitly-revoked source persistence. The
daemon keeps only the latest rotated restore token in a
private, owner-only, atomically replaced state file keyed by a hash of the KWin
target id. On the next open after a daemon restart it supplies that token to
`SelectSources`; raw tokens never enter protocol responses, journals, logs, or
MCP content. Session status exposes only a short opaque
`restore_token_reference`. Exact KWin sessions never create or consume portal
restore tokens.

An exact `requested_window_id` is resolved and policy-checked before capture,
then routed to KWin `org.kde.KWin.ScreenShot2.CaptureWindow`. KWin resolves the
UUID again and writes compositor-rendered window pixels through a bounded file
descriptor. This path opens no portal chooser, creates no ScreenCast/PipeWire
stream, reports `backend=kwin_screenshot2_window`, and is not occlusion-prone.
Repeated `snapshot` and `wait` calls recapture only the pinned window.

KWin restricts this interface by executable desktop metadata. The local install
creates `org.seatgeist.daemon.desktop` with
`X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`; policy checks,
trusted session ownership, target revalidation, output bounds, and journaling
remain in Seatgeist. Missing authorization or KWin support fails closed and does
not silently open a chooser. A caller can still explicitly request portal
ScreenCast by omitting the exact window id, or request monitor/virtual-output
capture. Those portal sources remain chooser-authoritative.

Backend fallback rules:

1. Prefer KWin ScreenShot2 for an exact policy-checked window UUID.
2. Use portal ScreenCast for a window request without an exact UUID.
3. Permit monitor or virtual-output streams only when the requested session
   mode and policy allow that source.
4. Use Screenshot v3 one-shot targeting only when the portal advertises the
   requested target.
5. Use desktop capture/crop only as `desktop_compatibility`, with provenance
   and occlusion risk visible to callers.

The two compatibility implementations now use the lifecycle trait as one-shot
sessions. An explicit Screenshot v3 `portal_target` is portal-authoritative;
failure or cancellation returns an error and cannot fall through to Spectacle.
The expert-only `visible_window_crop_id` path is separately explicit. It
resolves the exact KWin id, captures the composed desktop through the ordinary
compatibility source, crops only current visible pixels, bounds the output, and
reports `backend=visible_window_crop`, window-local coordinates, and
`occlusion_possible=true`. It currently rejects blank/reopened targets,
spanning or off-screen windows, transformed monitors, conflicting portal
target/interactive options, and any mismatch between KWin physical bounds and
the composed image. It is never a hidden-window substitute.
## Opt-in retained-window evaluation

`WINDOW_ID=<approved-kwin-id> make gui-eval-retained-capture` opens one portal
chooser and guides the operator through focused, unfocused, partially and fully
occluded, minimized, popup/dialog, moved/resized, and monitor/scale states. The
runner requires a fresh retained-stream revision per state and writes bounded
mode-0600 PNG evidence under `target/seatgeist-retained-capture-eval/`. Its JSON
evidence hashes the requested KWin id and omits the raw capture session id.
Set `REQUIRE_MULTI_OUTPUT_NONZERO_ORIGIN=1` on a real or isolated multi-output
layout. The runner then fails before opening the portal unless KWin reports at
least two outputs and at least one non-zero logical origin; evidence keeps only
geometry and scale, not monitor ids or names. This matches KDE Wayland's
normalized non-negative global layout. Negative-origin coordinate math remains
covered by deterministic backend tests rather than fabricated live metadata.

`make probe-nested-kde-multi-output` launches a headless KWin under a private
D-Bus session and isolated HOME/XDG directories, requires two real virtual
outputs plus a non-zero logical origin from `kscreen-doctor`, writes a private
probe artifact, applies the isolated runtime before the private D-Bus starts so
every auto-activated portal or accessibility service stays out of the host
AT-SPI runtime, requires KDE's ScreenCast backend to advertise window capture,
installs and requires the repository's KWin bridge only inside the private
fixture tree, rebuilds a private KService cache, and uses a fixture-only
authorized `wayland-info` desktop entry to require
`zkde_screencast_unstable_v1` before any payload or visible chooser can start.
It preserves KWin's restricted-interface permission checks rather than
disabling them. Portal, protocol, and bridge activation are retried only within
the bounded fixture startup period so an early KWin/D-Bus race cannot leak into
the visible test. The modular launcher also has a
deliberate `--visible` mode for the later portal-consent run; visible mode nests
KWin in the host Wayland session and must only be started with the operator
present because it opens one host window per virtual output. The probe is a
fixture prerequisite, not a substitute for the retained PipeWire artifact.

`make probe-nested-seatgeist` adds an observation-only nested daemon probe. It
starts `seatgeistd` inside the same private D-Bus/Wayland/XDG boundary, waits for
the isolated bridge heartbeat, and requires the daemon's own monitor endpoint
to preserve the two-output/non-zero-origin topology. Evidence omits monitor
names, window ids, titles, socket paths, and journal contents. This proves the
visible runner can use the normal daemon/CLI path rather than a synthetic
KScreen document. The nested process environment is allowlisted and does not
inherit host token, password, cookie, credential, or arbitrary application
variables.

`make probe-nested-retained-apps` extends that headless proof through a private
Firefox profile and helper Konsole window, requiring the nested daemon to find
one exact Firefox target and at least one non-target work window. With the
operator present, `I_AM_PRESENT=1 make gui-eval-nested-retained-capture` runs
the same supervised workload in two visible nested KWin outputs and hands its
exact window id and daemon socket directly to the standard retained-capture runner.
The operator selects Firefox once and follows the existing eight scenario
prompts; daemon, apps, compositor, portal bus, and all streams are torn down on
success, failure, EOF, or interruption.

Set `SCENARIO=<name>` on the visible make target to rerun one named scenario
without repeating the full matrix. The disposable Firefox profile disables
client-side tab-strip title bars so the nested KDE decoration exposes normal
window controls, including minimize.

The first visible matrix exposed a consumer-side freshness defect: the old
bounded channel retained its oldest unread frames and dropped newer frames.
That made later popup, resize, and monitor samples replay earlier UI even
though KWin's stream sequence kept advancing. The latest-value mailbox fixes
that behavior with bounded memory. A focused live rerun then passed minimized,
Firefox popup, resized, and cross-output states with current page revisions and
scale-correct dimensions. The complete eight-scenario artifact remains the
acceptance gate; focused partial evidence is diagnostic only.

For that visible nested run, make every state change inside the two KWin output
windows, not by covering the outer output windows on the host:

1. `focused_visible`: activate nested Firefox and leave it unobstructed.
2. `unfocused_visible`: activate nested Konsole while Firefox remains visible.
3. `partially_occluded`: move/resize Konsole over only part of Firefox.
4. `fully_occluded`: cover Firefox completely with Konsole.
5. `minimized`: minimize Firefox from its nested title bar.
6. `popup_or_dialog`: restore Firefox and open its application menu or another
   browser-owned popup.
7. `moved_resized`: close the popup, then move and resize Firefox within its
   current nested output.
8. `monitor_or_scale_change`: move Firefox across to the other nested output.

The fixture page changes continuously, so each sample can prove a fresh
revision. After every sample the operator or reviewing agent must inspect the
bounded PNG and accept it only when it contains the approved Firefox source and
the requested state. Host focus and outer-window occlusion are not evidence for
these inner-compositor scenarios.

This target is deliberately excluded from `make verify`: it opens consent UI
and captures real window pixels. The deterministic harness test is included in
`make validate-computer-use-baseline`, but it does not establish live KDE
behavior.

Portal revocation has its own opt-in target:

```bash
WINDOW_ID=<approved-kwin-id> make gui-eval-capture-revocation
```

It captures one initial bounded frame, asks the operator to stop/revoke sharing
through KDE's portal UI, then requires inactive status with
`last_end_reason=portal_closed` and proves the old session id cannot capture
again. Post-revocation reuse may fail as either `session_ended` or the stronger
non-oracular `session_owner_mismatch`, because reaping clears the former owner
before the centralized owner gate runs. If revocation does not occur, the
runner explicitly closes only the session it opened. No focus or raw-input call
is made.

Close/reopen identity has another no-input live target:

```bash
WINDOW_ID=<original-kwin-id> make gui-eval-target-reopen
```

After the approved original is closed, the runner requires a distinct KWin id
for the same application and then asks capture status to revalidate the sticky
binding. Passing evidence requires that Seatgeist neither retain the old target
nor silently bind the replacement. The portal may end the stream itself;
otherwise the runner explicitly closes the still-active capture. It never uses
the stale session for focus or raw input.

Restart reuse has a separate two-phase opt-in target:

```bash
WINDOW_ID=<approved-kwin-id> make gui-eval-capture-restore-prepare
# Record the printed artifact directory, then restart seatgeistd.
WINDOW_ID=<same-kwin-id> EVIDENCE_DIR=<prepare-artifact-dir> \
  make gui-eval-capture-restore-resume
```

If `[daemon].capture_restore_file` is overridden, pass the identical
`RESTORE_FILE=/absolute/path` to both Make commands. Resume fails before portal
side effects if the target hash, daemon socket replacement, private file
identity, or inactive-session preflight does not match the prepared phase. The
JSON records only the opaque reference and file/socket metadata; raw target
ids, restore tokens, and capture session ids are omitted.
