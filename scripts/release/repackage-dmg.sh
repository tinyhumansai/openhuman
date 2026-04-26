#!/usr/bin/env bash
# Re-create and notarize a DMG after the .app has been notarized.
#
# Usage:
#   repackage-dmg.sh <app_path> <bundle_dir>
#
# Required environment variables:
#   APPLE_ID
#   APPLE_PASSWORD    (app-specific password)
#   APPLE_TEAM_ID
# Re-packaging involves:
# 1. Converting the original DMG (with correct layout/background/DS_Store) to writable.
# 2. Resizing it to ensure enough space.
# 3. Replacing the original .app with the notarized one using ditto.
# 4. Converting back to compressed format and notarizing.
#
# Assets used:
# - app/src-tauri/images/background-dmg.png (baked into the original DMG)
# - /Applications symlink (baked into the original DMG)
set -euo pipefail

APP_PATH="${1:?Usage: repackage-dmg.sh <app_path> <bundle_dir>}"
BUNDLE_DIR="${2:?}"

DMG_PATH="$(find "$BUNDLE_DIR/dmg" -name '*.dmg' -maxdepth 1 2>/dev/null | head -1)"
if [ -z "$DMG_PATH" ]; then
  echo "[dmg] No DMG found — skipping DMG re-package"
  exit 0
fi

# ── Cleanup ──────────────────────────────────────────────────────────────────
# Clean up temporary files and unmount images on exit
cleanup() {
  set +e
  if [ -n "${VERIFY_MOUNT:-}" ] && [ -d "$VERIFY_MOUNT" ]; then
    echo "[dmg] Cleaning up verification mount..."
    hdiutil detach "$VERIFY_MOUNT" -force 2>/dev/null || true
    rmdir "$VERIFY_MOUNT" 2>/dev/null || true
  fi
  if [ -n "${MOUNT_DIR:-}" ] && [ -d "$MOUNT_DIR" ]; then
    echo "[dmg] Cleaning up rebuild mount..."
    hdiutil detach "$MOUNT_DIR" -force 2>/dev/null || true
    rmdir "$MOUNT_DIR" 2>/dev/null || true
  fi
  if [ -f "${DMG_RW:-}" ]; then
    rm -f "$DMG_RW"
  fi
}
trap cleanup EXIT

echo "[dmg] Re-packaging DMG to preserve layout (background, icons, symlinks)..."
# 1. Convert the original Tauri-generated DMG to a writable format (UDRW)
# Note: XXXXXX must be at the end of the template for BSD mktemp (macOS).
# We append .dmg to ensure hdiutil doesn't add it implicitly, causing mismatch.
DMG_RW="$(mktemp /tmp/OpenHuman-RW-XXXXXX).dmg"
hdiutil convert "$DMG_PATH" -format UDRW -ov -o "$DMG_RW"

# 2. Resize and replace the app
# Increase size to ensure the notarized bundle fits (may be slightly larger due to stapling)
hdiutil resize -size 1g "$DMG_RW"

# Mount the writable image using a temporary directory
MOUNT_DIR="$(mktemp -d /tmp/OpenHuman-Rebuild-XXXXXX)"
hdiutil attach "$DMG_RW" -mountpoint "$MOUNT_DIR" -noautoopen

# Replace the non-notarized app with the notarized one
# We use ditto to preserve all metadata and handles the .app bundle correctly
APP_NAME="$(basename "$APP_PATH")"
rm -rf "$MOUNT_DIR/$APP_NAME"
ditto "$APP_PATH" "$MOUNT_DIR/$APP_NAME"

# Unmount
hdiutil detach "$MOUNT_DIR"
rmdir "$MOUNT_DIR"
MOUNT_DIR=""

# 3. Convert back to compressed format (UDZO)
hdiutil convert "$DMG_RW" -format UDZO -ov -o "$DMG_PATH"
rm -f "$DMG_RW"
DMG_RW=""

echo "[dmg] Notarizing DMG..."
xcrun notarytool submit "$DMG_PATH" \
  --apple-id "$APPLE_ID" \
  --password "$APPLE_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" \
  --wait

xcrun stapler staple "$DMG_PATH"
echo "[dmg] DMG notarization complete: $DMG_PATH"

# 4. Final verification
echo "[dmg] Verifying final DMG layout..."
VERIFY_MOUNT="$(mktemp -d /tmp/OpenHuman-Verify-XXXXXX)"
hdiutil attach "$DMG_PATH" -mountpoint "$VERIFY_MOUNT" -noautoopen

if [ ! -d "$VERIFY_MOUNT/$APP_NAME" ]; then
  echo "[dmg] ERROR: .app bundle missing in final DMG"
  exit 1
fi

if [ ! -L "$VERIFY_MOUNT/Applications" ]; then
  echo "[dmg] ERROR: Applications symlink missing in final DMG"
  exit 1
fi

hdiutil detach "$VERIFY_MOUNT"
rmdir "$VERIFY_MOUNT"
VERIFY_MOUNT=""
echo "[dmg] Verification successful: layout preserved."
