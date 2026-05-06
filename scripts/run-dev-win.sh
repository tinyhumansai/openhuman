#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
APP_DIR="$REPO_ROOT/app"
cd "$APP_DIR"

# Load .env first so project env vars are available, but before we compute
# Windows-specific paths so tailored values (CEF_PATH, PATH, etc.) are set
# after .env is applied and cannot be clobbered by it.
# shellcheck source=../scripts/load-dotenv.sh
source "$REPO_ROOT/scripts/load-dotenv.sh"

if ! command -v cygpath >/dev/null 2>&1; then
  echo "[run-dev-win] cygpath not found. Run this script from Git Bash or MSYS2."
  exit 1
fi

if [[ -z "${LOCALAPPDATA:-}" ]]; then
  echo "[run-dev-win] LOCALAPPDATA is unset; cannot resolve the CEF cache directory." >&2
  exit 1
fi

export LIBCLANG_PATH="/c/Program Files/LLVM/bin"

# Bootstrap the MSVC C++ build environment in this shell so cl.exe / link.exe /
# Windows SDK headers are reachable without launching the "x64 Native Tools
# Command Prompt for VS 2022" first. This is a no-op if the env is already
# loaded (cl.exe is on PATH). Otherwise we discover the latest VS install via
# vswhere, run `vcvars64.bat` inside cmd, and re-export the relevant variables
# back into this bash session.
#
# Without this, the Ninja generator fails to find cl.exe and CMake-driven
# native crates (whisper-rs-sys, etc.) error out at the C++ compilation step.
if ! command -v cl.exe >/dev/null 2>&1; then
  vswhere_exe="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
  if [[ ! -x "$vswhere_exe" ]]; then
    echo "[run-dev-win] vswhere.exe not found at $vswhere_exe" >&2
    echo "[run-dev-win] install Visual Studio 2022 Build Tools with the 'Desktop development with C++' workload." >&2
    exit 1
  fi
  vs_install_path="$("$vswhere_exe" -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath || true)"
  if [[ -z "$vs_install_path" ]]; then
    echo "[run-dev-win] no VS install with MSVC C++ tools found via vswhere" >&2
    exit 1
  fi
  vcvars_bat="${vs_install_path}\\VC\\Auxiliary\\Build\\vcvars64.bat"
  echo "[run-dev-win] loading MSVC env from $vcvars_bat"
  # Git Bash's MSYS layer mangles inner quotes when we invoke `cmd //c`
  # directly (the literal backslash-quotes get passed through to cmd, which
  # rejects the path). Workaround: write a small launcher .bat to a temp
  # file, then have cmd execute the file. Avoids inner quoting entirely.
  vcvars_launcher="$(mktemp --suffix=.bat)"
  vcvars_launcher_win="$(cygpath -w "$vcvars_launcher")"
  # Note: we deliberately do NOT redirect vcvars64.bat's stdout to NUL — MSYS
  # would rewrite `NUL` to `/dev/null` while writing the .bat. Instead we let
  # vcvars64 print its banner and filter for `KEY=VALUE` lines below.
  printf '@echo off\r\ncall "%s"\r\nset\r\n' "$vcvars_bat" > "$vcvars_launcher"
  # Note: do NOT set MSYS_NO_PATHCONV here — disabling path conversion stops
  # MSYS from rewriting `//c` to `/c`, leaving cmd to treat `//c` as an
  # unknown switch and open an interactive shell instead of executing the
  # launcher.
  msvc_env="$(cmd //c "$vcvars_launcher_win" 2>&1 || true)"
  rm -f "$vcvars_launcher"
  # Strip lines that aren't key=value (vcvars banner, blank lines).
  msvc_env="$(printf '%s\n' "$msvc_env" | grep -E '^[A-Za-z_][A-Za-z0-9_()]*=' || true)"
  if [[ -z "$msvc_env" ]]; then
    echo "[run-dev-win] failed to capture MSVC env from vcvars64.bat" >&2
    exit 1
  fi
  while IFS='=' read -r key value; do
    case "$key" in
      PATH)
        # cmd's PATH uses ; and Windows paths; convert each entry to bash form.
        new_path=""
        IFS=';' read -ra path_entries <<< "$value"
        for entry in "${path_entries[@]}"; do
          [[ -z "$entry" ]] && continue
          unix_entry="$(cygpath -u "$entry" 2>/dev/null || printf '%s' "$entry")"
          new_path="${new_path}${new_path:+:}${unix_entry}"
        done
        export PATH="$new_path"
        ;;
      INCLUDE|LIB|LIBPATH)
        # Compiler/linker want Windows-style ;-separated paths — leave as-is.
        export "$key=$value"
        ;;
      VSCMD_*|VS[0-9]*COMNTOOLS|VCToolsInstallDir|VCToolsRedistDir|VCINSTALLDIR|VSINSTALLDIR|WindowsSdkDir|WindowsSDKVersion|UCRTVersion|UniversalCRTSdkDir|Platform)
        export "$key=$value"
        ;;
    esac
  done <<< "$msvc_env"
  if ! command -v cl.exe >/dev/null 2>&1; then
    echo "[run-dev-win] MSVC env load failed — cl.exe still not on PATH" >&2
    exit 1
  fi
  echo "[run-dev-win] MSVC env loaded (cl.exe at $(command -v cl.exe))"
