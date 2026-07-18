#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/install-kwin-screenshot-authorization.py"


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="seatgeist-kwin-screenshot-auth-") as temporary:
        root = Path(temporary)
        executable = root / "home/.local/bin/seatgeistd"
        applications = root / "home/.local/share/applications"
        cache_log = root / "kbuildsycoca.log"
        cache = root / "kbuildsycoca6"
        cache.write_text(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" > " + str(cache_log) + "\n",
            encoding="utf-8",
        )
        cache.chmod(0o755)
        completed = subprocess.run(
            [
                str(SCRIPT),
                "--daemon-path",
                str(executable),
                "--applications-dir",
                str(applications),
                "--kbuildsycoca",
                str(cache),
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
        )
        report = json.loads(completed.stdout)
        assert report["ok"] is True
        assert report["restricted_interface"] == "org.kde.KWin.ScreenShot2"
        desktop = applications / "org.seatgeist.daemon.desktop"
        content = desktop.read_text(encoding="utf-8")
        assert f"Exec={executable}" in content
        assert "X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2" in content
        assert desktop.stat().st_mode & 0o777 == 0o644
        assert cache_log.read_text(encoding="utf-8").strip() == "--noincremental"

    print("test-install-kwin-screenshot-authorization: ok")


if __name__ == "__main__":
    main()
