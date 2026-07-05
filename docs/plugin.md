# Seatgeist Codex Plugin

The repository ships a local Codex plugin bundle under `plugin/`.

## Contents

- `plugin/.codex-plugin/plugin.json`: plugin metadata, skill path, and MCP config path.
- `plugin/.mcp.json`: stdio MCP server entry for `seatgeist-mcp --stdio`.
- `plugin/skills/`: Seatgeist operating workflows for computer use, GUI testing, browser debugging, and desktop triage.
- `plugin/hooks/hooks.json`: Codex `Stop` hook config for writing a compact local audit summary.
- `plugin/hooks/seatgeist_audit_summary.py`: fail-open audit hook that writes `target/seatgeist-hook-audit/latest.json` with git status, recent Seatgeist journal metadata, failure counts, unguarded-control counts, method/safety-class/client tool counts with process/pid fallback, compact active-window context, and opt-in artifact metadata when present.

## Preconditions

Build and install or otherwise expose these binaries on `PATH` for the Codex process:

```bash
cargo build --workspace
```

Start `seatgeistd` through the user service or a private socket before using MCP tools. The MCP server uses the default daemon socket when `SEATGEIST_SOCKET` is unset.

For development, a direct plugin source install should point at the repository's `plugin/` directory. The plugin expects `seatgeist-mcp` to be on `PATH` for the Codex process.

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

Codex loads plugin-bundled hooks through the normal hook trust flow. Review and trust the Seatgeist hook with `/hooks` before expecting it to run. The hook does not consume prompt text or hook stdin; it writes only repo status, HEAD, and compact Seatgeist journal metadata plus aggregate audit counts under `target/seatgeist-hook-audit/latest.json`. Client counts prefer explicit journal `client.tool` values such as `seatgeist-mcp` and fall back to peer process name or pid for older entries. If the daemon is configured to journal artifact metadata, the hook preserves the compact artifact kind, path, byte count, and SHA-256 fields from recent examples.

## Local Use Examples

Open Kate or KWrite and type into a disposable file:

```text
Use Seatgeist to open a disposable KWrite document, type hello, save it, and report the journal summary.
```

Verify a localhost web UI in Firefox:

```text
Use Seatgeist only if Playwright cannot see the current logged-in Firefox session. Observe the Firefox window, focus it with guards, navigate to localhost, and confirm the visible result.
```

Reproduce a GUI bug and turn it into a test:

```text
Use Seatgeist GUI testing to reproduce this KDE app bug, collect bounded screenshot evidence, then add a deterministic regression test.
```
