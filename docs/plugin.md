# PlasmaPilot Codex Plugin

The repository ships a local Codex plugin bundle under `plugin/`.

## Contents

- `plugin/.codex-plugin/plugin.json`: plugin metadata, skill path, and MCP config path.
- `plugin/.mcp.json`: stdio MCP server entry for `plasma-pilot-mcp --stdio`.
- `plugin/skills/`: PlasmaPilot operating workflows for computer use, GUI testing, browser debugging, and desktop triage.
- `plugin/hooks/hooks.json`: intentionally disabled hook skeleton until hook schema and trust behavior are validated locally.

## Preconditions

Build and install or otherwise expose these binaries on `PATH` for the Codex process:

```bash
cargo build --workspace
```

Start `plasma-pilotd` through the user service or a private socket before using MCP tools. The MCP server uses the default daemon socket when `PLASMA_PILOT_SOCKET` is unset.

For development, a direct plugin source install should point at the repository's `plugin/` directory. The plugin expects `plasma-pilot-mcp` to be on `PATH` for the Codex process.

## Validation

Run the repo-local plugin validator:

```bash
make validate-plugin
```

The normal project gate also runs this validator:

```bash
make verify
```

The validator checks manifest metadata, relative plugin paths, MCP server config, skill frontmatter, the required four skill names, and the disabled hook skeleton.

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
