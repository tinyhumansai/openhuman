#!/usr/bin/env bash
# Build and codesign a macOS Tauri release (.app + .dmg).
#
# Usage:
#   ./scripts/build-macos-signed.sh                # release build
#   ./scripts/build-macos-signed.sh --debug        # debug build
#   ./scripts/build-macos-signed.sh --skip-notarize  # sign but skip notarization
#
# Required environment variables (or export before running):
#   APPLE_CERTIFICATE_BASE64        - base64-encoded .p12 developer certificate
#   APPLE_CERTIFICATE_PASSWORD      - password for the .p12 certificate
#   APPLE_SIGNING_IDENTITY          - e.g. "Developer ID Application: Your Name (TEAMID)"
#   APPLE_ID                        - Apple ID email for notarization
#   APPLE_PASSWORD                  - app-specific password for notarization
#   APPLE_TEAM_ID                   - 10-char Apple Developer team ID
#
# Optional:
#   TAURI_SIGNING_PRIVATE_KEY       - Tauri updater private key (for update signatures)
#   TAURI_SIGNING_PRIVATE_KEY_PASSWORD - password for the updater key

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# ── Defaults ──────────────────────────────────────────────────────────
BUILD_MODE="release"
SKIP_NOTARIZE=false
BUNDLE_TARGETS="app,dmg"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug)          BUILD_MODE="debug"; shift ;;
    --skip-notarize)  SKIP_NOTARIZE=true; shift ;;
    --bundles)        BUNDLE_TARGETS="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,/^$/s/^# //p' "$0"
      exit 0
      ;;
    *) echo "Unknown flag: $1" >&2; exit 1 ;;
  esac
done

# ── Load .env if present ─────────────────────────────────────────────
if [[ -f .env ]]; then
  echo "Loading .env..."
  set -a; source .env; set +a
fi

# Also try ci-secrets.json for local CI parity
if [[ -f scripts/ci-secrets.json ]] && command -v jq >/dev/null 2>&1; then
  echo "Loading secrets from scripts/ci-secrets.json..."
  eval "$(jq -r '.secrets // {} | to_entries[] | select(.value | length > 0) | "export \(.key)=\"\(.value)\""' scripts/ci-secrets.json 2>/dev/null || true)"
  eval "$(jq -r '.vars // {} | to_entries[] | select(.value | length > 0) | "export \(.key)=\"\(.value)\""' scripts/ci-secrets.json 2>/dev/null || true)"
fi

# ── Validate required vars ───────────────────────────────────────────
MISSING=()
for var in APPLE_CERTIFICATE_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY; do
  [[ -z "${!var:-}" ]] && MISSING+=("$var")
done
if ! $SKIP_NOTARIZE; then
  for var in APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
    [[ -z "${!var:-}" ]] && MISSING+=("$var")
  done
