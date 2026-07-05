#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'USAGE'
Usage: scripts/gui-eval.sh [all|status|observe|clipboard-denied|screenshot-preview|screenshot-coordinate-map|screenshot-config-bounds|full-resolution-denied|control-safety]

Runs opt-in local GUI evals against a private PlasmaPilot daemon socket.
The default `all` set avoids control actions. `control-safety` starts a private
daemon with a method-scoped approval grant, then verifies guard and panic-stop
denials before any backend control action can execute.
USAGE
}

case_name="${1:-all}"
if [[ "$case_name" == "--help" || "$case_name" == "-h" ]]; then
	usage
	exit 0
fi

case "$case_name" in
	all | status | observe | clipboard-denied | screenshot-preview | screenshot-coordinate-map | screenshot-config-bounds | full-resolution-denied | control-safety) ;;
	*)
		usage >&2
		exit 2
		;;
esac

if ! command -v jq >/dev/null 2>&1; then
	echo "jq is required for GUI eval response validation" >&2
	exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

run_dir="target/plasma-pilot-gui-eval"
socket_dir="/tmp/plasma-pilot-gui-eval"
socket="$socket_dir/plasma-pilotd.sock"
log="$run_dir/daemon.log"
journal="$run_dir/journal.jsonl"
panic_stop_file="$run_dir/panic-stop.flag"
approval_file="$run_dir/approvals.jsonl"
config_file="$run_dir/config.toml"
pid=""

emit_failure_diagnostics() {
	local status="$1"
	echo "GUI eval $case_name failed with status $status" >&2
	echo "Artifacts: $run_dir" >&2
	if [[ -f "$log" ]]; then
		echo "--- daemon log tail ---" >&2
		tail -80 "$log" >&2 || true
	fi
	if [[ -f "$journal" ]]; then
		echo "--- journal tail ---" >&2
		tail -20 "$journal" >&2 || true
	fi
	if [[ -d "$run_dir" ]]; then
		echo "--- artifact files ---" >&2
		find "$run_dir" -maxdepth 1 -type f -printf '%f\n' | sort >&2 || true
	fi
}

cleanup() {
	if [[ -n "$pid" ]]; then
		kill "$pid" 2>/dev/null || true
		wait "$pid" 2>/dev/null || true
	fi
}

on_exit() {
	local status="$?"
	if [[ "$status" -ne 0 ]]; then
		emit_failure_diagnostics "$status"
	fi
	cleanup
	exit "$status"
}
trap on_exit EXIT

rm -rf "$run_dir" "$socket_dir"
mkdir -p "$run_dir"
chmod 700 "$run_dir"

cargo build -p plasma-pilotd -p plasma-pilot-cli
if [[ "$case_name" == "all" || "$case_name" == "screenshot-config-bounds" ]]; then
	cat >"$config_file" <<CONFIG
[daemon]
socket = "$socket"
journal = "$journal"
panic_stop_file = "$panic_stop_file"

[safety]
preview_max_edge = 800
tile_max_edge = 640
CONFIG
	daemon_args=(--config "$config_file")
else
	daemon_args=(--socket "$socket" --journal "$journal" --panic-stop-file "$panic_stop_file")
fi
if [[ "$case_name" == "control-safety" ]]; then
	daemon_args+=(--approval-file "$approval_file")
fi
target/debug/plasma-pilotd "${daemon_args[@]}" >"$log" 2>&1 &
pid=$!

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

cli() {
	target/debug/plasma-pilot-cli --socket "$socket" "$@"
}

eval_status() {
	cli doctor >"$run_dir/doctor.json"
	jq -e '.type == "health" and .data.status == "ok"' "$run_dir/doctor.json" >/dev/null
	cli capabilities >"$run_dir/capabilities.json"
	jq -e '.type == "capabilities"' "$run_dir/capabilities.json" >/dev/null
	cli policy-status >"$run_dir/policy-status.json"
	jq -e '.type == "policy_status" and .data.default_control == "prompt"' "$run_dir/policy-status.json" >/dev/null
}

eval_observe() {
	cli observe >"$run_dir/observe.json"
	jq -e '.type == "observation" and (.data.monitors | type == "array") and (.data.windows | type == "array")' "$run_dir/observe.json" >/dev/null
}

