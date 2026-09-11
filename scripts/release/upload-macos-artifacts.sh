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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/tauri-signer.sh
. "$SCRIPT_DIR/tauri-signer.sh"

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

# ── Upload .app as tar.gz + updater signature ────────────────────────────────
# We must re-sign the tarball with the Tauri updater key because re-tarring
# the hardened .app produces different bytes than the bundler's original
# .app.tar.gz — its .sig would no longer verify on installed clients.
if [ -n "$APP_PATH" ] && [ -d "$APP_PATH" ]; then
  APP_ZIP="/tmp/OpenHuman_${VERSION}_${ARCH}.app.tar.gz"
  tar -czf "$APP_ZIP" -C "$(dirname "$APP_PATH")" "$(basename "$APP_PATH")"

  if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    echo "[upload] ERROR: TAURI_SIGNING_PRIVATE_KEY not set — cannot sign updater tarball" >&2
    exit 1
  fi

  # Signs via the npm @tauri-apps/cli the lane installs, and fails the job if no
  # signer is available -- shipping an updater tarball whose .sig is missing or
  # stale breaks updates for every installed client (#5658).
  echo "[upload] Signing updater tarball with Tauri signer..."
  if ! tauri_signer_sign "$APP_ZIP"; then
    echo "[upload] ERROR: could not sign ${APP_ZIP}; refusing to upload an unsignatured updater artifact" >&2
    exit 1
  fi

  gh release upload "v${VERSION}" "$APP_ZIP" "${APP_ZIP}.sig" --repo "$UPLOAD_REPO" --clobber
  rm -f "$APP_ZIP" "${APP_ZIP}.sig"
  echo "[upload] Uploaded .app tarball + signature"
fi
