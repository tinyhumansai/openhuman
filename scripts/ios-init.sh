#!/usr/bin/env bash
# scripts/ios-init.sh
#
# Scaffolds the Xcode project for the iOS client via `tauri ios init`.
# Run from the repo root.
#
# The iOS host lives in `app/src-tauri-mobile/` (separate Cargo crate from
# the desktop host at `app/src-tauri/`) because the desktop crate is pinned
# to a vendored CEF Tauri fork that does not support iOS.
#
# After this script completes:
#   1. Open the generated .xcodeproj in Xcode and set your Development Team
#      (Signing & Capabilities tab).
#   2. Run `pnpm tauri:ios:dev` to start a hot-reload dev session.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MOBILE_DIR="$REPO_ROOT/app/src-tauri-mobile"

echo "[ios-init] Running tauri ios init from $MOBILE_DIR ..."
cd "$MOBILE_DIR"
# IPHONEOS_DEPLOYMENT_TARGET pins the Swift compiler target version; the PTT
# plugin (packages/tauri-plugin-ptt/) uses iOS 14+ APIs (OSLogMessage), so we
# match the Package.swift declaration of iOS 16.
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-16.0}"

# Tauri requires `bundle.iOS.developmentTeam` to be non-empty before it will
# generate the Xcode project. We keep it empty in committed tauri.conf.json so
# the repo doesn't ship a particular developer's team ID; pass it via TEAM_ID
# (env) or APPLE_DEVELOPMENT_TEAM at invocation time. Find your team ID with:
#   security find-identity -v -p codesigning
TEAM_ID="${TEAM_ID:-${APPLE_DEVELOPMENT_TEAM:-}}"
if [[ -z "$TEAM_ID" ]]; then
  echo "[ios-init] TEAM_ID is not set." >&2
  echo "[ios-init] Find your Apple developer team ID with:" >&2
  echo "[ios-init]   security find-identity -v -p codesigning" >&2
  echo "[ios-init] Then re-run as:  TEAM_ID=XXXXXXXXXX pnpm tauri:ios:init" >&2
  exit 1
fi

npx --package=@tauri-apps/cli@^2 tauri ios init \
  -c "{\"bundle\":{\"iOS\":{\"developmentTeam\":\"$TEAM_ID\"}}}"

# Overwrite the placeholder AppIcon set Tauri generates with the real
# OpenHuman brand icons committed to icons/ios/. The generated Xcode project
# uses `Assets.xcassets/AppIcon.appiconset/`, identical to the iOS source
# layout under our `icons/ios/`.
ICONSRC="$MOBILE_DIR/icons/ios/AppIcon.appiconset"
ICONDEST=$(find "$MOBILE_DIR/gen/apple" -type d -name "AppIcon.appiconset" 2>/dev/null | head -1)
if [[ -n "$ICONDEST" && -d "$ICONSRC" ]]; then
  echo "[ios-init] copying brand icons → $ICONDEST"
  rm -f "$ICONDEST"/*.png "$ICONDEST"/Contents.json
  cp -R "$ICONSRC"/. "$ICONDEST"/
fi

# Inject privacy usage descriptions into the generated Info.plist. The
# barcode scanner (camera) is mandatory for QR pairing; mic + speech are
# needed by the PTT plugin. Without these, iOS will hard-crash the app on
# first use of each API.
INFO_PLIST=$(find "$MOBILE_DIR/gen/apple" -name "Info.plist" -path "*openhuman-mobile_iOS*" 2>/dev/null | head -1)
if [[ -n "$INFO_PLIST" ]]; then
  echo "[ios-init] injecting privacy keys → $INFO_PLIST"
  /usr/libexec/PlistBuddy -c "Add :NSCameraUsageDescription string 'OpenHuman uses the camera to scan the pairing QR code from your desktop.'" "$INFO_PLIST" 2>/dev/null || true
  /usr/libexec/PlistBuddy -c "Add :NSMicrophoneUsageDescription string 'OpenHuman uses the microphone for push-to-talk voice messages.'" "$INFO_PLIST" 2>/dev/null || true
  /usr/libexec/PlistBuddy -c "Add :NSSpeechRecognitionUsageDescription string 'OpenHuman uses on-device speech recognition to transcribe your voice messages.'" "$INFO_PLIST" 2>/dev/null || true
fi

echo ""
echo "[ios-init] Done. Next steps:"
echo ""
echo "  1. Open Xcode project:"
echo "     open app/src-tauri-mobile/gen/apple/*.xcodeproj"
echo "     Set Development Team under Signing & Capabilities."
echo ""
echo "  2. Start dev session:"
echo "     pnpm tauri:ios:dev"
echo ""
echo "See docs/ios/SETUP.md for full documentation."
