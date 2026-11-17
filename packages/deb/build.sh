#!/usr/bin/env bash
# Build a .deb package for the openhuman core CLI and terminal UI binaries.
# Usage: build.sh <core_binary_path> <tui_binary_path> <version> <arch>
#   arch: amd64 | arm64
set -euo pipefail

CORE_BINARY="$1"
TUI_BINARY="$2"
VERSION="$3"
ARCH="$4"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

PKG_NAME="openhuman_${VERSION}_${ARCH}"
PKG_DIR="$WORK_DIR/$PKG_NAME"

mkdir -p "$PKG_DIR/usr/bin"
mkdir -p "$PKG_DIR/DEBIAN"
# The Debian format requires its control directory to be at least 0755. Keep
# package metadata readable even when the caller builds under a private umask.
chmod 755 "$PKG_DIR" "$PKG_DIR/usr" "$PKG_DIR/usr/bin" "$PKG_DIR/DEBIAN"

install -m 755 "$CORE_BINARY" "$PKG_DIR/usr/bin/openhuman"
install -m 755 "$TUI_BINARY" "$PKG_DIR/usr/bin/openhuman-tui"

sed \
  -e "s/@VERSION@/${VERSION}/g" \
  -e "s/@ARCH@/${ARCH}/g" \
  "$SCRIPT_DIR/control.in" > "$PKG_DIR/DEBIAN/control"

OUTPUT="${PKG_NAME}.deb"
dpkg-deb --build --root-owner-group "$PKG_DIR" "$OUTPUT"
echo "[deb] Built: $OUTPUT"
