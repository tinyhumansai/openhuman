#!/usr/bin/env bash
# Build, export, and optionally upload the OpenHuman iOS IPA to App Store Connect.
#
# Required local inputs:
#   TEAM_ID=XXXXXXXXXX
#   IOS_APPSTORE_PROVISIONING_PROFILE_PATH=/path/to/profile.mobileprovision
#
# Required only when UPLOAD=1:
#   ASC_KEY_ID=XXXXXXXXXX
#   ASC_ISSUER_ID=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
#   ASC_KEY_PATH=/path/to/AuthKey_XXXXXXXXXX.p8
#
# Optional:
#   BUILD_NUMBER=123
#   UPLOAD=0|1   (default: 0)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

APP_IDENTIFIER="com.tinyhumansai.openhuman"
MOBILE_DIR="$REPO_ROOT/app/src-tauri-mobile"
APPLE_DIR="$MOBILE_DIR/gen/apple"
ARCHIVE_PATH="$APPLE_DIR/build/openhuman-mobile_iOS.xcarchive"
EXPORT_DIR="$APPLE_DIR/build/appstore-export"
PROFILE_PATH="${IOS_APPSTORE_PROVISIONING_PROFILE_PATH:-}"
TEAM_ID="${TEAM_ID:-${APPLE_DEVELOPMENT_TEAM:-}}"
UPLOAD="${UPLOAD:-0}"
BUILD_NUMBER="${BUILD_NUMBER:-$(date -u +%Y%m%d%H%M)}"
MARKETING_VERSION="$(node -p "require('./app/src-tauri-mobile/tauri.conf.json').version")"

die() {
  echo "[ios-appstore] ERROR: $*" >&2
  exit 1
}

[[ -n "$TEAM_ID" ]] || die "TEAM_ID is required"
[[ -n "$PROFILE_PATH" ]] || die "IOS_APPSTORE_PROVISIONING_PROFILE_PATH is required"
[[ -f "$PROFILE_PATH" ]] || die "provisioning profile not found: $PROFILE_PATH"

if [[ "$UPLOAD" == "1" ]]; then
  [[ -n "${ASC_KEY_ID:-}" ]] || die "ASC_KEY_ID is required when UPLOAD=1"
  [[ -n "${ASC_ISSUER_ID:-}" ]] || die "ASC_ISSUER_ID is required when UPLOAD=1"
  [[ -n "${ASC_KEY_PATH:-}" ]] || die "ASC_KEY_PATH is required when UPLOAD=1"
  [[ -f "$ASC_KEY_PATH" ]] || die "App Store Connect key not found: $ASC_KEY_PATH"
fi

PROFILE_PLIST="$(mktemp -t openhuman-appstore-profile.XXXXXX.plist)"
security cms -D -i "$PROFILE_PATH" > "$PROFILE_PLIST"
PROFILE_UUID="$(/usr/libexec/PlistBuddy -c 'Print :UUID' "$PROFILE_PLIST")"
PROFILE_NAME="$(/usr/libexec/PlistBuddy -c 'Print :Name' "$PROFILE_PLIST")"
PROFILE_APP_ID="$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:application-identifier' "$PROFILE_PLIST")"
mkdir -p "$HOME/Library/MobileDevice/Provisioning Profiles"
cp "$PROFILE_PATH" "$HOME/Library/MobileDevice/Provisioning Profiles/$PROFILE_UUID.mobileprovision"

echo "[ios-appstore] team_id=$TEAM_ID"
echo "[ios-appstore] app_identifier=$APP_IDENTIFIER"
echo "[ios-appstore] profile_name=$PROFILE_NAME"
echo "[ios-appstore] profile_uuid=$PROFILE_UUID"
echo "[ios-appstore] profile_app_id=$PROFILE_APP_ID"
echo "[ios-appstore] version=$MARKETING_VERSION build=$BUILD_NUMBER"

