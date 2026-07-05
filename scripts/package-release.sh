#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
if [[ -z "$version" ]]; then
	echo "package-release: could not read workspace version from Cargo.toml" >&2
	exit 1
fi

git_short="$(git rev-parse --short=12 HEAD 2>/dev/null || true)"
if [[ -z "$git_short" ]]; then
	git_short="nogit"
fi

target_triple="${SEATGEIST_RELEASE_TARGET:-linux-x86_64}"
release_root="${SEATGEIST_RELEASE_DIR:-target/seatgeist-release}"
package_name="seatgeist-${version}-${git_short}-${target_triple}"
source_name="seatgeist-${version}-${git_short}-source"
stage="${release_root}/${package_name}"
archive="${release_root}/${package_name}.tar.gz"
checksum="${archive}.sha256"
source_archive="${release_root}/${source_name}.tar.gz"
source_checksum="${source_archive}.sha256"
source_file_list="${release_root}/${source_name}.files"
manifest="${release_root}/${package_name}.manifest.json"

rm -rf "$stage" "$archive" "$checksum" "$source_archive" "$source_checksum" "$source_file_list" "$manifest"
mkdir -p "$stage/bin" \
	"$stage/docs" \
	"$stage/examples/traces" \
	"$stage/kwin" \
	"$stage/plugin" \
	"$stage/polkit" \
	"$stage/scripts" \
	"$stage/systemd" \
	"$stage/udev"

cargo build --workspace --release

for bin in seatgeistd seatgeist-cli seatgeist-mcp; do
	install -Dm755 "target/release/$bin" "$stage/bin/$bin"
done

test -f scripts/seatgeist-panic-stop-hotkey

cp -a docs/. "$stage/docs/"
cp -a examples/traces/. "$stage/examples/traces/"
cp -a kwin/. "$stage/kwin/"
cp -a plugin/. "$stage/plugin/"
cp -a polkit/. "$stage/polkit/"
cp -a scripts/. "$stage/scripts/"
cp -a systemd/. "$stage/systemd/"
cp -a udev/. "$stage/udev/"
cp Cargo.toml Cargo.lock Makefile LICENSE-MIT LICENSE-APACHE "$stage/"

find "$stage" -type f -printf '%P\n' | sort >"$stage/MANIFEST.files"

cat >"$stage/MANIFEST.json" <<EOF
{
  "name": "Seatgeist",
  "package": "$package_name",
  "version": "$version",
  "git": "$git_short",
  "target": "$target_triple",
  "binaries": [
    "seatgeistd",
    "seatgeist-cli",
    "seatgeist-mcp"
  ],
  "artifacts": {
    "bundle": "$(basename "$archive")",
    "bundle_sha256": "$(basename "$checksum")",
    "source": "$(basename "$source_archive")",
    "source_sha256": "$(basename "$source_checksum")"
  },
  "notes": "Release artifact uses the canonical Seatgeist package and binary names."
}
EOF

tar --sort=name --owner=0 --group=0 --numeric-owner -C "$release_root" -czf "$archive" "$package_name"
sha256sum "$archive" >"$checksum"

git ls-files -z >"$source_file_list"
tar --null \
	--files-from "$source_file_list" \
	--sort=name \
	--owner=0 \
	--group=0 \
	--numeric-owner \
	--transform "s#^#${source_name}/#" \
	-czf "$source_archive"
sha256sum "$source_archive" >"$source_checksum"
rm -f "$source_file_list"

cp "$stage/MANIFEST.json" "$manifest"

echo "package-release: archive=$archive"
echo "package-release: checksum=$checksum"
echo "package-release: source_archive=$source_archive"
echo "package-release: source_checksum=$source_checksum"
echo "package-release: manifest=$manifest"
