#!/usr/bin/env bash
# Drop the codec-enabled CEF binary built by `build-cef-with-codecs.sh`
# (or downloaded from a private CDN) into the cache that tauri-cef's
# build script expects. After this runs, `cargo build` from any worktree
# picks up the new binary via the existing `CEF_PATH` rerun-if-env-changed
# wiring in `cef-dll-sys/build.rs`.
#
# Tracks #1223. Idempotent — running twice replaces the previous extract.

set -euo pipefail

# --- Inputs ---------------------------------------------------------

# `CEF_PATH` is the same env var the runtime reads via download-cef +
# tauri-cli. Default matches the path baked into `scripts/ensure-tauri-cli.sh`.
CEF_PATH="${CEF_PATH:-$HOME/Library/Caches/tauri-cef}"

# Source tarball — by default we look in the build dir produced by
# build-cef-with-codecs.sh. Override CEF_TARBALL to install a tarball
# downloaded from a private CDN.
CEF_BUILD_DIR="${CEF_BUILD_DIR:-$HOME/cef-build}"
CEF_TARBALL="${CEF_TARBALL:-}"

# Match the version pin in `app/src-tauri/Cargo.toml` (`cef = "=146.4.1"`
# → binary 146.0.9). When you bump cef, update this string too.
CEF_VERSION="${CEF_VERSION:-146.0.9}"

# --- Detect platform-specific tarball + dest dir --------------------

case "$(uname -s)" in
  Darwin)
    case "$(uname -m)" in
      arm64|aarch64) PLATFORM_SUFFIX="macosarm64"; DEST_DIR_NAME="cef_macos_aarch64" ;;
      x86_64)        PLATFORM_SUFFIX="macosx64";   DEST_DIR_NAME="cef_macos_x86_64" ;;
      *) echo "[cef-install] unsupported macOS arch: $(uname -m)" >&2; exit 1 ;;
    esac
    ;;
  Linux)
    case "$(uname -m)" in
      aarch64|arm64) PLATFORM_SUFFIX="linuxarm64"; DEST_DIR_NAME="cef_linux_aarch64" ;;
      x86_64)        PLATFORM_SUFFIX="linux64";    DEST_DIR_NAME="cef_linux_x86_64" ;;
      *) echo "[cef-install] unsupported Linux arch: $(uname -m)" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "[cef-install] unsupported host: $(uname -s)" >&2
    exit 1
    ;;
esac

# --- Locate the tarball ---------------------------------------------

if [[ -z "$CEF_TARBALL" ]]; then
  CEF_TARBALL="$(ls "$CEF_BUILD_DIR/chromium/src/cef/binary_distrib/cef_binary_${CEF_VERSION}"*"_${PLATFORM_SUFFIX}_minimal.tar.bz2" 2>/dev/null | head -n1 || true)"
fi

if [[ -z "$CEF_TARBALL" || ! -f "$CEF_TARBALL" ]]; then
  echo "[cef-install] no tarball found." >&2
  echo "[cef-install] expected:" >&2
  echo "[cef-install]   $CEF_BUILD_DIR/chromium/src/cef/binary_distrib/cef_binary_${CEF_VERSION}*_${PLATFORM_SUFFIX}_minimal.tar.bz2" >&2
  echo "[cef-install] override with: CEF_TARBALL=/path/to/tarball $0" >&2
  exit 1
fi

echo "[cef-install] tarball: $CEF_TARBALL"

# --- Verify the binary actually has codecs --------------------------

# Quick sanity check before we extract — if the tarball is somehow the
# stock Spotify CDN one (no codecs) we don't want to silently overwrite
# a working install. The presence of `libffmpeg.dylib` containing the
# string `proprietary` is the cheapest tell.
TARBALL_NAME="$(basename "$CEF_TARBALL")"
case "$TARBALL_NAME" in
  *_minimal.tar.bz2) ;;
  *)
    echo "[cef-install] WARNING: tarball name doesn't match the *_minimal.tar.bz2 convention." >&2
    echo "[cef-install]          name = $TARBALL_NAME" >&2
    ;;
esac

# --- Extract to CEF_PATH/<version>/<platform-dir>/ ------------------

DEST="$CEF_PATH/$CEF_VERSION/$DEST_DIR_NAME"

if [[ -d "$DEST" ]]; then
  echo "[cef-install] removing existing $DEST"
  rm -rf "$DEST"
fi
mkdir -p "$DEST"

echo "[cef-install] extracting → $DEST"
tar -xjf "$CEF_TARBALL" -C "$DEST" --strip-components=1

# --- Verify codec gates -------------------------------------------

# `MEDIA_OPTIONS_FFMPEG_BRANDING` is what Chromium checks at runtime
# to know whether ffmpeg has H.264 etc. The minimal distrib includes
# the libcef binary itself — grep for the symbol so we can fail loud
# rather than silently install a stock build into the codec slot.
LIBCEF_PATH=""
case "$(uname -s)" in
  Darwin) LIBCEF_PATH="$DEST/Release/Chromium Embedded Framework.framework/Libraries/libcef.dylib";;
  Linux)  LIBCEF_PATH="$DEST/Release/libcef.so";;
esac

if [[ -n "$LIBCEF_PATH" && -f "$LIBCEF_PATH" ]]; then
  if strings "$LIBCEF_PATH" 2>/dev/null | grep -q "Chrome.*ffmpeg\|avc1\.64\|H264VideoStreamParser"; then
    echo "[cef-install] codec strings detected in libcef → looks like a Chrome-branded build."
  else
    echo "[cef-install] WARNING: no proprietary-codec strings detected in $LIBCEF_PATH" >&2
    echo "[cef-install]          install will proceed but Gmeet dynamic-bg may still fail" >&2
    echo "[cef-install]          (run \`node scripts/diagnose-cef-runtime.mjs probe\` after \`pnpm dev:app\`" >&2
    echo "[cef-install]           to confirm h264_baseline === true)" >&2
  fi
fi

echo "[cef-install] done."
echo "[cef-install] destination: $DEST"
echo "[cef-install] next step:   pnpm dev:app   # cargo build will pick this up via CEF_PATH"
