#!/usr/bin/env python3
from __future__ import annotations

import tempfile
from pathlib import Path

from computer_use_eval import EvalError
from nested_retained_capture import (
    TARGET_TITLE,
    daemon_command,
    find_fixture_windows,
    firefox_command,
    helper_command,
    prepare_firefox_profile,
)


def expect_error(call, message: str) -> None:
    try:
        call()
    except EvalError as err:
        assert message in str(err)
    else:
        raise AssertionError(f"expected EvalError containing {message!r}")


def main() -> None:
    response = {
        "type": "windows",
        "data": [
            {"id": "helper-private", "app_id": "org.kde.konsole", "pid": 41},
            {
                "id": "target-private",
                "app_id": "firefox",
                "pid": 42,
                "title": f"{TARGET_TITLE} - Mozilla Firefox",
            },
        ],
    }
    target, helper_count = find_fixture_windows(response)
    assert target == {"window_id": "target-private", "pid": 42}
    assert helper_count == 1

    with tempfile.TemporaryDirectory(prefix="seatgeist-nested-browser-") as temporary:
        root = Path(temporary)
        profile = root / "profile"
        prepare_firefox_profile(profile)
        user_js = profile / "user.js"
        assert user_js.is_file()
        assert user_js.stat().st_mode & 0o777 == 0o600
        assert "checkDefaultBrowser" in user_js.read_text(encoding="utf-8")
        assert 'user_pref("browser.tabs.inTitlebar", 0);' in user_js.read_text(
            encoding="utf-8"
        )

        daemon = daemon_command(Path("daemon"), root / "d", root / "state")
        assert daemon[:3] == ["daemon", "--socket", str(root / "d")]
        firefox = firefox_command(Path("firefox"), profile, "file:///fixture.html")
        assert "--new-instance" in firefox and "file:///fixture.html" in firefox
        helper = helper_command(root)
        assert helper[0] == "konsole" and "--separate" in helper

    expect_error(
        lambda: find_fixture_windows({"type": "windows", "data": response["data"][:1]}),
        "Firefox target",
    )
    expect_error(
        lambda: find_fixture_windows(
            {"type": "windows", "data": [response["data"][1]]}
        ),
        "helper window",
    )
    print("test-nested-retained-capture-eval: ok")


if __name__ == "__main__":
    main()
