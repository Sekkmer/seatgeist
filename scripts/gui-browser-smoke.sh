#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'USAGE'
Usage: scripts/gui-browser-smoke.sh [firefox-localhost-button]

Runs an opt-in local GUI smoke that launches Firefox with a disposable profile,
visits a temporary localhost page, and clicks a large test button through
seatgeistd with short-lived approval-file grants.
USAGE
}

case_name="${1:-firefox-localhost-button}"
if [[ "$case_name" == "--help" || "$case_name" == "-h" ]]; then
	usage
	exit 0
fi
if [[ "$case_name" != "firefox-localhost-button" ]]; then
	usage >&2
	exit 2
fi

require_cmd() {
	if ! command -v "$1" >/dev/null 2>&1; then
		echo "$1 is required for Firefox localhost button GUI smoke" >&2
		exit 1
	fi
}

require_cmd firefox
require_cmd jq
require_cmd python3

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

run_dir="target/seatgeist-gui-browser-smoke"
socket_dir="/tmp/seatgeist-gui-browser-smoke"
socket="$socket_dir/seatgeistd.sock"
log="$run_dir/daemon.log"
journal="$run_dir/journal.jsonl"
approval_file="$run_dir/approvals.jsonl"
web_root="$run_dir/web"
profile_dir="$run_dir/firefox-profile"
http_log="$run_dir/http.log"
http_ready="$run_dir/http-ready"
clicked_json="$run_dir/clicked.json"
windows_json="$run_dir/windows.json"
window_json="$run_dir/window.json"
active_json="$run_dir/active-window.json"
uinput_json="$run_dir/uinput-status.json"
focus_json="$run_dir/focus.json"
click_json="$run_dir/click.json"
journal_tail_json="$run_dir/journal-tail.json"
screenshot_json="$run_dir/screenshot.json"
screenshot_err="$run_dir/screenshot.err"
screenshot_png="$run_dir/firefox-localhost-button.png"
window_id=""
firefox_pid=""
http_pid=""
app_id=""

rm -rf "$run_dir" "$socket_dir"
mkdir -p "$web_root" "$profile_dir"
chmod 700 "$run_dir" "$profile_dir"

cat >"$web_root/index.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Seatgeist Localhost Eval</title>
  <style>
    :root {
      color-scheme: light;
      font-family: system-ui, sans-serif;
      background: #f5f7fb;
      color: #14213d;
    }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
    }
    button {
      width: min(72vw, 960px);
      height: min(34vh, 280px);
      border: 0;
      border-radius: 8px;
      background: #14532d;
      color: white;
      font-size: 42px;
      font-weight: 700;
      cursor: pointer;
    }
    #result {
      position: fixed;
      left: 24px;
      bottom: 24px;
      font-size: 28px;
      font-weight: 700;
    }
  </style>
</head>
<body>
  <button id="pilot-button" type="button">Seatgeist Localhost Button</button>
  <output id="result">waiting</output>
  <script>
    document.getElementById("pilot-button").addEventListener("click", async () => {
      document.getElementById("result").textContent = "clicked";
      await fetch("/clicked", {
        method: "POST",
        headers: {"content-type": "application/json"},
        body: JSON.stringify({clicked: true, source: "firefox-localhost-button"})
      });
    });
  </script>
</body>
</html>
HTML

cat >"$profile_dir/user.js" <<'JS'
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("browser.aboutConfig.showWarning", false);
JS

python3 - "$web_root" "$clicked_json" "$http_ready" >"$http_log" 2>&1 <<'PY' &
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import json
import os
import sys
import time

web_root = Path(sys.argv[1]).resolve()
clicked_json = Path(sys.argv[2]).resolve()
ready_file = Path(sys.argv[3]).resolve()
os.chdir(web_root)

class Handler(SimpleHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/clicked":
            self.send_response(404)
            self.end_headers()
            return
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length).decode("utf-8", errors="replace")
        clicked_json.write_text(json.dumps({
            "clicked": True,
            "unix_time_ms": int(time.time() * 1000),
            "body": body,
        }) + "\n", encoding="utf-8")
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}\n')

    def log_message(self, fmt, *args):
        print(fmt % args, file=sys.stderr, flush=True)

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
ready_file.write_text(str(server.server_address[1]), encoding="utf-8")
server.serve_forever()
PY
http_pid=$!

cargo build -p seatgeistd -p seatgeist-cli

target/debug/seatgeistd --socket "$socket" --journal "$journal" --approval-file "$approval_file" >"$log" 2>&1 &
daemon_pid=$!

cli() {
	target/debug/seatgeist-cli --socket "$socket" "$@"
}

guard_args=()

