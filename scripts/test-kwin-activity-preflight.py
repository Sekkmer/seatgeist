#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import subprocess
import tempfile
from pathlib import Path
from unittest import mock


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

    with mock.patch.object(module.subprocess, "run", side_effect=subprocess.TimeoutExpired("qdbus6", 2)) as run:
        assert module.command_lines(["qdbus6"]) == []
        assert run.call_args.kwargs["timeout"] == 2
    with mock.patch.object(module, "command_lines", return_value=["4242"]) as command, mock.patch.object(Path, "read_text", return_value=maps) as read:
        pid, abi, deleted = module.running_kwin()
        assert (pid, abi, deleted) == (4242, "6.6.5", True)
        assert "org.freedesktop.DBus.GetConnectionUnixProcessID" in command.call_args.args[0]
        assert read.call_count == 1  # no cross-session /proc scan
    for response in [[], ["1", "2"], ["not-a-pid"], ["0"]]:
        with mock.patch.object(module, "command_lines", return_value=response), mock.patch.object(Path, "iterdir") as scan:
            assert module.running_kwin() == (None, None, False)
            scan.assert_not_called()
    print("test-kwin-activity-preflight: ok")


if __name__ == "__main__":
    main()
