#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

release_root="${SEATGEIST_RELEASE_DIR:-target/seatgeist-release}"
manifest="${1:-}"
if [[ -z "$manifest" ]]; then
	manifest="$(find "$release_root" -maxdepth 1 -type f -name 'seatgeist-*.manifest.json' -printf '%T@ %p\n' | sort -nr | awk 'NR == 1 {print $2}')"
fi
if [[ -z "$manifest" || ! -f "$manifest" ]]; then
	echo "write-release-evidence: no release manifest found in $release_root" >&2
	exit 1
fi

prefix="${manifest%.manifest.json}"
readiness="${prefix}.readiness.json"
portal_v3="${prefix}.portal-screenshot-v3-status.json"

scripts/release-readiness.py --json >"$readiness"
scripts/portal-screenshot-v3-status.py >"$portal_v3"
scripts/verify-release-evidence.py "$manifest"

echo "write-release-evidence: readiness=$readiness"
echo "write-release-evidence: portal_screenshot_v3=$portal_v3"
