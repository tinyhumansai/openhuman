#!/usr/bin/env bash
# Package the core CLI binary into a release tarball and optionally upload it.
#
# Usage:
#   package-cli-tarball.sh <binary_path> <version> <target>
#
# Environment:
#   GITHUB_TOKEN  — if set, uploads tarball + sha256 to the GitHub release
#   UPLOAD_REPO   — GitHub repo slug (default: tinyhumansai/openhuman)
#
# Example:
#   package-cli-tarball.sh target/release/openhuman-core 0.5.0 aarch64-apple-darwin
set -euo pipefail

BIN_PATH="${1:?Usage: package-cli-tarball.sh <binary_path> <version> <target>}"
VERSION="${2:?}"
TARGET="${3:?}"
UPLOAD_REPO="${UPLOAD_REPO:-tinyhumansai/openhuman}"

TARBALL="openhuman-core-${VERSION}-${TARGET}.tar.gz"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp "$BIN_PATH" "$WORK/openhuman-core"
chmod +x "$WORK/openhuman-core"
tar -czf "$TARBALL" -C "$WORK" openhuman-core

# openssl dgst works on both macOS and Linux
openssl dgst -sha256 -r "$TARBALL" | awk '{print $1}' > "${TARBALL}.sha256"

echo "[package-cli] Created $TARBALL (sha256: $(cat "${TARBALL}.sha256"))"

# ── Optional upload ──────────────────────────────────────────────────────────
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  gh release upload "v${VERSION}" \
    "$TARBALL" "${TARBALL}.sha256" \
    --repo "$UPLOAD_REPO" --clobber
  echo "[package-cli] Uploaded $TARBALL to v${VERSION}"
fi