fi

# Pin the linker by absolute path — runs whether or not we just bootstrapped
# the MSVC env. PATH ordering alone isn't reliable: the bash-side reorder
# doesn't always survive into the Windows-side %PATH% that rustc sees when
# it resolves `link.exe`, so it can still find Git's
# `C:\Program Files\Git\usr\bin\link.exe` (GNU coreutils symlink utility)
# first and produce `/usr/bin/link: extra operand '...rcgu.o'`. Setting
# `CARGO_TARGET_<TRIPLE>_LINKER` makes cargo pass `-C linker=<path>` to
# rustc directly, no PATH lookup involved.
#
# This block sits outside the bootstrap `if` so the pin still runs when
# the user launches from a shell that already has `cl.exe` on PATH (e.g.
# the "x64 Native Tools Command Prompt for VS 2022"). Without that, a
# ready-to-go MSVC shell would skip the linker pin and fall back to PATH
# resolution, where Git's coreutils `link.exe` can still win.
msvc_cl_dir="$(dirname "$(command -v cl.exe)")"
msvc_link_unix="$msvc_cl_dir/link.exe"
if [[ ! -x "$msvc_link_unix" ]]; then
  echo "[run-dev-win] expected link.exe alongside cl.exe at $msvc_link_unix" >&2
  exit 1
fi
msvc_link_win="$(cygpath -w "$msvc_link_unix")"
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER="$msvc_link_win"
# Also push MSVC bin to the front of PATH so any other tool that bare-resolves
# `link.exe` (CMake-driven builds, etc.) hits MSVC's, not Git's.
export PATH="$msvc_cl_dir:$PATH"
echo "[run-dev-win] linker pinned: $msvc_link_win"

# Pin Ninja as the CMake generator end-to-end. The default on Windows would be
# the Visual Studio generator, which produces .sln/.vcxproj files; if anything
# downstream then invokes ninja (because CMAKE_MAKE_PROGRAM is set below),
# you get the "ninja: error: loading 'build.ninja'" mismatch.
export CMAKE_GENERATOR=Ninja

# CEF runtime lives under LOCALAPPDATA on Windows.
# ensure-tauri-cli.sh stages it here; fall back to a default if unset.
CEF_PATH="${CEF_PATH:-$(cygpath -u "$LOCALAPPDATA")/tauri-cef}"
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
# Use the vendored tauri-cef CLI (via the pnpm tauri script) so the
# CEF runtime is correctly bundled. APPLE_SIGNING_IDENTITY is macOS-only
# and is intentionally omitted here.
"$PNPM_EXE" tauri dev
