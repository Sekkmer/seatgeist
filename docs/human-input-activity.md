# Human-Input Activity Provenance

Cooperative focus restoration uses a KWin-side input observer rather than
inferring user activity from focus changes or the legacy activity file. The
binary plugin under `kwin/seatgeist-activity` implements KWin's
`InputEventSpy`, reduces each event to a class and provenance, and sends only
this compact payload to the existing local daemon D-Bus object:

- backend contract: `kwin_input_spy_v2`
- logical seat: `default`
- monotonic plugin timestamp
- class: `keyboard`, `pointer`, or `touch`
- provenance: `trusted_physical`, `seatgeist_injected`, or `unknown`
- target: KWin window UUID, or `desktop` when the event has no window

It never sends keys, text, modifiers, pointer coordinates, window titles,
application ids, device names, device paths, vendor/product identifiers, or
touch positions. The daemon also rejects payloads containing fields outside
this contract, and accepts registration and updates only from the D-Bus
connection that owns `org.kde.KWin`.

Target metadata lets the independent agent-seat path pause only when physical
input reaches the same window. A matching event cancels queued delivery and
starts a 350 ms target-local quiet period, and invalidates retained preview
metadata for that window. Events in other windows do not
interrupt the background lane. A matching physical event racing with delivery
produces `confirmation=user_preempted` instead of a successful-click
assumption.

Physical libinput devices are recognized by a non-virtual sysfs path.
Seatgeist's uinput and EIS devices are recognized by their explicit
`Seatgeist` device/client names. Empty paths, other virtual devices, and touch
events for which KWin's spy event does not expose a device are `unknown`.
Both trusted physical and unknown activity are treated as user interference;
only an exactly recognized Seatgeist source is excluded.

## Build and deployment

The plugin uses KWin's binary plugin API, which explicitly requires rebuilding
for each KWin release. Build it against the currently installed headers and
library with:

```bash
make check-kwin-activity-plugin
```

The CMake build derives a small generated ABI header from the installed
`config-kwin.h`. This makes Qt AUTOMOC regenerate the plugin metadata after a
KWin package upgrade even when the package preserves an older header timestamp.

The build artifact is
`target/kwin-seatgeist-activity/seatgeistactivity.so`. A distribution package
or administrator can install it into the KWin Qt 6 plugin directory with the
equivalent of:

```bash
cmake --install target/kwin-seatgeist-activity --prefix /usr
```

For a rootless workstation install, use:

```bash
make install-kwin-activity-user
```

This copies only the plugin into
`~/.local/lib/qt6/plugins/kwin/plugins` and creates a KWin-service-only systemd
drop-in that adds that root to `QT_PLUGIN_PATH`. It deliberately does not
export a global Qt plugin path, because Qt warns that a system-wide
`QT_PLUGIN_PATH` can interfere with other Qt installations. Remove both files
with `make uninstall-kwin-activity-user`.

The rootless installer also enables a user-systemd ABI watcher. It checks once
when Plasma starts and watches `/usr/include/kwin/config-kwin.h` while the
session is running. If the exact ABI embedded in the installed plugin no longer
matches KWin, it sends one desktop notification per boot with the rebuild
command. The check never rebuilds code as root, restarts KWin, or blocks Plasma
startup. Inspect it without notifying with:

```bash
~/.local/libexec/seatgeist/kwin-activity-abi-watch --check-only
```

Wrapping this source build in an AUR package alone would not provide that
guarantee: pacman can upgrade an official KWin dependency without rebuilding an
already-installed foreign package. The ABI watcher covers both repository and
AUR update paths at the compatibility boundary itself. A future package can
install the same watcher while leaving the rebuild an explicit user action.

Inspect the built, installed, and currently running compositor ABIs before
loading it:

```bash
make kwin-activity-preflight
```

The preflight prefers mapped-library evidence and falls back to KWin's D-Bus
support-information version when Linux protects `/proc/<pid>/maps`, as happens
when the compositor binary carries `CAP_SYS_NICE`.

KWin discovers binary plugins from its Qt library paths and exposes dynamic
`LoadPlugin`/`UnloadPlugin` methods on `/Plugins`. Dynamic loading is safe only
when the running compositor and embedded plugin-factory ABI match. If KWin is
still using deleted libraries from an earlier package version, restart through
a normal Plasma session restart first. The repository does not install,
restart, or dynamically mutate KWin during ordinary verification. This follows
KWin's exact-version binary plugin contract and
[`KPluginMetaData::findPlugins`](https://api.kde.org/kpluginmetadata.html),
which searches relative plugin namespaces under Qt's active library paths.

The helper watches for the daemon's D-Bus service and re-registers when the
daemon starts or restarts, so KWin and Seatgeist service ordering does not
silently disable provenance.

The daemon accepts legacy `kwin_input_spy_v1` payloads after a daemon-only
restart. They remain trusted for global pause and restoration decisions, but
cannot provide target-local preemption; agent action summaries report that
capability as unavailable until v2 loads. There is no silent JavaScript, X11,
idle-time, or file-based provenance fallback. `seatgeist.safety_status` reports
`activity_trusted=false` when the
binary plugin is absent or has not registered. Sticky input remains fail-safe
and leaves the target focused in that state; it does not restore prior focus.
The legacy activity file remains supported only as a conservative control
pause signal and can never authorize restoration.

## Cooperative restoration

For a sticky raw action, the daemon snapshots the active window and the
activity generation after the normal policy/panic/human-pause gates. After one
input action it restores the prior window only when all of these remain true:

- the KWin activity backend is registered and unchanged
- no physical or unknown activity occurred during the lease
- focus is still on the pinned target
- the prior window still has the same KWin id, app id, and PID
- app policy and ordinary focus policy allow the restoration
- KWin confirms the prior window as active after the focus request

The activity check is repeated immediately before restoration. Any failure
skips restoration without retrying or failing the input action. Activity,
policy, focus, and verification decisions are journaled under the same action
id as the raw action, without titles or input details.

After installation and any required session restart, run the opt-in acceptance
scenario with the exact Firefox id from `seatgeist-cli windows`:

```bash
WINDOW_ID=<firefox-kwin-id> make gui-eval-cooperative-sticky
```

The harness performs 20 harmless `Shift` actions. Between actions it lets the
operator keep the terminal active and generate ordinary physical activity. It
requires one sticky raw call to reacquire Firefox and restore the terminal on
every iteration, then writes a bounded version 2 budget report under `target/`.
That report is one of the eight same-worktree artifacts consumed by the
read-only `make verify-cooperative-use-acceptance` Step 12 gate documented in
`docs/computer-use-modernization-acceptance.md`.
