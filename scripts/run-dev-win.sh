#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
APP_DIR="$REPO_ROOT/app"
cd "$APP_DIR"

export LIBCLANG_PATH="/c/Program Files/LLVM/bin"
export CEF_PATH="$HOME/Library/Caches/tauri-cef"

to_unix_path() {
  if [[ -z "${1:-}" ]]; then
    return 1
  fi
  cygpath -u "$1"
}

find_pnpm() {
  if command -v pnpm >/dev/null 2>&1; then
    command -v pnpm
    return 0
  fi
  local local_appdata_unix
  local_appdata_unix="$(to_unix_path "${LOCALAPPDATA:-}")" || return 1
  local candidate
  candidate="$(ls -d "$local_appdata_unix"/Microsoft/WinGet/Packages/pnpm.pnpm_* 2>/dev/null | head -n1 || true)"
  if [[ -n "$candidate" && -x "$candidate/pnpm.exe" ]]; then
    printf '%s\n' "$candidate/pnpm.exe"
    return 0
  fi
  return 1
}

find_ninja() {
  if command -v ninja >/dev/null 2>&1; then
    command -v ninja
    return 0
  fi
  local local_appdata_unix
  local_appdata_unix="$(to_unix_path "${LOCALAPPDATA:-}")" || return 1
  local candidate
  candidate="$(ls -d "$local_appdata_unix"/Microsoft/WinGet/Packages/Ninja-build.Ninja_* 2>/dev/null | head -n1 || true)"
  if [[ -n "$candidate" && -x "$candidate/ninja.exe" ]]; then
    printf '%s\n' "$candidate/ninja.exe"
    return 0
  fi
  return 1
}

PNPM_EXE="$(find_pnpm || true)"
if [[ -z "$PNPM_EXE" ]]; then
  echo "[run-dev-win] pnpm not found. Install pnpm and retry."
  exit 1
fi

NINJA_EXE="$(find_ninja || true)"
if [[ -z "$NINJA_EXE" ]]; then
  echo "[run-dev-win] ninja not found. Install ninja and retry."
  exit 1
fi
export CMAKE_MAKE_PROGRAM="$NINJA_EXE"

CEF_RUNTIME_PATH="$(ls -d "$CEF_PATH"/*/cef_windows_x86_64 2>/dev/null | sort -Vr | head -n1 || true)"
if [[ -n "$CEF_RUNTIME_PATH" ]]; then
  export CEF_RUNTIME_PATH
fi

PATH_PREFIX="/c/Program Files/CMake/bin:$(dirname "$NINJA_EXE")"
if [[ -n "${CEF_RUNTIME_PATH:-}" ]]; then
  PATH_PREFIX="$PATH_PREFIX:$CEF_RUNTIME_PATH"
fi
export PATH="$PATH_PREFIX:$PATH"

"$PNPM_EXE" tauri:ensure
"$PNPM_EXE" core:stage
source ../scripts/load-dotenv.sh
APPLE_SIGNING_IDENTITY='OpenHuman Dev Signer' cargo tauri dev
