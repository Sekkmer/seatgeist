SHELL := /usr/bin/bash
.ONESHELL:

.PHONY: fmt check test clippy validate-plugin validate-install-assets validate-traces verify smoke smoke-monitors smoke-windows smoke-focus smoke-clipboard smoke-atspi smoke-uinput-status smoke-capture-backends smoke-pointer-calibration smoke-human-input-pause smoke-trace-replay smoke-gui-input smoke-mcp gui-eval gui-eval-status gui-eval-session-preflight gui-eval-observe gui-eval-a11y-quality-status gui-eval-a11y-focused-tree gui-eval-a11y-find gui-eval-clipboard-status gui-eval-clipboard-denied gui-eval-kwin-bridge-status gui-eval-keymap-status gui-eval-screenshot-preview gui-eval-screenshot-coordinate-map gui-eval-screenshot-config-bounds gui-eval-journal-artifacts gui-eval-full-resolution-denied gui-eval-control-safety gui-eval-text-editor-input gui-eval-kcalc-visual gui-eval-firefox-localhost-button gui-eval-portal-screenshot gui-eval-remote-desktop-probe gui-eval-remote-desktop-eis-session install-kwin-script

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

validate-install-assets:
	scripts/validate-install-assets.py

validate-traces:
	cargo build -p plasma-pilot-cli
	target/debug/plasma-pilot-cli trace validate --dir examples/traces >/dev/null

verify: fmt check test clippy validate-plugin validate-install-assets validate-traces smoke smoke-uinput-status smoke-capture-backends smoke-pointer-calibration smoke-human-input-pause smoke-trace-replay smoke-mcp gui-eval-status gui-eval-session-preflight gui-eval-observe gui-eval-a11y-quality-status gui-eval-a11y-focused-tree gui-eval-a11y-find gui-eval-clipboard-status gui-eval-clipboard-denied gui-eval-full-resolution-denied gui-eval-kwin-bridge-status gui-eval-keymap-status gui-eval-control-safety
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
	cargo run -p plasma-pilot-cli -- --socket "$$socket" readiness
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
	status_json="target/plasma-pilot-clipboard-status.json"
	set_result="target/plasma-pilot-clipboard-set.json"
	rm -rf /tmp/plasma-pilot-clipboard-smoke target/plasma-pilot-clipboard-smoke "$$log" "$$journal" "$$previous_json" "$$previous_text" "$$current_json" "$$status_json" "$$set_result"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" --approval-file "$$approval_file" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		if [[ -f "$$previous_text" ]]; then
			target/debug/plasma-pilot-cli --socket "$$socket" clipboard set "$$(<"$$previous_text")" >/dev/null 2>&1 || true
		fi
		kill "$$pid" 2>/dev/null || true
		wait "$$pid" 2>/dev/null || true
		rm -f "$$previous_json" "$$previous_text" "$$current_json" "$$status_json" "$$set_result"
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
	target/debug/plasma-pilot-cli --socket "$$socket" clipboard status >"$$status_json"
	jq -e '.type == "clipboard_backend_status" and (.data.read_backend == null or (.data.read_backend | type == "string")) and (.data.write_backend == null or (.data.write_backend | type == "string")) and (.data.setup_hint | type == "string")' "$$status_json" >/dev/null
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
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi set-caret --node atspi://:1.42/org/a11y/atspi/accessible/7 --offset 0 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/plasma-pilot-cli --socket "$$socket" atspi set-selection --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
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
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_set_caret"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_set_selection"
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
	jq -e '.type == "input_backend_status" and (.data.uinput_available | type == "boolean") and ((.data.implemented_available_backend == null) or (.data.implemented_available_backend == "uinput")) and (.data.remote_desktop_portal.setup_hint | type == "string") and (.data.libei.setup_hint | type == "string") and (.data.eis_keymap.source | type == "string") and (.data.eis_keymap.setup_hint | type == "string")' "$$out" >/dev/null
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
	jq -e '.type == "capture_backend_status" and ((.data.implemented_available_backend == null) or (.data.implemented_available_backend == "spectacle") or (.data.implemented_available_backend == "portal_screenshot")) and (.data.screenshot_portal.setup_hint | type == "string") and (.data.kwin_metadata.setup_hint | type == "string") and (.data.spectacle.setup_hint | type == "string") and (.data.setup_hint | type == "string")' "$$out" >/dev/null
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

