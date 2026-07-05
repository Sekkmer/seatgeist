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
	echo "verify-release-install: no release manifest found in $release_root" >&2
	exit 1
fi

mapfile -t metadata < <(python3 - "$manifest" <<'PY'
import json
import sys
from pathlib import Path

manifest = Path(sys.argv[1])
release_dir = manifest.parent
data = json.loads(manifest.read_text(encoding="utf-8"))
artifacts = data.get("artifacts", {})
bundle = artifacts.get("bundle")
package = data.get("package")
version = data.get("version")
if not isinstance(bundle, str) or "/" in bundle:
    raise SystemExit("manifest.artifacts.bundle must be a plain filename")
if not isinstance(package, str) or not package:
    raise SystemExit("manifest.package must be a non-empty string")
if not isinstance(version, str) or not version:
    raise SystemExit("manifest.version must be a non-empty string")
print(release_dir / bundle)
print(package)
print(version)
PY
)

bundle="${metadata[0]}"
package="${metadata[1]}"
version="${metadata[2]}"
if [[ ! -f "$bundle" ]]; then
	echo "verify-release-install: bundle is missing: $bundle" >&2
	exit 1
fi

tmp_dir="$(mktemp -d -t seatgeist-release-install.XXXXXX)"
cleanup() {
	rm -rf "$tmp_dir"
}
trap cleanup EXIT

tar -xzf "$bundle" -C "$tmp_dir"
install_root="$tmp_dir/$package"
if [[ ! -d "$install_root" ]]; then
	echo "verify-release-install: archive did not extract expected package dir: $package" >&2
	exit 1
fi

for bin in seatgeistd seatgeist-cli seatgeist-mcp; do
	path="$install_root/bin/$bin"
	if [[ ! -x "$path" ]]; then
		echo "verify-release-install: packaged binary is not executable: $path" >&2
		exit 1
	fi
	output="$("$path" --version)"
	if [[ "$output" != "$bin $version" ]]; then
		echo "verify-release-install: unexpected $bin --version output: $output" >&2
		exit 1
	fi
done

"$install_root/scripts/validate-plugin.py" "$install_root/plugin"
"$install_root/scripts/validate-install-assets.py" "$install_root"
test -f "$install_root/docs/arch-kde-install.md"
test -f "$install_root/docs/release-checklist.md"
test -f "$install_root/plugin/.mcp.json"
test -f "$install_root/systemd/seatgeistd.service"
test -f "$install_root/udev/99-seatgeist-uinput.rules"

echo "verify-release-install: ok $manifest"
