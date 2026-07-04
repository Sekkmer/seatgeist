SHELL := /usr/bin/bash
.ONESHELL:

.PHONY: fmt check test clippy verify smoke smoke-monitors smoke-windows

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
	rm -rf target/plasma-pilot-smoke "$$log"
	mkdir -p target
	cargo run -p plasma-pilotd -- --socket "$$socket" >"$$log" 2>&1 &
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
	test "$$(stat -c '%a' target/plasma-pilot-smoke)" = "700"
	test "$$(stat -c '%a' "$$socket")" = "600"

smoke-monitors:
	set -euo pipefail
	socket="/tmp/plasma-pilot-monitor-smoke/plasma-pilotd.sock"
	log="target/plasma-pilot-monitor-smoke-daemon.log"
	rm -rf /tmp/plasma-pilot-monitor-smoke "$$log"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" >"$$log" 2>&1 &
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
	rm -rf /tmp/plasma-pilot-window-smoke "$$log" "$$active_log"
	cargo build -p plasma-pilotd -p plasma-pilot-cli
	target/debug/plasma-pilotd --socket "$$socket" >"$$log" 2>&1 &
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
		cat "$$active_log"
		exit 1
	fi
	grep -q "KWin script bridge" "$$active_log"