smoke-human-input-pause:
	set -euo pipefail
	socket="/tmp/plasma-pilot-human-input-pause-smoke/plasma-pilotd.sock"
	run_dir="target/plasma-pilot-human-input-pause-smoke"
	log="target/plasma-pilot-human-input-pause-smoke-daemon.log"
	journal="target/plasma-pilot-human-input-pause-smoke-journal.jsonl"
	config="$$run_dir/config.toml"
	approval_file="$$run_dir/approvals.jsonl"
	activity_file="$$run_dir/human-input-active"
	status_json="$$run_dir/safety-status.json"
	approval_json="$$run_dir/approval.json"
	denied_out="$$run_dir/focus-denied.txt"
	rm -rf /tmp/plasma-pilot-human-input-pause-smoke "$$run_dir" "$$log" "$$journal"
	mkdir -p "$$run_dir"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	printf '[daemon]\napproval_file = "%s"\n\n[safety]\nrequire_focus_guard = false\npause_on_human_input = true\nhuman_input_activity_file = "%s"\nhuman_input_quiet_ms = 60000\n' "$$(pwd)/$$approval_file" "$$(pwd)/$$activity_file" >"$$config"
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" --config "$$config" >"$$log" 2>&1 &
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
	target/debug/plasma-pilot-cli --socket "$$socket" approve --approval-file "$$approval_file" --safety-class control-semantic --method focus_window --ttl-ms 60000 --reason "human-input-pause smoke" >"$$approval_json"
	test "$$(stat -c '%a' "$$approval_file")" = "600"
	: >"$$activity_file"
	target/debug/plasma-pilot-cli --socket "$$socket" safety-status >"$$status_json"
	jq -e '.type == "safety_status" and .data.pause_on_human_input == true and .data.human_input_signal_fresh == true and .data.human_input_quiet_ms == 60000' "$$status_json" >/dev/null
	if target/debug/plasma-pilot-cli --socket "$$socket" focus --window "__plasma_pilot_human_pause_never__" >"$$denied_out" 2>&1; then
		cat "$$denied_out"
		exit 1
	fi
	grep -q "human input activity signal is fresh" "$$denied_out"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 --method focus_window --ok false | jq -e '.type == "journal" and (.data | length) >= 1 and all(.data[]; .summary | contains("human input activity signal is fresh"))' >/dev/null

