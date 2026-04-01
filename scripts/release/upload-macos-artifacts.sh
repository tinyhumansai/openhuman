#!/usr/bin/env bash
# Re-upload notarized macOS artifacts (DMG + .app tarball) to GitHub release.
#
# Usage:
#   upload-macos-artifacts.sh <app_path> <bundle_dir> <version> <arch>
#
# Required environment:
#   GITHUB_TOKEN
#   RELEASE_ID
set -euo pipefail

APP_PATH="${1:?Usage: upload-macos-artifacts.sh <app_path> <bundle_dir> <version> <arch>}"
BUNDLE_DIR="${2:?}"
VERSION="${3:?}"
ARCH="${4:?}"
UPLOAD_REPO="${UPLOAD_REPO:-tinyhumansai/openhuman}"

# ── Re-upload DMG ────────────────────────────────────────────────────────────
DMG_PATH="$(find "$BUNDLE_DIR/dmg" -name '*.dmg' -maxdepth 1 2>/dev/null | head -1)"
if [ -n "$DMG_PATH" ]; then
  DMG_NAME="$(basename "$DMG_PATH")"
  echo "[upload] Deleting old DMG asset from release..."
  ASSET_ID="$(gh api "repos/${UPLOAD_REPO}/releases/${RELEASE_ID}/assets" \
    --jq ".[] | select(.name == \"$DMG_NAME\") | .id" 2>/dev/null || true)"
  if [ -n "$ASSET_ID" ]; then
    gh api -X DELETE "repos/${UPLOAD_REPO}/releases/assets/$ASSET_ID" || true
  fi
  echo "[upload] Uploading notarized DMG..."
  gh release upload "v${VERSION}" "$DMG_PATH" --repo "$UPLOAD_REPO" --clobber
fi

# ── Upload .app as tar.gz ────────────────────────────────────────────────────
if [ -n "$APP_PATH" ] && [ -d "$APP_PATH" ]; then
  APP_ZIP="/tmp/OpenHuman_${VERSION}_${ARCH}.app.tar.gz"
  tar -czf "$APP_ZIP" -C "$(dirname "$APP_PATH")" "$(basename "$APP_PATH")"
  gh release upload "v${VERSION}" "$APP_ZIP" --repo "$UPLOAD_REPO" --clobber
  rm -f "$APP_ZIP"
  echo "[upload] Uploaded .app tarball"
fi
