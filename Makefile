SHELL := /usr/bin/bash
.ONESHELL:

.PHONY: fmt check test clippy verify smoke smoke-monitors smoke-windows smoke-focus smoke-clipboard smoke-mcp install-kwin-script

fmt:
	cargo fmt --all

check:
	cargo check --workspace --all-targets

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

verify: fmt check test clippy
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
	windows="target/plasma-pilot-focus-smoke-windows.json"
	focus="target/plasma-pilot-focus-smoke-action.json"
	rm -rf /tmp/plasma-pilot-focus-smoke "$$log" "$$journal" "$$windows" "$$focus"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" --allow-control >"$$log" 2>&1 &
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
	previous_json="target/plasma-pilot-clipboard-previous.json"
	previous_text="target/plasma-pilot-clipboard-previous.txt"
	current_json="target/plasma-pilot-clipboard-current.json"
	set_result="target/plasma-pilot-clipboard-set.json"
	rm -rf /tmp/plasma-pilot-clipboard-smoke "$$log" "$$journal" "$$previous_json" "$$previous_text" "$$current_json" "$$set_result"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" --journal "$$journal" --allow-clipboard-read >"$$log" 2>&1 &
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
	if target/debug/plasma-pilot-cli --socket "$$socket" clipboard get >"$$previous_json" 2>/dev/null; then
		jq -r '.data.text' "$$previous_json" >"$$previous_text"
	fi
	sentinel="plasma-pilot-clipboard-smoke-$$(date +%s)"
	target/debug/plasma-pilot-cli --socket "$$socket" clipboard set "$$sentinel" >"$$set_result"
	target/debug/plasma-pilot-cli --socket "$$socket" clipboard get >"$$current_json"
	jq -e --arg text "$$sentinel" '.type == "clipboard_text" and .data.text == $$text' "$$current_json" >/dev/null
	grep -q '"type": "action"' "$$set_result"
	target/debug/plasma-pilot-cli --socket "$$socket" journal tail --limit 10 | grep -q "clipboard"

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
	} | PLASMA_PILOT_SOCKET="$$socket" target/debug/plasma-pilot-mcp --stdio >"$$out"
	test "$$(wc -l <"$$out")" = "4"
	jq -e 'select(.id == 1) | .result.capabilities.tools.listChanged == false' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.list_windows")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.clipboard_get_text")' "$$out" >/dev/null
	jq -e 'select(.id == 2) | any(.result.tools[]; .name == "plasma.clipboard_set_text")' "$$out" >/dev/null
	jq -e 'select(.id == 3) | .result.isError == false and .result.structuredContent.type == "health"' "$$out" >/dev/null
	jq -e 'select(.id == 4) | .result.isError == false and .result.structuredContent.type == "observation"' "$$out" >/dev/null

install-kwin-script:
	set -euo pipefail
	if kpackagetool6 --type=KWin/Script --list | grep -q "plasma-pilot-bridge"; then
		kpackagetool6 --type=KWin/Script -u kwin/plasma-pilot-bridge
	else
		kpackagetool6 --type=KWin/Script -i kwin/plasma-pilot-bridge
	fi
	kwriteconfig6 --file kwinrc --group Plugins --key plasma-pilot-bridgeEnabled true
	qdbus6 org.kde.KWin /KWin reconfigure
