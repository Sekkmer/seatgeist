#!/usr/bin/env python3
"""Validate the repo-local Codex plugin bundle without external dependencies."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


REQUIRED_SKILLS = {
    "plasma-computer-use",
    "plasma-gui-testing",
    "plasma-browser-debugging",
    "plasma-desktop-triage",
}


def fail(message: str) -> None:
    raise SystemExit(f"plugin validation failed: {message}")


def load_json(path: Path) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        fail(f"{path} is not valid JSON: {err}")


def require_object(value: object, path: str) -> dict[str, object]:
    if not isinstance(value, dict):
        fail(f"{path} must be an object")
    return value


def require_string(obj: dict[str, object], key: str, path: str) -> str:
    value = obj.get(key)
    if not isinstance(value, str) or not value.strip():
        fail(f"{path}.{key} must be a non-empty string")
    return value


def require_relative_path(root: Path, value: object, field: str) -> Path:
    if not isinstance(value, str) or not value.startswith("./"):
        fail(f"plugin.json {field} must be a ./-prefixed relative path")
    target = root / value[2:]
    if not target.exists():
        fail(f"plugin.json {field} points at missing path {value}")
    return target


def reject_todos(root: Path) -> None:
    for path in root.rglob("*"):
        if path.is_file() and path.suffix in {".json", ".md"}:
            text = path.read_text(encoding="utf-8")
            if "[TODO:" in text:
                fail(f"{path} contains a scaffold TODO placeholder")


def parse_frontmatter(path: Path) -> dict[str, str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "---":
        fail(f"{path} must start with YAML-style frontmatter")
    fields: dict[str, str] = {}
    for line in lines[1:]:
        if line == "---":
            return fields
        if ":" not in line:
            fail(f"{path} has invalid frontmatter line: {line}")
        key, value = line.split(":", 1)
        fields[key.strip()] = value.strip().strip('"')
    fail(f"{path} frontmatter is not closed")


def validate_manifest(root: Path) -> None:
    manifest_path = root / ".codex-plugin" / "plugin.json"
    manifest = require_object(load_json(manifest_path), "plugin.json")

    name = require_string(manifest, "name", "plugin.json")
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", name):
        fail("plugin.json name must be lower-case hyphen-case and <=64 chars")
    version = require_string(manifest, "version", "plugin.json")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?", version):
        fail("plugin.json version must be semver")
    require_string(manifest, "description", "plugin.json")

    author = require_object(manifest.get("author"), "plugin.json.author")
    require_string(author, "name", "plugin.json.author")

    interface = require_object(manifest.get("interface"), "plugin.json.interface")
    for key in [
        "displayName",
        "shortDescription",
        "longDescription",
        "developerName",
        "category",
    ]:
        require_string(interface, key, "plugin.json.interface")
    capabilities = interface.get("capabilities")
    if not isinstance(capabilities, list) or not all(isinstance(item, str) for item in capabilities):
        fail("plugin.json.interface.capabilities must be a string array")

    skills_path = require_relative_path(root, manifest.get("skills"), "skills")
    mcp_path = require_relative_path(root, manifest.get("mcpServers"), "mcpServers")
    validate_skills(skills_path)
    validate_mcp(mcp_path)

    if "hooks" in manifest:
        require_relative_path(root, manifest.get("hooks"), "hooks")
    else:
        default_hooks = root / "hooks" / "hooks.json"
        if not default_hooks.exists():
            fail("plugin uses default hooks discovery but hooks/hooks.json is missing")
    validate_hooks(root / "hooks" / "hooks.json")


def validate_mcp(path: Path) -> None:
    config = require_object(load_json(path), ".mcp.json")
    servers = require_object(config.get("mcp_servers"), ".mcp.json.mcp_servers")
    server = require_object(servers.get("plasmapilot"), ".mcp.json.mcp_servers.plasmapilot")
    if require_string(server, "command", "plasmapilot MCP server") != "plasma-pilot-mcp":
        fail("plasmapilot MCP command must be plasma-pilot-mcp")
    args = server.get("args")
    if args != ["--stdio"]:
        fail("plasmapilot MCP args must be [\"--stdio\"]")


def validate_hooks(path: Path) -> None:
    hooks = require_object(load_json(path), "hooks/hooks.json")
    active_hooks = hooks.get("hooks")
    active_hooks = require_object(active_hooks, "hooks/hooks.json.hooks")
    stop_groups = active_hooks.get("Stop")
    if not isinstance(stop_groups, list) or len(stop_groups) != 1:
        fail("hooks/hooks.json must define exactly one Stop hook group")
    group = require_object(stop_groups[0], "hooks/hooks.json.hooks.Stop[0]")
    handlers = group.get("hooks")
    if not isinstance(handlers, list) or len(handlers) != 1:
        fail("hooks/hooks.json Stop group must define exactly one command hook")
    handler = require_object(handlers[0], "hooks/hooks.json.hooks.Stop[0].hooks[0]")
    if handler.get("type") != "command":
        fail("PlasmaPilot hook must use type=command")
    command = require_string(handler, "command", "PlasmaPilot hook")
    expected = 'python3 "$(git rev-parse --show-toplevel)/plugin/hooks/plasma_audit_summary.py"'
    if command != expected:
        fail("PlasmaPilot hook command must run plugin/hooks/plasma_audit_summary.py from git root")
    if handler.get("timeout") != 10:
        fail("PlasmaPilot hook timeout must be 10 seconds")
    require_string(handler, "statusMessage", "PlasmaPilot hook")
    if not (path.parent / "plasma_audit_summary.py").is_file():
        fail("PlasmaPilot hook script is missing")


def validate_skills(skills_root: Path) -> None:
    seen = set()
    for skill_path in sorted(skills_root.glob("*/SKILL.md")):
        frontmatter = parse_frontmatter(skill_path)
        skill_name = frontmatter.get("name")
        if skill_name != skill_path.parent.name:
            fail(f"{skill_path} frontmatter name must match directory name")
        description = frontmatter.get("description", "")
        if len(description) < 40:
            fail(f"{skill_path} description is too short for skill selection")
        seen.add(skill_name)
    if seen != REQUIRED_SKILLS:
        fail(f"skills must be exactly {sorted(REQUIRED_SKILLS)}, got {sorted(seen)}")


def main() -> None:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else "plugin").resolve()
    if not root.is_dir():
        fail(f"{root} is not a plugin directory")
    reject_todos(root)
    validate_manifest(root)
    print(f"plugin validation passed: {root}")


if __name__ == "__main__":
    main()
