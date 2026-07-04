# PlasmaPilot Codex Plugin

The repository ships a local Codex plugin bundle under `plugin/`.

## Contents

- `plugin/.codex-plugin/plugin.json`: plugin metadata, skill path, and MCP config path.
- `plugin/.mcp.json`: stdio MCP server entry for `plasma-pilot-mcp --stdio`.
- `plugin/skills/`: PlasmaPilot operating workflows for computer use, GUI testing, browser debugging, and desktop triage.
- `plugin/hooks/hooks.json`: Codex `Stop` hook config for writing a compact local audit summary.
- `plugin/hooks/plasma_audit_summary.py`: fail-open audit hook that writes `target/plasma-pilot-hook-audit/latest.json` with git status, recent PlasmaPilot journal metadata, failure counts, unguarded-control counts, method/safety-class counts, and compact active-window context.

## Preconditions

Build and install or otherwise expose these binaries on `PATH` for the Codex process:

```bash
cargo build --workspace
```

Start `plasma-pilotd` through the user service or a private socket before using MCP tools. The MCP server uses the default daemon socket when `PLASMA_PILOT_SOCKET` is unset.

For development, a direct plugin source install should point at the repository's `plugin/` directory. The plugin expects `plasma-pilot-mcp` to be on `PATH` for the Codex process.

For the full Arch Linux + KDE Plasma setup path, including packages, user service, KWin bridge, safe diagnostics, optional uinput, and plugin validation, see `docs/arch-kde-install.md`.

## Validation

Run the repo-local plugin validator:

```bash
make validate-plugin
```

The normal project gate also runs this validator:

```bash
make verify
```

The validator checks manifest metadata, relative plugin paths, MCP server config, skill frontmatter, the required four skill names, the bundled Stop audit hook, and the hook's compact audit aggregation behavior.

## Hook Trust

Codex loads plugin-bundled hooks through the normal hook trust flow. Review and trust the PlasmaPilot hook with `/hooks` before expecting it to run. The hook does not consume prompt text or hook stdin; it writes only repo status, HEAD, and compact PlasmaPilot journal metadata plus aggregate audit counts under `target/plasma-pilot-hook-audit/latest.json`.

## Local Use Examples

Open Kate or KWrite and type into a disposable file:

```text
Use PlasmaPilot to open a disposable KWrite document, type hello, save it, and report the journal summary.
```

Verify a localhost web UI in Firefox:

```text
Use PlasmaPilot only if Playwright cannot see the current logged-in Firefox session. Observe the Firefox window, focus it with guards, navigate to localhost, and confirm the visible result.
```

Reproduce a GUI bug and turn it into a test:

```text
Use PlasmaPilot GUI testing to reproduce this KDE app bug, collect bounded screenshot evidence, then add a deterministic regression test.
```
