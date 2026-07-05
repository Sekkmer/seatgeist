#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

smoke_root="target/codex-plugin-smoke"
codex_home="$repo_root/$smoke_root/home"
rm -rf "$smoke_root"
mkdir -p "$codex_home"

export CODEX_HOME="$codex_home"

cargo build -p seatgeist-mcp >/dev/null
export PATH="$repo_root/target/debug:$PATH"

codex plugin marketplace add . --json >"$smoke_root/marketplace-add.json"
codex plugin marketplace list >"$smoke_root/marketplace-list.txt"
codex plugin list --marketplace seatgeist-local --available --json >"$smoke_root/plugin-available.json"
codex plugin add seatgeist@seatgeist-local --json >"$smoke_root/plugin-add.json"
codex plugin list --marketplace seatgeist-local --json >"$smoke_root/plugin-installed.json"

python3 - "$smoke_root" <<'PY'
import json
import sys
from pathlib import Path

smoke_root = Path(sys.argv[1])

marketplace = json.loads((smoke_root / "marketplace-add.json").read_text(encoding="utf-8"))
if marketplace.get("marketplaceName") != "seatgeist-local":
    raise SystemExit("marketplace add did not register seatgeist-local")

available = json.loads((smoke_root / "plugin-available.json").read_text(encoding="utf-8"))
available_ids = {item.get("pluginId") for item in available.get("available", [])}
if "seatgeist@seatgeist-local" not in available_ids:
    raise SystemExit("seatgeist@seatgeist-local was not available before install")

installed = json.loads((smoke_root / "plugin-installed.json").read_text(encoding="utf-8"))
matches = [item for item in installed.get("installed", []) if item.get("pluginId") == "seatgeist@seatgeist-local"]
if len(matches) != 1:
    raise SystemExit("seatgeist@seatgeist-local was not installed exactly once")
if not matches[0].get("enabled"):
    raise SystemExit("seatgeist@seatgeist-local was installed but not enabled")

plugin_add = json.loads((smoke_root / "plugin-add.json").read_text(encoding="utf-8"))
installed_path = Path(plugin_add.get("installedPath", ""))
if not installed_path.is_dir():
    raise SystemExit(f"installed plugin path is missing: {installed_path}")
expected = [
    ".codex-plugin/plugin.json",
    ".mcp.json",
    "hooks/hooks.json",
    "hooks/seatgeist_audit_summary.py",
    "skills/seatgeist-browser-debugging/SKILL.md",
    "skills/seatgeist-computer-use/SKILL.md",
    "skills/seatgeist-desktop-triage/SKILL.md",
    "skills/seatgeist-gui-testing/SKILL.md",
]
missing = [name for name in expected if not (installed_path / name).is_file()]
if missing:
    raise SystemExit(f"installed plugin is missing files: {', '.join(missing)}")

print(installed_path)
PY

installed_path="$(python3 -c 'import json, pathlib; print(json.loads(pathlib.Path("target/codex-plugin-smoke/plugin-add.json").read_text())["installedPath"])')"
scripts/validate-plugin.py "$installed_path"
seatgeist-mcp --help >/dev/null

echo "smoke-codex-plugin-install: ok $installed_path"
