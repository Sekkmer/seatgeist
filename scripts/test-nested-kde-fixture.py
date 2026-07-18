#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

from computer_use_eval import ROOT, EvalError
from nested_kde_assets import (
    install_bridge,
    install_protocol_probe_desktop,
    validate_bridge_source,
)
from nested_kde_contract import (
    NestedKdeConfig,
    absolute_wayland_display,
    fixture_paths,
    isolated_environment,
    kwin_command,
    normalized_payload,
    parse_unsigned_property,
    portal_capabilities,
    require_multi_output_layout,
    require_prepared_fixture_directories,
    sanitized_output_layout,
    prepare_fixture_directories,
    validate_config,
)
from nested_kde_fixture import has_screencast_protocol


def expect_error(call, message: str) -> None:
    try:
        call()
    except EvalError as err:
        assert message in str(err)
    else:
        raise AssertionError(f"expected EvalError containing {message!r}")


def output(name: str, x: int, y: int, *, enabled: bool = True) -> dict:
    return {
        "name": name,
        "enabled": enabled,
        "pos": {"x": x, "y": y},
        "scale": 1.0,
    }


def main() -> None:
    visible_denied = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts/nested-kde-fixture.py"),
            "--visible",
            "--",
            "true",
        ],
        capture_output=True,
        check=False,
        text=True,
    )
    assert visible_denied.returncode == 2
    assert "--visible requires --operator-present" in visible_denied.stderr

    with tempfile.TemporaryDirectory(prefix="seatgeist-nested-fixture-") as temporary:
        root = Path(temporary) / "fixture"
        headless = NestedKdeConfig(root=root, socket_name="nested-test")
        validate_config(headless)
        command = kwin_command(headless)
        assert "--virtual" in command
        assert "--wayland-display" not in command
        assert command[command.index("--output-count") + 1] == "2"

        paths = fixture_paths(headless)
        environment = isolated_environment(
            headless,
            paths,
            {
                "PATH": "/usr/bin",
                "HOME": "/host-home",
                "OPENAI_API_KEY": "must-not-leak",
                "DBUS_SESSION_BUS_ADDRESS": "unix:path=/private-bus",
                "SEATGEIST_NESTED_KDE_PRIVATE_BUS": "1",
            },
        )
        assert environment["HOME"] == str(root / "home")
        assert environment["XDG_RUNTIME_DIR"] == str(root / "runtime")
        assert environment["XDG_CONFIG_DIRS"] == "/etc/xdg"
        assert environment["XDG_MENU_PREFIX"] == "plasma-"
        assert environment["KDE_SESSION_VERSION"] == "6"
        assert environment["WAYLAND_DISPLAY"] == "nested-test"
        assert environment["NO_AT_BRIDGE"] == "1"
        assert environment["QT_ACCESSIBILITY"] == "0"
        assert environment["QT_NO_XDG_DESKTOP_PORTAL"] == "1"
        assert environment["PLASMA_INTEGRATION_USE_PORTAL"] == "0"
        assert environment["GTK_A11Y"] == "none"
        assert environment["GDK_BACKEND"] == "wayland"
        assert environment["PATH"] == "/usr/bin"
        assert environment["DBUS_SESSION_BUS_ADDRESS"] == "unix:path=/private-bus"
        assert environment["SEATGEIST_NESTED_KDE_PRIVATE_BUS"] == "1"
        assert "OPENAI_API_KEY" not in environment

        prepared = prepare_fixture_directories(headless)
        assert require_prepared_fixture_directories(headless) == prepared
        bridge_source = root.parent / "bridge-source"
        (bridge_source / "contents/code").mkdir(parents=True)
        (bridge_source / "contents/code/main.js").write_text(
            "// fixture\n", encoding="utf-8"
        )
        (bridge_source / "metadata.json").write_text(
            json.dumps({"KPlugin": {"Id": "seatgeist-bridge"}}),
            encoding="utf-8",
        )
        validate_bridge_source(bridge_source)
        installed = install_bridge(bridge_source, prepared)
        assert (installed / "contents/code/main.js").is_file()
        assert (prepared["config"] / "kwinrc").read_text(encoding="utf-8") == (
            "[Plugins]\nseatgeist-bridgeEnabled=true\n"
        )
        assert (prepared["config"] / "kwinrc").stat().st_mode & 0o777 == 0o600
        protocol_desktop = install_protocol_probe_desktop(prepared)
        protocol_text = protocol_desktop.read_text(encoding="utf-8")
        assert "Exec=/usr/bin/wayland-info" in protocol_text
        assert "zkde_screencast_unstable_v1" in protocol_text
        assert protocol_desktop.stat().st_mode & 0o777 == 0o644
        (prepared["cache"]).chmod(0o755)
        expect_error(
            lambda: require_prepared_fixture_directories(headless),
            "not private",
        )

        visible = NestedKdeConfig(
            root=root,
            socket_name="nested-visible",
            visible=True,
            host_wayland_display="/run/user/1000/wayland-0",
        )
        visible_command = kwin_command(visible)
        index = visible_command.index("--wayland-display")
        assert visible_command[index + 1] == "/run/user/1000/wayland-0"
        assert "--virtual" not in visible_command

    layout = sanitized_output_layout(
        {
            "outputs": [
                output("Virtual-1", 1280, 0),
                output("disabled", 2560, 0, enabled=False),
                output("Virtual-0", 0, 0),
            ]
        }
    )
    assert layout == {
        "monitor_count": 2,
        "has_nonzero_logical_origin": True,
        "outputs": [
            {"logical_origin_x": 0, "logical_origin_y": 0, "scale": 1.0},
            {"logical_origin_x": 1280, "logical_origin_y": 0, "scale": 1.0},
        ],
    }
    require_multi_output_layout(layout, 2)
    assert parse_unsigned_property(b"u 4\n") == 4
    assert portal_capabilities(4, 7, 7) == {
        "version": 4,
        "available_source_types": 7,
        "available_cursor_modes": 7,
    }
    assert (
        absolute_wayland_display("wayland-0", Path("/run/user/1000"))
        == "/run/user/1000/wayland-0"
    )
    assert (
        absolute_wayland_display("/run/user/1000/wayland-1", Path("/ignored"))
        == "/run/user/1000/wayland-1"
    )
    assert normalized_payload(("--", "true")) == ("true",)
    assert normalized_payload(("true",)) == ("true",)
    assert has_screencast_protocol(
        b"interface: 'zkde_screencast_unstable_v1', version: 1"
    )
    assert not has_screencast_protocol(b"interface: 'wl_output', version: 4")

    expect_error(
        lambda: parse_unsigned_property(b"s not-unsigned\n"),
        "malformed unsigned property",
    )
    expect_error(
        lambda: portal_capabilities(4, 1, 7),
        "no window source",
    )
    expect_error(
        lambda: absolute_wayland_display("nested/path", Path("/run/user/1000")),
        "socket basename",
    )
    expect_error(
        lambda: validate_config(
            NestedKdeConfig(root=Path("fixture"), socket_name="bad/socket")
        ),
        "socket name",
    )
    expect_error(
        lambda: validate_config(
            NestedKdeConfig(root=Path("fixture"), socket_name="fixture", output_count=1)
        ),
        "between two and eight",
    )
    expect_error(
        lambda: validate_config(
            NestedKdeConfig(
                root=Path("fixture"),
                socket_name="fixture",
                visible=True,
            )
        ),
        "host Wayland display",
    )
    expect_error(
        lambda: prepare_fixture_directories(
            NestedKdeConfig(
                root=Path("/tmp") / ("long-fixture-path-" * 8),
                socket_name="nested",
            )
        ),
        "Unix socket limit",
    )
    expect_error(
        lambda: require_multi_output_layout(
            sanitized_output_layout({"outputs": [output("Virtual-0", 0, 0)]}),
            2,
        ),
        "output count",
    )
    expect_error(
        lambda: require_multi_output_layout(
            sanitized_output_layout(
                {"outputs": [output("Virtual-0", 0, 0), output("Virtual-1", 0, 0)]}
            ),
            2,
        ),
        "non-zero",
    )
    assert "Virtual-0" not in str(layout)
    print("test-nested-kde-fixture: ok")


if __name__ == "__main__":
    main()
