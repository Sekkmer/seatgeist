#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'USAGE'
Usage: scripts/gui-calculator-smoke.sh [kcalc]

Runs an opt-in local GUI smoke that sends real keyboard input to KCalc through
seatgeistd with short-lived approval-file grants, then captures a visual
artifact for human inspection when screenshot capture is available.
USAGE
}

case_name="${1:-kcalc}"
if [[ "$case_name" == "--help" || "$case_name" == "-h" ]]; then
	usage
	exit 0
fi
if [[ "$case_name" != "kcalc" ]]; then
	usage >&2
	exit 2
fi

require_cmd() {
	if ! command -v "$1" >/dev/null 2>&1; then
		echo "$1 is required for KCalc GUI smoke" >&2
		exit 1
	fi
}

require_cmd jq
require_cmd kcalc

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

run_dir="target/seatgeist-gui-calculator-smoke"
socket_dir="/tmp/seatgeist-gui-calculator-smoke"
socket="$socket_dir/seatgeistd.sock"
log="$run_dir/daemon.log"
journal="$run_dir/journal.jsonl"
approval_file="$run_dir/approvals.jsonl"
windows_json="$run_dir/windows.json"
window_json="$run_dir/window.json"
active_json="$run_dir/active-window.json"
uinput_json="$run_dir/uinput-status.json"
focus_json="$run_dir/focus.json"
clear_json="$run_dir/clear.json"
type_json="$run_dir/type.json"
screenshot_json="$run_dir/screenshot.json"
screenshot_err="$run_dir/screenshot.err"
screenshot_png="$run_dir/kcalc-2-plus-2.png"
journal_tail_json="$run_dir/journal-tail.json"
window_id=""
kcalc_pid=""
app_id=""

rm -rf "$run_dir" "$socket_dir"
mkdir -p "$run_dir"
chmod 700 "$run_dir"

cargo build -p seatgeistd -p seatgeist-cli

target/debug/seatgeistd --socket "$socket" --journal "$journal" --approval-file "$approval_file" >"$log" 2>&1 &
daemon_pid=$!

cli() {
	target/debug/seatgeist-cli --socket "$socket" "$@"
}

guard_args=()

