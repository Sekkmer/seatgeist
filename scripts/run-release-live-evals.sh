#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ "${SEATGEIST_RELEASE_LIVE_EVALS_APPROVED:-0}" != "1" ]]; then
	cat >&2 <<'EOF'
run-release-live-evals: refusing to run without explicit operator approval.

Set SEATGEIST_RELEASE_LIVE_EVALS_APPROVED=1 only when this KDE session is ready
for release-blocking live evals. This target may open KWrite/Kate, KCalc, and
Firefox; may show xdg-desktop-portal consent dialogs; and can send tightly scoped
keyboard, pointer, scroll, and key-combo input through Seatgeist policy gates.
EOF
	exit 1
fi

export SEATGEIST_PORTAL_SCREENSHOT_STRICT=1
export SEATGEIST_REMOTE_DESKTOP_STRICT=1
export SEATGEIST_REMOTE_DESKTOP_EIS_STRICT=1
export SEATGEIST_REMOTE_DESKTOP_EIS_INPUT_STRICT=1

make gui-eval-text-editor-input
make gui-eval-kcalc-visual
make gui-eval-firefox-localhost-button
make gui-eval-portal-screenshot
make gui-eval-remote-desktop-probe
make gui-eval-remote-desktop-eis-session

readiness_json="target/seatgeist-release/release-live-evals-readiness.json"
mkdir -p "$(dirname "$readiness_json")"
scripts/release-readiness.py --json >"$readiness_json"
python3 - "$readiness_json" <<'PY'
import json
import sys
from pathlib import Path

report = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
checks = {check.get("name"): check for check in report.get("checks", []) if isinstance(check, dict)}
live_eval = checks.get("live_eval_evidence")
if not live_eval:
    raise SystemExit("run-release-live-evals: release-readiness did not report live_eval_evidence")
if not live_eval.get("ok"):
    print("run-release-live-evals: live_eval_evidence is still incomplete", file=sys.stderr)
    for item in live_eval.get("evidence", []):
        print(f"  {item}", file=sys.stderr)
    raise SystemExit(1)
print("run-release-live-evals: live_eval_evidence ok")
PY
