#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'USAGE'
Usage: scripts/gui-eval.sh [all|status|observe|clipboard-denied|screenshot-preview|screenshot-coordinate-map|screenshot-config-bounds|portal-screenshot|remote-desktop-probe|remote-desktop-eis-session|full-resolution-denied|control-safety]

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
	all | status | observe | clipboard-denied | screenshot-preview | screenshot-coordinate-map | screenshot-config-bounds | portal-screenshot | remote-desktop-probe | remote-desktop-eis-session | full-resolution-denied | control-safety) ;;
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
if [[ "$case_name" == "control-safety" || "$case_name" == "remote-desktop-probe" || "$case_name" == "remote-desktop-eis-session" ]]; then
	daemon_args+=(--approval-file "$approval_file")
fi
if [[ "$case_name" == "remote-desktop-eis-session" ]]; then
	daemon_args+=(--input-backend portal_remote_desktop)
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

eval_portal_screenshot() {
	cli capture-backends >"$run_dir/portal-capture-backends.json"
	if ! jq -e '.type == "capture_backend_status" and .data.screenshot_portal.screenshot_interface_available == true' "$run_dir/portal-capture-backends.json" >/dev/null; then
		echo "SKIP portal-screenshot: xdg-desktop-portal Screenshot interface is not visible"
		return 0
	fi
	if ! jq -e '.data.implemented_available_backend == "portal_screenshot"' "$run_dir/portal-capture-backends.json" >/dev/null; then
		echo "portal Screenshot is visible but not selected as implemented backend" >&2
		cat "$run_dir/portal-capture-backends.json" >&2
		exit 1
	fi

	if ! cli screenshot --output "$run_dir/portal-screenshot.png" >"$run_dir/portal-screenshot.json" 2>"$run_dir/portal-screenshot.err"; then
		if grep -qi "portal screenshot request was cancelled or ended without a screenshot" "$run_dir/portal-screenshot.err" && [[ "${PLASMA_PILOT_PORTAL_SCREENSHOT_STRICT:-0}" != "1" ]]; then
			echo "SKIP portal-screenshot: portal cancelled or ended the request without a screenshot"
			return 0
		fi
		cat "$run_dir/portal-screenshot.err" >&2
		exit 1
	fi
	test -s "$run_dir/portal-screenshot.png"
	jq -e '
		.type == "screenshot"
		and .data.backend == "portal_screenshot"
		and .data.source_width >= .data.output_width
		and .data.source_height >= .data.output_height
		and .data.output_width <= 1600
		and .data.output_height <= 1600
		and .data.transform.scale_x > 0
		and .data.transform.scale_y > 0
	' "$run_dir/portal-screenshot.json" >/dev/null
	cli journal tail --limit 20 --method screenshot --ok true >"$run_dir/portal-screenshot-journal.json"
	jq -e '.type == "journal" and any(.data[]; .summary | contains("backend=portal_screenshot"))' "$run_dir/portal-screenshot-journal.json" >/dev/null
}

eval_remote_desktop_probe() {
	cli input backends >"$run_dir/remote-desktop-backends.json"
	if ! jq -e '.type == "input_backend_status" and .data.remote_desktop_portal.remote_desktop_interface_available == true' "$run_dir/remote-desktop-backends.json" >/dev/null; then
		echo "SKIP remote-desktop-probe: xdg-desktop-portal RemoteDesktop interface is not visible"
		return 0
	fi

	cli active-window >"$run_dir/remote-desktop-active-window.json" 2>"$run_dir/remote-desktop-active-window.err" || true
	active_title="$(jq -r '.data.title // empty' "$run_dir/remote-desktop-active-window.json" 2>/dev/null || true)"
	active_id="$(jq -r '.data.id // empty' "$run_dir/remote-desktop-active-window.json" 2>/dev/null || true)"
	if [[ -z "$active_title" && -z "$active_id" ]]; then
		echo "SKIP remote-desktop-probe: active-window guard metadata is unavailable"
		return 0
	fi

	guard_args=()
	if [[ -n "$active_title" ]]; then
		guard_args+=(--active-title-contains "$active_title")
	else
		guard_args+=(--expected-active-window "$active_id")
	fi

	cli approve \
		--approval-file "$approval_file" \
		--safety-class control-pointer \
		--method remote_desktop_session_probe \
		--ttl-ms 120000 \
		--reason "gui-eval remote-desktop-probe" >"$run_dir/remote-desktop-approval.json"
	jq -e '.method == "remote_desktop_session_probe" and .safety_class == "control_pointer"' "$run_dir/remote-desktop-approval.json" >/dev/null
	test "$(stat -c '%a' "$approval_file")" = "600"

	if ! cli input remote-desktop-probe --keyboard --pointer --timeout-ms 120000 "${guard_args[@]}" >"$run_dir/remote-desktop-probe.json" 2>"$run_dir/remote-desktop-probe.err"; then
		cat "$run_dir/remote-desktop-probe.err" >&2
		exit 1
	fi
	jq -e '
		.type == "remote_desktop_session_probe"
		and (.data.requested_devices | index("keyboard"))
		and (.data.requested_devices | index("pointer"))
		and .data.transient_session_closed == true
	' "$run_dir/remote-desktop-probe.json" >/dev/null
	if [[ "${PLASMA_PILOT_REMOTE_DESKTOP_STRICT:-0}" == "1" ]]; then
		jq -e '.data.started == true and (.data.selected_devices | length) >= 1' "$run_dir/remote-desktop-probe.json" >/dev/null
	fi
	cli journal tail --limit 20 --method remote_desktop_session_probe --ok true >"$run_dir/remote-desktop-journal.json"
	jq -e '.type == "journal" and any(.data[]; .summary | contains("remote desktop session probe"))' "$run_dir/remote-desktop-journal.json" >/dev/null
}

