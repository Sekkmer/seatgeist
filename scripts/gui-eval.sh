#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'USAGE'
Usage: scripts/gui-eval.sh [all|status|observe|clipboard-denied|screenshot-preview|control-safety]

Runs opt-in local GUI evals against a private PlasmaPilot daemon socket.
The default `all` set avoids control actions. `control-safety` starts a private
daemon with control approval, then verifies guard and panic-stop denials before
any backend control action can execute.
USAGE
}

case_name="${1:-all}"
if [[ "$case_name" == "--help" || "$case_name" == "-h" ]]; then
	usage
	exit 0
fi

case "$case_name" in
	all | status | observe | clipboard-denied | screenshot-preview | control-safety) ;;
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

rm -rf "$run_dir" "$socket_dir"
mkdir -p "$run_dir"

cargo build -p plasma-pilotd -p plasma-pilot-cli
daemon_args=(--socket "$socket" --journal "$journal" --panic-stop-file "$panic_stop_file")
if [[ "$case_name" == "control-safety" ]]; then
	daemon_args+=(--allow-control)
fi
target/debug/plasma-pilotd "${daemon_args[@]}" >"$log" 2>&1 &
pid=$!

cleanup() {
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
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

eval_control_safety() {
	cli panic-stop status >"$run_dir/panic-stop-initial.json"
	jq -e '.type == "panic_stop" and .data.enabled == false' "$run_dir/panic-stop-initial.json" >/dev/null

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
		control-safety) eval_control_safety ;;
	esac
}

if [[ "$case_name" == "all" ]]; then
	for eval_name in status observe clipboard-denied screenshot-preview; do
		run_case "$eval_name"
	done
else
	run_case "$case_name"
fi

cli journal tail --limit 20 >"$run_dir/journal-tail.json"
jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/journal-tail.json" >/dev/null
echo "GUI eval $case_name passed; artifacts are in $run_dir"
