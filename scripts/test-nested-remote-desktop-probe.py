#!/usr/bin/env python3
from __future__ import annotations

from computer_use_eval import EvalError
from nested_remote_desktop_probe import parse_methods, sanitized_capabilities


def expect_error(call, message: str) -> None:
    try:
        call()
    except EvalError as err:
        assert message in str(err)
    else:
        raise AssertionError(f"expected EvalError containing {message!r}")


def main() -> None:
    methods = parse_methods(
        b"""NAME TYPE SIGNATURE RESULT/VALUE FLAGS
.ConnectToEIS method osa{sv} h -
.CreateSession method oosa{sv} ua{sv} -
.SelectDevices method oosa{sv} ua{sv} -
.Start method oossa{sv} ua{sv} -
.version property u 2 emits-change
"""
    )
    capabilities = sanitized_capabilities(2, 7, methods)
    assert capabilities == {
        "version": 2,
        "available_device_types": 7,
        "keyboard": True,
        "pointer": True,
        "touchscreen": True,
        "connect_to_eis": True,
        "lifecycle_methods": True,
    }
    assert parse_methods(b".version property u 2 emits-change\n") == set()

    expect_error(
        lambda: sanitized_capabilities(1, 7, methods),
        "does not support ConnectToEIS",
    )
    expect_error(
        lambda: sanitized_capabilities(2, 1, methods),
        "lacks keyboard or pointer",
    )
    expect_error(
        lambda: sanitized_capabilities(2, 7, methods - {"ConnectToEIS"}),
        "missing required methods",
    )
    print("test-nested-remote-desktop-probe: ok")


if __name__ == "__main__":
    main()
