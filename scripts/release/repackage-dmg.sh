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
set -euo pipefail

APP_PATH="${1:?Usage: repackage-dmg.sh <app_path> <bundle_dir>}"
BUNDLE_DIR="${2:?}"

DMG_PATH="$(find "$BUNDLE_DIR/dmg" -name '*.dmg' -maxdepth 1 2>/dev/null | head -1)"
if [ -z "$DMG_PATH" ]; then
  echo "[dmg] No DMG found — skipping DMG re-package"
  exit 0
fi

echo "[dmg] Re-creating DMG with notarized .app..."
DMG_TEMP="$(mktemp /tmp/OpenHuman-XXXXXX.dmg)"
hdiutil create -volname "OpenHuman" -srcfolder "$APP_PATH" -ov -format UDZO "$DMG_TEMP"
mv "$DMG_TEMP" "$DMG_PATH"

echo "[dmg] Notarizing DMG..."
xcrun notarytool submit "$DMG_PATH" \
  --apple-id "$APPLE_ID" \
  --password "$APPLE_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" \
  --wait

xcrun stapler staple "$DMG_PATH"
echo "[dmg] DMG notarization complete: $DMG_PATH"
