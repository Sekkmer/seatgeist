#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PLUGIN_ID = "seatgeist@seatgeist-local"
MARKETPLACE = "seatgeist-local"
BINARIES = ("seatgeist-mcp", "seatgeist-cli", "seatgeistd")


@dataclass(frozen=True)
class Check:
    name: str
    level: str
    ok: bool
    summary: str
    evidence: list[str]

    def to_json(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "level": self.level,
            "ok": self.ok,
            "summary": self.summary,
            "evidence": self.evidence,
        }


def codex_home() -> Path:
    value = os.environ.get("CODEX_HOME")
    return Path(value).expanduser().resolve() if value else (Path.home() / ".codex").resolve()


def run(args: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, cwd=cwd or ROOT, text=True, capture_output=True, check=False)


def load_config(path: Path) -> dict[str, Any] | None:
    try:
        value = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return None
    return value if isinstance(value, dict) else None


def inside_current_checkout(path: Path) -> bool:
    try:
        path.relative_to(ROOT)
    except ValueError:
        return False
    return True


def foreign_build_output_path(path: Path) -> bool:
    parts = set(path.parts)
    return "target" in parts and not inside_current_checkout(path)


def check_config(home: Path) -> tuple[dict[str, Any] | None, Check]:
    path = home / "config.toml"
    config = load_config(path)
    if config is None:
        return None, Check("codex_config", "blocker", False, "Codex config is missing or invalid", [str(path)])
    return config, Check("codex_config", "required", True, "Codex config is readable", [str(path)])


def check_marketplace(config: dict[str, Any] | None) -> Check:
    if config is None:
        return Check("marketplace_source", "blocker", False, "Codex config is unavailable", [])
    marketplaces = config.get("marketplaces")
    marketplace = marketplaces.get(MARKETPLACE) if isinstance(marketplaces, dict) else None
    if not isinstance(marketplace, dict):
        return Check("marketplace_source", "blocker", False, "Seatgeist local marketplace is not registered", [MARKETPLACE])
    source = marketplace.get("source")
    source_type = marketplace.get("source_type")
    evidence = [f"source_type={source_type}", f"source={source}"]
    expected = str(ROOT)
    if source_type != "local" or source != expected:
        return Check(
            "marketplace_source",
            "blocker",
            False,
            "Seatgeist marketplace source does not point at this checkout",
            evidence + [f"expected={expected}"],
        )
    return Check("marketplace_source", "required", True, "Seatgeist marketplace source points at this checkout", evidence)


def check_plugin_enabled(config: dict[str, Any] | None) -> Check:
    if config is None:
        return Check("plugin_enabled", "blocker", False, "Codex config is unavailable", [])
    plugins = config.get("plugins")
    plugin = plugins.get(PLUGIN_ID) if isinstance(plugins, dict) else None
    evidence = [PLUGIN_ID]
    if not isinstance(plugin, dict) or plugin.get("enabled") is not True:
        return Check("plugin_enabled", "blocker", False, "Seatgeist plugin is not enabled in Codex config", evidence)
    return Check("plugin_enabled", "required", True, "Seatgeist plugin is enabled in Codex config", evidence)


def check_trusted_project(config: dict[str, Any] | None) -> Check:
    if config is None:
        return Check("trusted_project", "warning", False, "Codex config is unavailable", [])
    projects = config.get("projects")
    project = projects.get(str(ROOT)) if isinstance(projects, dict) else None
    level = project.get("trust_level") if isinstance(project, dict) else None
    if level != "trusted":
        return Check(
            "trusted_project",
            "warning",
            False,
            "this checkout is not marked trusted in Codex config",
            [f"checkout={ROOT}", f"trust_level={level}"],
        )
    return Check("trusted_project", "advisory", True, "this checkout is trusted by Codex", [str(ROOT)])


def check_installed_plugin(home: Path) -> Check:
    path = home / "plugins" / "cache" / MARKETPLACE / "seatgeist" / "0.1.0"
    if not path.is_dir():
        return Check("installed_plugin_cache", "blocker", False, "installed Seatgeist plugin cache is missing", [str(path)])
    result = run(["scripts/validate-plugin.py", str(path)])
    evidence = [str(path)]
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        return Check("installed_plugin_cache", "blocker", False, "installed Seatgeist plugin cache does not validate", evidence + [detail])
    return Check("installed_plugin_cache", "required", True, "installed Seatgeist plugin cache validates", evidence)


def check_binary(name: str) -> Check:
    found = shutil.which(name)
    if not found:
        return Check(f"binary_{name}", "blocker", False, f"{name} is not on PATH", [])
    path = Path(found)
    resolved = path.resolve(strict=False)
    evidence = [f"path={path}", f"resolved={resolved}"]
    if not resolved.exists():
        return Check(f"binary_{name}", "blocker", False, f"{name} resolves to a missing target", evidence)
    if foreign_build_output_path(resolved):
        return Check(f"binary_{name}", "blocker", False, f"{name} resolves to another checkout build output", evidence)
    result = run([str(path), "--version"])
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        return Check(f"binary_{name}", "blocker", False, f"{name} --version failed", evidence + [detail])
    version = result.stdout.strip()
    if not version.startswith(f"{name} "):
        return Check(f"binary_{name}", "blocker", False, f"{name} --version output is unexpected", evidence + [version])
    return Check(f"binary_{name}", "required", True, f"{name} resolves and runs", evidence + [version])


def build_report() -> dict[str, Any]:
    home = codex_home()
    config, config_check = check_config(home)
    checks = [
        config_check,
        check_marketplace(config),
        check_plugin_enabled(config),
        check_trusted_project(config),
        check_installed_plugin(home),
        *(check_binary(name) for name in BINARIES),
    ]
    blockers = [check for check in checks if check.level == "blocker" and not check.ok]
    warnings = [check for check in checks if check.level == "warning" and not check.ok]
    return {
        "type": "seatgeist_local_codex_install",
        "ok": not blockers,
        "blocker_count": len(blockers),
        "warning_count": len(warnings),
        "repo": str(ROOT),
        "codex_home": str(home),
        "checks": [check.to_json() for check in checks],
    }


def print_text(report: dict[str, Any]) -> None:
    status = "ok" if report["ok"] else f"not ok ({report['blocker_count']} blockers)"
    print(f"local-codex-install: {status}")
    print(f"repo: {report['repo']}")
    print(f"codex_home: {report['codex_home']}")
    for check in report["checks"]:
        if check["ok"]:
            mark = "ok"
        elif check["level"] == "warning":
            mark = "warning"
        else:
            mark = "blocker"
        print(f"- {mark}: {check['name']}: {check['summary']}")
        for item in check["evidence"][:4]:
            print(f"  {item}")
        if len(check["evidence"]) > 4:
            print(f"  ... {len(check['evidence']) - 4} more")


def main() -> None:
    parser = argparse.ArgumentParser(description="Check the user's local Codex Seatgeist plugin install.")
    parser.add_argument("--json", action="store_true", help="Print machine-readable JSON.")
    parser.add_argument("--strict", action="store_true", help="Exit non-zero when blockers remain.")
    args = parser.parse_args()

    report = build_report()
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_text(report)
    if args.strict and not report["ok"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
