#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import os
import re
import subprocess
import tempfile
import time
from dataclasses import asdict, dataclass
from pathlib import Path


DEFAULT_HEADER = Path("/usr/include/kwin/config-kwin.h")
DEFAULT_PLUGIN = (
    Path.home() / ".local/lib/qt6/plugins/kwin/plugins/seatgeistactivity.so"
)
DEFAULT_AGENT_SEAT_PLUGIN = (
    Path.home() / ".local/lib/qt6/plugins/kwin/plugins/seatgeistagentseat.so"
)
DEFAULT_STATE = Path.home() / ".local/state/seatgeist/kwin-activity-abi.json"
DEFAULT_NOTIFY_COMMAND = Path("/usr/bin/notify-send")
DEFAULT_NOTIFICATION_TIMEOUT_SECONDS = 3.0
DEFAULT_CLI = Path.home() / ".local/bin/seatgeist-cli"
PROBE_TIMEOUT_SECONDS = 2.0
KWIN_VERSION_PATTERN = re.compile(r"^KWin version:\s*(\d+\.\d+\.\d+)\s*$", re.MULTILINE)

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
    plugins: tuple["PluginAbiReport", ...] = ()


@dataclass(frozen=True)
class PluginAbiReport:
    name: str
    status: str
    plugin_abi: str | None


@dataclass(frozen=True)
class RuntimeReport:
    status: str
    running_abi: str | None = None
    loaded_plugins: tuple[str, ...] = ()
    activity_trusted: bool | None = None
    configured_input: str | None = None
    implemented_input: str | None = None
    next_step: str = "Run --check-only --runtime to check the current session."


class ProbeError(Exception):
    pass