cleanup() {
	if [[ -n "${window_id:-}" && -S "$socket" && ${#guard_args[@]} -gt 0 ]]; then
		cli input key-combo Alt+F4 "${guard_args[@]}" >/dev/null 2>&1 || true
	fi
	if [[ -n "${kcalc_pid:-}" ]]; then
		kill "$kcalc_pid" 2>/dev/null || true
		wait "$kcalc_pid" 2>/dev/null || true
	fi
	kill "$daemon_pid" 2>/dev/null || true
	wait "$daemon_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in {1..50}; do
	if [[ -S "$socket" ]]; then
		break
	fi
	sleep 0.1
done
if [[ ! -S "$socket" ]]; then
	cat "$log" >&2
	exit 1
fi

grant_approval() {
	local safety_class="$1"
	local method="$2"
	cli approve \
		--approval-file "$approval_file" \
		--safety-class "$safety_class" \
		--method "$method" \
		--ttl-ms 300000 \
		--reason "gui-calculator-smoke $method" >"$run_dir/approval-$method.json"
	jq -e --arg method "$method" '.method == $method' "$run_dir/approval-$method.json" >/dev/null
}

grant_approval control-semantic focus_window
grant_approval control-keyboard type_text
grant_approval control-keyboard key_combo
test "$(stat -c '%a' "$approval_file")" = "600"

cli input status >"$uinput_json"
jq -e '.type == "uinput_status" and .data.available == true' "$uinput_json" >/dev/null

kcalc &
kcalc_pid=$!

for _ in {1..100}; do
	cli windows >"$windows_json"
	if jq -e '
		[
			.data[]
			| select((.app_id // "") == "org.kde.kcalc" or ((.title // "") | contains("KCalc")))
			| select(.geometry != null)
		][0]
	' "$windows_json" >"$window_json"; then
		break
	fi
	sleep 0.1
done
if [[ ! -s "$window_json" ]]; then
	echo "could not find KCalc window" >&2
	cat "$windows_json" >&2
	exit 1
fi

window_id="$(jq -r '.id' "$window_json")"
app_id="$(jq -r '.app_id // empty' "$window_json")"
guard_args=(--expected-active-window "$window_id" --active-title-contains "KCalc")
if [[ -n "$app_id" ]]; then
	guard_args+=(--expected-active-app "$app_id")
fi

for _ in {1..50}; do
	if cli active-window >"$active_json" 2>/dev/null \
		&& jq -e --arg id "$window_id" '
			.type == "active_window"
			and (.data.id == $id or (.data.app_id // "") == "org.kde.kcalc")
			and ((.data.title // "") | contains("KCalc"))
		' "$active_json" >/dev/null; then
		break
	fi
	sleep 0.1
done
if ! jq -e --arg id "$window_id" '
	.type == "active_window"
	and (.data.id == $id or (.data.app_id // "") == "org.kde.kcalc")
	and ((.data.title // "") | contains("KCalc"))
' "$active_json" >/dev/null 2>&1; then
	echo "KWin active-window bridge did not report KCalc as active; click or focus KCalc, run make install-kwin-script if needed, and retry" >&2
	cat "$active_json" >&2 || true
	exit 1
fi

cli focus --window "$window_id" "${guard_args[@]}" >"$focus_json"
jq -e '.type == "action"' "$focus_json" >/dev/null

cli input key-combo Escape "${guard_args[@]}" >"$clear_json"
jq -e '.type == "action"' "$clear_json" >/dev/null
sleep 0.2

expression_chunks=("2" "+" "2" "=")
chunk_index=0
for chunk in "${expression_chunks[@]}"; do
	chunk_index=$((chunk_index + 1))
	chunk_json="$run_dir/type-$chunk_index.json"
	if [[ "$chunk_index" == "1" ]]; then
		chunk_json="$type_json"
	fi
	cli input type-text "$chunk" "${guard_args[@]}" >"$chunk_json"
	jq -e '.type == "action"' "$chunk_json" >/dev/null
	sleep 0.4
done
sleep 0.8

cli journal tail --limit 40 >"$journal_tail_json"
grep -q "focus_window" "$journal_tail_json"
grep -q "type_text" "$journal_tail_json"
grep -q "key_combo" "$journal_tail_json"

if command -v spectacle >/dev/null 2>&1; then
	if spectacle -b -n --activewindow -o "$screenshot_png" >/dev/null 2>"$screenshot_err"; then
		if [[ -s "$screenshot_png" ]]; then
			printf '{"type":"visual_artifact","data":{"source":"spectacle_active_window","output":"%s"}}\n' "$screenshot_png" >"$screenshot_json"
		elif [[ "${SEATGEIST_KCALC_SCREENSHOT_STRICT:-0}" == "1" ]]; then
			echo "KCalc Spectacle active-window capture wrote no output" >&2
			exit 1
		else
			echo "SKIP KCalc screenshot artifact: Spectacle wrote no output"
		fi
	elif [[ "${SEATGEIST_KCALC_SCREENSHOT_STRICT:-0}" == "1" ]]; then
		cat "$screenshot_err" >&2
		exit 1
	else
		echo "SKIP KCalc screenshot artifact: Spectacle active-window capture failed"
	fi
elif [[ "${SEATGEIST_KCALC_SCREENSHOT_STRICT:-0}" == "1" ]]; then
	echo "Spectacle is required for strict KCalc screenshot artifacts" >&2
	exit 1
else
	echo "SKIP KCalc screenshot artifact: Spectacle is not available"
fi

if [[ -s "$screenshot_png" ]]; then
	scripts/write-eval-evidence.py --run-dir "$run_dir" --case "kcalc-visual" --kind "visual"
	echo "KCalc GUI smoke passed; visual artifact is $screenshot_png"
else
	scripts/write-eval-evidence.py --run-dir "$run_dir" --case "kcalc-visual" --kind "visual"
	echo "KCalc GUI smoke passed; screenshot artifact was skipped"
fi
