#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / "target" / "seatgeist-release" / "name-collision-check.json"
DEFAULT_NAMES = [
    "seatgeist",
    "libseatgeist",
    "seatgeistd",
    "seatgeist-cli",
    "seatgeist-mcp",
    "seatgeist-plugin",
]
USER_AGENT = "Seatgeist release name checker"


def git(args: list[str]) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()


def crates_sparse_path(name: str) -> str:
    lowered = name.lower()
    if len(lowered) == 1:
        return f"1/{lowered}"
    if len(lowered) == 2:
        return f"2/{lowered}"
    if len(lowered) == 3:
        return f"3/{lowered[0]}/{lowered}"
    return f"{lowered[0:2]}/{lowered[2:4]}/{lowered}"


def request_json(url: str, timeout: float) -> tuple[int | None, Any | None, str | None]:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read()
            status = int(response.status)
    except urllib.error.HTTPError as err:
        return int(err.code), None, None
    except urllib.error.URLError as err:
        return None, None, str(err.reason)
    except TimeoutError:
        return None, None, "request timed out"
    try:
        return status, json.loads(body.decode("utf-8")), None
    except (UnicodeDecodeError, json.JSONDecodeError) as err:
        return status, None, str(err)


def request_text(url: str, timeout: float) -> tuple[int | None, str | None, str | None]:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read()
            status = int(response.status)
    except urllib.error.HTTPError as err:
        return int(err.code), None, None
    except urllib.error.URLError as err:
        return None, None, str(err.reason)
    except TimeoutError:
        return None, None, "request timed out"
    try:
        return status, body.decode("utf-8"), None
    except UnicodeDecodeError as err:
        return status, None, str(err)


def registry_state(status: int | None, error: str | None) -> str:
    if status == 404:
        return "available"
    if status is not None and 200 <= status < 300:
        return "taken"
    if status is not None:
        return "error"
    if error:
        return "error"
    return "unknown"


def check_crates(name: str, timeout: float) -> dict[str, Any]:
    url = f"https://index.crates.io/{crates_sparse_path(name)}"
    status, text, error = request_text(url, timeout)
    state = registry_state(status, error)
    versions = 0
    if state == "taken" and text:
        versions = len([line for line in text.splitlines() if line.strip()])
    return {
        "registry": "crates.io",
        "name": name,
        "url": url,
        "status_code": status,
        "state": state,
        "published_versions": versions,
        "error": error,
    }


def check_simple_json_registry(registry: str, name: str, url: str, timeout: float) -> dict[str, Any]:
    status, _value, error = request_json(url, timeout)
    return {
        "registry": registry,
        "name": name,
        "url": url,
        "status_code": status,
        "state": registry_state(status, error),
        "error": error,
    }


def check_github(name: str, timeout: float) -> dict[str, Any]:
    query = urllib.parse.urlencode({"q": f"{name} in:name", "per_page": "10"})
    url = f"https://api.github.com/search/repositories?{query}"
    status, value, error = request_json(url, timeout)
    exact_matches: list[str] = []
    if isinstance(value, dict):
        for item in value.get("items", []):
            if isinstance(item, dict) and str(item.get("name", "")).lower() == name.lower():
                full_name = item.get("full_name")
                if isinstance(full_name, str):
                    exact_matches.append(full_name)
    api_ok_without_exact_match = status is not None and 200 <= status < 300 and not exact_matches
    state = "taken" if exact_matches else registry_state(404 if api_ok_without_exact_match else status, error)
    return {
        "registry": "github-repositories",
        "name": name,
        "url": url,
        "status_code": status,
        "state": state,
        "exact_matches": exact_matches,
        "error": error,
    }


def build_report(names: list[str], timeout: float) -> dict[str, Any]:
    checks: list[dict[str, Any]] = []
    for name in names:
        npm_name = urllib.parse.quote(name, safe="")
        pypi_name = urllib.parse.quote(name, safe="")
        checks.append(check_crates(name, timeout))
        checks.append(
            check_simple_json_registry(
                "npm",
                name,
                f"https://registry.npmjs.org/{npm_name}",
                timeout,
            )
        )
        checks.append(
            check_simple_json_registry(
                "pypi",
                name,
                f"https://pypi.org/pypi/{pypi_name}/json",
                timeout,
            )
        )
        checks.append(check_github(name, timeout))

    collisions = [check for check in checks if check["state"] == "taken"]
    errors = [check for check in checks if check["state"] == "error"]
    return {
        "type": "seatgeist_name_collision_check",
        "unix_time_ms": int(time.time() * 1000),
        "git": git(["rev-parse", "--short=12", "HEAD"]),
        "names": names,
        "collision_count": len(collisions),
        "error_count": len(errors),
        "checks": checks,
        "note": "Exact package and repository-name checks only; not a trademark or legal clearance.",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Check public package and repository name collisions for Seatgeist.")
    parser.add_argument("--name", action="append", dest="names", help="Name to check; may be repeated.")
    parser.add_argument("--output", default=str(DEFAULT_OUTPUT), help="Path for JSON evidence output.")
    parser.add_argument("--timeout", type=float, default=10.0, help="Per-request timeout in seconds.")
    parser.add_argument("--strict", action="store_true", help="Exit non-zero if collisions or registry errors are found.")
    args = parser.parse_args()

    names = args.names or DEFAULT_NAMES
    report = build_report(names, args.timeout)
    output = Path(args.output)
    if not output.is_absolute():
        output = ROOT / output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print(
        "check-public-name: "
        f"collisions={report['collision_count']} errors={report['error_count']} output={output}"
    )
    if args.strict and (report["collision_count"] or report["error_count"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