def probe_json(arguments: list[str]) -> dict[str, object]:
    """Bound every read-only subprocess; never retain its raw output in state."""
    try:
        completed = subprocess.run(
            arguments, text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            check=False, timeout=PROBE_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        raise ProbeError("timeout") from error
    except OSError as error:
        raise ProbeError("unavailable") from error
    except UnicodeError as error:
        raise ProbeError("invalid_response") from error
    if completed.returncode != 0:
        raise ProbeError("failed")
    try:
        result = json.loads(completed.stdout)
    except (ValueError, TypeError) as error:
        raise ProbeError("invalid_response") from error
    if not isinstance(result, dict):
        raise ProbeError("invalid_response")
    return result


def bus_probe(*arguments: str) -> dict[str, object]:
    return probe_json([
        "/usr/bin/busctl", "--user", "--auto-start=no", "--timeout=2",
        "--json=short", *arguments,
    ])


def cli_probe(cli: Path, response_type: str, *arguments: str) -> dict[str, object]:
    result = probe_json([str(cli), *arguments])
    if result.get("type") != response_type or not isinstance(result.get("data"), dict):
        raise ProbeError("invalid_response")
    return result["data"]


def inspect_runtime(report: AbiReport, cli: Path) -> RuntimeReport:
    """Read only the session owning our user bus; do not guess a KWin PID."""
    if report.status != "current":
        return RuntimeReport("blocked_by_install", next_step="Repair the installed plugins first; do not hot-load them.")
    running_abi = None
    loaded: tuple[str, ...] = ()
    try:
        support = bus_probe("call", "org.kde.KWin", "/KWin", "org.kde.KWin", "supportInformation")
        data = support.get("data")
        if support.get("type") != "s" or not isinstance(data, list) or len(data) != 1 or not isinstance(data[0], str):
            raise ProbeError("invalid_response")
        match = KWIN_VERSION_PATTERN.search(data[0])
        if not match:
            raise ProbeError("unknown_running_abi")
        running_abi = match.group(1)
        if running_abi != report.required_abi:
            return RuntimeReport("session_restart_required", running_abi, next_step="Installed files and running KWin differ. Save work and use a normal logout/login; do not hot-load plugins.")
        plugins = bus_probe("get-property", "org.kde.KWin", "/Plugins", "org.kde.KWin.Plugins", "LoadedPlugins")
        data = plugins.get("data")
        if plugins.get("type") != "as" or not isinstance(data, list) or not all(isinstance(item, str) for item in data):
            raise ProbeError("invalid_response")
        required = {"seatgeistactivity"}
        if any(plugin.name == "agent-seat" and plugin.status == "current" for plugin in report.plugins):
            required.add("seatgeistagentseat")
        loaded = tuple(sorted(required.intersection(data)))
        if not required.issubset(data):
            return RuntimeReport("plugins_not_loaded", running_abi, loaded, next_step="Check KWin plugin enablement and user plugin paths, then use a normal logout/login. Do not hot-load into the current compositor.")
        # Avoid starting a socket-activated daemon just for a periodic probe.
        owner = bus_probe("call", "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", "s", "org.seatgeist.KWinBridge")
        data = owner.get("data")
        if owner.get("type") != "b" or not isinstance(data, list) or len(data) != 1 or not isinstance(data[0], bool):
            raise ProbeError("invalid_response")
        if owner["data"] == [False]:
            return RuntimeReport("daemon_unavailable", running_abi, loaded, next_step="Check the Seatgeist user service and its D-Bus bridge; no service was started by this check.")
        safety = cli_probe(cli, "safety_status", "safety-status")
        inputs = cli_probe(cli, "input_backend_status", "input", "status")
        trusted = safety.get("human_input_activity_trusted")
        activity_backend = safety.get("human_input_activity_backend")
        configured = inputs.get("configured_backend")
        implemented = inputs.get("implemented_available_backend")
        if (
            not isinstance(trusted, bool)
            or not isinstance(configured, str)
            or not configured
            or (activity_backend is not None and not isinstance(activity_backend, str))
            or (implemented is not None and (not isinstance(implemented, str) or not implemented))
        ):
            raise ProbeError("invalid_response")
        if not trusted or activity_backend not in {"kwin_input_spy_v1", "kwin_input_spy_v2"}:
            return RuntimeReport("activity_unregistered", running_abi, loaded, trusted, configured, implemented, "Plugins are loaded but trusted activity registration is absent. Inspect Seatgeist diagnostics; do not weaken the safety gates.")
        if configured == "kwin_agent_seat" and implemented != "kwin_agent_seat":
            return RuntimeReport("agent_seat_unavailable", running_abi, loaded, trusted, configured, implemented, "The selected agent-seat backend is unavailable. Inspect daemon/plugin compatibility; no fallback was selected.")
        if implemented is None:
            return RuntimeReport("input_backend_unavailable", running_abi, loaded, trusted, configured, implemented, "No executable input backend is available. Inspect Seatgeist input status; no fallback was selected.")
        return RuntimeReport("ready", running_abi, loaded, trusted, configured, implemented, "Native plugin load/registration checks passed. This is not an end-to-end GUI interaction test.")
    except ProbeError as error:
        return RuntimeReport("unknown", running_abi, loaded, next_step=f"Runtime probe {error}. Check the user bus, CLI/daemon compatibility and logs; do not assume the plugins are ready.")


def extract_header_abi(content: bytes) -> str | None:
    match = HEADER_ABI_PATTERN.search(content)
    return match.group(1).decode("ascii") if match else None


def extract_plugin_abi(content: bytes) -> str | None:
    match = PLUGIN_ABI_PATTERN.search(content)
    return match.group(1).decode("ascii") if match else None


def inspect_abis(
    header: Path,
    plugin: Path,
    agent_seat_plugin: Path | None = None,
) -> AbiReport:
    if not header.is_file():
        return AbiReport("missing_header", None, None)
    try:
        required_abi = extract_header_abi(header.read_bytes())
    except OSError:
        return AbiReport("unreadable_header", None, None)
    if required_abi is None:
        return AbiReport("invalid_header", None, None)
    plugin_reports = [inspect_plugin("activity", plugin, required_abi, required=True)]
    if agent_seat_plugin is not None:
        plugin_reports.append(
            inspect_plugin(
                "agent-seat", agent_seat_plugin, required_abi, required=False
            )
        )
    activity = plugin_reports[0]
    actionable = [
        report
        for report in plugin_reports
        if report.status not in {"current", "not_installed"}
    ]
    status = actionable[0].status if actionable else "current"
    return AbiReport(
        status,
        required_abi,
        activity.plugin_abi,
        tuple(plugin_reports),
    )


def inspect_plugin(
    name: str, plugin: Path, required_abi: str, *, required: bool
) -> PluginAbiReport:
    if not plugin.is_file():
        return PluginAbiReport(
            name, "missing_plugin" if required else "not_installed", None
        )
    try:
        plugin_abi = extract_plugin_abi(plugin.read_bytes())
    except OSError:
        return PluginAbiReport(name, "unreadable_plugin", None)
    if plugin_abi is None:
        return PluginAbiReport(name, "invalid_plugin", None)
    status = "current" if required_abi == plugin_abi else "rebuild_required"
    return PluginAbiReport(name, status, plugin_abi)


def read_boot_id() -> str:
    path = Path("/proc/sys/kernel/random/boot_id")
    try:
        return path.read_text(encoding="ascii").strip()
    except OSError:
        return "unknown"


def read_state(path: Path) -> dict[str, object]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, UnicodeError):
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


def notification_text(report: AbiReport, runtime: RuntimeReport | None = None) -> tuple[str, str]:
    title = "Seatgeist KWin plugin needs attention"
    if report.status == "current" and runtime is not None:
        return title, f"Installed plugins match KWin, but session status is {runtime.status}. {runtime.next_step}"
    affected = [
        plugin
        for plugin in report.plugins
        if plugin.status not in {"current", "not_installed"}
    ]
    if report.status == "rebuild_required" and affected:
        plugins = ", ".join(
            f"{plugin.name} ABI {plugin.plugin_abi or 'unknown'}"
            for plugin in affected
        )
        detail = f"KWin ABI {report.required_abi}; {plugins}."
    elif report.status == "missing_plugin":
        detail = f"The Seatgeist plugin is missing for KWin ABI {report.required_abi}."
    elif report.status in {"missing_header", "invalid_header"}:
        detail = "The installed KWin plugin ABI could not be determined."
    else:
        detail = "The installed Seatgeist plugin ABI could not be determined."
    return title, (
        f"{detail} Rebuild and reinstall the affected Seatgeist KWin plugins "
        "before logout or reboot."
    )


