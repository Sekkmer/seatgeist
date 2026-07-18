SHELL := /usr/bin/bash
.ONESHELL:

.PHONY: fmt check test clippy check-kwin-activity-plugin validate-kwin-bridge validate-plugin validate-install-assets validate-release validate-computer-use-baseline verify-cooperative-use-acceptance package-release verify-release-artifacts verify-release-install sign-release-artifacts verify-release-signatures write-release-evidence verify-release-evidence check-public-name check-local-codex-install release-readiness release-external-preflight release-live-evals portal-screenshot-v3-status deploy-user-daemon validate-traces verify smoke smoke-codex-plugin-install smoke-monitors smoke-windows smoke-focus smoke-clipboard smoke-atspi smoke-uinput-status smoke-capture-backends smoke-pointer-calibration smoke-human-input-pause smoke-trace-replay smoke-gui-input smoke-mcp gui-eval gui-eval-status gui-eval-session-preflight gui-eval-observe gui-eval-a11y-quality-status gui-eval-a11y-focused-tree gui-eval-a11y-find gui-eval-a11y-text-attributes gui-eval-a11y-control-denied gui-eval-semantic-denied gui-eval-input-denied gui-eval-clipboard-status gui-eval-clipboard-denied gui-eval-kwin-bridge-status gui-eval-keymap-status gui-eval-screenshot-preview gui-eval-screenshot-coordinate-map gui-eval-screenshot-config-bounds gui-eval-journal-artifacts gui-eval-full-resolution-denied gui-eval-control-safety gui-eval-text-editor-input gui-eval-kcalc-visual gui-eval-firefox-localhost-button gui-eval-portal-screenshot gui-eval-remote-desktop-probe gui-eval-remote-desktop-eis-session install-kwin-script
.PHONY: kwin-activity-preflight install-kwin-activity-user uninstall-kwin-activity-user probe-nested-kde-multi-output probe-nested-seatgeist probe-nested-remote-desktop probe-nested-retained-apps gui-eval-nested-retained-capture gui-eval-nested-eis-isolation gui-eval-cooperative-sticky gui-eval-retained-capture gui-eval-capture-restore-prepare gui-eval-capture-restore-resume gui-eval-capture-revocation gui-eval-target-reopen gui-eval-background-semantic
.PHONY: refresh-local-codex-plugin

fmt:
	cargo fmt --all

check:
	cargo check --workspace --all-targets

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

check-kwin-activity-plugin:
	set -euo pipefail
	cmake -S kwin/seatgeist-activity -B target/kwin-seatgeist-activity -DCMAKE_BUILD_TYPE=RelWithDebInfo
	cmake --build target/kwin-seatgeist-activity --parallel

validate-kwin-bridge:
	set -euo pipefail
	scripts/test-kwin-bridge.js
	scripts/test-install-kwin-bridge.py

validate-plugin:
	scripts/validate-plugin.py plugin

validate-install-assets:
	scripts/validate-install-assets.py

validate-release:
	scripts/validate-release.py

validate-computer-use-baseline: validate-kwin-bridge
	set -euo pipefail
	scripts/test-computer-use-eval.py
	scripts/test-computer-use-baseline.py
	scripts/test-check-local-codex-install.py
	scripts/test-deploy-seatgeistd-user.py
	scripts/test-install-kwin-screenshot-authorization.py
	scripts/test-plugin-hook-resolution.py
	scripts/test-kwin-activity-preflight.py
	scripts/test-kwin-activity-abi-watch.py
	scripts/test-install-kwin-activity-user.py
	scripts/test-cooperative-sticky-eval.py
	scripts/test-retained-capture-eval.py
	scripts/test-capture-restore-eval.py
	scripts/test-capture-lifecycle-eval.py
	scripts/test-target-reopen-eval.py
	scripts/test-background-semantic-eval.py
	scripts/test-cooperative-use-acceptance.py
	scripts/test-nested-kde-fixture.py
	scripts/test-nested-seatgeist-probe.py
	scripts/test-nested-remote-desktop-probe.py
	scripts/test-nested-retained-capture-eval.py

