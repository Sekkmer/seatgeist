#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

from computer_use_eval import EvalError
from nested_seatgeist_probe import (
    daemon_socket_path,
    sanitized_bridge,
    sanitized_monitors,
)


def expect_error(call, message: str) -> None:
    try:
        call()
    except EvalError as err:
        assert message in str(err)
    else:
        raise AssertionError(f"expected EvalError containing {message!r}")


def main() -> None:
    monitors = sanitized_monitors(
        {
            "type": "monitors",
            "data": [
                {"id": "private-1", "logical_origin_x": 1280, "logical_origin_y": 0},
                {"id": "private-0", "logical_origin_x": 0, "logical_origin_y": 0},
            ],
        }
    )
    assert monitors["monitor_count"] == 2
    assert monitors["has_nonzero_logical_origin"] is True
    assert "private" not in str(monitors)

    bridge = sanitized_bridge(
        {
            "type": "kwin_bridge_status",
            "data": {
                "dbus_service_registered": True,
                "active_window_update_seen": True,
                "window_list_update_seen": True,
                "package_installed": True,
                "script_enabled": True,
                "active_window": {"title": "private"},
            },
        }
    )
    assert all(bridge.values())
    assert "private" not in str(bridge)
    assert daemon_socket_path(Path("/tmp/private-runtime")) == Path(
        "/tmp/private-runtime/d"
    )

    expect_error(
        lambda: daemon_socket_path(Path("/tmp") / ("long-runtime-" * 9)),
        "Unix socket limit",
    )
    expect_error(
        lambda: sanitized_monitors(
            {
                "type": "monitors",
                "data": [{"logical_origin_x": 0, "logical_origin_y": 0}],
            }
        ),
        "two-output topology",
    )
    expect_error(
        lambda: sanitized_bridge(
            {
                "type": "kwin_bridge_status",
                "data": {"dbus_service_registered": True},
            }
        ),
        "complete KWin bridge snapshot",
    )
    print("test-nested-seatgeist-probe: ok")


if __name__ == "__main__":
    main()
