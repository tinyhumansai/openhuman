#!/usr/bin/env bash
# scripts/android-init.sh
#
# Scaffolds the Android Studio project for the Android client via
# `tauri android init`. Run from the repo root.
#
# The Android host shares the `app/src-tauri-mobile/` crate with iOS — both
# mobile targets are wired into the same Tauri host (the desktop crate at
# `app/src-tauri/` is pinned to a vendored CEF Tauri fork and does not
# support mobile targets).
#
# Prereqs:
#   - Android SDK + NDK installed (Android Studio's SDK Manager).
#   - ANDROID_HOME and NDK_HOME exported, or ANDROID_HOME with NDK installed
#     in $ANDROID_HOME/ndk/<version>/.
#   - JDK 17+ on PATH.
#
# After this script completes:
#   1. Open the generated Android Studio project under
#      app/src-tauri-mobile/gen/android/.
#   2. Run `pnpm tauri:android:dev` to start a hot-reload dev session on a
#      connected device or emulator.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MOBILE_DIR="$REPO_ROOT/app/src-tauri-mobile"

if [[ -z "${ANDROID_HOME:-}" ]]; then
  echo "[android-init] ANDROID_HOME is not set." >&2
  echo "[android-init] Install Android Studio, then export ANDROID_HOME=\"\$HOME/Library/Android/sdk\" (macOS) or the equivalent for your OS." >&2
  exit 1
fi

if [[ -z "${NDK_HOME:-}" ]]; then
  # Try the canonical $ANDROID_HOME/ndk/<latest>/ layout.
  if [[ -d "$ANDROID_HOME/ndk" ]]; then
    LATEST_NDK=$(ls -1 "$ANDROID_HOME/ndk" 2>/dev/null | sort -V | tail -1 || true)
    if [[ -n "$LATEST_NDK" ]]; then
      export NDK_HOME="$ANDROID_HOME/ndk/$LATEST_NDK"
      echo "[android-init] inferred NDK_HOME=$NDK_HOME"
    fi
  fi
fi

if [[ -z "${NDK_HOME:-}" ]]; then
  echo "[android-init] NDK_HOME is not set and no NDK was found under \$ANDROID_HOME/ndk/." >&2
  echo "[android-init] Install an NDK via Android Studio SDK Manager (Tools > SDK Manager > SDK Tools > NDK)." >&2
  exit 1
fi

echo "[android-init] Running tauri android init from $MOBILE_DIR ..."
cd "$MOBILE_DIR"
npx --package=@tauri-apps/cli@^2 tauri android init

# Overwrite the placeholder launcher icons Tauri generates with the
# OpenHuman brand icons committed under icons/android/. The Android Studio
# project layout uses `app/src/main/res/mipmap-*/` mirroring our sources.
RES_DIR=$(find "$MOBILE_DIR/gen/android" -type d -path "*/src/main/res" 2>/dev/null | head -1)
if [[ -n "$RES_DIR" ]]; then
  echo "[android-init] copying brand icons → $RES_DIR/mipmap-*"
  for d in "$MOBILE_DIR"/icons/android/mipmap-*; do
    name=$(basename "$d")
    mkdir -p "$RES_DIR/$name"
    cp "$d"/ic_launcher.png "$RES_DIR/$name/ic_launcher.png"
    # Tauri/Android also looks for the round launcher icon by default;
    # reuse the same asset (the source set ships a single square icon).
    cp "$d"/ic_launcher.png "$RES_DIR/$name/ic_launcher_round.png"
  done
fi

echo ""
echo "[android-init] Done. Next steps:"
echo ""
echo "  1. Open Android Studio:"
echo "     open -a 'Android Studio' app/src-tauri-mobile/gen/android"
echo ""
echo "  2. Start dev session (device or emulator must be connected):"
echo "     pnpm tauri:android:dev"
echo ""
echo "See docs/ios/SETUP.md (the iOS guide also covers Android prereqs)."
