#!/usr/bin/bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
marker="$XDG_STATE_HOME/nested-eis-input-received"
rm -f "$marker"

konsole --hold -e bash -c 'IFS= read -rsn1 _; touch "$1"' bash "$marker" &
fixture_pid=$!
cleanup() {
	kill "$fixture_pid" 2>/dev/null || true
	wait "$fixture_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in {1..50}; do
	if qdbus6 org.seatgeist.KWinBridge /org/seatgeist/KWinBridge1 \
		org.seatgeist.KWinBridge1.GetWindows 2>/dev/null | grep -q 'org.kde.konsole'; then
		break
	fi
	sleep 0.1
done

SEATGEIST_REMOTE_DESKTOP_EIS_STRICT=1 \
	SEATGEIST_REMOTE_DESKTOP_EIS_INPUT_STRICT=1 \
	SEATGEIST_REMOTE_DESKTOP_EIS_PAUSE_AFTER_START=1 \
	SEATGEIST_REMOTE_DESKTOP_EIS_SKIP_SCROLL=1 \
	SEATGEIST_REMOTE_DESKTOP_EIS_KEY_COMBO=F12 \
	SEATGEIST_GUI_EVAL_SKIP_BUILD=1 \
	"$root/scripts/gui-eval.sh" remote-desktop-eis-session

test -f "$marker"
echo "nested-eis-isolation-fixture: nested Konsole received F12"
