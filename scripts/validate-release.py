#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    print(f"validate-release: {message}", file=sys.stderr)
    raise SystemExit(1)


def read(path: str) -> str:
    file_path = ROOT / path
    if not file_path.exists():
        fail(f"{path} is missing")
    return file_path.read_text(encoding="utf-8")


def require_contains(path: str, text: str, needle: str) -> None:
    if needle not in text:
        fail(f"{path} does not contain expected text: {needle}")


def main() -> None:
    cargo = read("Cargo.toml")
    require_contains("Cargo.toml", cargo, 'license = "MIT OR Apache-2.0"')

    mit = read("LICENSE-MIT")
    require_contains("LICENSE-MIT", mit, "MIT License")
    require_contains("LICENSE-MIT", mit, "Copyright (c) 2026 Sekkmer")
    require_contains("LICENSE-MIT", mit, "Permission is hereby granted")

    apache = read("LICENSE-APACHE")
    require_contains("LICENSE-APACHE", apache, "Apache License")
    require_contains("LICENSE-APACHE", apache, "Version 2.0, January 2004")
    require_contains("LICENSE-APACHE", apache, "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION")

    checklist = read("docs/release-checklist.md")
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [x] Final license files match the workspace `MIT OR Apache-2.0` declaration.",
    )
    require_contains(
        "docs/release-checklist.md",
        checklist,
        "- [ ] Add real public repository metadata before publishing",
    )

    ci = read(".github/workflows/ci.yml")
    require_contains(".github/workflows/ci.yml", ci, "make verify")
    require_contains(".github/workflows/ci.yml", ci, "libei-dev")
    require_contains(".github/workflows/ci.yml", ci, "libxkbcommon-dev")

    print("validate-release: ok")


if __name__ == "__main__":
    main()