verify-cooperative-use-acceptance:
	set -euo pipefail
	required=(
		RETAINED_CAPTURE_EVIDENCE
		MULTI_OUTPUT_EVIDENCE
		CAPTURE_RESTORE_EVIDENCE
		CAPTURE_REVOCATION_EVIDENCE
		TARGET_REOPEN_EVIDENCE
		BACKGROUND_FIREFOX_EVIDENCE
		BACKGROUND_KDE_EVIDENCE
		COOPERATIVE_STICKY_EVIDENCE
	)
	for name in "$${required[@]}"; do
		if [[ -z "$${!name:-}" ]]; then
			echo "set $$name to its private Step 12 evidence JSON file" >&2
			exit 2
		fi
	done
	args=(
		"--retained-capture" "$$RETAINED_CAPTURE_EVIDENCE"
		"--retained-capture-multi-output" "$$MULTI_OUTPUT_EVIDENCE"
		"--capture-restore-restart" "$$CAPTURE_RESTORE_EVIDENCE"
		"--capture-revocation" "$$CAPTURE_REVOCATION_EVIDENCE"
		"--target-reopen" "$$TARGET_REOPEN_EVIDENCE"
		"--background-semantic-firefox" "$$BACKGROUND_FIREFOX_EVIDENCE"
		"--background-semantic-kde" "$$BACKGROUND_KDE_EVIDENCE"
		"--cooperative-sticky" "$$COOPERATIVE_STICKY_EVIDENCE"
		"--max-age-hours" "$${ACCEPTANCE_MAX_AGE_HOURS:-24}"
		"--max-span-hours" "$${ACCEPTANCE_MAX_SPAN_HOURS:-24}"
	)
	if [[ -n "$${ACCEPTANCE_OUTPUT:-}" ]]; then
		args+=(--output "$$ACCEPTANCE_OUTPUT")
	fi
	scripts/cooperative-use-acceptance.py "$${args[@]}"

kwin-activity-preflight: check-kwin-activity-plugin
	scripts/kwin-activity-preflight.py

install-kwin-activity-user: check-kwin-activity-plugin
	scripts/install-kwin-activity-user.py

uninstall-kwin-activity-user:
	scripts/install-kwin-activity-user.py --remove

probe-nested-kde-multi-output:
	scripts/nested-kde-fixture.py

probe-nested-seatgeist:
	set -euo pipefail
	cargo build -p seatgeistd -p seatgeist-cli
	scripts/nested-kde-fixture.py -- scripts/nested-seatgeist-probe.py

probe-nested-remote-desktop:
	scripts/nested-kde-fixture.py -- scripts/nested-remote-desktop-probe.py

probe-nested-retained-apps:
	set -euo pipefail
	cargo build -p seatgeistd -p seatgeist-cli
	scripts/nested-kde-fixture.py -- scripts/nested-retained-capture-eval.py --probe-only

gui-eval-nested-retained-capture:
	set -euo pipefail
	test "$${I_AM_PRESENT:-0}" = "1" || { echo "set I_AM_PRESENT=1 with the operator at the desktop" >&2; exit 2; }
	cargo build -p seatgeistd -p seatgeist-cli
	scenario_args=()
	if [[ -n "$${SCENARIO:-}" ]]; then scenario_args+=(--scenario "$$SCENARIO"); fi
	scripts/nested-kde-fixture.py --visible --operator-present -- scripts/nested-retained-capture-eval.py "$${scenario_args[@]}"

gui-eval-nested-eis-isolation:
	set -euo pipefail
	test "$${I_AM_PRESENT:-0}" = "1" || { echo "set I_AM_PRESENT=1 with the operator at the desktop" >&2; exit 2; }
	cargo build -p seatgeistd -p seatgeist-cli
	scripts/nested-kde-fixture.py --visible --operator-present -- scripts/nested-eis-isolation-fixture.sh

gui-eval-cooperative-sticky: check-kwin-activity-plugin
	set -euo pipefail
	test -n "$${WINDOW_ID:-}" || { echo "set WINDOW_ID to the exact Firefox KWin window id" >&2; exit 2; }
	scripts/cooperative-sticky-eval.py --window-id "$$WINDOW_ID"

gui-eval-retained-capture:
	set -euo pipefail
	test -n "$${WINDOW_ID:-}" || { echo "set WINDOW_ID to the exact approved KWin window id" >&2; exit 2; }
	cargo build -p seatgeist-cli
	extra_args=()
	if [[ "$${REQUIRE_MULTI_OUTPUT_NONZERO_ORIGIN:-0}" == "1" ]]; then extra_args+=(--require-multi-output-nonzero-origin); fi
	scripts/retained-capture-eval.py --window-id "$$WINDOW_ID" "$${extra_args[@]}"

gui-eval-capture-restore-prepare:
	set -euo pipefail
	test -n "$${WINDOW_ID:-}" || { echo "set WINDOW_ID to the exact approved KWin window id" >&2; exit 2; }
	cargo build -p seatgeist-cli
	restore_args=()
	if [[ -n "$${RESTORE_FILE:-}" ]]; then restore_args+=(--restore-file "$$RESTORE_FILE"); fi
	scripts/capture-restore-eval.py prepare --window-id "$$WINDOW_ID" "$${restore_args[@]}"