smoke-trace-replay:
	set -euo pipefail
	socket="/tmp/plasma-pilot-trace-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-trace-smoke-daemon.log"
	journal="target/plasma-pilot-trace-smoke-journal.jsonl"
	validate_out="target/plasma-pilot-trace-validate-smoke.json"
	replay_out="target/plasma-pilot-trace-replay-smoke.json"
	denied_capture="/tmp/plasma-pilot-denied-full-resolution.png"
	rm -rf /tmp/plasma-pilot-trace-smoke "$$log" "$$journal" "$$validate_out" "$$replay_out" "$$denied_capture"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilot-cli trace validate --dir examples/traces >"$$validate_out"
	jq -e '.type == "trace_validation_set" and .trace_count >= 5 and .step_count >= 36 and any(.traces[]; (.file | endswith("status-smoke.json")) and .step_count == 14 and any(.steps[]; .method == "safety_status" and .expect_json_count == 1) and any(.steps[]; .method == "computer_use_readiness" and .expect_json_count == 6) and any(.steps[]; .method == "accessibility_quality_status" and .expect_json_count == 4) and any(.steps[]; .method == "kwin_bridge_status") and any(.steps[]; .method == "uinput_status") and any(.steps[]; .method == "capture_backend_status") and any(.steps[]; .method == "clipboard_backend_status" and .expect_json_count == 6) and any(.steps[]; .method == "input_backend_status") and any(.steps[]; .method == "remote_desktop_eis_session_status") and any(.steps[]; .method == "remote_desktop_eis_stop")) and any(.traces[]; (.file | endswith("journal-tail-smoke.json")) and .step_count == 3 and any(.steps[]; .method == "journal_tail" and .expect_json_count == 8)) and any(.traces[]; (.file | endswith("policy-denials-smoke.json")) and .step_count == 5 and all(.steps[]; .expect_response_type == "error" and .expect_ok == false and (.expect_error_contains | type == "string") and .expect_json_count == 1) and any(.steps[]; .method == "accessibility_set_caret") and any(.steps[]; .method == "accessibility_set_selection")) and any(.traces[]; (.file | endswith("input-denials-smoke.json")) and .step_count == 9 and all(.steps[]; .expect_response_type == "error" and .expect_ok == false and (.expect_error_contains | test("Control(Keyboard|Pointer)")) and .expect_json_count == 1) and any(.steps[]; .method == "remote_desktop_session_probe") and any(.steps[]; .method == "remote_desktop_eis_probe") and any(.steps[]; .method == "remote_desktop_eis_start")) and any(.traces[]; (.file | endswith("panic-stop-smoke.json")) and .step_count == 5 and all(.steps[]; .expect_json_count == 1))' "$$validate_out" >/dev/null
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
	target/debug/plasma-pilot-cli --socket "$$socket" trace replay --dir examples/traces >"$$replay_out"
	jq -e '.type == "trace_replay_set" and .trace_count >= 5 and .step_count >= 36 and any(.traces[]; (.file | endswith("status-smoke.json")) and (.steps | length) == 14 and all(.steps[]; .ok == true) and any(.steps[]; .method == "safety_status") and any(.steps[]; .method == "computer_use_readiness" and .response_type == "computer_use_readiness") and any(.steps[]; .method == "accessibility_quality_status" and .response_type == "accessibility_quality_status") and any(.steps[]; .method == "kwin_bridge_status") and any(.steps[]; .method == "uinput_status") and any(.steps[]; .method == "capture_backend_status") and any(.steps[]; .method == "clipboard_backend_status" and .response_type == "clipboard_backend_status") and any(.steps[]; .method == "input_backend_status") and any(.steps[]; .method == "remote_desktop_eis_session_status") and any(.steps[]; .method == "remote_desktop_eis_stop")) and any(.traces[]; (.file | endswith("journal-tail-smoke.json")) and (.steps | length) == 3 and all(.steps[]; .ok == true) and any(.steps[]; .method == "journal_tail" and .response_type == "journal")) and any(.traces[]; (.file | endswith("policy-denials-smoke.json")) and (.steps | length) == 5 and all(.steps[]; .response_type == "error" and .ok == false and .error_kind == "policy_prompt_required") and any(.steps[]; .method == "focus_window") and any(.steps[]; .method == "accessibility_set_caret") and any(.steps[]; .method == "accessibility_set_selection")) and any(.traces[]; (.file | endswith("input-denials-smoke.json")) and (.steps | length) == 9 and all(.steps[]; .response_type == "error" and .ok == false and .error_kind == "policy_prompt_required") and any(.steps[]; .method == "type_text") and any(.steps[]; .method == "click_pointer") and any(.steps[]; .method == "remote_desktop_session_probe") and any(.steps[]; .method == "remote_desktop_eis_probe") and any(.steps[]; .method == "remote_desktop_eis_start")) and any(.traces[]; (.file | endswith("panic-stop-smoke.json")) and (.steps | length) == 5 and all(.steps[]; .response_type == "panic_stop" and .ok == true) and any(.steps[]; .method == "set_panic_stop"))' "$$replay_out" >/dev/null
	test ! -e "$$denied_capture"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 --method safety_status --ok true | jq -e '.type == "journal" and (.data | length) >= 1' >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 --ok false | jq -e '.type == "journal" and (.data | length) >= 3' >/dev/null
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 --method set_panic_stop --ok true | jq -e '.type == "journal" and (.data | length) >= 2' >/dev/null

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
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.clipboard_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.clipboard_get_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.clipboard_set_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.desktop_session_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.computer_use_readiness")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.panic_stop_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.panic_stop_enable")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.panic_stop_disable")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.kwin_bridge_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.uinput_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.input_backend_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.remote_desktop_session_probe")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.remote_desktop_eis_probe")' "$$out" >/dev/null
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
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_quality_status")' "$$out" >/dev/null
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
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_set_caret")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.a11y_set_selection")' "$$out" >/dev/null
	jq -e 'select(.id == 3) | .result.isError == false and .result.structuredContent.type == "health"' "$$out" >/dev/null
	jq -e 'select(.id == 4) | .result.isError == false and .result.structuredContent.type == "observation"' "$$out" >/dev/null
	jq -e 'select(.id == 5) | .result.isError == true and .result.structuredContent.type == "error" and .result.structuredContent.data.kind == "accessibility_unavailable" and (.result.structuredContent.data.message | contains("invalid AT-SPI node id")) and (.result.content[0].text | contains("invalid AT-SPI node id"))' "$$out" >/dev/null

