#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/kwin-activity-preflight.py"


def load_module():
    spec = importlib.util.spec_from_file_location("kwin_activity_preflight", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> None:
    module = load_module()
    with tempfile.TemporaryDirectory(prefix="seatgeist-kwin-preflight-") as temporary:
        artifact = Path(temporary) / "seatgeistactivity.so"
        artifact.write_bytes(
            b"prefix org.kde.kwin.PluginFactoryInterface6.7.1 suffix"
        )
        assert module.plugin_abi(artifact) == "6.7.1"
        assert module.plugin_abi(Path(temporary) / "missing") is None

    maps = """\
7f00-7f10 r-xp 00000000 00:00 0 /usr/lib/libkwin.so.6.6.5 (deleted)
7f20-7f30 r--p 00000000 00:00 0 /usr/lib/libQt6Core.so.6.11.1
"""
    assert module.libkwin_abi_from_maps(maps) == "6.6.5"
    assert module.libkwin_abi_from_maps("no kwin mapping") is None
    support = (
        "Version\n"
        + ("=" * 7)
        + "\nKWin version: 6.7.2\nQt Version: 6.11.1\n"
    )
    assert module.kwin_abi_from_support_information(support) == "6.7.2"
    assert module.kwin_abi_from_support_information("no version") is None
    print("test-kwin-activity-preflight: ok")


if __name__ == "__main__":
    main()