echo "[ios-appstore] installed signing identities:"
security find-identity -v -p codesigning | sed 's/^/[ios-appstore]   /'

echo "[ios-appstore] building web assets"
bash scripts/ci-cancel-aware.sh pnpm --filter openhuman-app run build:app

echo "[ios-appstore] generating iOS Xcode project"
TEAM_ID="$TEAM_ID" APPLE_DEVELOPMENT_TEAM="$TEAM_ID" bash scripts/ios-init.sh
mkdir -p "$APPLE_DIR/assets"
rsync -a --delete app/dist/ "$APPLE_DIR/assets/"

INFO_PLIST="$APPLE_DIR/openhuman-mobile_iOS/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $MARKETING_VERSION" "$INFO_PLIST" 2>/dev/null \
  || /usr/libexec/PlistBuddy -c "Add :CFBundleShortVersionString string $MARKETING_VERSION" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$INFO_PLIST" 2>/dev/null \
  || /usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $BUILD_NUMBER" "$INFO_PLIST"

echo "[ios-appstore] archiving iphoneos app"
xcodebuild \
  -workspace "$APPLE_DIR/openhuman-mobile.xcodeproj/project.xcworkspace" \
  -scheme openhuman-mobile_iOS \
  -configuration release \
  -sdk iphoneos \
  -destination "generic/platform=iOS" \
  -archivePath "$ARCHIVE_PATH" \
    DEVELOPMENT_TEAM="$TEAM_ID" \
    CODE_SIGN_STYLE=Manual \
    CODE_SIGN_IDENTITY="Apple Distribution" \
    PROVISIONING_PROFILE_SPECIFIER="$PROFILE_NAME" \
    MARKETING_VERSION="$MARKETING_VERSION" \
    CURRENT_PROJECT_VERSION="$BUILD_NUMBER" \
  archive

EXPORT_OPTIONS="$(mktemp -t openhuman-export-options.XXXXXX.plist)"
mkdir -p "$EXPORT_DIR"
cat > "$EXPORT_OPTIONS" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key>
  <string>app-store-connect</string>
  <key>signingStyle</key>
  <string>manual</string>
  <key>teamID</key>
  <string>$TEAM_ID</string>
  <key>provisioningProfiles</key>
  <dict>
    <key>$APP_IDENTIFIER</key>
    <string>$PROFILE_NAME</string>
  </dict>
  <key>stripSwiftSymbols</key>
  <true/>
  <key>uploadSymbols</key>
  <true/>
</dict>
</plist>
PLIST

echo "[ios-appstore] exporting IPA"
xcodebuild \
  -exportArchive \
  -archivePath "$ARCHIVE_PATH" \
  -exportPath "$EXPORT_DIR" \
  -exportOptionsPlist "$EXPORT_OPTIONS" \
  -allowProvisioningUpdates

IPA_PATH="$(find "$EXPORT_DIR" -maxdepth 1 -name '*.ipa' -print -quit)"
[[ -n "$IPA_PATH" ]] || die "IPA export completed but no .ipa was found in $EXPORT_DIR"
echo "[ios-appstore] IPA ready: $IPA_PATH"

if [[ "$UPLOAD" != "1" ]]; then
  echo "[ios-appstore] UPLOAD=0, stopping before App Store Connect upload."
  exit 0
fi

ASC_KEY_DIR="$(mktemp -d -t openhuman-asc-keys.XXXXXX)"
cp "$ASC_KEY_PATH" "$ASC_KEY_DIR/AuthKey_${ASC_KEY_ID}.p8"

echo "[ios-appstore] uploading IPA to App Store Connect"
API_PRIVATE_KEYS_DIR="$ASC_KEY_DIR" \
  xcrun altool --upload-app \
    --type ios \
    --file "$IPA_PATH" \
    --apiKey "$ASC_KEY_ID" \
    --apiIssuer "$ASC_ISSUER_ID"

echo "[ios-appstore] upload submitted. Apple will process the build before it appears in App Store Connect."