def send_notification(
    command: Path,
    report: AbiReport,
    *,
    timeout_seconds: float = DEFAULT_NOTIFICATION_TIMEOUT_SECONDS,
    runtime: RuntimeReport | None = None,
) -> tuple[bool, str | None]:
    title, body = notification_text(report, runtime)
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
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired:
        return False, "timeout"
    except OSError:
        return False, "unavailable"
    if completed.returncode != 0:
        return False, "failed"
    return True, None


def notification_fingerprint(report: AbiReport, boot_id: str) -> dict[str, object]:
    return {
        "boot_id": boot_id,
        "status": report.status,
        "required_abi": report.required_abi,
        "plugin_abi": report.plugin_abi,
        "plugins": [asdict(plugin) for plugin in report.plugins],
    }


def run_check(
    header: Path,
    plugin: Path,
    agent_seat_plugin: Path | None,
    state_path: Path,
    notify_command: Path,
    *,
    check_only: bool,
    boot_id: str | None = None,
    notification_timeout_seconds: float = DEFAULT_NOTIFICATION_TIMEOUT_SECONDS,
    runtime_check: bool = False,
    cli: Path = DEFAULT_CLI,
) -> dict[str, object]:
    report = inspect_abis(header, plugin, agent_seat_plugin)
    runtime = inspect_runtime(report, cli) if runtime_check else RuntimeReport("not_checked")
    runtime_data = {**asdict(runtime), "loaded_plugins": list(runtime.loaded_plugins)}
    result: dict[str, object] = {
        "type": "seatgeist_kwin_activity_abi",
        "version": 4,
        **asdict(report),
        "runtime": runtime_data,
        "runtime_ready": runtime.status == "ready",
        "notification_sent": False,
        "notification_suppressed": False,
        "notification_failure": None,
    }
    if check_only:
        return result

    current_boot = boot_id or read_boot_id()
    fingerprint = notification_fingerprint(report, current_boot)
    if runtime_check:
        fingerprint["runtime"] = runtime_data
    previous = read_state(state_path)
    needs_notification = report.status != "current" or (runtime_check and runtime.status != "ready")
    already_notified = previous.get("fingerprint") == fingerprint and bool(
        previous.get("notified")
    )
    sent = False
    notification_failure = None
    if needs_notification and not already_notified:
        sent, notification_failure = send_notification(
            notify_command,
            report,
            timeout_seconds=notification_timeout_seconds,
            runtime=runtime,
        )

    result["notification_sent"] = sent
    result["notification_suppressed"] = needs_notification and already_notified
    result["notification_failure"] = notification_failure
    write_state(
        state_path,
        {
            "checked_at_unix_ms": int(time.time() * 1000),
            "fingerprint": fingerprint,
            "last_notification_failure": notification_failure,
            "notified": needs_notification and (sent or already_notified),
            "report": result,
        },
    )
    return result


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Check whether the user-installed Seatgeist plugin matches KWin."
    )
    parser.add_argument("--header", type=Path, default=DEFAULT_HEADER)
    parser.add_argument("--plugin", type=Path, default=DEFAULT_PLUGIN)
    parser.add_argument(
        "--agent-seat-plugin", type=Path, default=DEFAULT_AGENT_SEAT_PLUGIN
    )
    parser.add_argument("--state", type=Path, default=DEFAULT_STATE)
    parser.add_argument("--notify-command", type=Path, default=DEFAULT_NOTIFY_COMMAND)
    parser.add_argument("--runtime", action="store_true", help="Also check current KWin plugin load and daemon registration (read-only, bounded)")
    parser.add_argument("--cli", type=Path, default=DEFAULT_CLI)
    parser.add_argument(
        "--notification-timeout-seconds",
        type=float,
        default=DEFAULT_NOTIFICATION_TIMEOUT_SECONDS,
        help="Maximum time to wait for notify-send (default: 3 seconds)",
    )
    parser.add_argument(
        "--check-only", action="store_true", help="Do not notify or write state"
    )
    args = parser.parse_args()
    if not math.isfinite(args.notification_timeout_seconds) or not 0 < args.notification_timeout_seconds <= 3:
        parser.error("--notification-timeout-seconds must be finite, greater than zero and at most 3")
    result = run_check(
        args.header,
        args.plugin,
        args.agent_seat_plugin,
        args.state,
        args.notify_command,
        check_only=args.check_only,
        notification_timeout_seconds=args.notification_timeout_seconds,
        runtime_check=args.runtime,
        cli=args.cli,
    )
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
