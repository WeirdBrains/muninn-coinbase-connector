#!/bin/sh
# Require the pushed tag (v1.2.3) to match muninn-coinbase-connector's version.
set -eu

tag="${GITHUB_REF_NAME#v}"
file=$(awk '
  $0 == "[package]" { in_pkg = 1; next }
  in_pkg && $0 ~ /^\[/ { exit }
  in_pkg && $0 ~ /^version = / {
    gsub(/"/, "", $3)
    print $3
    exit
  }
' Cargo.toml)

if [ -z "$file" ]; then
  echo "Cargo.toml has no [package] version" >&2
  exit 1
fi
if [ "$tag" != "$file" ]; then
  echo "tag ${GITHUB_REF_NAME} does not match Cargo.toml version ${file}" >&2
  exit 1
fi

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "version=${tag}" >> "$GITHUB_OUTPUT"
fi
echo "$tag"
