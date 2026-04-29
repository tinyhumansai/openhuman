#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
APP_DIR="$REPO_ROOT/app"
cd "$APP_DIR"

if ! command -v cygpath >/dev/null 2>&1; then
  echo "[run-dev-win] cygpath not found. Run this script from Git Bash or MSYS2."
  exit 1
fi

export LIBCLANG_PATH="/c/Program Files/LLVM/bin"

# CEF runtime lives under LOCALAPPDATA on Windows, not the macOS Library cache.
# ensure-tauri-cli.sh stages it here; fall back to a default if unset.
CEF_PATH="${CEF_PATH:-$(cygpath -u "${LOCALAPPDATA:-$APPDATA}")/tauri-cef}"
export CEF_PATH

to_unix_path() {
  if [[ -z "${1:-}" ]]; then
    return 1
  fi
  cygpath -u "$1"
}

# Resolve a WinGet-installed executable.
# Usage: find_winget_exe <package-glob> <exe-name>
# Prints the full path to the exe and returns 0, or returns 1 if not found.
find_winget_exe() {
  local pkg_glob="$1"
  local exe_name="$2"
  local local_appdata_unix
  local_appdata_unix="$(to_unix_path "${LOCALAPPDATA:-}")" || return 1
  local candidate
  # Sort by version (lexicographic on directory name) and pick the newest.
  candidate="$(ls -d "$local_appdata_unix"/Microsoft/WinGet/Packages/${pkg_glob}_* 2>/dev/null \
    | sort -V | tail -n1 || true)"
  if [[ -n "$candidate" && -x "$candidate/$exe_name" ]]; then
    printf '%s\n' "$candidate/$exe_name"
    return 0
  fi
  return 1
}

find_pnpm() {
  if command -v pnpm >/dev/null 2>&1; then
    command -v pnpm
    return 0
  fi
  find_winget_exe "pnpm.pnpm" "pnpm.exe"
}

find_ninja() {
  if command -v ninja >/dev/null 2>&1; then
    command -v ninja
    return 0
  fi
  find_winget_exe "Ninja-build.Ninja" "ninja.exe"
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
# Use the vendored tauri-cef CLI (via the pnpm tauri script) so the
# CEF runtime is correctly bundled. APPLE_SIGNING_IDENTITY is macOS-only
# and is intentionally omitted here.
"$PNPM_EXE" tauri dev
