#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'USAGE'
Usage: scripts/gui-input-smoke.sh [text-editor]

Runs an opt-in local GUI smoke that sends real pointer and keyboard input to a
disposable KWrite/Kate document through plasma-pilotd with --allow-control.
USAGE
}

case_name="${1:-text-editor}"
if [[ "$case_name" == "--help" || "$case_name" == "-h" ]]; then
	usage
	exit 0
fi
if [[ "$case_name" != "text-editor" ]]; then
	usage >&2
	exit 2
fi

require_cmd() {
	if ! command -v "$1" >/dev/null 2>&1; then
		echo "$1 is required for GUI input smoke" >&2
		exit 1
	fi
}

require_cmd jq
require_cmd qdbus6

if command -v kwrite >/dev/null 2>&1; then
	editor=(kwrite)
	editor_name="KWrite"
elif command -v kate >/dev/null 2>&1; then
	editor=(kate --new --startanon)
	editor_name="Kate"
else
	echo "KWrite or Kate is required for GUI input smoke" >&2
	exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

run_dir="target/plasma-pilot-gui-input-smoke"
socket_dir="/tmp/plasma-pilot-gui-input-smoke"
socket="$socket_dir/plasma-pilotd.sock"
log="$run_dir/daemon.log"
journal="$run_dir/journal.jsonl"
windows_json="$run_dir/windows.json"
window_json="$run_dir/window.json"
active_json="$run_dir/active-window.json"
calibration_json="$run_dir/pointer-calibration.json"
uinput_json="$run_dir/uinput-status.json"
focus_json="$run_dir/focus.json"
click_json="$run_dir/click.json"
type_json="$run_dir/type.json"
save_json="$run_dir/save.json"
journal_tail_json="$run_dir/journal-tail.json"
stamp="$(date +%s)"
test_file="$run_dir/plasma-pilot-input-smoke-$stamp.txt"
sentinel="plasma-pilot-input-smoke-$stamp"
window_id=""
editor_pid=""
app_id=""

rm -rf "$run_dir" "$socket_dir"
mkdir -p "$run_dir"
printf '' >"$test_file"

cargo build -p plasma-pilotd -p plasma-pilot-cli

target/debug/plasma-pilotd --socket "$socket" --journal "$journal" --allow-control >"$log" 2>&1 &
daemon_pid=$!

cli() {
	target/debug/plasma-pilot-cli --socket "$socket" "$@"
}

guard_args=()

cleanup() {
	if [[ -n "${window_id:-}" && -S "$socket" ]]; then
		cli focus --window "$window_id" >/dev/null 2>&1 || true
		sleep 0.2
		if [[ ${#guard_args[@]} -gt 0 ]]; then
			cli input key-combo Alt+F4 "${guard_args[@]}" >/dev/null 2>&1 || true
		fi
	fi
	if [[ -n "${editor_pid:-}" ]]; then
		kill "$editor_pid" 2>/dev/null || true
		wait "$editor_pid" 2>/dev/null || true
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

cli input status >"$uinput_json"
jq -e '.type == "uinput_status" and .data.available == true' "$uinput_json" >/dev/null

"${editor[@]}" "$test_file" &
editor_pid=$!
basename="$(basename "$test_file")"

for _ in {1..100}; do
	cli windows >"$windows_json"
	if jq -e --arg basename "$basename" '
		[
			.data[]
			| select((.title // "") | contains($basename))
			| select(.geometry != null)
		][0]
	' "$windows_json" >"$window_json"; then
		break
	fi
	sleep 0.1
done
if [[ ! -s "$window_json" ]]; then
	echo "could not find $editor_name window for $basename" >&2
	cat "$windows_json" >&2
	exit 1
fi

window_id="$(jq -r '.id' "$window_json")"
app_id="$(jq -r '.app_id // empty' "$window_json")"
guard_args=(--expected-active-window "$window_id" --active-title-contains "$basename")
if [[ -n "$app_id" ]]; then
	guard_args+=(--expected-active-app "$app_id")
fi

cli focus --window "$window_id" >"$focus_json"
jq -e '.type == "action"' "$focus_json" >/dev/null

for _ in {1..50}; do
	if cli active-window >"$active_json" 2>/dev/null \
		&& jq -e --arg id "$window_id" --arg title "$basename" '
			.type == "active_window"
			and (.data.id == $id or ((.data.title // "") | contains($title)))
		' "$active_json" >/dev/null; then
		break
	fi
	sleep 0.1
done
if ! jq -e --arg id "$window_id" --arg title "$basename" '
	.type == "active_window"
	and (.data.id == $id or ((.data.title // "") | contains($title)))
' "$active_json" >/dev/null 2>&1; then
	echo "KWin active-window bridge did not report the test window; run make install-kwin-script and retry" >&2
	cat "$active_json" >&2 || true
	exit 1
fi

cli input pointer-calibration >"$calibration_json"
read -r click_x click_y < <(
	jq -r --slurpfile win "$window_json" '
		($win[0].geometry // error("test window has no geometry")) as $g
		| ($g.x + (($g.width * 0.50) | floor)) as $lx
		| ($g.y + (($g.height * 0.60) | floor)) as $ly
		| .data.monitors[]
		| select(
			$lx >= .logical_origin_x
			and $lx < (.logical_origin_x + .logical_width)
			and $ly >= .logical_origin_y
			and $ly < (.logical_origin_y + .logical_height)
		)
		| [
			(.physical_origin_x + (($lx - .logical_origin_x) * .scale_factor) | floor),
			(.physical_origin_y + (($ly - .logical_origin_y) * .scale_factor) | floor)
		]
		| @tsv
	' "$calibration_json" | head -n 1
)
if [[ -z "${click_x:-}" || -z "${click_y:-}" ]]; then
	echo "could not map test-window logical point to physical pointer coordinates" >&2
	cat "$window_json" >&2
	cat "$calibration_json" >&2
	exit 1
fi

cli input click-pointer \
	--x "$click_x" \
	--y "$click_y" \
	--coordinate-space physical-pixel \
	--button left \
	"${guard_args[@]}" >"$click_json"
jq -e '.type == "action"' "$click_json" >/dev/null
sleep 0.3

cli input type-text "$sentinel" "${guard_args[@]}" >"$type_json"
jq -e '.type == "action"' "$type_json" >/dev/null
sleep 0.5

cli input key-combo Ctrl+S "${guard_args[@]}" >"$save_json"
jq -e '.type == "action"' "$save_json" >/dev/null

for _ in {1..50}; do
	if grep -q "$sentinel" "$test_file"; then
		break
	fi
	sleep 0.1
done
if ! grep -q "$sentinel" "$test_file"; then
	echo "typed sentinel was not saved to $test_file" >&2
	cat "$test_file" >&2 || true
	exit 1
fi

cli journal tail --limit 40 >"$journal_tail_json"
grep -q "click_pointer" "$journal_tail_json"
grep -q "type_text" "$journal_tail_json"
grep -q "key_combo" "$journal_tail_json"

echo "GUI input smoke passed with $editor_name; artifacts are in $run_dir"
