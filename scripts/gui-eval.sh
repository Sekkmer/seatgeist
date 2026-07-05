#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'USAGE'
Usage: scripts/gui-eval.sh [all|status|session-preflight|observe|a11y-quality-status|a11y-focused-tree|a11y-find|a11y-text-attributes|a11y-control-denied|semantic-denied|input-denied|clipboard-status|clipboard-denied|kwin-bridge-status|keymap-status|screenshot-preview|screenshot-coordinate-map|screenshot-config-bounds|journal-artifacts|portal-screenshot|remote-desktop-probe|remote-desktop-eis-session|full-resolution-denied|control-safety]

Runs opt-in local GUI evals against a private Seatgeist daemon socket.
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
	all | status | session-preflight | observe | a11y-quality-status | a11y-focused-tree | a11y-find | a11y-text-attributes | a11y-control-denied | semantic-denied | input-denied | clipboard-status | clipboard-denied | kwin-bridge-status | keymap-status | screenshot-preview | screenshot-coordinate-map | screenshot-config-bounds | journal-artifacts | portal-screenshot | remote-desktop-probe | remote-desktop-eis-session | full-resolution-denied | control-safety) ;;
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

run_root="target/seatgeist-gui-eval"
run_id="${case_name}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="$run_root/$run_id"
latest_link="$run_root/latest"
socket_dir="$(mktemp -d -t "seatgeist-gui-eval-${case_name}.XXXXXX")"
socket="$socket_dir/seatgeistd.sock"
log="$run_dir/daemon.log"
journal="$run_dir/journal.jsonl"
panic_stop_file="$run_dir/panic-stop.flag"
approval_file="$run_dir/approvals.jsonl"
config_file="$run_dir/config.toml"
kwin_bridge_lock_file="$run_root/kwin-bridge-dbus.lock"
kwin_bridge_lock_fd=""
pid=""
evidence_status="passed"

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
	rm -rf "$socket_dir"
}

skip_eval() {
	evidence_status="skipped"
	echo "SKIP $*"
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

mkdir -p "$run_root" "$run_dir"
chmod 700 "$run_root" "$run_dir"
ln -sfnT "$run_id" "$latest_link"

if [[ "$case_name" == "all" || "$case_name" == "kwin-bridge-status" || "$case_name" == "control-safety" ]]; then
	if ! command -v flock >/dev/null 2>&1; then
		echo "flock is required for KWin bridge DBus eval serialization" >&2
		exit 1
	fi
	exec {kwin_bridge_lock_fd}>"$kwin_bridge_lock_file"
	flock "$kwin_bridge_lock_fd"
fi

cargo build -p seatgeistd -p seatgeist-cli
if [[ "$case_name" == "all" || "$case_name" == "screenshot-config-bounds" || "$case_name" == "journal-artifacts" || "$case_name" == "remote-desktop-probe" || "$case_name" == "remote-desktop-eis-session" ]]; then
	cat >"$config_file" <<CONFIG
[daemon]
socket = "$socket"
journal = "$journal"
panic_stop_file = "$panic_stop_file"

[journal]
include_artifact_metadata = $([[ "$case_name" == "journal-artifacts" ]] && echo true || echo false)

[safety]
preview_max_edge = 800
tile_max_edge = 640
require_focus_guard = $([[ "$case_name" == "remote-desktop-probe" || "$case_name" == "remote-desktop-eis-session" ]] && echo false || echo true)
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
target/debug/seatgeistd "${daemon_args[@]}" >"$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$socket" "$@"
}

grant_approval() {
	local safety_class="$1"
	local method="$2"
	local reason="$3"
	local output="$4"
	cli approve \
		--approval-file "$approval_file" \
		--safety-class "$safety_class" \
		--method "$method" \
		--ttl-ms 120000 \
		--reason "$reason" >"$output"
	jq -e --arg method "$method" '.method == $method' "$output" >/dev/null
}

