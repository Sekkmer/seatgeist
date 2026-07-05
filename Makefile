SHELL := /usr/bin/bash
.ONESHELL:

.PHONY: fmt check test clippy validate-plugin verify smoke smoke-monitors smoke-windows smoke-focus smoke-clipboard smoke-atspi smoke-uinput-status smoke-capture-backends smoke-pointer-calibration smoke-trace-replay smoke-gui-input smoke-mcp gui-eval gui-eval-control-safety install-kwin-script

fmt:
	cargo fmt --all

check:
	cargo check --workspace --all-targets

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

validate-plugin:
	scripts/validate-plugin.py plugin

verify: fmt check test clippy validate-plugin
	git diff --check -- . ':(exclude)target'

smoke:
	set -euo pipefail
	socket="target/plasma-pilot-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-smoke-daemon.log"
	journal="target/plasma-pilot-smoke-journal.jsonl"
	rm -rf target/plasma-pilot-smoke "$$log" "$$journal"
	mkdir -p target
	cargo run -p plasma-pilotd -- --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	cargo run -p plasma-pilot-cli -- --socket "$$socket" doctor
	cargo run -p plasma-pilot-cli -- --socket "$$socket" capabilities
	cargo run -p plasma-pilot-cli -- --socket "$$socket" policy-status
	cargo run -p plasma-pilot-cli -- --socket "$$socket" desktop-session-status
	cargo run -p plasma-pilot-cli -- --socket "$$socket" journal tail --limit 10
	test "$$(stat -c '%a' target/plasma-pilot-smoke)" = "700"
	test "$$(stat -c '%a' "$$socket")" = "600"
	test "$$(stat -c '%a' "$$journal")" = "600"

smoke-monitors:
	set -euo pipefail
	socket="/tmp/plasma-pilot-monitor-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-monitor-smoke-daemon.log"
	journal="target/plasma-pilot-monitor-smoke-journal.jsonl"
	rm -rf /tmp/plasma-pilot-monitor-smoke "$$log" "$$journal"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" monitors

smoke-windows:
	set -euo pipefail
	socket="/tmp/plasma-pilot-window-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-window-smoke-daemon.log"
	active_log="target/plasma-pilot-window-active.log"
	journal="target/plasma-pilot-window-smoke-journal.jsonl"
	rm -rf /tmp/plasma-pilot-window-smoke "$$log" "$$active_log" "$$journal"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" windows >/dev/null
	if target/debug/plasma-pilot-cli --socket "$$socket" active-window >"$$active_log" 2>&1; then
		grep -q '"type": "active_window"' "$$active_log"
	else
		grep -q "KWin script bridge" "$$active_log"
	fi

