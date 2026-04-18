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
echo "[sign] Bundle contents (MacOS/):"
ls -la "$APP_PATH/Contents/MacOS/"
if [ -d "$APP_PATH/Contents/Frameworks" ]; then
  echo "[sign] Bundle contents (Frameworks/):"
  ls -la "$APP_PATH/Contents/Frameworks/"
fi

MAIN_EXE="$(defaults read "$APP_PATH/Contents/Info.plist" CFBundleExecutable 2>/dev/null || echo "OpenHuman")"
echo "[sign] Main executable (from plist): $MAIN_EXE"

codesign_hardened() {
  codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APPLE_SIGNING_IDENTITY" \
    --timestamp \
    "$@"
}

# ── Nested Frameworks/ (CEF + Helper apps) ──────────────────────────────────
# Must be signed from the inside out, before the outer .app bundle.
if [ -d "$APP_PATH/Contents/Frameworks" ]; then
  # 1. Sign loose dylibs / binaries inside any *.framework
  while IFS= read -r -d '' fw; do
    echo "[sign]   Scanning framework: $(basename "$fw")"
    while IFS= read -r -d '' item; do
      # Skip symlinks; codesign will sign the real file via Versions/Current
      [ -L "$item" ] && continue
      case "$item" in
        *.dylib|*.so)
          echo "[sign]     Signing lib: \"${item#${APP_PATH}/}\""
          codesign_hardened "$item"
          ;;
      esac
    done < <(find "$fw" -type f -print0)
    # Sign the framework bundle itself
    echo "[sign]   Signing framework bundle: $(basename "$fw")"
    codesign_hardened "$fw"
  done < <(find "$APP_PATH/Contents/Frameworks" -maxdepth 1 -type d -name '*.framework' -print0)

  # 2. Sign each nested Helper.app (inner binary first, then the bundle)
  while IFS= read -r -d '' helper; do
    HELPER_EXE="$(defaults read "$helper/Contents/Info.plist" CFBundleExecutable 2>/dev/null || true)"
    if [ -n "$HELPER_EXE" ] && [ -f "$helper/Contents/MacOS/$HELPER_EXE" ]; then
      echo "[sign]   Signing helper binary: $(basename "$helper")/$HELPER_EXE"
      codesign_hardened "$helper/Contents/MacOS/$HELPER_EXE"
    fi
    echo "[sign]   Signing helper bundle: $(basename "$helper")"
    codesign_hardened "$helper"
  done < <(find "$APP_PATH/Contents/Frameworks" -maxdepth 1 -type d -name '*.app' -print0)
fi

# ── Sidecars and loose binaries in MacOS/ ───────────────────────────────────
for bin in "$APP_PATH/Contents/MacOS/"*; do
  [ -f "$bin" ] && [ -x "$bin" ] || continue
  BASENAME="$(basename "$bin")"
  [ "$BASENAME" = "$MAIN_EXE" ] && continue
  echo "[sign]   Signing sidecar: $BASENAME"
  codesign_hardened "$bin"
done

# Sign sidecars in Resources/ if any
for bin in "$APP_PATH/Contents/Resources/"openhuman-core-*; do
  [ -f "$bin" ] || continue
  echo "[sign]   Signing resource sidecar: $(basename "$bin")"
  codesign_hardened "$bin"
done

# ── Outer .app bundle ───────────────────────────────────────────────────────
echo "[sign]   Signing .app bundle..."
codesign_hardened "$APP_PATH"

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
