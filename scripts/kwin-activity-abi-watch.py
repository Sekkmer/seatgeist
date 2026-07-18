#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path


DEFAULT_HEADER = Path("/usr/include/kwin/config-kwin.h")
DEFAULT_PLUGIN = (
    Path.home() / ".local/lib/qt6/plugins/kwin/plugins/seatgeistactivity.so"
)
DEFAULT_STATE = Path.home() / ".local/state/seatgeist/kwin-activity-abi.json"
DEFAULT_NOTIFY_COMMAND = Path("/usr/bin/notify-send")

HEADER_ABI_PATTERN = re.compile(
    rb'^\s*#define\s+KWIN_PLUGIN_VERSION_STRING\s+"([^"]+)"', re.MULTILINE
)
PLUGIN_ABI_PATTERN = re.compile(
    rb"org\.kde\.kwin\.PluginFactoryInterface(\d+\.\d+\.\d+)"
)


@dataclass(frozen=True)
class AbiReport:
    status: str
    required_abi: str | None
    plugin_abi: str | None


def extract_header_abi(content: bytes) -> str | None:
    match = HEADER_ABI_PATTERN.search(content)
    return match.group(1).decode("ascii") if match else None


def extract_plugin_abi(content: bytes) -> str | None:
    match = PLUGIN_ABI_PATTERN.search(content)
    return match.group(1).decode("ascii") if match else None


def inspect_abis(header: Path, plugin: Path) -> AbiReport:
    if not header.is_file():
        return AbiReport("missing_header", None, None)
    required_abi = extract_header_abi(header.read_bytes())
    if required_abi is None:
        return AbiReport("invalid_header", None, None)
    if not plugin.is_file():
        return AbiReport("missing_plugin", required_abi, None)
    plugin_abi = extract_plugin_abi(plugin.read_bytes())
    if plugin_abi is None:
        return AbiReport("invalid_plugin", required_abi, None)
    status = "current" if required_abi == plugin_abi else "rebuild_required"
    return AbiReport(status, required_abi, plugin_abi)


def read_boot_id() -> str:
    path = Path("/proc/sys/kernel/random/boot_id")
    try:
        return path.read_text(encoding="ascii").strip()
    except OSError:
        return "unknown"


def read_state(path: Path) -> dict[str, object]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    return data if isinstance(data, dict) else {}


def write_state(path: Path, data: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, delete=False
    ) as temporary:
        json.dump(data, temporary, sort_keys=True, separators=(",", ":"))
        temporary.write("\n")
        temporary_path = Path(temporary.name)
    try:
        temporary_path.chmod(0o600)
        os.replace(temporary_path, path)
    finally:
        temporary_path.unlink(missing_ok=True)


def notification_text(report: AbiReport) -> tuple[str, str]:
    title = "Seatgeist KWin plugin needs attention"
    if report.status == "rebuild_required":
        detail = f"KWin ABI {report.required_abi}; plugin ABI {report.plugin_abi}."
    elif report.status == "missing_plugin":
        detail = f"The Seatgeist plugin is missing for KWin ABI {report.required_abi}."
    elif report.status in {"missing_header", "invalid_header"}:
        detail = "The installed KWin plugin ABI could not be determined."
    else:
        detail = "The installed Seatgeist plugin ABI could not be determined."
    return title, f"{detail} Run make install-kwin-activity-user from the Seatgeist source tree."


def send_notification(command: Path, report: AbiReport) -> bool:
    title, body = notification_text(report)
    try:
        completed = subprocess.run(
            [
                str(command),
                "--app-name=Seatgeist",
                "--urgency=critical",
                "--icon=dialog-warning",
                title,
                body,
            ],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except OSError:
        return False
    return completed.returncode == 0


def notification_fingerprint(report: AbiReport, boot_id: str) -> dict[str, object]:
    return {"boot_id": boot_id, **asdict(report)}


def run_check(
    header: Path,
    plugin: Path,
    state_path: Path,
    notify_command: Path,
    *,
    check_only: bool,
    boot_id: str | None = None,
) -> dict[str, object]:
    report = inspect_abis(header, plugin)
    result: dict[str, object] = {
        "type": "seatgeist_kwin_activity_abi",
        "version": 1,
        **asdict(report),
        "notification_sent": False,
        "notification_suppressed": False,
    }
    if check_only:
        return result

    current_boot = boot_id or read_boot_id()
    fingerprint = notification_fingerprint(report, current_boot)
    previous = read_state(state_path)
    needs_notification = report.status != "current"
    already_notified = previous.get("fingerprint") == fingerprint and bool(
        previous.get("notified")
    )
    sent = False
    if needs_notification and not already_notified:
        sent = send_notification(notify_command, report)

    result["notification_sent"] = sent
    result["notification_suppressed"] = needs_notification and already_notified
    write_state(
        state_path,
        {
            "fingerprint": fingerprint,
            "notified": needs_notification and (sent or already_notified),
        },
    )
    return result


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Check whether the user-installed Seatgeist plugin matches KWin."
    )
    parser.add_argument("--header", type=Path, default=DEFAULT_HEADER)
    parser.add_argument("--plugin", type=Path, default=DEFAULT_PLUGIN)
    parser.add_argument("--state", type=Path, default=DEFAULT_STATE)
    parser.add_argument("--notify-command", type=Path, default=DEFAULT_NOTIFY_COMMAND)
    parser.add_argument(
        "--check-only", action="store_true", help="Do not notify or write state"
    )
    args = parser.parse_args()
    result = run_check(
        args.header,
        args.plugin,
        args.state,
        args.notify_command,
        check_only=args.check_only,
    )
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