smoke-focus:
	set -euo pipefail
	socket="/tmp/plasma-pilot-focus-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-focus-smoke-daemon.log"
	journal="target/plasma-pilot-focus-smoke-journal.jsonl"
	approval_file="target/plasma-pilot-focus-smoke/approvals.jsonl"
	windows="target/plasma-pilot-focus-smoke-windows.json"
	focus="target/plasma-pilot-focus-smoke-action.json"
	rm -rf /tmp/plasma-pilot-focus-smoke target/plasma-pilot-focus-smoke "$$log" "$$journal" "$$windows" "$$focus"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" --approval-file "$$approval_file" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" approve --approval-file "$$approval_file" --safety-class control-semantic --method focus_window --ttl-ms 60000 --reason "smoke-focus" >/dev/null
	test "$$(stat -c '%a' "$$approval_file")" = "600"
	target/debug/plasma-pilot-cli --socket "$$socket" windows >"$$windows"
	match_id=$$(qdbus6 --literal org.kde.KWin /WindowsRunner org.kde.krunner1.Match "" | sed -n 's/.*(sssida{sv}) "\(0_{[^"]*}\)".*/\1/p' | head -n 1)
	if [[ -z "$$match_id" ]]; then
		echo "no KWin runner window id found"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" focus --window "$${match_id#0_}" >"$$focus"
	grep -q '"type": "action"' "$$focus"
	grep -q "focused window" "$$focus"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 | grep -q "focus_window"

smoke-clipboard:
	set -euo pipefail
	socket="/tmp/plasma-pilot-clipboard-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-clipboard-smoke-daemon.log"
	journal="target/plasma-pilot-clipboard-smoke-journal.jsonl"
	approval_file="target/plasma-pilot-clipboard-smoke/approvals.jsonl"
	previous_json="target/plasma-pilot-clipboard-previous.json"
	previous_text="target/plasma-pilot-clipboard-previous.txt"
	current_json="target/plasma-pilot-clipboard-current.json"
	set_result="target/plasma-pilot-clipboard-set.json"
	rm -rf /tmp/plasma-pilot-clipboard-smoke target/plasma-pilot-clipboard-smoke "$$log" "$$journal" "$$previous_json" "$$previous_text" "$$current_json" "$$set_result"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" --approval-file "$$approval_file" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		if [[ -f "$$previous_text" ]]; then
			target/debug/plasma-pilot-cli --socket "$$socket" clipboard set "$$(<"$$previous_text")" >/dev/null 2>&1 || true
		fi
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
		rm -f "$$previous_json" "$$previous_text" "$$current_json" "$$set_result"
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" approve --approval-file "$$approval_file" --safety-class clipboard-read --method clipboard_get --ttl-ms 60000 --reason "smoke-clipboard read" >/dev/null
	test "$$(stat -c '%a' "$$approval_file")" = "600"
	if target/debug/plasma-pilot-cli --socket "$$socket" clipboard get >"$$previous_json" 2>/dev/null; then
		jq -r '.data.text' "$$previous_json" >"$$previous_text"
	fi
	sentinel="plasma-pilot-clipboard-smoke-$$(date +%s)"
	target/debug/plasma-pilot-cli --socket "$$socket" clipboard set "$$sentinel" >"$$set_result"
	target/debug/plasma-pilot-cli --socket "$$socket" clipboard get >"$$current_json"
	jq -e --arg text "$$sentinel" '.type == "clipboard_text" and .data.text == $$text and (.data.backend | type == "string") and (.data.backend | length > 0)' "$$current_json" >/dev/null
	grep -q '"type": "action"' "$$set_result"
	grep -q "backend=" "$$set_result"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 | grep -q "clipboard"

smoke-atspi:
	set -euo pipefail
	socket="/tmp/plasma-pilot-atspi-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-atspi-smoke-daemon.log"
	journal="target/plasma-pilot-atspi-smoke-journal.jsonl"
	out="target/plasma-pilot-atspi-smoke.json"
	rm -rf /tmp/plasma-pilot-atspi-smoke "$$log" "$$journal" "$$out"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" atspi tree --focused --depth 1 --max-nodes 256 >"$$out"
	jq -e '.type == "accessibility_tree"' "$$out" >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" atspi find --role application --max-results 1 --max-nodes 128 >"$$out"
	jq -e '.type == "accessibility_matches" and (.data | length) >= 1' "$$out" >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" atspi find --role slider --max-results 1 --max-nodes 1500 >"$$out"
	jq -e '.type == "accessibility_matches" and (.data | length) >= 1 and .data[0].value != null' "$$out" >/dev/null
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi invoke --node atspi://:1.42/org/a11y/atspi/accessible/7 --action press >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi set-text --node atspi://:1.42/org/a11y/atspi/accessible/7 smoke-text >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi insert-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --offset 0 smoke-text >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi delete-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi copy-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi cut-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi paste-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --offset 0 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi text-attributes --node "" --offset 0 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "node_id must be non-empty" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic click-button --name OK --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic set-text-field --name Search smoke-text --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic focus-text-field --name Search --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic activate-tab --name General --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic activate-link --name Help --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic toggle-check --name Enable --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic set-value --name Volume --value 0.5 --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic select-item --name Printer --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" semantic select-menu --path File/Open --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "focused_accessibility_tree"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_find"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_invoke"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_set_text"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_insert_text"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_delete_text"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_copy_text"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_cut_text"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_paste_text"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_text_attributes"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "click_button"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "set_text_field"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "focus_text_field"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "activate_tab"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "activate_link"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "toggle_check"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "set_value"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "select_item"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "select_menu"

smoke-uinput-status:
	set -euo pipefail
	socket="/tmp/plasma-pilot-uinput-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-uinput-smoke-daemon.log"
	journal="target/plasma-pilot-uinput-smoke-journal.jsonl"
	out="target/plasma-pilot-uinput-smoke.json"
	rm -rf /tmp/plasma-pilot-uinput-smoke "$$log" "$$journal" "$$out"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" input status >"$$out"
	jq -e '.type == "uinput_status" and (.data.available | type == "boolean") and (.data.setup_hint | type == "string")' "$$out" >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" input backends >"$$out"
	jq -e '.type == "input_backend_status" and (.data.uinput_available | type == "boolean") and (.data.remote_desktop_portal.setup_hint | type == "string") and (.data.libei.setup_hint | type == "string")' "$$out" >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 | grep -q "uinput_status"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 | grep -q "input_backend_status"

smoke-capture-backends:
	set -euo pipefail
	socket="/tmp/plasma-pilot-capture-backends-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-capture-backends-smoke-daemon.log"
	journal="target/plasma-pilot-capture-backends-smoke-journal.jsonl"
	out="target/plasma-pilot-capture-backends-smoke.json"
	rm -rf /tmp/plasma-pilot-capture-backends-smoke "$$log" "$$journal" "$$out"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" capture-backends >"$$out"
	jq -e '.type == "capture_backend_status" and (.data.screenshot_portal.setup_hint | type == "string") and (.data.kwin_metadata.setup_hint | type == "string") and (.data.spectacle.setup_hint | type == "string") and (.data.setup_hint | type == "string")' "$$out" >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 | grep -q "capture_backend_status"

smoke-pointer-calibration:
	set -euo pipefail
	socket="/tmp/plasma-pilot-pointer-calibration-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-pointer-calibration-smoke-daemon.log"
	journal="target/plasma-pilot-pointer-calibration-smoke-journal.jsonl"
	out="target/plasma-pilot-pointer-calibration-smoke.json"
	rm -rf /tmp/plasma-pilot-pointer-calibration-smoke "$$log" "$$journal" "$$out"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" input pointer-calibration >"$$out"
	jq -e '.type == "pointer_calibration" and .data.coordinate_space == "physical_pixel" and (.data.monitors | length) >= 1 and (.data.sample_points | length) >= 3' "$$out" >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 | grep -q "pointer_calibration"

smoke-trace-replay:
	set -euo pipefail
	socket="/tmp/plasma-pilot-trace-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-trace-smoke-daemon.log"
	journal="target/plasma-pilot-trace-smoke-journal.jsonl"
	status_out="target/plasma-pilot-trace-status-smoke.json"
	denial_out="target/plasma-pilot-trace-denial-smoke.json"
	status_validate_out="target/plasma-pilot-trace-status-validate-smoke.json"
	denial_validate_out="target/plasma-pilot-trace-denial-validate-smoke.json"
	denied_capture="/tmp/plasma-pilot-denied-full-resolution.png"
	rm -rf /tmp/plasma-pilot-trace-smoke "$$log" "$$journal" "$$status_out" "$$denial_out" "$$status_validate_out" "$$denial_validate_out" "$$denied_capture"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilot-cli trace validate --file examples/traces/status-smoke.json >"$$status_validate_out"
	jq -e '.type == "trace_validation" and .trace_version == 1 and .step_count == 5 and any(.steps[]; .method == "safety_status")' "$$status_validate_out" >/dev/null
	target/debug/plasma-pilot-cli trace validate --file examples/traces/policy-denials-smoke.json >"$$denial_validate_out"
	jq -e '.type == "trace_validation" and .trace_version == 1 and .step_count == 3 and all(.steps[]; .expect_response_type == "error" and .expect_ok == false and (.expect_error_contains | type == "string"))' "$$denial_validate_out" >/dev/null
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
		rm -f "$$denied_capture"
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	target/debug/plasma-pilot-cli --socket "$$socket" trace replay --file examples/traces/status-smoke.json >"$$status_out"
	jq -e '.type == "trace_replay" and .trace_version == 1 and (.steps | length) == 5 and all(.steps[]; .ok == true) and any(.steps[]; .method == "safety_status")' "$$status_out" >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" trace replay --file examples/traces/policy-denials-smoke.json >"$$denial_out"
	jq -e '.type == "trace_replay" and .trace_version == 1 and (.steps | length) == 3 and all(.steps[]; .response_type == "error" and .ok == false) and any(.steps[]; .method == "focus_window")' "$$denial_out" >/dev/null
	test ! -e "$$denied_capture"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 --method safety_status --ok true | jq -e '.type == "journal" and (.data | length) >= 1' >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 --ok false | jq -e '.type == "journal" and (.data | length) >= 3' >/dev/null

smoke-gui-input:
	scripts/gui-input-smoke.sh text-editor

smoke-mcp:
	set -euo pipefail
	socket="/tmp/plasma-pilot-mcp-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-mcp-smoke-daemon.log"
	journal="target/plasma-pilot-mcp-smoke-journal.jsonl"
	out="target/plasma-pilot-mcp-smoke.jsonl"
	rm -rf /tmp/plasma-pilot-mcp-smoke "$$log" "$$journal" "$$out"
	cargo build -p plasma-pilotd -p plasma-pilot-mcp
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
	}
	trap cleanup EXIT
	for _ in {1..50}; do
		if [[ -S "$$socket" ]]; then
			break
		fi
		sleep 0.1
	done
	if [[ ! -S "$$socket" ]]; then
		cat "$$log"
		exit 1
	fi
	{
		printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"make-smoke","version":"0"}}}'
		printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
		printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
		printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"plasma.health","arguments":{}}}'
		printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"plasma.observe","arguments":{}}}'
		printf '%s\n' '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"plasma.a11y_text_attributes","arguments":{"node_id":"invalid-atspi-node","offset":0}}}'
	} | PLASMA_PILOT_SOCKET="$$socket" target/debug/plasma-pilot-mcp --stdio >"$$out"
	test "$$(wc -l <"$$out")" = "5"
	jq -e 'select(.id == 1) | .result.capabilities.tools.listChanged == false' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.list_windows")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.clipboard_get_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.clipboard_set_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.desktop_session_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.panic_stop_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.panic_stop_enable")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.panic_stop_disable")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.kwin_bridge_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.uinput_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.input_backend_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.capture_backend_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.pointer_calibration")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.type_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.key_combo")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.move_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.click_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.drag_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.scroll_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.click_button")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.set_text_field")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.focus_text_field")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.activate_tab")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.select_item")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.select_menu")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_focused_tree")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_find")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_text_attributes")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_invoke")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_set_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_insert_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_delete_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_copy_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_cut_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_paste_text")' "$$out" >/dev/null
	jq -e 'select(.id == 3) | .result.isError == false and .result.structuredContent.type == "health"' "$$out" >/dev/null
	jq -e 'select(.id == 4) | .result.isError == false and .result.structuredContent.type == "observation"' "$$out" >/dev/null
	jq -e 'select(.id == 5) | .result.isError == true and .result.structuredContent.type == "error" and (.result.structuredContent.data.message | contains("invalid AT-SPI node id")) and (.result.content[0].text | contains("invalid AT-SPI node id"))' "$$out" >/dev/null

gui-eval:
	scripts/gui-eval.sh all

gui-eval-control-safety:
	scripts/gui-eval.sh control-safety

install-kwin-script:
	set -euo pipefail
	if kpackagetool6 --type=KWin/Script --list | grep -q "plasma-pilot-bridge"; then
		kpackagetool6 --type=KWin/Script -u kwin/plasma-pilot-bridge
	else
		kpackagetool6 --type=KWin/Script -i kwin/plasma-pilot-bridge
	fi
	kwriteconfig6 --file kwinrc --group Plugins --key plasma-pilot-bridgeEnabled true
	qdbus6 org.kde.KWin /KWin reconfigure
