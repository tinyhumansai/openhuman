#!/usr/bin/env bash
# Build CEF 146.0.9 (chromium 146.0.7680.165) with proprietary codecs
# (H.264, AAC) enabled. Output is a tarball compatible with tauri-cef's
# `download-cef` extractor — drop it into `$CEF_PATH/<version>/<platform>/`
# (or run the install-local.sh sibling helper) and `cargo build` picks
# it up via the existing rerun-if-env-changed=CEF_PATH wiring in
# `cef-dll-sys/build.rs`.
#
# License: H.264 / AAC carry MPEG-LA royalty obligations. Read
# scripts/cef-with-codecs/README.md and get legal sign-off before running.
#
# Tracks #1223. Reuses the upstream `automate-git.py` toolchain rather
# than wrapping cef_create_projects.sh / `gn gen` ourselves so the build
# matches what Spotify CDN ships in everything except the codec flags.
#
# Usage:
#   ./build-cef-with-codecs.sh              # build for the host platform
#   CEF_BUILD_DIR=/Volumes/Big/cef ./build-cef-with-codecs.sh
#   CEF_BRANCH=7704 ./build-cef-with-codecs.sh   # newer chromium milestone
#
# Outputs (per the automate-git.py contract):
#   $CEF_BUILD_DIR/chromium/src/cef/binary_distrib/cef_binary_<ver>_<plat>_minimal/
#   $CEF_BUILD_DIR/chromium/src/cef/binary_distrib/cef_binary_<ver>_<plat>_minimal.tar.bz2
#
# Wall-clock: ~2-4 hours on M2/M3, longer on Linux/Windows.
# Disk: ~150 GB peak.

set -euo pipefail

# --- Build inputs ---------------------------------------------------

# Match the cef crate version pinned in `app/src-tauri/Cargo.toml`
# (`cef = "=146.4.1"`), which `download_cef::default_version` maps to
# the Chromium 146.0.7680.165 line. Bump in lock-step with the crate
# version when upgrading.
CEF_BRANCH="${CEF_BRANCH:-7680}"

# Where to put the ~150 GB Chromium checkout + build cache. Default to
# the user's home; override to an external disk if home is small.
CEF_BUILD_DIR="${CEF_BUILD_DIR:-$HOME/cef-build}"

# Target platform. The script auto-detects from the host but you can
# override (e.g. cross-build x86_64 on an arm64 Mac via macOS universal).
ARCH="${ARCH:-$(uname -m)}"
case "$(uname -s)" in
  Darwin)
    case "$ARCH" in
      arm64|aarch64) PLATFORM_FLAG="--arm64-build" ;;
      x86_64)        PLATFORM_FLAG="--x64-build" ;;
      *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  Linux)
    case "$ARCH" in
      aarch64|arm64) PLATFORM_FLAG="--arm64-build" ;;
      x86_64)        PLATFORM_FLAG="--x64-build" ;;
      *) echo "Unsupported Linux arch: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported host: $(uname -s). Build on macOS or Linux; use the Windows VS shell separately." >&2
    exit 1
    ;;
esac

# --- Prerequisite check ---------------------------------------------

require() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "[cef-build] missing dependency: $1" >&2
    echo "[cef-build] see scripts/cef-with-codecs/README.md → 'Build host requirements'" >&2
    exit 1
  }
}

require git
require python3

# --- Set up depot_tools + CEF source --------------------------------

mkdir -p "$CEF_BUILD_DIR"
cd "$CEF_BUILD_DIR"

if [[ ! -d depot_tools ]]; then
  echo "[cef-build] cloning depot_tools"
  git clone --depth 1 https://chromium.googlesource.com/chromium/tools/depot_tools.git
fi
export PATH="$CEF_BUILD_DIR/depot_tools:$PATH"

if [[ ! -d cef ]]; then
  echo "[cef-build] cloning cef wrapper"
  git clone https://bitbucket.org/chromiumembedded/cef.git cef
fi

# --- Run the build --------------------------------------------------

# `automate-git.py` orchestrates: chromium fetch / sync, depot_tools
# bootstrap, GN gen with the merged custom + default args, and
# cef_create_projects.sh + ninja invocation. The CEF docs are the
# authoritative source for these flags:
# https://bitbucket.org/chromiumembedded/cef/wiki/AutomatedBuildSetup.md

# CEF takes its build flags as a colon-separated list passed via
# `--build-target` GN_DEFINES env, NOT as `--build-arg`. The
# proprietary_codecs + ffmpeg_branding pair is what unlocks H.264 / AAC
# in the resulting libcef.
export GN_DEFINES='proprietary_codecs=true ffmpeg_branding=Chrome is_official_build=true'

echo "[cef-build] starting automate-git.py — this will take 2-4 hours and consume ~150 GB"
echo "[cef-build]   branch:        $CEF_BRANCH (chromium 146.0.7680.165 line)"
echo "[cef-build]   platform flag: $PLATFORM_FLAG"
echo "[cef-build]   GN_DEFINES:    $GN_DEFINES"
echo "[cef-build]   build dir:     $CEF_BUILD_DIR"

python3 cef/tools/automate/automate-git.py \
  --download-dir="$CEF_BUILD_DIR/chromium" \
  --depot-tools-dir="$CEF_BUILD_DIR/depot_tools" \
  --branch="$CEF_BRANCH" \
  --no-debug-build \
  --no-distrib-docs \
  --minimal-distrib \
  "$PLATFORM_FLAG"

echo "[cef-build] done. Distrib artefacts at:"
echo "[cef-build]   $CEF_BUILD_DIR/chromium/src/cef/binary_distrib/"
ls -lh "$CEF_BUILD_DIR/chromium/src/cef/binary_distrib/" 2>/dev/null | grep "cef_binary_.*_minimal" || true

echo
echo "[cef-build] next step:"
echo "[cef-build]   ./scripts/cef-with-codecs/install-local.sh"
echo "[cef-build] then:"
echo "[cef-build]   pnpm dev:app   # cargo will pick up the codec-enabled binary via CEF_PATH"
