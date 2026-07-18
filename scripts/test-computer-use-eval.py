#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

from computer_use_eval import workspace_revision


def git(root: Path, *arguments: str) -> None:
    subprocess.run(
        ["git", *arguments],
        cwd=root,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="seatgeist-workspace-revision-") as temporary:
        root = Path(temporary)
        git(root, "init", "-q")
        git(root, "config", "user.email", "seatgeist-test@example.invalid")
        git(root, "config", "user.name", "Seatgeist Test")
        (root / ".gitignore").write_text("ignored\n", encoding="utf-8")
        (root / "tracked.txt").write_text("one\n", encoding="utf-8")
        git(root, "add", ".gitignore", "tracked.txt")
        git(root, "commit", "-qm", "fixture")

        clean = workspace_revision(root)
        assert clean["dirty"] is False
        assert len(clean["git_head"]) == 40
        assert len(clean["tree_sha256"]) == 64
        assert workspace_revision(root) == clean

        (root / "tracked.txt").write_text("two\n", encoding="utf-8")
        modified = workspace_revision(root)
        assert modified["dirty"] is True
        assert modified["git_head"] == clean["git_head"]
        assert modified["tree_sha256"] != clean["tree_sha256"]

        (root / "untracked.txt").write_text("three\n", encoding="utf-8")
        untracked = workspace_revision(root)
        assert untracked["tree_sha256"] != modified["tree_sha256"]

        (root / "ignored").write_text("not evidence\n", encoding="utf-8")
        assert workspace_revision(root) == untracked

    print("test-computer-use-eval: ok")


if __name__ == "__main__":
    main()
