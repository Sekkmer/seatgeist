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
plugin = artifacts.get("plugin")
package = data.get("package")
version = data.get("version")
git = data.get("git")
if not isinstance(bundle, str) or "/" in bundle:
    raise SystemExit("manifest.artifacts.bundle must be a plain filename")
if not isinstance(plugin, str) or "/" in plugin:
    raise SystemExit("manifest.artifacts.plugin must be a plain filename")
if not isinstance(package, str) or not package:
    raise SystemExit("manifest.package must be a non-empty string")
if not isinstance(version, str) or not version:
    raise SystemExit("manifest.version must be a non-empty string")
if not isinstance(git, str) or not git:
    raise SystemExit("manifest.git must be a non-empty string")
print(release_dir / bundle)
print(release_dir / plugin)
print(package)
print(version)
print(git)
PY
)

bundle="${metadata[0]}"
plugin="${metadata[1]}"
package="${metadata[2]}"
version="${metadata[3]}"
git="${metadata[4]}"
if [[ ! -f "$bundle" ]]; then
	echo "verify-release-install: bundle is missing: $bundle" >&2
	exit 1
fi
if [[ ! -f "$plugin" ]]; then
	echo "verify-release-install: plugin bundle is missing: $plugin" >&2
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

tar -xzf "$plugin" -C "$tmp_dir"
plugin_root="$tmp_dir/seatgeist-$version-$git-plugin"
if [[ ! -d "$plugin_root" ]]; then
	echo "verify-release-install: plugin archive did not extract expected package dir: seatgeist-$version-$git-plugin" >&2
	exit 1
fi
"$install_root/scripts/validate-plugin.py" "$plugin_root"
test -f "$plugin_root/MANIFEST.json"
test -f "$plugin_root/MANIFEST.files"
test -f "$plugin_root/.mcp.json"

echo "verify-release-install: ok $manifest"
