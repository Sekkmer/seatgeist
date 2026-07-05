#!/usr/bin/env python3
"""Validate the repo-local Codex plugin bundle without external dependencies."""

from __future__ import annotations

import json
import importlib.util
import re
import sys
from pathlib import Path


REQUIRED_SKILLS = {
    "seatgeist-computer-use",
    "seatgeist-gui-testing",
    "seatgeist-browser-debugging",
    "seatgeist-desktop-triage",
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


def validate_marketplace(repo_root: Path, plugin_root: Path) -> None:
    marketplace_path = repo_root / ".agents" / "plugins" / "marketplace.json"
    if not marketplace_path.exists():
        if (repo_root / ".git").exists():
            fail("repo-local marketplace file is missing: .agents/plugins/marketplace.json")
        return
    marketplace = require_object(load_json(marketplace_path), "marketplace.json")
    if require_string(marketplace, "name", "marketplace.json") != "seatgeist-local":
        fail("marketplace.json name must be seatgeist-local")
    plugins = marketplace.get("plugins")
    if not isinstance(plugins, list) or len(plugins) != 1:
        fail("marketplace.json must contain exactly one plugin entry")
    entry = require_object(plugins[0], "marketplace.json.plugins[0]")
    if require_string(entry, "name", "marketplace.json.plugins[0]") != "seatgeist":
        fail("marketplace plugin name must be seatgeist")
    source = require_object(entry.get("source"), "marketplace.json.plugins[0].source")
    if source.get("source") != "local":
        fail("marketplace plugin source must be local")
    if source.get("path") != "./plugin":
        fail("marketplace plugin source.path must be ./plugin")
    resolved = (repo_root / "plugin").resolve()
    if resolved != plugin_root:
        fail("marketplace plugin path must resolve to the validated plugin root")
    policy = require_object(entry.get("policy"), "marketplace.json.plugins[0].policy")
    if policy.get("installation") != "AVAILABLE":
        fail("marketplace plugin installation policy must be AVAILABLE")
    interface = require_object(entry.get("interface"), "marketplace.json.plugins[0].interface")
    if require_string(interface, "displayName", "marketplace.json.plugins[0].interface") != "Seatgeist":
        fail("marketplace plugin displayName must be Seatgeist")


def validate_mcp(path: Path) -> None:
    config = require_object(load_json(path), ".mcp.json")
    servers = require_object(config.get("mcp_servers"), ".mcp.json.mcp_servers")
    server = require_object(servers.get("seatgeist"), ".mcp.json.mcp_servers.seatgeist")
    if require_string(server, "command", "seatgeist MCP server") != "seatgeist-mcp":
        fail("seatgeist MCP command must be seatgeist-mcp")
    args = server.get("args")
    if args != ["--stdio"]:
        fail("seatgeist MCP args must be [\"--stdio\"]")


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
        fail("Seatgeist hook must use type=command")
    command = require_string(handler, "command", "Seatgeist hook")
    expected = 'python3 "$(git rev-parse --show-toplevel)/plugin/hooks/seatgeist_audit_summary.py"'
    if command != expected:
        fail("Seatgeist hook command must run plugin/hooks/seatgeist_audit_summary.py from git root")
    if handler.get("timeout") != 10:
        fail("Seatgeist hook timeout must be 10 seconds")
    require_string(handler, "statusMessage", "Seatgeist hook")
    hook_script = path.parent / "seatgeist_audit_summary.py"
    if not hook_script.is_file():
        fail("Seatgeist hook script is missing")
    validate_hook_summary_script(hook_script)


def validate_hook_summary_script(path: Path) -> None:
    old_dont_write_bytecode = sys.dont_write_bytecode
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location("seatgeist_audit_summary", path)
    try:
        if spec is None or spec.loader is None:
            fail("Seatgeist hook script cannot be imported")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    finally:
        sys.dont_write_bytecode = old_dont_write_bytecode
    summarize_journal = getattr(module, "summarize_journal", None)
    if not callable(summarize_journal):
        fail("Seatgeist hook script must expose summarize_journal")

    audit = summarize_journal(
        [
            {
                "journal": "target/seatgeist-smoke-journal.jsonl",
                "sequence": 1,
                "client": {
                    "tool": "seatgeist-cli",
                    "pid": 1001,
                    "process_name": "seatgeist-cl",
                },
                "method": "observe",
                "ok": True,
                "safety_class": "observe",
                "guard_present": False,
                "summary": "observe 1 monitors 1 windows",
            },
            {
                "journal": "target/seatgeist-smoke-journal.jsonl",
                "sequence": 2,
                "client": {
                    "tool": "seatgeist-mcp",
                    "pid": 1002,
                    "process_name": "seatgeist-mc",
                },
                "method": "type_text",
                "ok": False,
                "safety_class": "control_keyboard",
                "guard_present": False,
                "active_window_before": {
                    "id": "window-1",
                    "app_id": "org.kde.kate",
                    "title": "scratch.txt",
                },
                "summary": "focus guard is required",
                "artifacts": [
                    {
                        "kind": "screenshot",
                        "path": "target/seatgeist/preview.png",
                        "sha256": "a" * 64,
                        "bytes": 42,
                    }
                ],
            },
            {
                "journal": "target/seatgeist-smoke-journal.jsonl",
                "sequence": 3,
                "client": {
                    "tool": "seatgeist-mcp",
                    "pid": 1002,
                    "process_name": "seatgeist-mc",
                },
                "method": "click_button",
                "ok": True,
                "safety_class": "control_semantic",
                "guard_present": True,
                "active_window_after": {
                    "id": "window-1",
                    "app_id": "org.kde.kate",
                    "title": "scratch.txt",
                },
                "summary": "clicked button name=OK",
            },
        ]
    )

    expected = {
        "entry_count": 3,
        "ok_count": 2,
        "failure_count": 1,
        "control_count": 2,
        "unguarded_control_count": 1,
    }
    for key, value in expected.items():
        if audit.get(key) != value:
            fail(f"Seatgeist hook audit {key} expected {value}, got {audit.get(key)}")
    if audit.get("methods", {}).get("type_text") != 1:
        fail("Seatgeist hook audit must count methods")
    if audit.get("safety_classes", {}).get("control_keyboard") != 1:
        fail("Seatgeist hook audit must count safety classes")
    if audit.get("clients", {}).get("seatgeist-mcp") != 2:
        fail("Seatgeist hook audit must count explicit client tool identities")
    if not audit.get("recent_failures"):
        fail("Seatgeist hook audit must include recent failures")
    if not audit.get("unguarded_control_examples"):
        fail("Seatgeist hook audit must include unguarded control examples")
    artifacts = audit["unguarded_control_examples"][0].get("artifacts")
    if not artifacts or artifacts[0].get("sha256") != "a" * 64:
        fail("Seatgeist hook audit must preserve compact artifact metadata when present")
    last_window = audit.get("last_active_window")
    if not isinstance(last_window, dict) or last_window.get("app_id") != "org.kde.kate":
        fail("Seatgeist hook audit must include last active window context")

    examples = audit.get("recent_failures", []) + audit.get("unguarded_control_examples", [])
    if any("raw prompt" in json.dumps(example) for example in examples):
        fail("Seatgeist hook audit examples must remain compact")


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
    repo_root = Path(__file__).resolve().parents[1]
    reject_todos(root)
    validate_manifest(root)
    if root == (repo_root / "plugin").resolve():
        validate_marketplace(repo_root, root)
    print(f"plugin validation passed: {root}")


if __name__ == "__main__":
    main()
