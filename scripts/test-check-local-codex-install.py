#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import shutil
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/check-local-codex-install.py"


def load_module():
    spec = importlib.util.spec_from_file_location("check_local_codex_install", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def main() -> None:
    module = load_module()
    with tempfile.TemporaryDirectory(prefix="seatgeist-local-install-") as temporary:
        root = Path(temporary)
        home = root / "codex-home"
        plugin = root / "plugin"
        manifest = plugin / ".codex-plugin/plugin.json"
        manifest.parent.mkdir(parents=True)
        version = "0.1.0+codex.local-20260710-134446"
        manifest.write_text(f'{{"version":"{version}"}}\n', encoding="utf-8")
        assert module.source_plugin_version(plugin) == version

        validator = root / "validate-plugin"
        validator.write_text("#!/bin/sh\nexit 0\n", encoding="ascii")
        validator.chmod(0o755)
        cache = home / "plugins/cache/seatgeist-local/seatgeist" / version
        shutil.copytree(plugin, cache)
        check = module.check_installed_plugin(home, plugin, validator)
        assert check.ok
        assert str(cache) in check.evidence

        (plugin / ".mcp.json").write_text('{"changed":true}\n', encoding="utf-8")
        stale = module.check_installed_plugin(home, plugin, validator)
        assert not stale.ok
        assert "stale relative" in stale.summary
        assert "run make refresh-local-codex-plugin" in stale.evidence

        shutil.rmtree(cache)
        shutil.copytree(plugin, cache)
        refreshed = module.check_installed_plugin(home, plugin, validator)
        assert refreshed.ok

        manifest.write_text('{"version":"../../escape"}\n', encoding="utf-8")
        assert module.source_plugin_version(plugin) is None
        invalid = module.check_installed_plugin(home, plugin, validator)
        assert not invalid.ok
        assert "source Seatgeist plugin version" in invalid.summary

    print("test-check-local-codex-install: ok")


if __name__ == "__main__":
    main()
