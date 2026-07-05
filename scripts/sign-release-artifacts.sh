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
	echo "sign-release-artifacts: no release manifest found in $release_root" >&2
	exit 1
fi

signing_key="${SEATGEIST_RELEASE_SIGNING_KEY:-}"
if [[ -z "$signing_key" ]]; then
	echo "sign-release-artifacts: set SEATGEIST_RELEASE_SIGNING_KEY to a local GPG key id or fingerprint" >&2
	exit 1
fi

command -v gpg >/dev/null || {
	echo "sign-release-artifacts: gpg is required" >&2
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
for key in ("bundle", "bundle_sha256", "source", "source_sha256"):
    name = artifacts.get(key)
    if not isinstance(name, str) or "/" in name:
        raise SystemExit(f"manifest.artifacts.{key} must be a plain filename")
    print(release_dir / name)
print(manifest)
PY
)

for artifact in "${artifacts[@]}"; do
	if [[ ! -f "$artifact" ]]; then
		echo "sign-release-artifacts: artifact is missing: $artifact" >&2
		exit 1
	fi
	gpg --batch --yes --armor --local-user "$signing_key" --detach-sign --output "$artifact.asc" "$artifact"
done

signature_manifest="${manifest%.manifest.json}.signatures.sha256"
{
	for artifact in "${artifacts[@]}"; do
		printf '%s\n' "$artifact.asc"
	done
} | sort | xargs sha256sum >"$signature_manifest"
gpg --batch --yes --armor --local-user "$signing_key" --detach-sign --output "$signature_manifest.asc" "$signature_manifest"

echo "sign-release-artifacts: signed ${#artifacts[@]} artifacts"
echo "sign-release-artifacts: signature_manifest=$signature_manifest"
