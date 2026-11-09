#!/bin/sh
# Build a per-machine package. The library lands in /usr/local/lib/muninn.
set -eu

version="${VERSION:?VERSION is required}"
arch="${ARCH:?ARCH is required}"
target="${TARGET:?TARGET is required}"
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
lib="$root/target/$target/release/libmuninn_coinbase_connector.dylib"
if [ ! -f "$lib" ]; then
  echo "missing $lib" >&2
  exit 1
fi

payload=$(mktemp -d)
trap 'rm -rf "$payload"' EXIT
mkdir -p "$payload/usr/local/lib/muninn" \
  "$payload/usr/local/share/doc/muninn-coinbase-connector"
cp "$lib" "$payload/usr/local/lib/muninn/libmuninn_coinbase_connector.dylib"
cp "$root/LICENSE" "$root/README.md" \
  "$payload/usr/local/share/doc/muninn-coinbase-connector/"
chmod 755 "$payload/usr/local/lib/muninn/libmuninn_coinbase_connector.dylib"
chmod 644 \
  "$payload/usr/local/share/doc/muninn-coinbase-connector/LICENSE" \
  "$payload/usr/local/share/doc/muninn-coinbase-connector/README.md"

mkdir -p "$root/dist"
out="$root/dist/muninn-coinbase-connector-${version}-macos-${arch}.pkg"
pkgbuild \
  --root "$payload" \
  --identifier llc.weirdbrains.muninn-coinbase-connector \
  --version "$version" \
  --install-location / \
  --ownership recommended \
  "$out"
echo "$out"