eval_clipboard_denied() {
	if cli clipboard get >"$run_dir/clipboard-denied.txt" 2>&1; then
		echo "clipboard get unexpectedly succeeded without clipboard-read approval" >&2
		exit 1
	fi
	grep -qi "policy" "$run_dir/clipboard-denied.txt"
}

eval_screenshot_preview() {
	if ! command -v spectacle >/dev/null 2>&1; then
		echo "SKIP screenshot-preview: spectacle is not available"
		return 0
	fi
	cli screenshot --output "$run_dir/preview.png" >"$run_dir/screenshot-preview.json"
	jq -e '
		.type == "screenshot"
		and .data.source_width >= .data.output_width
		and .data.source_height >= .data.output_height
		and .data.output_width <= 1600
		and .data.output_height <= 1600
		and .data.transform.scale_x > 0
		and .data.transform.scale_y > 0
	' "$run_dir/screenshot-preview.json" >/dev/null
}

eval_screenshot_coordinate_map() {
	if ! command -v spectacle >/dev/null 2>&1; then
		echo "SKIP screenshot-coordinate-map: spectacle is not available"
		return 0
	fi
	cli screenshot --output "$run_dir/coordinate-map.png" >"$run_dir/screenshot-coordinate-map.json"
	jq -e '
		.type == "screenshot"
		and .data.output_width > 0
		and .data.output_height > 0
		and .data.source_width >= .data.output_width
		and .data.source_height >= .data.output_height
		and .data.transform.source_coordinate_space == "physical_pixel"
		and .data.transform.output_coordinate_space == "physical_pixel"
		and .data.transform.scale_x > 0
		and .data.transform.scale_y > 0
		and (
			.data.transform.source_origin_x
			+ ((.data.output_width / 2) / .data.transform.scale_x)
		) >= .data.transform.source_origin_x
		and (
			.data.transform.source_origin_x
			+ ((.data.output_width / 2) / .data.transform.scale_x)
		) <= (.data.transform.source_origin_x + .data.source_width)
		and (
			.data.transform.source_origin_y
			+ ((.data.output_height / 2) / .data.transform.scale_y)
		) >= .data.transform.source_origin_y
		and (
			.data.transform.source_origin_y
			+ ((.data.output_height / 2) / .data.transform.scale_y)
		) <= (.data.transform.source_origin_y + .data.source_height)
	' "$run_dir/screenshot-coordinate-map.json" >/dev/null
}

eval_screenshot_config_bounds() {
	if ! command -v spectacle >/dev/null 2>&1; then
		echo "SKIP screenshot-config-bounds: spectacle is not available"
		return 0
	fi
	cli safety-status >"$run_dir/screenshot-config-safety.json"
	jq -e '
		.type == "safety_status"
		and .data.preview_max_edge == 800
		and .data.tile_max_edge == 640
	' "$run_dir/screenshot-config-safety.json" >/dev/null

	cli screenshot --output "$run_dir/config-preview.png" >"$run_dir/screenshot-config-preview.json"
	jq -e '
		.type == "screenshot"
		and .data.source_width >= .data.output_width
		and .data.source_height >= .data.output_height
		and .data.output_width <= 800
		and .data.output_height <= 800
	' "$run_dir/screenshot-config-preview.json" >/dev/null

	source_width="$(jq -r '.data.source_width' "$run_dir/screenshot-config-preview.json")"
	source_height="$(jq -r '.data.source_height' "$run_dir/screenshot-config-preview.json")"
	tile_width="$source_width"
	tile_height="$source_height"
	if (( tile_width > 1200 )); then
		tile_width=1200
	fi
	if (( tile_height > 1000 )); then
		tile_height=1000
	fi
	cli screenshot-tile \
		--output "$run_dir/config-tile.png" \
		--x 0 \
		--y 0 \
		--width "$tile_width" \
		--height "$tile_height" >"$run_dir/screenshot-config-tile.json"
	jq -e '
		.type == "screenshot"
		and .data.output_width <= 640
		and .data.output_height <= 640
		and .data.transform.source_origin_x == 0
		and .data.transform.source_origin_y == 0
	' "$run_dir/screenshot-config-tile.json" >/dev/null
}