eval_remote_desktop_eis_session() {
	cli input backends >"$run_dir/remote-desktop-eis-backends-before.json"
	if ! jq -e '.type == "input_backend_status" and .data.remote_desktop_portal.remote_desktop_interface_available == true' "$run_dir/remote-desktop-eis-backends-before.json" >/dev/null; then
		echo "SKIP remote-desktop-eis-session: xdg-desktop-portal RemoteDesktop interface is not visible"
		return 0
	fi

	cli active-window >"$run_dir/remote-desktop-eis-active-window.json" 2>"$run_dir/remote-desktop-eis-active-window.err" || true
	active_title="$(jq -r '.data.title // empty' "$run_dir/remote-desktop-eis-active-window.json" 2>/dev/null || true)"
	active_id="$(jq -r '.data.id // empty' "$run_dir/remote-desktop-eis-active-window.json" 2>/dev/null || true)"
	if [[ -z "$active_title" && -z "$active_id" ]]; then
		echo "SKIP remote-desktop-eis-session: active-window guard metadata is unavailable"
		return 0
	fi

	guard_args=()
	if [[ -n "$active_title" ]]; then
		guard_args+=(--active-title-contains "$active_title")
	else
		guard_args+=(--expected-active-window "$active_id")
	fi

	cli approve \
		--approval-file "$approval_file" \
		--safety-class control-pointer \
		--method remote_desktop_eis_start \
		--ttl-ms 120000 \
		--reason "gui-eval remote-desktop-eis-session start" >"$run_dir/remote-desktop-eis-start-approval.json"
	jq -e '.method == "remote_desktop_eis_start" and .safety_class == "control_pointer"' "$run_dir/remote-desktop-eis-start-approval.json" >/dev/null
	cli approve \
		--approval-file "$approval_file" \
		--safety-class control-pointer \
		--method scroll_pointer \
		--ttl-ms 120000 \
		--reason "gui-eval remote-desktop-eis-session minimal input" >"$run_dir/remote-desktop-eis-scroll-approval.json"
	jq -e '.method == "scroll_pointer" and .safety_class == "control_pointer"' "$run_dir/remote-desktop-eis-scroll-approval.json" >/dev/null
	cli approve \
		--approval-file "$approval_file" \
		--safety-class control-keyboard \
		--method key_combo \
		--ttl-ms 120000 \
		--reason "gui-eval remote-desktop-eis-session keyboard input" >"$run_dir/remote-desktop-eis-key-combo-approval.json"
	jq -e '.method == "key_combo" and .safety_class == "control_keyboard"' "$run_dir/remote-desktop-eis-key-combo-approval.json" >/dev/null
	test "$(stat -c '%a' "$approval_file")" = "600"

	if ! cli input remote-desktop-eis-start --keyboard --pointer --timeout-ms 120000 "${guard_args[@]}" >"$run_dir/remote-desktop-eis-start.json" 2>"$run_dir/remote-desktop-eis-start.err"; then
		cat "$run_dir/remote-desktop-eis-start.err" >&2
		exit 1
	fi
	jq -e '.type == "remote_desktop_eis_session_status"' "$run_dir/remote-desktop-eis-start.json" >/dev/null
	if ! jq -e '.data.active == true' "$run_dir/remote-desktop-eis-start.json" >/dev/null; then
		if [[ "${PLASMA_PILOT_REMOTE_DESKTOP_EIS_STRICT:-0}" == "1" ]]; then
			echo "remote-desktop-eis-session did not start in strict mode" >&2
			cat "$run_dir/remote-desktop-eis-start.json" >&2
			exit 1
		fi
		echo "SKIP remote-desktop-eis-session: portal cancelled or ended before a stored EIS session was active"
		cli input remote-desktop-eis-stop >"$run_dir/remote-desktop-eis-stop-after-cancel.json"
		return 0
	fi

	cli input remote-desktop-eis-session-status >"$run_dir/remote-desktop-eis-status.json"
	jq -e '
		.type == "remote_desktop_eis_session_status"
		and .data.active == true
		and (.data.selected_devices | index("keyboard"))
		and (.data.selected_devices | index("pointer"))
	' "$run_dir/remote-desktop-eis-status.json" >/dev/null
	cli input backends >"$run_dir/remote-desktop-eis-backends-active.json"
	jq -e '
		.type == "input_backend_status"
		and .data.configured_backend == "portal_remote_desktop"
		and .data.implemented_available_backend == "portal_remote_desktop"
	' "$run_dir/remote-desktop-eis-backends-active.json" >/dev/null

	scroll_ok=0
	if cli input scroll-pointer --vertical 1 "${guard_args[@]}" >"$run_dir/remote-desktop-eis-scroll.json" 2>"$run_dir/remote-desktop-eis-scroll.err"; then
		scroll_ok=1
		jq -e '.type == "action" and (.data.message | contains("backend=portal_remote_desktop"))' "$run_dir/remote-desktop-eis-scroll.json" >/dev/null
	else
		if [[ "${PLASMA_PILOT_REMOTE_DESKTOP_EIS_INPUT_STRICT:-0}" == "1" ]]; then
			cat "$run_dir/remote-desktop-eis-scroll.err" >&2
			cli input remote-desktop-eis-stop >"$run_dir/remote-desktop-eis-stop-after-scroll-failure.json" || true
			exit 1
		fi
		if ! grep -Eiq 'EIS|readiness|resumed|capabilit|selected device|connected session' "$run_dir/remote-desktop-eis-scroll.err"; then
			cat "$run_dir/remote-desktop-eis-scroll.err" >&2
			cli input remote-desktop-eis-stop >"$run_dir/remote-desktop-eis-stop-after-unexpected-scroll-failure.json" || true
			exit 1
		fi
	fi
	key_combo_ok=0
	if cli input key-combo Shift "${guard_args[@]}" >"$run_dir/remote-desktop-eis-key-combo.json" 2>"$run_dir/remote-desktop-eis-key-combo.err"; then
		key_combo_ok=1
		jq -e '.type == "action" and (.data.message | contains("backend=portal_remote_desktop"))' "$run_dir/remote-desktop-eis-key-combo.json" >/dev/null
	else
		if [[ "${PLASMA_PILOT_REMOTE_DESKTOP_EIS_INPUT_STRICT:-0}" == "1" ]]; then
			cat "$run_dir/remote-desktop-eis-key-combo.err" >&2
			cli input remote-desktop-eis-stop >"$run_dir/remote-desktop-eis-stop-after-key-combo-failure.json" || true
			exit 1
		fi
		if ! grep -Eiq 'EIS|readiness|resumed|capabilit|selected device|connected session' "$run_dir/remote-desktop-eis-key-combo.err"; then
			cat "$run_dir/remote-desktop-eis-key-combo.err" >&2
			cli input remote-desktop-eis-stop >"$run_dir/remote-desktop-eis-stop-after-unexpected-key-combo-failure.json" || true
			exit 1
		fi
	fi

	cli input remote-desktop-eis-stop >"$run_dir/remote-desktop-eis-stop.json"
	jq -e '.type == "remote_desktop_eis_session_status" and .data.active == false' "$run_dir/remote-desktop-eis-stop.json" >/dev/null
	cli journal tail --limit 40 --method remote_desktop_eis_start --ok true >"$run_dir/remote-desktop-eis-start-journal.json"
	jq -e '.type == "journal" and any(.data[]; .summary | contains("remote desktop EIS session"))' "$run_dir/remote-desktop-eis-start-journal.json" >/dev/null
	cli journal tail --limit 40 --method remote_desktop_eis_stop --ok true >"$run_dir/remote-desktop-eis-stop-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/remote-desktop-eis-stop-journal.json" >/dev/null
	if [[ "$scroll_ok" == "1" ]]; then
		cli journal tail --limit 40 --method scroll_pointer --ok true >"$run_dir/remote-desktop-eis-scroll-journal.json"
		jq -e '.type == "journal" and any(.data[]; .summary | contains("backend=portal_remote_desktop"))' "$run_dir/remote-desktop-eis-scroll-journal.json" >/dev/null
	else
		cli journal tail --limit 40 --method scroll_pointer --ok false >"$run_dir/remote-desktop-eis-scroll-journal.json"
		jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/remote-desktop-eis-scroll-journal.json" >/dev/null
	fi
	if [[ "$key_combo_ok" == "1" ]]; then
		cli journal tail --limit 40 --method key_combo --ok true >"$run_dir/remote-desktop-eis-key-combo-journal.json"
		jq -e '.type == "journal" and any(.data[]; .summary | contains("backend=portal_remote_desktop"))' "$run_dir/remote-desktop-eis-key-combo-journal.json" >/dev/null
	else
		cli journal tail --limit 40 --method key_combo --ok false >"$run_dir/remote-desktop-eis-key-combo-journal.json"
		jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/remote-desktop-eis-key-combo-journal.json" >/dev/null
	fi
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
		portal-screenshot) eval_portal_screenshot ;;
		remote-desktop-probe) eval_remote_desktop_probe ;;
		remote-desktop-eis-session) eval_remote_desktop_eis_session ;;
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