cleanup() {
	if [[ -n "${window_id:-}" && -S "$socket" && ${#guard_args[@]} -gt 0 ]]; then
		cli input key-combo Alt+F4 "${guard_args[@]}" >/dev/null 2>&1 || true
	fi
	if [[ -n "${firefox_pid:-}" ]]; then
		kill "$firefox_pid" 2>/dev/null || true
		wait "$firefox_pid" 2>/dev/null || true
	fi
	if [[ -n "${http_pid:-}" ]]; then
		kill "$http_pid" 2>/dev/null || true
		wait "$http_pid" 2>/dev/null || true
	fi
	kill "$daemon_pid" 2>/dev/null || true
	wait "$daemon_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in {1..50}; do
	if [[ -s "$http_ready" ]]; then
		break
	fi
	sleep 0.1
done
if [[ ! -s "$http_ready" ]]; then
	echo "temporary localhost server did not start" >&2
	cat "$http_log" >&2 || true
	exit 1
fi
port="$(cat "$http_ready")"
url="http://127.0.0.1:$port/index.html"

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

grant_approval() {
	local safety_class="$1"
	local method="$2"
	cli approve \
		--approval-file "$approval_file" \
		--safety-class "$safety_class" \
		--method "$method" \
		--ttl-ms 300000 \
		--reason "gui-browser-smoke $method" >"$run_dir/approval-$method.json"
	jq -e --arg method "$method" '.method == $method' "$run_dir/approval-$method.json" >/dev/null
}

grant_approval control-semantic focus_window
grant_approval control-pointer click_pointer
grant_approval control-keyboard key_combo
test "$(stat -c '%a' "$approval_file")" = "600"

cli input status >"$uinput_json"
jq -e '.type == "uinput_status" and .data.available == true' "$uinput_json" >/dev/null

firefox --no-remote --profile "$profile_dir" --new-window "$url" >/dev/null 2>"$run_dir/firefox.err" &
firefox_pid=$!

for _ in {1..150}; do
	cli windows >"$windows_json"
	if jq -e '
		[
			.data[]
			| select(((.title // "") | contains("Seatgeist Localhost Eval")) or ((.title // "") | contains("127.0.0.1")))
			| select(.geometry != null)
		][0]
	' "$windows_json" >"$window_json"; then
		break
	fi
	sleep 0.1
done
if [[ ! -s "$window_json" ]]; then
	echo "could not find Firefox localhost eval window" >&2
	cat "$windows_json" >&2
	exit 1
fi

window_id="$(jq -r '.id' "$window_json")"
app_id="$(jq -r '.app_id // empty' "$window_json")"
guard_args=(--expected-active-window "$window_id")
if [[ -n "$app_id" ]]; then
	guard_args+=(--expected-active-app "$app_id")
fi

for _ in {1..80}; do
	if cli active-window >"$active_json" 2>/dev/null \
		&& jq -e --arg id "$window_id" '
			.type == "active_window"
			and (.data.id == $id or ((.data.title // "") | contains("Seatgeist Localhost Eval")))
		' "$active_json" >/dev/null; then
		break
	fi
	sleep 0.1
done
if ! jq -e --arg id "$window_id" '
	.type == "active_window"
	and (.data.id == $id or ((.data.title // "") | contains("Seatgeist Localhost Eval")))
' "$active_json" >/dev/null 2>&1; then
	echo "KWin active-window bridge did not report Firefox localhost eval as active; click or focus the Firefox eval window, run make install-kwin-script if needed, and retry" >&2
	cat "$active_json" >&2 || true
	exit 1
fi

cli focus --window "$window_id" "${guard_args[@]}" >"$focus_json"
jq -e '.type == "action"' "$focus_json" >/dev/null
sleep 0.3

button_x="$(jq -r '((.geometry.width * 0.50) | floor)' "$window_json")"
button_y="$(jq -r '((.geometry.height * 0.58) | floor)' "$window_json")"
if [[ -z "$button_x" || -z "$button_y" || "$button_x" == "null" || "$button_y" == "null" ]]; then
	echo "could not calculate Firefox window-local button coordinates" >&2
	cat "$window_json" >&2
	exit 1
fi

cli input click-pointer \
	--x "$button_x" \
	--y "$button_y" \
	--coordinate-space window-local \
	--button left \
	"${guard_args[@]}" >"$click_json"
jq -e '.type == "action"' "$click_json" >/dev/null

for _ in {1..50}; do
	if [[ -s "$clicked_json" ]]; then
		break
	fi
	sleep 0.1
done
if [[ ! -s "$clicked_json" ]]; then
	echo "Firefox localhost button click did not reach the test server" >&2
	cat "$click_json" >&2
	cat "$http_log" >&2 || true
	exit 1
fi
jq -e '.clicked == true and (.body | contains("firefox-localhost-button"))' "$clicked_json" >/dev/null

cli journal tail --limit 40 >"$journal_tail_json"
grep -q "focus_window" "$journal_tail_json"
grep -q "click_pointer" "$journal_tail_json"

if command -v spectacle >/dev/null 2>&1; then
	if spectacle -b -n --activewindow -o "$screenshot_png" >/dev/null 2>"$screenshot_err"; then
		if [[ -s "$screenshot_png" ]]; then
			printf '{"type":"visual_artifact","data":{"source":"spectacle_active_window","output":"%s"}}\n' "$screenshot_png" >"$screenshot_json"
		elif [[ "${SEATGEIST_FIREFOX_SCREENSHOT_STRICT:-0}" == "1" ]]; then
			echo "Firefox Spectacle active-window capture wrote no output" >&2
			exit 1
		else
			echo "SKIP Firefox screenshot artifact: Spectacle wrote no output"
		fi
	elif [[ "${SEATGEIST_FIREFOX_SCREENSHOT_STRICT:-0}" == "1" ]]; then
		cat "$screenshot_err" >&2
		exit 1
	else
		echo "SKIP Firefox screenshot artifact: Spectacle active-window capture failed"
	fi
elif [[ "${SEATGEIST_FIREFOX_SCREENSHOT_STRICT:-0}" == "1" ]]; then
	echo "Spectacle is required for strict Firefox screenshot artifacts" >&2
	exit 1
else
	echo "SKIP Firefox screenshot artifact: Spectacle is not available"
fi

if [[ -s "$screenshot_png" ]]; then
	echo "Firefox localhost button GUI smoke passed; visual artifact is $screenshot_png"
else
	echo "Firefox localhost button GUI smoke passed; screenshot artifact was skipped"
fi
