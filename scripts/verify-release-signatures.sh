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
	echo "verify-release-signatures: no release manifest found in $release_root" >&2
	exit 1
fi

command -v gpg >/dev/null || {
	echo "verify-release-signatures: gpg is required" >&2
	exit 1
}

mapfile -t artifacts < <(python3 - "$manifest" <<'PY'
import json
import sys
from pathlib import Path

manifest = Path(sys.argv[1])
release_dir = manifest.parent
data = json.loads(manifest.read_text(encoding="utf-8"))
artifacts = data.get("artifacts", {})
for key in ("bundle", "bundle_sha256", "plugin", "plugin_sha256", "source", "source_sha256"):
    name = artifacts.get(key)
    if not isinstance(name, str) or "/" in name:
        raise SystemExit(f"manifest.artifacts.{key} must be a plain filename")
    print(release_dir / name)
print(manifest)
PY
)

for artifact in "${artifacts[@]}"; do
	signature="$artifact.asc"
	if [[ ! -f "$signature" ]]; then
		echo "verify-release-signatures: signature is missing: $signature" >&2
		exit 1
	fi
	gpg --batch --verify "$signature" "$artifact" >/dev/null
done

signature_manifest="${manifest%.manifest.json}.signatures.sha256"
if [[ ! -f "$signature_manifest" ]]; then
	echo "verify-release-signatures: signature manifest is missing: $signature_manifest" >&2
	exit 1
fi
if [[ ! -f "$signature_manifest.asc" ]]; then
	echo "verify-release-signatures: signature manifest signature is missing: $signature_manifest.asc" >&2
	exit 1
fi
sha256sum --check "$signature_manifest" >/dev/null
gpg --batch --verify "$signature_manifest.asc" "$signature_manifest" >/dev/null

echo "verify-release-signatures: ok $manifest"
