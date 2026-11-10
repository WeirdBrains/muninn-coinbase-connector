#!/bin/sh
# Build a package that installs into the logged-in user's ~/.muninn/code/plugins.
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

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/scripts"
cp "$root/packaging/macos-postinstall" "$work/scripts/postinstall"
cp "$lib" "$work/scripts/libmuninn_coinbase_connector.dylib"
cp "$root/LICENSE" "$root/README.md" "$work/scripts/"
chmod 755 "$work/scripts/postinstall"

mkdir -p "$root/dist"
out="$root/dist/muninn-coinbase-connector-${version}-macos-${arch}.pkg"
pkgbuild \
  --nopayload \
  --scripts "$work/scripts" \
  --identifier llc.weirdbrains.muninn-coinbase-connector \
  --version "$version" \
  "$out"
echo "$out"
