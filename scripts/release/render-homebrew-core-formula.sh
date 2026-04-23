#!/usr/bin/env bash
# Render the homebrew/core candidate formula from the tagged source tarball.
#
# Usage:
#   render-homebrew-core-formula.sh <tag> [output_path]
#
# Example:
#   render-homebrew-core-formula.sh v0.52.27 /tmp/openhuman.rb
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEMPLATE_PATH="$REPO_ROOT/packages/homebrew-core/openhuman.rb.in"

TAG="${1:?Usage: render-homebrew-core-formula.sh <tag> [output_path]}"
OUT="${2:-$REPO_ROOT/packages/homebrew-core/openhuman.rb}"
VERSION="${TAG#v}"
SOURCE_URL="https://github.com/tinyhumansai/openhuman/archive/refs/tags/${TAG}.tar.gz"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

ARCHIVE_PATH="$TMPDIR/${TAG}.tar.gz"

echo "[homebrew-core] Downloading source tarball: $SOURCE_URL"
curl -fsSL "$SOURCE_URL" -o "$ARCHIVE_PATH"

SOURCE_SHA256="$(openssl dgst -sha256 -r "$ARCHIVE_PATH" | awk '{print $1}')"
echo "[homebrew-core] Source sha256: $SOURCE_SHA256"

mkdir -p "$(dirname "$OUT")"

sed \
  -e "s/@VERSION@/${VERSION}/g" \
  -e "s/@SOURCE_SHA256@/${SOURCE_SHA256}/g" \
  "$TEMPLATE_PATH" > "$OUT"

echo "[homebrew-core] Rendered formula -> $OUT"
