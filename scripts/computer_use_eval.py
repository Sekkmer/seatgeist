from __future__ import annotations

import hashlib
import json
import os
import stat
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


class EvalError(RuntimeError):
    pass


def response_data(response: dict[str, Any], expected_type: str) -> dict[str, Any]:
    if response.get("type") == "error":
        data = response.get("data")
        message = data.get("message") if isinstance(data, dict) else None
        raise EvalError(str(message or "Seatgeist returned an error"))
    if response.get("type") != expected_type or not isinstance(response.get("data"), dict):
        raise EvalError(f"expected {expected_type} response")
    return response["data"]


def parse_json_output(output: str) -> dict[str, Any]:
    try:
        value = json.loads(output)
    except json.JSONDecodeError as err:
        raise EvalError(f"CLI returned invalid JSON: {err}") from err
    if not isinstance(value, dict):
        raise EvalError("CLI response is not an object")
    return value


def run_cli(
    cli: Path,
    socket: Path | None,
    *arguments: str,
    cwd: Path = ROOT,
) -> dict[str, Any]:
    command = [str(cli)]
    if socket is not None:
        command.extend(["--socket", str(socket)])
    command.extend(arguments)
    completed = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise EvalError(f"CLI command failed: {detail}")
    return parse_json_output(completed.stdout)


def unix_time_ms() -> int:
    return int(time.time() * 1000)


def workspace_revision(root: Path = ROOT) -> dict[str, Any]:
    try:
        head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=root,
            text=True,
            stderr=subprocess.PIPE,
        ).strip()
        listed = subprocess.check_output(
            [
                "git",
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
            cwd=root,
            stderr=subprocess.PIPE,
        )
        dirty = bool(
            subprocess.check_output(
                ["git", "status", "--porcelain=v1", "--untracked-files=all"],
                cwd=root,
                stderr=subprocess.PIPE,
            ).strip()
        )
    except (OSError, subprocess.CalledProcessError) as err:
        raise EvalError(f"read workspace revision: {err}") from err

    digest = hashlib.sha256()
    digest.update(b"seatgeist-workspace-v1\0")
    paths = sorted(path for path in listed.split(b"\0") if path)
    for encoded_path in paths:
        relative = Path(os.fsdecode(encoded_path))
        path = root / relative
        digest.update(len(encoded_path).to_bytes(8, "big"))
        digest.update(encoded_path)
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            digest.update(b"missing\0")
            continue
        digest.update(stat.S_IMODE(metadata.st_mode).to_bytes(4, "big"))
        if path.is_symlink():
            digest.update(b"symlink\0")
            digest.update(os.fsencode(os.readlink(path)))
        elif path.is_file():
            digest.update(b"file\0")
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
        else:
            digest.update(b"other\0")
    return {
        "git_head": head,
        "tree_sha256": digest.hexdigest(),
        "dirty": dirty,
    }


def default_socket_path() -> Path:
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    root = Path(runtime) if runtime else Path(f"/run/user/{os.geteuid()}")
    return root / "seatgeist/seatgeistd.sock"


def default_capture_restore_path() -> Path:
    configured = os.environ.get("XDG_STATE_HOME")
    root = Path(configured) if configured else Path.home() / ".local/state"
    return root / "seatgeist/capture-restore.json"


def default_approval_file_path() -> Path:
    configured = os.environ.get("XDG_STATE_HOME")
    root = Path(configured) if configured else Path.home() / ".local/state"
    return root / "seatgeist/approvals.jsonl"


def socket_identity(socket: Path | None) -> dict[str, int] | None:
    path = socket or default_socket_path()
    try:
        metadata = path.stat()
    except OSError:
        return None
    return {"device": metadata.st_dev, "inode": metadata.st_ino}


def private_png_info(path: Path, max_edge: int) -> dict[str, Any]:
    try:
        metadata = path.lstat()
    except OSError as err:
        raise EvalError(f"capture artifact is missing: {path}") from err
    if not stat.S_ISREG(metadata.st_mode) or path.is_symlink():
        raise EvalError(f"capture artifact is not a regular file: {path}")
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise EvalError(f"capture artifact is accessible by group or other: {path}")
    with path.open("rb") as capture:
        if capture.read(8) != b"\x89PNG\r\n\x1a\n":
            raise EvalError(f"capture artifact is not PNG: {path}")
    return {"path": str(path), "bytes": metadata.st_size, "max_edge": max_edge}


def private_file_identity(path: Path) -> dict[str, int]:
    try:
        metadata = path.lstat()
    except OSError as err:
        raise EvalError(f"private state file is missing: {path}") from err
    if not stat.S_ISREG(metadata.st_mode) or path.is_symlink():
        raise EvalError(f"private state path is not a regular file: {path}")
    if metadata.st_uid != os.geteuid():
        raise EvalError(f"private state file has a different owner: {path}")
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise EvalError(f"private state file is accessible by group or other: {path}")
    return {
        "device": metadata.st_dev,
        "inode": metadata.st_ino,
        "bytes": metadata.st_size,
        "mtime_ns": metadata.st_mtime_ns,
    }


def read_private_json(path: Path) -> dict[str, Any]:
    private_file_identity(path)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as err:
        raise EvalError(f"private JSON state is unreadable: {path}") from err
    if not isinstance(value, dict):
        raise EvalError("private JSON state is not an object")
    return value


def write_private_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, delete=False
    ) as temporary:
        json.dump(value, temporary, indent=2, sort_keys=True)
        temporary.write("\n")
        temporary_path = Path(temporary.name)
    try:
        temporary_path.chmod(0o600)
        os.replace(temporary_path, path)
    finally:
        temporary_path.unlink(missing_ok=True)