gui-eval:
	scripts/gui-eval.sh all

gui-eval-status:
	scripts/gui-eval.sh status

gui-eval-session-preflight:
	scripts/gui-eval.sh session-preflight

gui-eval-observe:
	scripts/gui-eval.sh observe

gui-eval-a11y-quality-status:
	scripts/gui-eval.sh a11y-quality-status

gui-eval-a11y-focused-tree:
	scripts/gui-eval.sh a11y-focused-tree

gui-eval-a11y-find:
	scripts/gui-eval.sh a11y-find

gui-eval-clipboard-status:
	scripts/gui-eval.sh clipboard-status

gui-eval-clipboard-denied:
	scripts/gui-eval.sh clipboard-denied

gui-eval-kwin-bridge-status:
	scripts/gui-eval.sh kwin-bridge-status

gui-eval-keymap-status:
	scripts/gui-eval.sh keymap-status

gui-eval-screenshot-preview:
	scripts/gui-eval.sh screenshot-preview

gui-eval-screenshot-coordinate-map:
	scripts/gui-eval.sh screenshot-coordinate-map

gui-eval-screenshot-config-bounds:
	scripts/gui-eval.sh screenshot-config-bounds

gui-eval-journal-artifacts:
	scripts/gui-eval.sh journal-artifacts

gui-eval-full-resolution-denied:
	scripts/gui-eval.sh full-resolution-denied

gui-eval-control-safety:
	scripts/gui-eval.sh control-safety

gui-eval-text-editor-input:
	scripts/gui-input-smoke.sh text-editor

gui-eval-kcalc-visual:
	scripts/gui-calculator-smoke.sh kcalc

gui-eval-firefox-localhost-button:
	scripts/gui-browser-smoke.sh firefox-localhost-button

gui-eval-portal-screenshot:
	scripts/gui-eval.sh portal-screenshot

gui-eval-remote-desktop-probe:
	scripts/gui-eval.sh remote-desktop-probe

gui-eval-remote-desktop-eis-session:
	scripts/gui-eval.sh remote-desktop-eis-session

install-kwin-script:
	set -euo pipefail
	if kpackagetool6 --type=KWin/Script --list | grep -q "plasma-pilot-bridge"; then
		kpackagetool6 --type=KWin/Script -u kwin/plasma-pilot-bridge
	else
		kpackagetool6 --type=KWin/Script -i kwin/plasma-pilot-bridge
	fi
	kwriteconfig6 --file kwinrc --group Plugins --key plasma-pilot-bridgeEnabled true
	qdbus6 org.kde.KWin /KWin reconfigure