gui-eval-capture-restore-resume:
	set -euo pipefail
	test -n "$${WINDOW_ID:-}" || { echo "set WINDOW_ID to the same approved KWin window id" >&2; exit 2; }
	test -n "$${EVIDENCE_DIR:-}" || { echo "set EVIDENCE_DIR to the prepare-phase artifact directory" >&2; exit 2; }
	cargo build -p seatgeist-cli
	restore_args=()
	if [[ -n "$${RESTORE_FILE:-}" ]]; then restore_args+=(--restore-file "$$RESTORE_FILE"); fi
	scripts/capture-restore-eval.py resume --window-id "$$WINDOW_ID" --output-dir "$$EVIDENCE_DIR" "$${restore_args[@]}"

gui-eval-capture-revocation:
	set -euo pipefail
	test -n "$${WINDOW_ID:-}" || { echo "set WINDOW_ID to the exact approved KWin window id" >&2; exit 2; }
	cargo build -p seatgeist-cli
	scripts/capture-lifecycle-eval.py --window-id "$$WINDOW_ID"

gui-eval-target-reopen:
	set -euo pipefail
	test -n "$${WINDOW_ID:-}" || { echo "set WINDOW_ID to the exact original KWin window id" >&2; exit 2; }
	cargo build -p seatgeist-cli
	scripts/target-reopen-eval.py --window-id "$$WINDOW_ID"

gui-eval-background-semantic:
	set -euo pipefail
	test -n "$${SCENARIO:-}" || { echo "set SCENARIO to firefox or kde" >&2; exit 2; }
	test -n "$${TARGET_WINDOW_ID:-}" || { echo "set TARGET_WINDOW_ID to the background target" >&2; exit 2; }
	test -n "$${USER_WINDOW_ID:-}" || { echo "set USER_WINDOW_ID to the work window that must stay active" >&2; exit 2; }
	test -n "$${BUTTON_NAME:-}" || { echo "set BUTTON_NAME to one safe accessible button" >&2; exit 2; }
	cargo build -p seatgeist-cli
	extra_args=()
	if [[ -n "$${APP_FILTER:-}" ]]; then extra_args+=(--app-filter "$$APP_FILTER"); fi
	if [[ -n "$${APPROVAL_FILE:-}" ]]; then extra_args+=(--approval-file "$$APPROVAL_FILE"); fi
	scripts/background-semantic-eval.py --scenario "$$SCENARIO" --target-window-id "$$TARGET_WINDOW_ID" --user-window-id "$$USER_WINDOW_ID" --button-name "$$BUTTON_NAME" "$${extra_args[@]}"

package-release:
	scripts/package-release.sh

verify-release-artifacts: package-release
	scripts/verify-release-artifacts.py

verify-release-install: verify-release-artifacts
	scripts/verify-release-install.sh

sign-release-artifacts: verify-release-artifacts
	scripts/sign-release-artifacts.sh

verify-release-signatures:
	scripts/verify-release-signatures.sh

write-release-evidence:
	scripts/write-release-evidence.sh

verify-release-evidence:
	scripts/verify-release-evidence.py

check-public-name:
	scripts/check-public-name.py

check-local-codex-install:
	scripts/check-local-codex-install.py --strict

refresh-local-codex-plugin:
	set -euo pipefail
	skill_root="$${SEATGEIST_PLUGIN_CREATOR_SKILL_ROOT:-$$HOME/.codex/skills/.system/plugin-creator}"
	python3 "$$skill_root/scripts/update_plugin_cachebuster.py" plugin
	codex plugin add seatgeist@seatgeist-local --json
	scripts/check-local-codex-install.py --strict

release-readiness:
	scripts/release-readiness.py

release-external-preflight:
	scripts/release-external-preflight.py

release-live-evals:
	scripts/run-release-live-evals.sh

portal-screenshot-v3-status:
	scripts/portal-screenshot-v3-status.py

deploy-user-daemon:
	set -euo pipefail
	scripts/install-kwin-screenshot-authorization.py
	scripts/deploy-seatgeistd-user.py

validate-traces:
	set -euo pipefail
	cargo build -p seatgeist-cli
	target/debug/seatgeist-cli trace validate --dir examples/traces >/dev/null

verify: fmt check test clippy check-kwin-activity-plugin validate-plugin validate-install-assets validate-release validate-computer-use-baseline validate-traces smoke smoke-uinput-status smoke-capture-backends smoke-pointer-calibration smoke-human-input-pause smoke-trace-replay smoke-mcp gui-eval-status gui-eval-session-preflight gui-eval-observe gui-eval-a11y-quality-status gui-eval-a11y-focused-tree gui-eval-a11y-find gui-eval-a11y-text-attributes gui-eval-a11y-control-denied gui-eval-semantic-denied gui-eval-input-denied gui-eval-clipboard-status gui-eval-clipboard-denied gui-eval-full-resolution-denied gui-eval-kwin-bridge-status gui-eval-keymap-status gui-eval-control-safety
	git diff --check -- . ':(exclude)target'