fi
if [[ ${#MISSING[@]} -gt 0 ]]; then
  echo "ERROR: Missing required environment variables:" >&2
  printf '  %s\n' "${MISSING[@]}" >&2
  echo >&2
  echo "Set them in .env, scripts/ci-secrets.json, or export them before running." >&2
  exit 1
fi

# ── Import certificate into a temporary keychain ─────────────────────
KEYCHAIN_NAME="build-$(date +%s).keychain-db"
KEYCHAIN_PASSWORD="$(openssl rand -base64 32)"
CERT_PATH="$(mktemp /tmp/cert-XXXXXX.p12)"

cleanup_keychain() {
  echo "Cleaning up keychain..."
  security delete-keychain "$KEYCHAIN_NAME" 2>/dev/null || true
  rm -f "$CERT_PATH"
}
trap cleanup_keychain EXIT

echo "Importing signing certificate..."
echo "$APPLE_CERTIFICATE_BASE64" | base64 --decode > "$CERT_PATH"

security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_NAME"
security set-keychain-settings -lut 21600 "$KEYCHAIN_NAME"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_NAME"

security import "$CERT_PATH" \
  -k "$KEYCHAIN_NAME" \
  -P "$APPLE_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign \
  -T /usr/bin/security

security set-key-partition-list -S apple-tool:,apple: -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_NAME"

# Prepend build keychain so codesign finds the cert
security list-keychains -d user -s "$KEYCHAIN_NAME" $(security list-keychains -d user | tr -d '"')

echo "Verifying signing identity..."
security find-identity -v -p codesigning "$KEYCHAIN_NAME" | head -5
echo

# ── Build (signing only, no notarization) ─────────────────────────────
# We hide APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID from Tauri so it signs
# but does NOT attempt notarization. We'll fix the sidecar signature
# and notarize ourselves afterwards.
echo "Building Tauri app (mode=$BUILD_MODE, bundles=$BUNDLE_TARGETS)..."

BUILD_ARGS=(--bundles "$BUNDLE_TARGETS")
if [[ "$BUILD_MODE" == "debug" ]]; then
  BUILD_ARGS+=(--debug)
fi

# Tauri picks up signing identity from env
export APPLE_SIGNING_IDENTITY

# Save and unset notarization vars so Tauri doesn't try to notarize
_SAVED_APPLE_ID="${APPLE_ID:-}"
_SAVED_APPLE_PASSWORD="${APPLE_PASSWORD:-}"
_SAVED_APPLE_TEAM_ID="${APPLE_TEAM_ID:-}"
unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID

env | grep -E 'APPLE|TAURI|VITE' || true

cd app
echo "Building now... ${BUILD_ARGS[@]}"
npx tauri build "${BUILD_ARGS[@]}"
echo "Done building"
cd ..

# Restore notarization vars
export APPLE_ID="$_SAVED_APPLE_ID"
export APPLE_PASSWORD="$_SAVED_APPLE_PASSWORD"
export APPLE_TEAM_ID="$_SAVED_APPLE_TEAM_ID"

# ── Locate artifacts ─────────────────────────────────────────────────
if [[ "$BUILD_MODE" == "debug" ]]; then
  BUNDLE_DIR="app/src-tauri/target/debug/bundle"
else
  BUNDLE_DIR="app/src-tauri/target/release/bundle"
fi

APP_PATH="$(find "$BUNDLE_DIR/macos" -name '*.app' -maxdepth 1 | head -1)"

if [[ -z "$APP_PATH" ]]; then
  echo "ERROR: No .app bundle found in $BUNDLE_DIR/macos/" >&2
  exit 1
fi

echo
echo "App bundle: $APP_PATH"

# ── Sign .app contents and bundle ─────────────────────────────────────
ENTITLEMENTS="app/src-tauri/entitlements.sidecar.plist"
MAIN_EXE="$(defaults read "$APP_PATH/Contents/Info.plist" CFBundleExecutable 2>/dev/null || echo "OpenHuman")"

echo
echo "Bundle contents:"
ls -la "$APP_PATH/Contents/MacOS/"
echo "Main executable (from plist): $MAIN_EXE"

# Sign all non-main binaries (sidecars) first
for bin in "$APP_PATH/Contents/MacOS/"*; do
  [[ -f "$bin" && -x "$bin" ]] || continue
  BASENAME="$(basename "$bin")"
  [[ "$BASENAME" == "$MAIN_EXE" ]] && continue
  echo "  Signing sidecar: $BASENAME"
  codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APPLE_SIGNING_IDENTITY" \
    --timestamp \
    "$bin"
done

# Sign sidecars in Resources/ if any
for bin in "$APP_PATH/Contents/Resources/"openhuman-core-*; do
  [[ -f "$bin" ]] || continue
  echo "  Signing resource sidecar: $(basename "$bin")"
  codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APPLE_SIGNING_IDENTITY" \
    --timestamp \
    "$bin"
done

# Sign the .app bundle (signs main exe + updates seal)
echo "  Signing .app bundle..."
codesign --force --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$APPLE_SIGNING_IDENTITY" \
  --timestamp \
  "$APP_PATH"

echo
echo "Verifying code signature..."
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
echo "Signature OK."

# ── Notarize ──────────────────────────────────────────────────────────
if $SKIP_NOTARIZE; then
  echo
  echo "Skipping notarization (--skip-notarize)."
else
  NOTARIZE_FILE="$(mktemp /tmp/OpenHuman-XXXXXX.zip)"
  echo
  echo "Creating zip for notarization..."
  ditto -c -k --keepParent "$APP_PATH" "$NOTARIZE_FILE"

  echo "Submitting for notarization..."
  xcrun notarytool submit "$NOTARIZE_FILE" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait

  rm -f "$NOTARIZE_FILE"

  echo
  echo "Stapling notarization ticket..."
  xcrun stapler staple "$APP_PATH"

  # Re-create DMG with stapled .app, notarize the DMG, then staple it
  DMG_PATH="$(find "$BUNDLE_DIR/dmg" -name '*.dmg' -maxdepth 1 2>/dev/null | head -1)"
  if [[ -n "$DMG_PATH" ]]; then
    echo "Re-creating DMG with stapled .app..."
    DMG_TEMP="$(mktemp /tmp/OpenHuman-XXXXXX.dmg)"
    hdiutil create -volname "OpenHuman" -srcfolder "$APP_PATH" -ov -format UDZO "$DMG_TEMP"
    mv "$DMG_TEMP" "$DMG_PATH"

    echo "Notarizing DMG..."
    xcrun notarytool submit "$DMG_PATH" \
      --apple-id "$APPLE_ID" \
      --password "$APPLE_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" \
      --wait

    xcrun stapler staple "$DMG_PATH"
  fi

  echo "Notarization complete."
fi

# ── Summary ───────────────────────────────────────────────────────────
echo
echo "===== Build complete ====="
echo "  App:  $APP_PATH"
[[ -n "$DMG_PATH" ]] && echo "  DMG:  $DMG_PATH"
echo
echo "To install:"
echo "  cp -R \"$APP_PATH\" /Applications/"
echo "  # or open \"$DMG_PATH\""
