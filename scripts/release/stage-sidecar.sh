#!/usr/bin/env bash
# Stage the core sidecar binary next to Tauri resources for bundling.
#
# Usage:
#   stage-sidecar.sh <target> <core_target_dir> <core_bin_name> <sidecar_base>
#
# Example:
#   stage-sidecar.sh aarch64-apple-darwin target/aarch64-apple-darwin/release openhuman-core openhuman-core
set -euo pipefail

TARGET="${1:?Usage: stage-sidecar.sh <target> <core_target_dir> <core_bin_name> <sidecar_base>}"
CORE_TARGET_DIR="${2:?}"
CORE_BIN_NAME="${3:?}"
SIDECAR_BASE="${4:?}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

EXE_SUFFIX=""
if [[ "$TARGET" == *"windows"* ]]; then
  EXE_SUFFIX=".exe"
fi

SOURCE="${REPO_ROOT}/${CORE_TARGET_DIR}/${CORE_BIN_NAME}${EXE_SUFFIX}"
DEST_DIR="${REPO_ROOT}/app/src-tauri/binaries"
DEST="${DEST_DIR}/${SIDECAR_BASE}-${TARGET}${EXE_SUFFIX}"

mkdir -p "$DEST_DIR"
cp "$SOURCE" "$DEST"

if [[ "$TARGET" != *"windows"* ]]; then
  chmod +x "$DEST"
fi

echo "[stage-sidecar] Staged: $DEST"

# ── Verify ───────────────────────────────────────────────────────────────────
if [ ! -f "$DEST" ]; then
  echo "[stage-sidecar] ERROR: Missing staged sidecar binary: $DEST"
  exit 1
fi

if [[ "$TARGET" != *"windows"* ]] && [ ! -x "$DEST" ]; then
  echo "[stage-sidecar] ERROR: Staged sidecar is not executable: $DEST"
  exit 1
fi

echo "[stage-sidecar] Verified OK"
