SHELL := /usr/bin/bash
.ONESHELL:

.PHONY: fmt check test clippy verify smoke smoke-monitors smoke-windows smoke-focus install-kwin-script

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

install-kwin-script:
	set -euo pipefail
	if kpackagetool6 --type=KWin/Script --list | grep -q "plasma-pilot-bridge"; then
		kpackagetool6 --type=KWin/Script -u kwin/plasma-pilot-bridge
	else
		kpackagetool6 --type=KWin/Script -i kwin/plasma-pilot-bridge
	fi
	kwriteconfig6 --file kwinrc --group Plugins --key plasma-pilot-bridgeEnabled true
	qdbus6 org.kde.KWin /KWin reconfigure
