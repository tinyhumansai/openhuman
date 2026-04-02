#!/usr/bin/env bash
# Re-sign all binaries inside a macOS .app bundle with hardened runtime
# and submit for Apple notarization.
#
# Usage:
#   sign-and-notarize-macos.sh <app_path> [entitlements_plist]
#
# Required environment variables:
#   APPLE_CERTIFICATE_BASE64
#   APPLE_CERTIFICATE_PASSWORD
#   APPLE_SIGNING_IDENTITY
#   APPLE_ID
#   APPLE_PASSWORD          (app-specific password)
#   APPLE_TEAM_ID
set -euo pipefail

APP_PATH="${1:?Usage: sign-and-notarize-macos.sh <app_path> [entitlements_plist]}"
ENTITLEMENTS="${2:-app/src-tauri/entitlements.sidecar.plist}"

for var in APPLE_CERTIFICATE_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  if [ -z "${!var:-}" ]; then
    echo "[sign] ERROR: Missing required env var: $var"
    exit 1
  fi
done

# ── Import signing certificate ───────────────────────────────────────────────
KEYCHAIN="resign-$$.keychain-db"
KEYCHAIN_PW="$(openssl rand -base64 24)"
CERT_FILE="$(mktemp /tmp/cert-XXXXXX.p12)"

echo "$APPLE_CERTIFICATE_BASE64" | base64 --decode > "$CERT_FILE"
security create-keychain -p "$KEYCHAIN_PW" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PW" "$KEYCHAIN"
security import "$CERT_FILE" -k "$KEYCHAIN" \
  -P "$APPLE_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple: -k "$KEYCHAIN_PW" "$KEYCHAIN"
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
rm -f "$CERT_FILE"
echo "[sign] Signing identity imported into $KEYCHAIN"

# ── Sign .app contents ──────────────────────────────────────────────────────
echo "[sign] Signing .app contents and bundle"
echo "[sign] Bundle contents:"
ls -la "$APP_PATH/Contents/MacOS/"

MAIN_EXE="$(defaults read "$APP_PATH/Contents/Info.plist" CFBundleExecutable 2>/dev/null || echo "OpenHuman")"
echo "[sign] Main executable (from plist): $MAIN_EXE"

# Sign all non-main binaries (sidecars) in MacOS/
for bin in "$APP_PATH/Contents/MacOS/"*; do
  [ -f "$bin" ] && [ -x "$bin" ] || continue
  BASENAME="$(basename "$bin")"
  [ "$BASENAME" = "$MAIN_EXE" ] && continue
  echo "[sign]   Signing sidecar: $BASENAME"
  codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APPLE_SIGNING_IDENTITY" \
    --timestamp \
    "$bin"
done

# Sign sidecars in Resources/ if any
for bin in "$APP_PATH/Contents/Resources/"openhuman-core-*; do
  [ -f "$bin" ] || continue
  echo "[sign]   Signing resource sidecar: $(basename "$bin")"
  codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APPLE_SIGNING_IDENTITY" \
    --timestamp \
    "$bin"
done

# Sign the .app bundle itself
echo "[sign]   Signing .app bundle..."
codesign --force --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$APPLE_SIGNING_IDENTITY" \
  --timestamp \
  "$APP_PATH"

# ── Verify ───────────────────────────────────────────────────────────────────
echo "[sign] Verifying signatures"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

# ── Notarize ─────────────────────────────────────────────────────────────────
echo "[sign] Notarizing..."
NOTARIZE_ZIP="$(mktemp /tmp/OpenHuman-notarize-XXXXXX.zip)"
ditto -c -k --keepParent "$APP_PATH" "$NOTARIZE_ZIP"

xcrun notarytool submit "$NOTARIZE_ZIP" \
  --apple-id "$APPLE_ID" \
  --password "$APPLE_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" \
  --wait

rm -f "$NOTARIZE_ZIP"

# ── Staple ───────────────────────────────────────────────────────────────────
echo "[sign] Stapling..."
xcrun stapler staple "$APP_PATH"

echo "[sign] Notarization complete"