eval_full_resolution_denied() {
	if cli screenshot --output "$run_dir/full-resolution-denied.png" --full-resolution >"$run_dir/full-resolution-denied.txt" 2>&1; then
		echo "full-resolution screenshot unexpectedly succeeded without explicit approval" >&2
		exit 1
	fi
	grep -qi "FullResolutionScreenshot" "$run_dir/full-resolution-denied.txt"
	if [[ -e "$run_dir/full-resolution-denied.png" ]]; then
		echo "full-resolution screenshot wrote output despite policy denial" >&2
		exit 1
	fi
}

eval_control_safety() {
	cli panic-stop status >"$run_dir/panic-stop-initial.json"
	jq -e '.type == "panic_stop" and .data.enabled == false' "$run_dir/panic-stop-initial.json" >/dev/null
	cli approve \
		--approval-file "$approval_file" \
		--safety-class control-semantic \
		--method focus_window \
		--ttl-ms 60000 \
		--reason "gui-eval control-safety" >"$run_dir/control-safety-approval.json"
	jq -e '.method == "focus_window" and .safety_class == "control_semantic"' "$run_dir/control-safety-approval.json" >/dev/null
	test "$(stat -c '%a' "$approval_file")" = "600"

	if command -v qdbus6 >/dev/null 2>&1; then
		for _ in {1..50}; do
			if qdbus6 org.plasmapilot.KWinBridge /org/plasmapilot/KWinBridge1 org.plasmapilot.KWinBridge1.UpdateActiveWindow '{"active":true,"id":"plasma-pilot-eval-window","title":"PlasmaPilot Eval Window","app_id":"org.plasmapilot.eval","geometry":{"x":0,"y":0,"width":100,"height":100}}' >/dev/null 2>&1; then
				break
			fi
			sleep 0.1
		done
	fi

	if cli active-window >"$run_dir/control-safety-active-window.json" 2>/dev/null; then
		if cli focus --window "__plasma_pilot_eval_never__" --expected-active-window "__plasma_pilot_wrong_window__" >"$run_dir/guard-denied.txt" 2>&1; then
			echo "focus unexpectedly passed with an incorrect active-window guard" >&2
			exit 1
		fi
		grep -q "active-window guard failed" "$run_dir/guard-denied.txt"
	else
		echo "SKIP active-window guard denial: KWin active-window bridge has not reported yet"
	fi

	cli panic-stop enable >"$run_dir/panic-stop-enabled.json"
	jq -e '.type == "panic_stop" and .data.enabled == true' "$run_dir/panic-stop-enabled.json" >/dev/null
	if cli focus --window "__plasma_pilot_eval_never__" >"$run_dir/panic-stop-denied.txt" 2>&1; then
		echo "focus unexpectedly passed while panic-stop was active" >&2
		exit 1
	fi
	grep -q "panic-stop is active" "$run_dir/panic-stop-denied.txt"
	cli panic-stop disable >"$run_dir/panic-stop-disabled.json"
	jq -e '.type == "panic_stop" and .data.enabled == false' "$run_dir/panic-stop-disabled.json" >/dev/null
	cli journal tail --limit 20 --method focus_window --ok false >"$run_dir/control-safety-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/control-safety-journal.json" >/dev/null
}

run_case() {
	case "$1" in
		status) eval_status ;;
		observe) eval_observe ;;
		clipboard-denied) eval_clipboard_denied ;;
		screenshot-preview) eval_screenshot_preview ;;
		screenshot-coordinate-map) eval_screenshot_coordinate_map ;;
		screenshot-config-bounds) eval_screenshot_config_bounds ;;
		full-resolution-denied) eval_full_resolution_denied ;;
		control-safety) eval_control_safety ;;
	esac
}

if [[ "$case_name" == "all" ]]; then
	for eval_name in status observe clipboard-denied screenshot-preview screenshot-coordinate-map screenshot-config-bounds full-resolution-denied; do
		run_case "$eval_name"
	done
else
	run_case "$case_name"
fi

cli journal tail --limit 20 >"$run_dir/journal-tail.json"
jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/journal-tail.json" >/dev/null
echo "GUI eval $case_name passed; artifacts are in $run_dir"