smoke:
	set -euo pipefail
	socket="target/seatgeist-smoke/seatgeistd.sock"
	log="target/seatgeist-smoke-daemon.log"
	journal="target/seatgeist-smoke-journal.jsonl"
	rm -rf target/seatgeist-smoke "$$log" "$$journal"
	mkdir -p target
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" doctor
	target/debug/seatgeist-cli --socket "$$socket" capabilities
	target/debug/seatgeist-cli --socket "$$socket" policy-status
	target/debug/seatgeist-cli --socket "$$socket" desktop-session-status
	target/debug/seatgeist-cli --socket "$$socket" readiness
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10
	test "$$(stat -c '%a' target/seatgeist-smoke)" = "700"
	test "$$(stat -c '%a' "$$socket")" = "600"
	test "$$(stat -c '%a' "$$journal")" = "600"

smoke-codex-plugin-install:
	scripts/smoke-codex-plugin-install.sh

smoke-monitors:
	set -euo pipefail
	socket="/tmp/seatgeist-monitor-smoke/seatgeistd.sock"
	log="target/seatgeist-monitor-smoke-daemon.log"
	journal="target/seatgeist-monitor-smoke-journal.jsonl"
	rm -rf /tmp/seatgeist-monitor-smoke "$$log" "$$journal"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" monitors

smoke-windows:
	set -euo pipefail
	socket="/tmp/seatgeist-window-smoke/seatgeistd.sock"
	log="target/seatgeist-window-smoke-daemon.log"
	active_log="target/seatgeist-window-active.log"
	journal="target/seatgeist-window-smoke-journal.jsonl"
	rm -rf /tmp/seatgeist-window-smoke "$$log" "$$active_log" "$$journal"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" windows >/dev/null
	if target/debug/seatgeist-cli --socket "$$socket" active-window >"$$active_log" 2>&1; then
		grep -q '"type": "active_window"' "$$active_log"
	else
		grep -q "KWin script bridge" "$$active_log"
	fi

smoke-focus:
	set -euo pipefail
	socket="/tmp/seatgeist-focus-smoke/seatgeistd.sock"
	log="target/seatgeist-focus-smoke-daemon.log"
	journal="target/seatgeist-focus-smoke-journal.jsonl"
	approval_file="target/seatgeist-focus-smoke/approvals.jsonl"
	windows="target/seatgeist-focus-smoke-windows.json"
	focus="target/seatgeist-focus-smoke-action.json"
	rm -rf /tmp/seatgeist-focus-smoke target/seatgeist-focus-smoke "$$log" "$$journal" "$$windows" "$$focus"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" --approval-file "$$approval_file" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" approve --approval-file "$$approval_file" --safety-class control-semantic --method focus_window --ttl-ms 60000 --reason "smoke-focus" >/dev/null
	test "$$(stat -c '%a' "$$approval_file")" = "600"
	target/debug/seatgeist-cli --socket "$$socket" windows >"$$windows"
	match_id=$$(qdbus6 --literal org.kde.KWin /WindowsRunner org.kde.krunner1.Match "" | sed -n 's/.*(sssida{sv}) "\(0_{[^"]*}\)".*/\1/p' | head -n 1)
	if [[ -z "$$match_id" ]]; then
		echo "no KWin runner window id found"
		exit 1
	fi
	target/debug/seatgeist-cli --socket "$$socket" focus --window "$${match_id#0_}" >"$$focus"
	grep -q '"type": "action"' "$$focus"
	grep -q "focused window" "$$focus"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 | grep -q "focus_window"

