#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
HOOKS = ROOT / "plugin/hooks/hooks.json"


def main() -> None:
    payload = json.loads(HOOKS.read_text(encoding="utf-8"))
    command = payload["hooks"]["Stop"][0]["hooks"][0]["command"]
    with tempfile.TemporaryDirectory(prefix="seatgeist-hook-resolution-") as temporary:
        root = Path(temporary)
        codex_home = root / "codex"
        script = (
            codex_home
            / "plugins/cache/seatgeist-local/seatgeist"
            / "0.1.0+codex.test"
            / "hooks/seatgeist_audit_summary.py"
        )
        script.parent.mkdir(parents=True)
        script.write_text(
            "from pathlib import Path\n"
            "import os\n"
            'Path(os.environ["SEATGEIST_HOOK_TEST_MARKER"]).write_text("ok")\n',
            encoding="utf-8",
        )
        work = root / "unrelated-checkout"
        work.mkdir()
        marker = root / "marker"
        environment = os.environ.copy()
        environment.update(
            {
                "CODEX_HOME": str(codex_home),
                "HOME": str(root / "home"),
                "SEATGEIST_HOOK_TEST_MARKER": str(marker),
            }
        )
        subprocess.run(command, cwd=work, env=environment, shell=True, check=True)
        assert marker.read_text(encoding="utf-8") == "ok"

    print("test-plugin-hook-resolution: ok")


if __name__ == "__main__":
    main()