prime_active_window_metadata() {
	local prefix="$1"
	local active_json="$run_dir/${prefix}-active-window.json"
	local active_err="$run_dir/${prefix}-active-window.err"
	local windows_json="$run_dir/${prefix}-windows.json"
	local bridge_status_json="$run_dir/${prefix}-kwin-bridge-status.json"
	local focus_approval="$run_dir/${prefix}-focus-approval.json"

	if cli active-window >"$active_json" 2>"$active_err" \
		&& jq -e '.type == "active_window" and ((.data.title // "") != "" or (.data.id // "") != "")' "$active_json" >/dev/null; then
		return 0
	fi

	cli kwin-bridge-status >"$bridge_status_json" 2>/dev/null || true
	cli windows >"$windows_json"
	local -a candidate_window_ids
	mapfile -t candidate_window_ids < <(jq -r '
		[
			.data[]
			| select((.id // "") != "" and .geometry != null)
			| select(
				(((.app_id // "") | ascii_downcase | contains("keepass")) | not)
				and (((.title // "") | ascii_downcase | contains("password")) | not)
			)
			| .id
		]
		| .[:8][]
	' "$windows_json")
	if [[ "${#candidate_window_ids[@]}" -eq 0 ]]; then
		return 1
	fi

	grant_approval control-semantic focus_window "gui-eval $prefix active-window prime" "$focus_approval"
	local window_id
	local focus_json
	local candidate_index=0
	for window_id in "${candidate_window_ids[@]}"; do
		focus_json="$run_dir/${prefix}-focus-${candidate_index}.json"
		candidate_index=$((candidate_index + 1))

		if ! cli focus --window "$window_id" >"$focus_json" 2>"${focus_json%.json}.err"; then
			continue
		fi
		jq -e '.type == "action"' "$focus_json" >/dev/null || continue

		for _ in {1..50}; do
			if cli active-window >"$active_json" 2>"$active_err" \
				&& jq -e --arg id "$window_id" '
					.type == "active_window"
					and (.data.id == $id or ((.data.title // "") != ""))
				' "$active_json" >/dev/null; then
				cp "$focus_json" "$run_dir/${prefix}-focus.json"
				return 0
			fi
			sleep 0.1
		done
	done
	cli kwin-bridge-status >"$bridge_status_json" 2>/dev/null || true
	return 1
}

eval_status() {
	cli doctor >"$run_dir/doctor.json"
	jq -e '.type == "health" and .data.status == "ok"' "$run_dir/doctor.json" >/dev/null
	cli capabilities >"$run_dir/capabilities.json"
	jq -e '.type == "capabilities"' "$run_dir/capabilities.json" >/dev/null
	cli policy-status >"$run_dir/policy-status.json"
	jq -e '.type == "policy_status" and .data.default_control == "prompt"' "$run_dir/policy-status.json" >/dev/null
	cli journal tail --limit 20 --method health --ok true >"$run_dir/status-health-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/status-health-journal.json" >/dev/null
	cli journal tail --limit 20 --method capabilities --ok true >"$run_dir/status-capabilities-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/status-capabilities-journal.json" >/dev/null
	cli journal tail --limit 20 --method policy_status --ok true >"$run_dir/status-policy-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/status-policy-journal.json" >/dev/null
}

eval_session_preflight() {
	cli safety-status >"$run_dir/session-preflight-safety.json"
	jq -e '
		.type == "safety_status"
		and (.data.require_focus_guard | type == "boolean")
		and (.data.pause_on_human_input | type == "boolean")
		and (.data.human_input_signal_fresh | type == "boolean")
		and (.data.human_input_quiet_ms | type == "number")
		and (.data.control_rate_limit_per_minute | type == "number")
		and (.data.preview_max_edge | type == "number")
		and (.data.tile_max_edge | type == "number")
		and (.data.screenshot_redaction_count | type == "number")
		and .data.journal_artifact_metadata_enabled == false
	' "$run_dir/session-preflight-safety.json" >/dev/null

	cli desktop-session-status >"$run_dir/session-preflight-desktop.json"
	jq -e '
		.type == "desktop_session_status"
		and (.data.dbus_session_bus_address_present | type == "boolean")
		and (.data.xdg_runtime_dir_present | type == "boolean")
		and (.data.setup_hint | type == "string")
		and (
			(.data.xdg_session_type | type == "string")
			or (.data.xdg_session_type == null)
		)
		and (
			(.data.xdg_current_desktop | type == "string")
			or (.data.xdg_current_desktop == null)
		)
		and (
			(.data.wayland_display | type == "string")
			or (.data.wayland_display == null)
		)
		and (
			(.data.display | type == "string")
			or (.data.display == null)
		)
	' "$run_dir/session-preflight-desktop.json" >/dev/null

	cli readiness >"$run_dir/session-preflight-readiness.json"
	jq -e '
		.type == "computer_use_readiness"
		and (.data.ready_for_observe | type == "boolean")
		and (.data.ready_for_screenshot | type == "boolean")
		and (.data.ready_for_window_control | type == "boolean")
		and (.data.ready_for_keyboard_input | type == "boolean")
		and (.data.ready_for_pointer_input | type == "boolean")
		and (.data.ready_for_semantic_actions | type == "boolean")
		and (.data.ready_for_clipboard_read | type == "boolean")
		and (.data.ready_for_clipboard_write | type == "boolean")
		and (.data.focus_guard_required | type == "boolean")
		and (.data.panic_stop_enabled | type == "boolean")
		and (.data.human_input_pause_enabled | type == "boolean")
		and (.data.human_input_signal_fresh | type == "boolean")
		and (.data.desktop_session_ready | type == "boolean")
		and (.data.dbus_session_bus_present | type == "boolean")
		and (.data.runtime_dir_present | type == "boolean")
		and (.data.issues | type == "array")
		and (.data.next_steps | type == "array")
		and (.data.accessibility_backend | type == "string")
		and (
			(.data.capture_backend | type == "string")
			or (.data.capture_backend == null)
		)
		and (
			(.data.input_backend | type == "string")
			or (.data.input_backend == null)
		)
		and (
			(.data.clipboard_read_backend | type == "string")
			or (.data.clipboard_read_backend == null)
		)
		and (
			(.data.clipboard_write_backend | type == "string")
			or (.data.clipboard_write_backend == null)
		)
	' "$run_dir/session-preflight-readiness.json" >/dev/null

	cli journal tail --limit 20 --method safety_status --ok true >"$run_dir/session-preflight-safety-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/session-preflight-safety-journal.json" >/dev/null
	cli journal tail --limit 20 --method desktop_session_status --ok true >"$run_dir/session-preflight-desktop-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/session-preflight-desktop-journal.json" >/dev/null
	cli journal tail --limit 20 --method computer_use_readiness --ok true >"$run_dir/session-preflight-readiness-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/session-preflight-readiness-journal.json" >/dev/null
}

eval_observe() {
	cli observe >"$run_dir/observe.json"
	jq -e '.type == "observation" and (.data.monitors | type == "array") and (.data.windows | type == "array")' "$run_dir/observe.json" >/dev/null
	cli journal tail --limit 20 --method observe --ok true >"$run_dir/observe-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/observe-journal.json" >/dev/null
}

eval_a11y_quality_status() {
	cli atspi quality-status >"$run_dir/a11y-quality-status.json"
	jq -e '
		.type == "accessibility_quality_status"
		and (.data.atspi_available | type == "boolean")
		and (.data.focused_node_present | type == "boolean")
		and (.data.sample_depth | type == "number")
		and (.data.sample_max_nodes | type == "number")
		and (.data.sampled_node_count | type == "number")
		and (.data.named_node_count | type == "number")
		and (.data.actionable_node_count | type == "number")
		and (.data.text_node_count | type == "number")
		and (.data.sensitive_node_count | type == "number")
		and (.data.generic_role_count | type == "number")
		and (.data.max_depth_seen | type == "number")
		and (.data.tree_flat | type == "boolean")
		and (.data.semantic_targeting_reliable | type == "boolean")
		and (.data.recommended_fallback | type == "string")
		and (.data.setup_hint | type == "string")
	' "$run_dir/a11y-quality-status.json" >/dev/null
	cli journal tail --limit 20 --method accessibility_quality_status --ok true >"$run_dir/a11y-quality-status-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/a11y-quality-status-journal.json" >/dev/null
}

eval_a11y_focused_tree() {
	cli atspi quality-status >"$run_dir/a11y-focused-tree-quality.json"
	jq -e '.type == "accessibility_quality_status"' "$run_dir/a11y-focused-tree-quality.json" >/dev/null
	if ! jq -e '.data.atspi_available == true and .data.focused_node_present == true' "$run_dir/a11y-focused-tree-quality.json" >/dev/null; then
		skip_eval "a11y-focused-tree: focused AT-SPI tree is unavailable"
		return 0
	fi

	cli atspi tree --focused --depth 1 --max-nodes 128 >"$run_dir/a11y-focused-tree.json"
	jq -e '
		.type == "accessibility_tree"
		and (.data != null)
		and (.data.id | type == "string")
		and (.data.role | type == "string")
		and (.data.sensitive | type == "boolean")
		and (.data.states | type == "array")
		and (.data.available_actions | type == "array")
		and (.data.actions | type == "array")
		and (.data.children | type == "array")
	' "$run_dir/a11y-focused-tree.json" >/dev/null
	cli journal tail --limit 20 --method focused_accessibility_tree --ok true >"$run_dir/a11y-focused-tree-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/a11y-focused-tree-journal.json" >/dev/null
}

eval_a11y_find() {
	cli atspi quality-status >"$run_dir/a11y-find-quality.json"
	jq -e '.type == "accessibility_quality_status"' "$run_dir/a11y-find-quality.json" >/dev/null
	if ! jq -e '.data.atspi_available == true' "$run_dir/a11y-find-quality.json" >/dev/null; then
		skip_eval "a11y-find: AT-SPI is unavailable"
		return 0
	fi

	if ! cli atspi find --role application --max-results 3 --max-nodes 256 >"$run_dir/a11y-find.json" 2>"$run_dir/a11y-find.stderr"; then
		cli journal tail --limit 20 --method accessibility_find --ok false >"$run_dir/a11y-find-unavailable-journal.json"
		if grep -Eq 'AccessibilityUnavailable|backend unavailable' "$run_dir/a11y-find.stderr" \
			&& jq -e '.type == "journal" and (.data | length) >= 1 and all(.data[]; .ok == false and (.summary | contains("AccessibilityUnavailable")))' "$run_dir/a11y-find-unavailable-journal.json" >/dev/null; then
			skip_eval "a11y-find: AT-SPI find is unavailable"
			return 0
		fi
		cat "$run_dir/a11y-find.stderr" >&2
		return 1
	fi
	jq -e '
		.type == "accessibility_matches"
		and (.data | type == "array")
		and all(.data[];
			(.id | type == "string")
			and (.role | type == "string")
			and (.sensitive | type == "boolean")
			and (.states | type == "array")
			and (.available_actions | type == "array")
			and (.actions | type == "array")
			and (.children | type == "array")
		)
	' "$run_dir/a11y-find.json" >/dev/null
	cli journal tail --limit 20 --method accessibility_find --ok true >"$run_dir/a11y-find-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/a11y-find-journal.json" >/dev/null
}

eval_a11y_text_attributes() {
	cli atspi quality-status >"$run_dir/a11y-text-attributes-quality.json"
	jq -e '.type == "accessibility_quality_status"' "$run_dir/a11y-text-attributes-quality.json" >/dev/null
	if ! jq -e '.data.atspi_available == true' "$run_dir/a11y-text-attributes-quality.json" >/dev/null; then
		skip_eval "a11y-text-attributes: AT-SPI is unavailable"
		return 0
	fi

	if ! cli atspi find --role text --max-results 1 --max-nodes 512 >"$run_dir/a11y-text-attributes-find.json" 2>"$run_dir/a11y-text-attributes-find.stderr"; then
		cli journal tail --limit 20 --method accessibility_find --ok false >"$run_dir/a11y-text-attributes-find-unavailable-journal.json"
		if grep -Eq 'AccessibilityUnavailable|backend unavailable' "$run_dir/a11y-text-attributes-find.stderr" \
			&& jq -e '.type == "journal" and (.data | length) >= 1 and all(.data[]; .ok == false and (.summary | contains("AccessibilityUnavailable")))' "$run_dir/a11y-text-attributes-find-unavailable-journal.json" >/dev/null; then
			skip_eval "a11y-text-attributes: AT-SPI find is unavailable"
			return 0
		fi
		cat "$run_dir/a11y-text-attributes-find.stderr" >&2
		return 1
	fi
	jq -e '.type == "accessibility_matches" and (.data | type == "array")' "$run_dir/a11y-text-attributes-find.json" >/dev/null
	local node_id
	if ! node_id="$(jq -er '.data[0].id // empty' "$run_dir/a11y-text-attributes-find.json")"; then
		skip_eval "a11y-text-attributes: no text node found"
		return 0
	fi

	if ! cli atspi text-attributes --node "$node_id" --offset 0 >"$run_dir/a11y-text-attributes.json" 2>"$run_dir/a11y-text-attributes.stderr"; then
		cli journal tail --limit 20 --method accessibility_text_attributes --ok false >"$run_dir/a11y-text-attributes-unavailable-journal.json"
		if grep -Eq 'AccessibilityUnavailable|backend unavailable' "$run_dir/a11y-text-attributes.stderr" \
			&& jq -e '.type == "journal" and (.data | length) >= 1 and all(.data[]; .ok == false and (.summary | contains("AccessibilityUnavailable")))' "$run_dir/a11y-text-attributes-unavailable-journal.json" >/dev/null; then
			skip_eval "a11y-text-attributes: AT-SPI text attributes are unavailable"
			return 0
		fi
		cat "$run_dir/a11y-text-attributes.stderr" >&2
		return 1
	fi
	jq -e --arg node_id "$node_id" '
		.type == "accessibility_text_attributes"
		and .data.node_id == $node_id
		and (.data.start_offset | type == "number")
		and (.data.end_offset | type == "number")
		and (.data.attributes | type == "array")
		and all(.data.attributes[];
			(.name | type == "string")
			and (.value | type == "string")
		)
	' "$run_dir/a11y-text-attributes.json" >/dev/null
	cli journal tail --limit 20 --method accessibility_text_attributes --ok true >"$run_dir/a11y-text-attributes-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/a11y-text-attributes-journal.json" >/dev/null
}

assert_a11y_control_denied() {
	local method="$1"
	local label="$2"
	shift 2
	if cli "$@" >"$run_dir/a11y-control-denied-$label.txt" 2>&1; then
		cat "$run_dir/a11y-control-denied-$label.txt" >&2
		return 1
	fi
	grep -Eq "PolicyPromptRequired|policy" "$run_dir/a11y-control-denied-$label.txt"
	cli journal tail --limit 20 --method "$method" --ok false >"$run_dir/a11y-control-denied-$label-journal.json"
	jq -e '
		.type == "journal"
		and any(.data[];
			.safety_class == "control_semantic"
			and .ok == false
			and (.summary | contains("PolicyPromptRequired"))
		)
	' "$run_dir/a11y-control-denied-$label-journal.json" >/dev/null
}

eval_a11y_control_denied() {
	local node="atspi://:1.42/org/a11y/atspi/accessible/7"
	assert_a11y_control_denied accessibility_invoke invoke atspi invoke --node "$node" --action press
	assert_a11y_control_denied accessibility_set_text set-text atspi set-text --node "$node" smoke-text
	assert_a11y_control_denied accessibility_insert_text insert-text atspi insert-text --node "$node" --offset 0 smoke-text
	assert_a11y_control_denied accessibility_delete_text delete-text atspi delete-text --node "$node" --start-offset 0 --end-offset 1
	assert_a11y_control_denied accessibility_copy_text copy-text atspi copy-text --node "$node" --start-offset 0 --end-offset 1
	assert_a11y_control_denied accessibility_cut_text cut-text atspi cut-text --node "$node" --start-offset 0 --end-offset 1
	assert_a11y_control_denied accessibility_paste_text paste-text atspi paste-text --node "$node" --offset 0
	assert_a11y_control_denied accessibility_set_caret set-caret atspi set-caret --node "$node" --offset 0
	assert_a11y_control_denied accessibility_set_selection set-selection atspi set-selection --node "$node" --start-offset 0 --end-offset 1
}

assert_semantic_denied() {
	local method="$1"
	local label="$2"
	shift 2
	if cli "$@" >"$run_dir/semantic-denied-$label.txt" 2>&1; then
		cat "$run_dir/semantic-denied-$label.txt" >&2
		return 1
	fi
	grep -Eq "PolicyPromptRequired|policy" "$run_dir/semantic-denied-$label.txt"
	cli journal tail --limit 20 --method "$method" --ok false >"$run_dir/semantic-denied-$label-journal.json"
	jq -e '
		.type == "journal"
		and any(.data[];
			.safety_class == "control_semantic"
			and .ok == false
			and (.summary | contains("PolicyPromptRequired"))
		)
	' "$run_dir/semantic-denied-$label-journal.json" >/dev/null
}

eval_semantic_denied() {
	assert_semantic_denied click_button click-button semantic click-button --name OK --max-nodes 128
	assert_semantic_denied set_text_field set-text-field semantic set-text-field --name Search smoke-text --max-nodes 128
	assert_semantic_denied focus_text_field focus-text-field semantic focus-text-field --name Search --max-nodes 128
	assert_semantic_denied activate_tab activate-tab semantic activate-tab --name General --max-nodes 128
	assert_semantic_denied activate_link activate-link semantic activate-link --name Help --max-nodes 128
	assert_semantic_denied toggle_check toggle-check semantic toggle-check --name Enable --max-nodes 128
	assert_semantic_denied set_value set-value semantic set-value --name Volume --value 0.5 --max-nodes 128
	assert_semantic_denied select_item select-item semantic select-item --name Printer --max-nodes 128
	assert_semantic_denied select_menu select-menu semantic select-menu --path File/Open --max-nodes 128
}

assert_input_denied() {
	local method="$1"
	local label="$2"
	local safety_class="$3"
	shift 3
	if cli "$@" >"$run_dir/input-denied-$label.txt" 2>&1; then
		cat "$run_dir/input-denied-$label.txt" >&2
		return 1
	fi
	grep -Eq "PolicyPromptRequired|policy" "$run_dir/input-denied-$label.txt"
	cli journal tail --limit 20 --method "$method" --ok false >"$run_dir/input-denied-$label-journal.json"
	jq -e --arg safety_class "$safety_class" '
		.type == "journal"
		and any(.data[];
			.safety_class == $safety_class
			and .ok == false
			and (.summary | contains("PolicyPromptRequired"))
		)
	' "$run_dir/input-denied-$label-journal.json" >/dev/null
}

eval_input_denied() {
	assert_input_denied type_text type-text control_keyboard input type-text smoke-text
	assert_input_denied key_combo key-combo control_keyboard input key-combo Shift
	assert_input_denied move_pointer move-pointer control_pointer input move-pointer --x 1 --y 1 --coordinate-space physical-pixel
	assert_input_denied click_pointer click-pointer control_pointer input click-pointer --x 1 --y 1 --coordinate-space physical-pixel --button left
	assert_input_denied drag_pointer drag-pointer control_pointer input drag-pointer --from-x 1 --from-y 1 --to-x 2 --to-y 2 --coordinate-space physical-pixel
	assert_input_denied scroll_pointer scroll-pointer control_pointer input scroll-pointer --vertical 1
}

eval_clipboard_status() {
	cli clipboard status >"$run_dir/clipboard-status.json"
	jq -e '
		.type == "clipboard_backend_status"
		and (.data.wl_paste_available | type == "boolean")
		and (.data.wl_copy_available | type == "boolean")
		and (.data.kde_klipper_available | type == "boolean")
		and (
			(.data.read_backend | type == "string")
			or (.data.read_backend == null)
		)
		and (
			(.data.write_backend | type == "string")
			or (.data.write_backend == null)
		)
		and (.data.setup_hint | type == "string")
	' "$run_dir/clipboard-status.json" >/dev/null

	cli journal tail --limit 20 --method clipboard_backend_status --ok true >"$run_dir/clipboard-status-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/clipboard-status-journal.json" >/dev/null
}

eval_clipboard_denied() {
	if cli clipboard get >"$run_dir/clipboard-denied.txt" 2>&1; then
		echo "clipboard get unexpectedly succeeded without clipboard-read approval" >&2
		exit 1
	fi
	grep -qi "policy" "$run_dir/clipboard-denied.txt"
	cli journal tail --limit 20 --method clipboard_get --ok false >"$run_dir/clipboard-denied-journal.json"
	jq -e '.type == "journal" and any(.data[]; .summary | contains("ClipboardRead"))' "$run_dir/clipboard-denied-journal.json" >/dev/null
}

portal_screenshot_cancelled() {
	grep -qi "portal screenshot request was cancelled or ended without a screenshot" "$1"
}

seed_kwin_bridge_updates() {
	if ! command -v qdbus6 >/dev/null 2>&1; then
		return 1
	fi

	local active_payload='{"active":true,"id":"seatgeist-eval-window","title":"Seatgeist Eval Window","app_id":"org.seatgeist.eval","geometry":{"x":0,"y":0,"width":100,"height":100}}'
	local windows_payload='{"windows":[{"id":"seatgeist-eval-window","title":"Seatgeist Eval Window","app_id":"org.seatgeist.eval","geometry":{"x":0,"y":0,"width":100,"height":100}},{"id":"seatgeist-eval-secondary","title":"Seatgeist Eval Secondary","app_id":"org.seatgeist.eval","geometry":{"x":120,"y":0,"width":80,"height":80}}]}'

	for _ in {1..50}; do
		if qdbus6 org.seatgeist.KWinBridge /org/seatgeist/KWinBridge1 org.seatgeist.KWinBridge1.UpdateActiveWindow "$active_payload" >/dev/null 2>&1 \
			&& qdbus6 org.seatgeist.KWinBridge /org/seatgeist/KWinBridge1 org.seatgeist.KWinBridge1.UpdateWindows "$windows_payload" >/dev/null 2>&1; then
			return 0
		fi
		sleep 0.1
	done

	return 1
}

skip_portal_screenshot_cancel() {
	local eval_name="$1"
	local err_file="$2"
	if portal_screenshot_cancelled "$err_file" && [[ "${SEATGEIST_PORTAL_SCREENSHOT_STRICT:-0}" != "1" ]]; then
		skip_eval "$eval_name: portal cancelled or ended the request without a screenshot"
		return 0
	fi
	return 1
}

eval_kwin_bridge_status() {
	local seeded="false"
	if seed_kwin_bridge_updates; then
		seeded="true"
	fi

	cli kwin-bridge-status >"$run_dir/kwin-bridge-status.json"
	jq -e '
		.type == "kwin_bridge_status"
		and (.data.dbus_service_registered | type == "boolean")
		and (.data.active_window_update_seen | type == "boolean")
		and (.data.window_list_update_seen | type == "boolean")
		and (.data.window_count | type == "number")
		and (.data.package_installed | type == "boolean")
		and (
			(.data.script_enabled | type == "boolean")
			or (.data.script_enabled == null)
		)
	' "$run_dir/kwin-bridge-status.json" >/dev/null
	if [[ "$seeded" == "true" ]]; then
		jq -e '
			.data.dbus_service_registered == true
			and .data.active_window_update_seen == true
			and .data.window_list_update_seen == true
			and .data.window_count == 2
			and .data.active_window.id == "seatgeist-eval-window"
			and .data.active_window.app_id == "org.seatgeist.eval"
		' "$run_dir/kwin-bridge-status.json" >/dev/null
	fi
	cli journal tail --limit 20 --method kwin_bridge_status --ok true >"$run_dir/kwin-bridge-status-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/kwin-bridge-status-journal.json" >/dev/null
}

eval_keymap_status() {
	cli input backends >"$run_dir/keymap-status.json"
	jq -e '
		.type == "input_backend_status"
		and (.data.eis_keymap.source | type == "string")
		and (.data.eis_keymap.setup_hint | type == "string")
		and (
			.data.eis_keymap.source == "config"
			or .data.eis_keymap.source == "kde_current_layout"
			or .data.eis_keymap.source == "kde_kxkbrc"
			or .data.eis_keymap.source == "xkbcommon_default"
		)
	' "$run_dir/keymap-status.json" >/dev/null

	if command -v qdbus6 >/dev/null 2>&1; then
		if qdbus6 org.kde.keyboard /Layouts org.kde.KeyboardLayouts.getCurrentLayout >"$run_dir/keymap-current-layout.txt" 2>/dev/null \
			|| qdbus6 org.kde.keyboard /Layouts getCurrentLayout >"$run_dir/keymap-current-layout.txt" 2>/dev/null; then
			current_layout="$(tr -d '\r\n' <"$run_dir/keymap-current-layout.txt" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
			if [[ -n "$current_layout" ]]; then
				jq -e --arg current_layout "$current_layout" '
					.data.eis_keymap.source == "kde_current_layout"
					and .data.eis_keymap.kde_current_layout == $current_layout
					and (.data.eis_keymap.layout | type == "string")
				' "$run_dir/keymap-status.json" >/dev/null
			fi
		fi
	fi

	if command -v kreadconfig6 >/dev/null 2>&1; then
		if kreadconfig6 --file kxkbrc --group Layout --key LayoutList >"$run_dir/keymap-layout-list.txt" 2>/dev/null; then
			layout_list="$(tr -d '\r\n' <"$run_dir/keymap-layout-list.txt" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
			if [[ -n "$layout_list" ]]; then
				jq -e --arg layout_list "$layout_list" '
					.data.eis_keymap.kde_config_layouts == $layout_list
					or .data.eis_keymap.source == "kde_current_layout"
				' "$run_dir/keymap-status.json" >/dev/null
			fi
		fi
	fi

	cli journal tail --limit 20 --method input_backend_status --ok true >"$run_dir/keymap-status-journal.json"
	jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/keymap-status-journal.json" >/dev/null
}

eval_screenshot_preview() {
	if ! command -v spectacle >/dev/null 2>&1; then
		skip_eval "screenshot-preview: spectacle is not available"
		return 0
	fi
	if ! cli screenshot --output "$run_dir/preview.png" >"$run_dir/screenshot-preview.json" 2>"$run_dir/screenshot-preview.err"; then
		if skip_portal_screenshot_cancel "screenshot-preview" "$run_dir/screenshot-preview.err"; then
			return 0
		fi
		cat "$run_dir/screenshot-preview.err" >&2
		exit 1
	fi
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
		skip_eval "screenshot-coordinate-map: spectacle is not available"
		return 0
	fi
	if ! cli screenshot --output "$run_dir/coordinate-map.png" >"$run_dir/screenshot-coordinate-map.json" 2>"$run_dir/screenshot-coordinate-map.err"; then
		if skip_portal_screenshot_cancel "screenshot-coordinate-map" "$run_dir/screenshot-coordinate-map.err"; then
			return 0
		fi
		cat "$run_dir/screenshot-coordinate-map.err" >&2
		exit 1
	fi
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
		skip_eval "screenshot-config-bounds: spectacle is not available"
		return 0
	fi
	cli safety-status >"$run_dir/screenshot-config-safety.json"
	jq -e '
		.type == "safety_status"
		and .data.preview_max_edge == 800
		and .data.tile_max_edge == 640
		and .data.journal_artifact_metadata_enabled == false
	' "$run_dir/screenshot-config-safety.json" >/dev/null

	if ! cli screenshot --output "$run_dir/config-preview.png" >"$run_dir/screenshot-config-preview.json" 2>"$run_dir/screenshot-config-preview.err"; then
		if skip_portal_screenshot_cancel "screenshot-config-bounds" "$run_dir/screenshot-config-preview.err"; then
			return 0
		fi
		cat "$run_dir/screenshot-config-preview.err" >&2
		exit 1
	fi
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
	if ! cli screenshot-tile \
		--output "$run_dir/config-tile.png" \
		--x 0 \
		--y 0 \
		--width "$tile_width" \
		--height "$tile_height" >"$run_dir/screenshot-config-tile.json" 2>"$run_dir/screenshot-config-tile.err"; then
		if skip_portal_screenshot_cancel "screenshot-config-bounds tile" "$run_dir/screenshot-config-tile.err"; then
			return 0
		fi
		cat "$run_dir/screenshot-config-tile.err" >&2
		exit 1
	fi
	jq -e '
		.type == "screenshot"
		and .data.output_width <= 640
		and .data.output_height <= 640
		and .data.transform.source_origin_x == 0
		and .data.transform.source_origin_y == 0
	' "$run_dir/screenshot-config-tile.json" >/dev/null
}

eval_journal_artifacts() {
	cli capture-backends >"$run_dir/journal-artifacts-capture-backends.json"
	if ! jq -e '.type == "capture_backend_status" and (.data.implemented_available_backend | type == "string")' "$run_dir/journal-artifacts-capture-backends.json" >/dev/null; then
		skip_eval "journal-artifacts: no screenshot backend is available"
		return 0
	fi

	if ! cli safety-status >"$run_dir/journal-artifacts-safety.json"; then
		echo "journal-artifacts could not read safety status" >&2
		exit 1
	fi
	jq -e '
		.type == "safety_status"
		and .data.preview_max_edge == 800
		and .data.tile_max_edge == 640
		and .data.journal_artifact_metadata_enabled == true
	' "$run_dir/journal-artifacts-safety.json" >/dev/null

	screenshot_path="$run_dir/journal-artifact-preview.png"
	if ! cli screenshot --output "$screenshot_path" >"$run_dir/journal-artifacts-screenshot.json" 2>"$run_dir/journal-artifacts-screenshot.err"; then
		if skip_portal_screenshot_cancel "journal-artifacts" "$run_dir/journal-artifacts-screenshot.err"; then
			return 0
		fi
		cat "$run_dir/journal-artifacts-screenshot.err" >&2
		exit 1
	fi
	test -s "$screenshot_path"
	jq -e --arg path "$screenshot_path" '
		.type == "screenshot"
		and .data.path == $path
		and .data.output_width <= 800
		and .data.output_height <= 800
	' "$run_dir/journal-artifacts-screenshot.json" >/dev/null

	artifact_sha="$(sha256sum "$screenshot_path" | awk '{print $1}')"
	artifact_bytes="$(stat -c '%s' "$screenshot_path")"
	cli journal tail --limit 20 --method screenshot --ok true >"$run_dir/journal-artifacts-journal.json"
	jq -e --arg path "$screenshot_path" --arg sha "$artifact_sha" --argjson bytes "$artifact_bytes" '
		.type == "journal"
		and any(.data[];
			.method == "screenshot"
			and .ok == true
			and (.artifacts | type == "array")
			and any(.artifacts[];
				.kind == "screenshot"
				and .path == $path
				and .sha256 == $sha
				and .bytes == $bytes
			)
		)
	' "$run_dir/journal-artifacts-journal.json" >/dev/null
}

eval_portal_screenshot() {
	cli capture-backends >"$run_dir/portal-capture-backends.json"
	if ! jq -e '.type == "capture_backend_status" and .data.screenshot_portal.screenshot_interface_available == true' "$run_dir/portal-capture-backends.json" >/dev/null; then
		skip_eval "portal-screenshot: xdg-desktop-portal Screenshot interface is not visible"
		return 0
	fi
	if ! jq -e '.data.implemented_available_backend == "portal_screenshot"' "$run_dir/portal-capture-backends.json" >/dev/null; then
		echo "portal Screenshot is visible but not selected as implemented backend" >&2
		cat "$run_dir/portal-capture-backends.json" >&2
		exit 1
	fi

	if ! cli screenshot --output "$run_dir/portal-screenshot.png" >"$run_dir/portal-screenshot.json" 2>"$run_dir/portal-screenshot.err"; then
		if portal_screenshot_cancelled "$run_dir/portal-screenshot.err" && [[ "${SEATGEIST_PORTAL_SCREENSHOT_STRICT:-0}" != "1" ]]; then
			skip_eval "portal-screenshot: portal cancelled or ended the request without a screenshot"
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

	source_width="$(jq -r '.data.source_width' "$run_dir/portal-screenshot.json")"
	source_height="$(jq -r '.data.source_height' "$run_dir/portal-screenshot.json")"
	tile_width="$source_width"
	tile_height="$source_height"
	if (( tile_width > 800 )); then
		tile_width=800
	fi
	if (( tile_height > 600 )); then
		tile_height=600
	fi
	if ! cli screenshot-tile \
		--output "$run_dir/portal-screenshot-tile.png" \
		--x 0 \
		--y 0 \
		--width "$tile_width" \
		--height "$tile_height" >"$run_dir/portal-screenshot-tile.json" 2>"$run_dir/portal-screenshot-tile.err"; then
		if portal_screenshot_cancelled "$run_dir/portal-screenshot-tile.err" && [[ "${SEATGEIST_PORTAL_SCREENSHOT_STRICT:-0}" != "1" ]]; then
			skip_eval "portal-screenshot tile: portal cancelled or ended the tile source request without a screenshot"
			return 0
		fi
		cat "$run_dir/portal-screenshot-tile.err" >&2
		exit 1
	fi
	test -s "$run_dir/portal-screenshot-tile.png"
	jq -e '
		.type == "screenshot"
		and .data.backend == "portal_screenshot"
		and .data.source_width >= .data.output_width
		and .data.source_height >= .data.output_height
		and .data.output_width <= 800
		and .data.output_height <= 600
		and .data.transform.source_origin_x == 0
		and .data.transform.source_origin_y == 0
		and .data.transform.scale_x > 0
		and .data.transform.scale_y > 0
	' "$run_dir/portal-screenshot-tile.json" >/dev/null
	cli journal tail --limit 20 --method screenshot_tile --ok true >"$run_dir/portal-screenshot-tile-journal.json"
	jq -e '.type == "journal" and any(.data[]; .summary | contains("backend=portal_screenshot"))' "$run_dir/portal-screenshot-tile-journal.json" >/dev/null
}

eval_remote_desktop_probe() {
	cli input backends >"$run_dir/remote-desktop-backends.json"
	if ! jq -e '.type == "input_backend_status" and .data.remote_desktop_portal.remote_desktop_interface_available == true' "$run_dir/remote-desktop-backends.json" >/dev/null; then
		skip_eval "remote-desktop-probe: xdg-desktop-portal RemoteDesktop interface is not visible"
		return 0
	fi

	prime_active_window_metadata "remote-desktop" || true
	active_title="$(jq -r '.data.title // empty' "$run_dir/remote-desktop-active-window.json" 2>/dev/null || true)"
	active_id="$(jq -r '.data.id // empty' "$run_dir/remote-desktop-active-window.json" 2>/dev/null || true)"
	if [[ -z "$active_title" && -z "$active_id" ]]; then
		skip_eval "remote-desktop-probe: active-window guard metadata is unavailable"
		return 0
	fi

	guard_args=()
	if [[ -n "$active_title" ]]; then
		guard_args+=(--active-title-contains "$active_title")
	else
		guard_args+=(--expected-active-window "$active_id")
	fi

	grant_approval control-pointer remote_desktop_session_probe "gui-eval remote-desktop-probe" "$run_dir/remote-desktop-approval.json"
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
	if [[ "${SEATGEIST_REMOTE_DESKTOP_STRICT:-0}" == "1" ]]; then
		jq -e '.data.started == true and (.data.selected_devices | length) >= 1' "$run_dir/remote-desktop-probe.json" >/dev/null
	fi
	cli journal tail --limit 20 --method remote_desktop_session_probe --ok true >"$run_dir/remote-desktop-journal.json"
	jq -e '.type == "journal" and any(.data[]; .summary | contains("remote desktop session probe"))' "$run_dir/remote-desktop-journal.json" >/dev/null
}

eval_remote_desktop_eis_session() {
	cli input backends >"$run_dir/remote-desktop-eis-backends-before.json"
	if ! jq -e '.type == "input_backend_status" and .data.remote_desktop_portal.remote_desktop_interface_available == true' "$run_dir/remote-desktop-eis-backends-before.json" >/dev/null; then
		skip_eval "remote-desktop-eis-session: xdg-desktop-portal RemoteDesktop interface is not visible"
		return 0
	fi

	prime_active_window_metadata "remote-desktop-eis" || true
	active_title="$(jq -r '.data.title // empty' "$run_dir/remote-desktop-eis-active-window.json" 2>/dev/null || true)"
	active_id="$(jq -r '.data.id // empty' "$run_dir/remote-desktop-eis-active-window.json" 2>/dev/null || true)"
	if [[ -z "$active_title" && -z "$active_id" ]]; then
		skip_eval "remote-desktop-eis-session: active-window guard metadata is unavailable"
		return 0
	fi

	guard_args=()
	if [[ -n "$active_title" ]]; then
		guard_args+=(--active-title-contains "$active_title")
	else
		guard_args+=(--expected-active-window "$active_id")
	fi

	grant_approval control-pointer remote_desktop_eis_start "gui-eval remote-desktop-eis-session start" "$run_dir/remote-desktop-eis-start-approval.json"
	jq -e '.method == "remote_desktop_eis_start" and .safety_class == "control_pointer"' "$run_dir/remote-desktop-eis-start-approval.json" >/dev/null
	grant_approval control-pointer scroll_pointer "gui-eval remote-desktop-eis-session minimal input" "$run_dir/remote-desktop-eis-scroll-approval.json"
	jq -e '.method == "scroll_pointer" and .safety_class == "control_pointer"' "$run_dir/remote-desktop-eis-scroll-approval.json" >/dev/null
	grant_approval control-keyboard key_combo "gui-eval remote-desktop-eis-session keyboard input" "$run_dir/remote-desktop-eis-key-combo-approval.json"
	jq -e '.method == "key_combo" and .safety_class == "control_keyboard"' "$run_dir/remote-desktop-eis-key-combo-approval.json" >/dev/null
	test "$(stat -c '%a' "$approval_file")" = "600"

	if ! cli input remote-desktop-eis-start --keyboard --pointer --timeout-ms 120000 "${guard_args[@]}" >"$run_dir/remote-desktop-eis-start.json" 2>"$run_dir/remote-desktop-eis-start.err"; then
		cat "$run_dir/remote-desktop-eis-start.err" >&2
		exit 1
	fi
	jq -e '.type == "remote_desktop_eis_session_status"' "$run_dir/remote-desktop-eis-start.json" >/dev/null
	if ! jq -e '.data.active == true' "$run_dir/remote-desktop-eis-start.json" >/dev/null; then
		if [[ "${SEATGEIST_REMOTE_DESKTOP_EIS_STRICT:-0}" == "1" ]]; then
			echo "remote-desktop-eis-session did not start in strict mode" >&2
			cat "$run_dir/remote-desktop-eis-start.json" >&2
			exit 1
		fi
		skip_eval "remote-desktop-eis-session: portal cancelled or ended before a stored EIS session was active"
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
		if [[ "${SEATGEIST_REMOTE_DESKTOP_EIS_INPUT_STRICT:-0}" == "1" ]]; then
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
		if [[ "${SEATGEIST_REMOTE_DESKTOP_EIS_INPUT_STRICT:-0}" == "1" ]]; then
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
	cli journal tail --limit 20 --method screenshot --ok false >"$run_dir/full-resolution-denied-journal.json"
	jq -e '.type == "journal" and any(.data[]; .summary | contains("FullResolutionScreenshot"))' "$run_dir/full-resolution-denied-journal.json" >/dev/null
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

	seed_kwin_bridge_updates || true

	if cli active-window >"$run_dir/control-safety-active-window.json" 2>/dev/null; then
		if cli focus --window "__seatgeist_eval_never__" --expected-active-window "__seatgeist_wrong_window__" >"$run_dir/guard-denied.txt" 2>&1; then
			echo "focus unexpectedly passed with an incorrect active-window guard" >&2
			exit 1
		fi
		grep -q "active-window guard failed" "$run_dir/guard-denied.txt"
	else
		skip_eval "active-window guard denial: KWin active-window bridge has not reported yet"
	fi

	cli panic-stop enable >"$run_dir/panic-stop-enabled.json"
	jq -e '.type == "panic_stop" and .data.enabled == true' "$run_dir/panic-stop-enabled.json" >/dev/null
	if cli focus --window "__seatgeist_eval_never__" >"$run_dir/panic-stop-denied.txt" 2>&1; then
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
		session-preflight) eval_session_preflight ;;
		observe) eval_observe ;;
		a11y-quality-status) eval_a11y_quality_status ;;
		a11y-focused-tree) eval_a11y_focused_tree ;;
		a11y-find) eval_a11y_find ;;
		a11y-text-attributes) eval_a11y_text_attributes ;;
		a11y-control-denied) eval_a11y_control_denied ;;
		semantic-denied) eval_semantic_denied ;;
		input-denied) eval_input_denied ;;
		clipboard-status) eval_clipboard_status ;;
		clipboard-denied) eval_clipboard_denied ;;
		kwin-bridge-status) eval_kwin_bridge_status ;;
		keymap-status) eval_keymap_status ;;
		screenshot-preview) eval_screenshot_preview ;;
		screenshot-coordinate-map) eval_screenshot_coordinate_map ;;
		screenshot-config-bounds) eval_screenshot_config_bounds ;;
		journal-artifacts) eval_journal_artifacts ;;
		portal-screenshot) eval_portal_screenshot ;;
		remote-desktop-probe) eval_remote_desktop_probe ;;
		remote-desktop-eis-session) eval_remote_desktop_eis_session ;;
		full-resolution-denied) eval_full_resolution_denied ;;
		control-safety) eval_control_safety ;;
	esac
}

if [[ "$case_name" == "all" ]]; then
	for eval_name in status session-preflight observe a11y-quality-status a11y-focused-tree a11y-find a11y-text-attributes a11y-control-denied semantic-denied input-denied clipboard-status clipboard-denied kwin-bridge-status keymap-status screenshot-preview screenshot-coordinate-map screenshot-config-bounds full-resolution-denied; do
		run_case "$eval_name"
	done
else
	run_case "$case_name"
fi

cli journal tail --limit 20 >"$run_dir/journal-tail.json"
jq -e '.type == "journal" and (.data | length) >= 1' "$run_dir/journal-tail.json" >/dev/null
scripts/write-eval-evidence.py --run-dir "$run_dir" --case "$case_name" --kind "safe-gui" --status "$evidence_status"
echo "GUI eval $case_name passed; artifacts are in $run_dir"
echo "Latest GUI eval artifacts symlink: $latest_link"