smoke-clipboard:
	set -euo pipefail
	socket="/tmp/seatgeist-clipboard-smoke/seatgeistd.sock"
	log="target/seatgeist-clipboard-smoke-daemon.log"
	journal="target/seatgeist-clipboard-smoke-journal.jsonl"
	approval_file="target/seatgeist-clipboard-smoke/approvals.jsonl"
	previous_json="target/seatgeist-clipboard-previous.json"
	previous_text="target/seatgeist-clipboard-previous.txt"
	current_json="target/seatgeist-clipboard-current.json"
	status_json="target/seatgeist-clipboard-status.json"
	set_result="target/seatgeist-clipboard-set.json"
	rm -rf /tmp/seatgeist-clipboard-smoke target/seatgeist-clipboard-smoke "$$log" "$$journal" "$$previous_json" "$$previous_text" "$$current_json" "$$status_json" "$$set_result"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" --approval-file "$$approval_file" >"$$log" 2>&1 &
	pid=$$!
	cleanup() {
		if [[ -f "$$previous_text" ]]; then
			target/debug/seatgeist-cli --socket "$$socket" clipboard set "$$(<"$$previous_text")" >/dev/null 2>&1 || true
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
	target/debug/seatgeist-cli --socket "$$socket" approve --approval-file "$$approval_file" --safety-class clipboard-read --method clipboard_get --ttl-ms 60000 --reason "smoke-clipboard read" >/dev/null
	test "$$(stat -c '%a' "$$approval_file")" = "600"
	target/debug/seatgeist-cli --socket "$$socket" clipboard status >"$$status_json"
	jq -e '.type == "clipboard_backend_status" and (.data.read_backend == null or (.data.read_backend | type == "string")) and (.data.write_backend == null or (.data.write_backend | type == "string")) and (.data.setup_hint | type == "string")' "$$status_json" >/dev/null
	if target/debug/seatgeist-cli --socket "$$socket" clipboard get >"$$previous_json" 2>/dev/null; then
		jq -r '.data.text' "$$previous_json" >"$$previous_text"
	fi
	sentinel="seatgeist-clipboard-smoke-$$(date +%s)"
	target/debug/seatgeist-cli --socket "$$socket" clipboard set "$$sentinel" >"$$set_result"
	target/debug/seatgeist-cli --socket "$$socket" clipboard get >"$$current_json"
	jq -e --arg text "$$sentinel" '.type == "clipboard_text" and .data.text == $$text and (.data.backend | type == "string") and (.data.backend | length > 0)' "$$current_json" >/dev/null
	grep -q '"type": "action"' "$$set_result"
	grep -q "backend=" "$$set_result"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 | grep -q "clipboard"

smoke-atspi:
	set -euo pipefail
	socket="/tmp/seatgeist-atspi-smoke/seatgeistd.sock"
	log="target/seatgeist-atspi-smoke-daemon.log"
	journal="target/seatgeist-atspi-smoke-journal.jsonl"
	out="target/seatgeist-atspi-smoke.json"
	rm -rf /tmp/seatgeist-atspi-smoke "$$log" "$$journal" "$$out"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" atspi tree --focused --depth 1 --max-nodes 256 >"$$out"
	jq -e '.type == "accessibility_tree"' "$$out" >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" atspi find --role application --max-results 1 --max-nodes 128 >"$$out"
	jq -e '.type == "accessibility_matches" and (.data | length) >= 1' "$$out" >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" atspi find --role slider --max-results 1 --max-nodes 1500 >"$$out"
	jq -e '.type == "accessibility_matches" and (.data | length) >= 1 and .data[0].value != null' "$$out" >/dev/null
	if target/debug/seatgeist-cli --socket "$$socket" atspi invoke --node atspi://:1.42/org/a11y/atspi/accessible/7 --action press >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi set-text --node atspi://:1.42/org/a11y/atspi/accessible/7 smoke-text >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi insert-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --offset 0 smoke-text >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi delete-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi copy-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi cut-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi paste-text --node atspi://:1.42/org/a11y/atspi/accessible/7 --offset 0 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi set-caret --node atspi://:1.42/org/a11y/atspi/accessible/7 --offset 0 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi set-selection --node atspi://:1.42/org/a11y/atspi/accessible/7 --start-offset 0 --end-offset 1 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" atspi text-attributes --node "" --offset 0 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "node_id must be non-empty" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic click-button --name OK --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic set-text-field --name Search smoke-text --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic focus-text-field --name Search --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic activate-tab --name General --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic activate-link --name Help --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic toggle-check --name Enable --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic set-value --name Volume --value 0.5 --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic select-item --name Printer --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	if target/debug/seatgeist-cli --socket "$$socket" semantic select-menu --path File/Open --max-nodes 128 >"$$out" 2>&1; then cat "$$out"; exit 1; fi
	grep -q "policy" "$$out"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "focused_accessibility_tree"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_find"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_invoke"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_set_text"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_insert_text"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_delete_text"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_copy_text"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_cut_text"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_paste_text"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_set_caret"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_set_selection"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "accessibility_text_attributes"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "click_button"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "set_text_field"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "focus_text_field"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "activate_tab"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "activate_link"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "toggle_check"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "set_value"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "select_item"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 30 | grep -q "select_menu"

smoke-uinput-status:
	set -euo pipefail
	socket="/tmp/seatgeist-uinput-smoke/seatgeistd.sock"
	log="target/seatgeist-uinput-smoke-daemon.log"
	journal="target/seatgeist-uinput-smoke-journal.jsonl"
	out="target/seatgeist-uinput-smoke.json"
	rm -rf /tmp/seatgeist-uinput-smoke "$$log" "$$journal" "$$out"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" input status >"$$out"
	jq -e '.type == "uinput_status" and (.data.available | type == "boolean") and (.data.setup_hint | type == "string")' "$$out" >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" input backends >"$$out"
	jq -e '.type == "input_backend_status" and (.data.uinput_available | type == "boolean") and ((.data.implemented_available_backend == null) or (.data.implemented_available_backend == "uinput")) and (.data.remote_desktop_portal.setup_hint | type == "string") and (.data.libei.setup_hint | type == "string") and (.data.eis_keymap.source | type == "string") and (.data.eis_keymap.setup_hint | type == "string")' "$$out" >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 | grep -q "uinput_status"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 | grep -q "input_backend_status"

smoke-capture-backends:
	set -euo pipefail
	socket="/tmp/seatgeist-capture-backends-smoke/seatgeistd.sock"
	log="target/seatgeist-capture-backends-smoke-daemon.log"
	journal="target/seatgeist-capture-backends-smoke-journal.jsonl"
	out="target/seatgeist-capture-backends-smoke.json"
	rm -rf /tmp/seatgeist-capture-backends-smoke "$$log" "$$journal" "$$out"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" capture-backends >"$$out"
	jq -e '.type == "capture_backend_status" and ((.data.implemented_available_backend == null) or (.data.implemented_available_backend == "spectacle") or (.data.implemented_available_backend == "portal_screenshot")) and (.data.screenshot_portal.setup_hint | type == "string") and ((.data.screenshot_portal.screenshot_interface_version == null) or (.data.screenshot_portal.screenshot_interface_version | type == "number")) and ((.data.screenshot_portal.screenshot_available_targets_mask == null) or (.data.screenshot_portal.screenshot_available_targets_mask | type == "number")) and (.data.screenshot_portal.screenshot_available_targets | type == "array") and (.data.screenshot_portal.screenshot_target_option_supported | type == "boolean") and (.data.kwin_metadata.setup_hint | type == "string") and (.data.spectacle.setup_hint | type == "string") and (.data.setup_hint | type == "string")' "$$out" >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 | grep -q "capture_backend_status"

smoke-pointer-calibration:
	set -euo pipefail
	socket="/tmp/seatgeist-pointer-calibration-smoke/seatgeistd.sock"
	log="target/seatgeist-pointer-calibration-smoke-daemon.log"
	journal="target/seatgeist-pointer-calibration-smoke-journal.jsonl"
	out="target/seatgeist-pointer-calibration-smoke.json"
	rm -rf /tmp/seatgeist-pointer-calibration-smoke "$$log" "$$journal" "$$out"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" input pointer-calibration >"$$out"
	jq -e '.type == "pointer_calibration" and .data.coordinate_space == "physical_pixel" and (.data.monitors | length) >= 1 and (.data.sample_points | length) >= 3' "$$out" >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 | grep -q "pointer_calibration"

smoke-human-input-pause:
	set -euo pipefail
	socket="/tmp/seatgeist-human-input-pause-smoke/seatgeistd.sock"
	run_dir="target/seatgeist-human-input-pause-smoke"
	log="target/seatgeist-human-input-pause-smoke-daemon.log"
	journal="target/seatgeist-human-input-pause-smoke-journal.jsonl"
	config="$$run_dir/config.toml"
	approval_file="$$run_dir/approvals.jsonl"
	activity_file="$$run_dir/human-input-active"
	status_json="$$run_dir/safety-status.json"
	approval_json="$$run_dir/approval.json"
	denied_out="$$run_dir/focus-denied.txt"
	rm -rf /tmp/seatgeist-human-input-pause-smoke "$$run_dir" "$$log" "$$journal"
	mkdir -p "$$run_dir"
	cargo build -p seatgeistd -p seatgeist-cli
	printf '[daemon]\napproval_file = "%s"\n\n[safety]\nrequire_focus_guard = false\npause_on_human_input = true\nhuman_input_activity_file = "%s"\nhuman_input_quiet_ms = 60000\n' "$$(pwd)/$$approval_file" "$$(pwd)/$$activity_file" >"$$config"
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" --config "$$config" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" approve --approval-file "$$approval_file" --safety-class control-semantic --method focus_window --ttl-ms 60000 --reason "human-input-pause smoke" >"$$approval_json"
	test "$$(stat -c '%a' "$$approval_file")" = "600"
	: >"$$activity_file"
	target/debug/seatgeist-cli --socket "$$socket" safety-status >"$$status_json"
	jq -e '.type == "safety_status" and .data.pause_on_human_input == true and .data.human_input_signal_fresh == true and .data.human_input_quiet_ms == 60000' "$$status_json" >/dev/null
	if target/debug/seatgeist-cli --socket "$$socket" focus --window "__seatgeist_human_pause_never__" >"$$denied_out" 2>&1; then
		cat "$$denied_out"
		exit 1
	fi
	grep -q "human input activity is fresh" "$$denied_out"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 --method focus_window --ok false | jq -e '.type == "journal" and (.data | length) >= 1 and all(.data[]; .summary == "error kind=human_input_pause")' >/dev/null

smoke-trace-replay:
	set -euo pipefail
	socket="/tmp/seatgeist-trace-smoke/seatgeistd.sock"
	log="target/seatgeist-trace-smoke-daemon.log"
	journal="target/seatgeist-trace-smoke-journal.jsonl"
	config="target/seatgeist-trace-smoke-config.toml"
	validate_out="target/seatgeist-trace-validate-smoke.json"
	replay_out="target/seatgeist-trace-replay-smoke.json"
	denied_capture="/tmp/seatgeist-denied-full-resolution.png"
	rm -rf /tmp/seatgeist-trace-smoke "$$log" "$$journal" "$$config" "$$validate_out" "$$replay_out" "$$denied_capture"
	cargo build -p seatgeistd -p seatgeist-cli
	target/debug/seatgeist-cli trace validate --dir examples/traces >"$$validate_out"
	jq -e '.type == "trace_validation_set" and .trace_count >= 6 and .step_count >= 45 and any(.traces[]; (.file | endswith("status-smoke.json")) and .step_count == 15 and any(.steps[]; .method == "safety_status" and .expect_json_count == 1) and any(.steps[]; .method == "computer_use_readiness" and .expect_json_count == 6) and any(.steps[]; .method == "accessibility_quality_status" and .expect_json_count == 7) and any(.steps[]; .method == "kwin_bridge_status") and any(.steps[]; .method == "uinput_status") and any(.steps[]; .method == "capture_backend_status") and any(.steps[]; .method == "capture_session_status" and .expect_json_count == 3) and any(.steps[]; .method == "clipboard_backend_status" and .expect_json_count == 6) and any(.steps[]; .method == "input_backend_status") and any(.steps[]; .method == "remote_desktop_eis_session_status") and any(.steps[]; .method == "remote_desktop_eis_stop")) and any(.traces[]; (.file | endswith("journal-tail-smoke.json")) and .step_count == 3 and any(.steps[]; .method == "journal_tail" and .expect_json_count == 8)) and any(.traces[]; (.file | endswith("policy-denials-smoke.json")) and .step_count == 5 and all(.steps[]; .expect_response_type == "error" and .expect_ok == false and (.expect_error_contains | type == "string") and .expect_json_count == 1) and any(.steps[]; .method == "accessibility_set_caret") and any(.steps[]; .method == "accessibility_set_selection")) and any(.traces[]; (.file | endswith("semantic-denials-smoke.json")) and .step_count == 9 and all(.steps[]; .expect_response_type == "error" and .expect_ok == false and .expect_error_contains == "policy prompt required for ControlSemantic" and .expect_json_count == 1) and any(.steps[]; .method == "click_button") and any(.steps[]; .method == "set_text_field") and any(.steps[]; .method == "select_menu")) and any(.traces[]; (.file | endswith("input-denials-smoke.json")) and .step_count == 9 and all(.steps[]; .expect_response_type == "error" and .expect_ok == false and (.expect_error_contains | test("Control(Keyboard|Pointer)")) and .expect_json_count == 1) and any(.steps[]; .method == "remote_desktop_session_probe") and any(.steps[]; .method == "remote_desktop_eis_probe") and any(.steps[]; .method == "remote_desktop_eis_start")) and any(.traces[]; (.file | endswith("panic-stop-smoke.json")) and .step_count == 5 and all(.steps[]; .expect_json_count == 1))' "$$validate_out" >/dev/null
	printf '[safety]\nrequire_focus_guard = false\n' >"$$config"
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" --config "$$config" >"$$log" 2>&1 &
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
	target/debug/seatgeist-cli --socket "$$socket" trace replay --dir examples/traces >"$$replay_out"
	jq -e '.type == "trace_replay_set" and .trace_count >= 6 and .step_count >= 45 and any(.traces[]; (.file | endswith("status-smoke.json")) and (.steps | length) == 15 and all(.steps[]; .ok == true) and any(.steps[]; .method == "safety_status") and any(.steps[]; .method == "computer_use_readiness" and .response_type == "computer_use_readiness") and any(.steps[]; .method == "accessibility_quality_status" and .response_type == "accessibility_quality_status") and any(.steps[]; .method == "kwin_bridge_status") and any(.steps[]; .method == "uinput_status") and any(.steps[]; .method == "capture_backend_status") and any(.steps[]; .method == "capture_session_status" and .response_type == "capture_session_status") and any(.steps[]; .method == "clipboard_backend_status" and .response_type == "clipboard_backend_status") and any(.steps[]; .method == "input_backend_status") and any(.steps[]; .method == "remote_desktop_eis_session_status") and any(.steps[]; .method == "remote_desktop_eis_stop")) and any(.traces[]; (.file | endswith("journal-tail-smoke.json")) and (.steps | length) == 3 and all(.steps[]; .ok == true) and any(.steps[]; .method == "journal_tail" and .response_type == "journal")) and any(.traces[]; (.file | endswith("policy-denials-smoke.json")) and (.steps | length) == 5 and all(.steps[]; .response_type == "error" and .ok == false and .error_kind == "policy_prompt_required") and any(.steps[]; .method == "focus_window") and any(.steps[]; .method == "accessibility_set_caret") and any(.steps[]; .method == "accessibility_set_selection")) and any(.traces[]; (.file | endswith("semantic-denials-smoke.json")) and (.steps | length) == 9 and all(.steps[]; .response_type == "error" and .ok == false and .error_kind == "policy_prompt_required") and any(.steps[]; .method == "click_button") and any(.steps[]; .method == "set_text_field") and any(.steps[]; .method == "select_menu")) and any(.traces[]; (.file | endswith("input-denials-smoke.json")) and (.steps | length) == 9 and all(.steps[]; .response_type == "error" and .ok == false and .error_kind == "policy_prompt_required") and any(.steps[]; .method == "type_text") and any(.steps[]; .method == "click_pointer") and any(.steps[]; .method == "remote_desktop_session_probe") and any(.steps[]; .method == "remote_desktop_eis_probe") and any(.steps[]; .method == "remote_desktop_eis_start")) and any(.traces[]; (.file | endswith("panic-stop-smoke.json")) and (.steps | length) == 5 and all(.steps[]; .response_type == "panic_stop" and .ok == true) and any(.steps[]; .method == "set_panic_stop"))' "$$replay_out" >/dev/null
	test ! -e "$$denied_capture"
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 --method safety_status --ok true | jq -e '.type == "journal" and (.data | length) >= 1' >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 --ok false | jq -e '.type == "journal" and (.data | length) >= 3' >/dev/null
	target/debug/seatgeist-cli --socket "$$socket" journal tail --limit 10 --method set_panic_stop --ok true | jq -e '.type == "journal" and (.data | length) >= 2' >/dev/null

smoke-gui-input:
	scripts/gui-input-smoke.sh text-editor

smoke-mcp:
	set -euo pipefail
	socket="/tmp/seatgeist-mcp-smoke/seatgeistd.sock"
	log="target/seatgeist-mcp-smoke-daemon.log"
	journal="target/seatgeist-mcp-smoke-journal.jsonl"
	out="target/seatgeist-mcp-smoke.jsonl"
	rm -rf /tmp/seatgeist-mcp-smoke "$$log" "$$journal" "$$out"
	cargo build -p seatgeistd -p seatgeist-mcp
	target/debug/seatgeistd --socket "$$socket" --journal "$$journal" >"$$log" 2>&1 &
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
		printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"seatgeist.health","arguments":{}}}'
		printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"seatgeist.observe","arguments":{}}}'
		printf '%s\n' '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"seatgeist.a11y_text_attributes","arguments":{"node_id":"invalid-atspi-node","offset":0}}}'
	} | SEATGEIST_SOCKET="$$socket" target/debug/seatgeist-mcp --stdio >"$$out"
	test "$$(wc -l <"$$out")" = "5"
	jq -e 'select(.id == 1) | .result.capabilities.tools.listChanged == false' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.list_windows")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.clipboard_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.clipboard_get_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.clipboard_set_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.desktop_session_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.computer_use_readiness")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.panic_stop_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.panic_stop_enable")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.panic_stop_disable")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.kwin_bridge_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.uinput_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.input_backend_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.remote_desktop_session_probe")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.remote_desktop_eis_probe")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.capture_backend_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.capture_open")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.pointer_calibration")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.type_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.key_combo")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.move_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.click_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.drag_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.scroll_pointer")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.click_button")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.set_text_field")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.focus_text_field")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.activate_tab")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.select_item")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.select_menu")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_quality_status")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_focused_tree")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_find")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_text_attributes")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_invoke")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_set_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_insert_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_delete_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_copy_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_cut_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_paste_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_set_caret")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "seatgeist.a11y_set_selection")' "$$out" >/dev/null
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

gui-eval-a11y-text-attributes:
	scripts/gui-eval.sh a11y-text-attributes

gui-eval-a11y-control-denied:
	scripts/gui-eval.sh a11y-control-denied

gui-eval-semantic-denied:
	scripts/gui-eval.sh semantic-denied

gui-eval-input-denied:
	scripts/gui-eval.sh input-denied

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
	scripts/install-kwin-bridge.py
