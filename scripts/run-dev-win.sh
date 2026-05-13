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

# When pnpm/PowerShell/cmd launch `bash.exe` directly, the spawned shell
# inherits the parent PATH and the MSYS utility directory (`Git\usr\bin`)
# may be absent — bash runs, but `cygpath`, `mktemp`, `grep`, `sort`, etc.
# are missing. Probe known Git-for-Windows install locations and prepend
# `usr/bin` so the rest of the script works regardless of launcher.
if ! command -v cygpath >/dev/null 2>&1; then
  for git_root in "/c/Program Files/Git" "/c/Program Files (x86)/Git"; do
    if [[ -x "$git_root/usr/bin/cygpath.exe" ]]; then
      export PATH="$git_root/usr/bin:$PATH"
      break
    fi
  done
fi

if ! command -v cygpath >/dev/null 2>&1; then
  echo "[run-dev-win] cygpath not found. Run this script from Git Bash or MSYS2,"
  echo "[run-dev-win] or install Git for Windows so cygpath.exe is available at"
  echo "[run-dev-win] 'C:\\Program Files\\Git\\usr\\bin\\cygpath.exe'."
  exit 1
fi

if [[ -z "${LOCALAPPDATA:-}" ]]; then
  echo "[run-dev-win] LOCALAPPDATA is unset; cannot resolve the CEF cache directory." >&2
  exit 1
fi

# ─────────────────────────────────────────────────────────────────────────────
# Restore the real Windows-side PATH.
#
# Git for Windows' bash sources /etc/profile + /etc/profile.d/* on every
# spawn, which REPLACES the inherited Windows PATH with an MSYS-only
# default (/usr/local/bin:/usr/bin:/bin:…). That wipes every tool the
# parent shell saw — node, cargo, pnpm, ninja, cmake, etc. — and breaks
# any downstream script that assumes PATH inheritance.
#
# Pull the full machine + user PATH from a cmd.exe subprocess (which DOES
# inherit the unaltered Windows PATH from its parent), convert each entry
# to MSYS form, and append it to the current PATH. We append (not prepend)
# so MSYS coreutils (cygpath, grep, sed, mktemp) still resolve first.
# ─────────────────────────────────────────────────────────────────────────────
cmd_exe_for_path="$(command -v cmd.exe 2>/dev/null || command -v cmd 2>/dev/null || echo /c/Windows/System32/cmd.exe)"
if [[ -x "$cmd_exe_for_path" ]]; then
  windows_path_raw="$("$cmd_exe_for_path" //c "echo %PATH%" 2>/dev/null | tr -d '\r' | head -n1 || true)"
  if [[ -n "$windows_path_raw" ]]; then
    windows_path_unix=""
    IFS=';' read -ra _wpe <<< "$windows_path_raw"
    for _entry in "${_wpe[@]}"; do
      [[ -z "$_entry" ]] && continue
      _u="$(cygpath -u "$_entry" 2>/dev/null || printf '%s' "$_entry")"
      windows_path_unix="${windows_path_unix}${windows_path_unix:+:}${_u}"
    done
    if [[ -n "$windows_path_unix" ]]; then
      export PATH="$PATH:$windows_path_unix"
      echo "[run-dev-win] appended Windows-side PATH (node/cargo/pnpm/… now findable)"
    else
      echo "[run-dev-win] WARNING: cmd.exe PATH query returned no entries — node/cargo may be missing downstream" >&2
    fi
  else
    echo "[run-dev-win] WARNING: cmd.exe PATH query returned empty — node/cargo may be missing downstream" >&2
  fi
else
  echo "[run-dev-win] WARNING: cmd.exe not found at '$cmd_exe_for_path' — Windows PATH restoration skipped; node/cargo may be missing downstream" >&2
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
  # vcvarsall.bat (called by vcvars64.bat) shells out to `vswhere` by bare
  # name to locate Windows SDK / MSVC component versions. If vswhere isn't
  # on cmd.exe's PATH, vcvarsall silently degrades — it sets `cl.exe` on
  # PATH but skips the Windows SDK `LIB` / `INCLUDE` entries, which then
  # fails the link step downstream with `LNK1181: cannot open input file
  # 'kernel32.lib'`. The VS Installer dir holding vswhere is rarely on the
  # system PATH (Microsoft expects you to invoke vswhere by absolute path),
  # so we prepend it inside the launcher .bat before calling vcvars.
  vswhere_dir_win="$(cygpath -w "$(dirname "$vswhere_exe")")"
  # Note: we deliberately do NOT redirect vcvars64.bat's stdout to NUL — MSYS
  # would rewrite `NUL` to `/dev/null` while writing the .bat. Instead we let
  # vcvars64 print its banner and filter for `KEY=VALUE` lines below.
  printf '@echo off\r\nset "PATH=%s;%%PATH%%"\r\ncall "%s"\r\nset\r\n' \
    "$vswhere_dir_win" "$vcvars_bat" > "$vcvars_launcher"
  # Note: do NOT set MSYS_NO_PATHCONV here — disabling path conversion stops
  # MSYS from rewriting `//c` to `/c`, leaving cmd to treat `//c` as an
  # unknown switch and open an interactive shell instead of executing the
  # launcher.
  # `cmd` may be missing from PATH when bash.exe is spawned by pnpm/PowerShell
  # with a stripped environment. Fall back to the well-known absolute path.
  cmd_exe="$(command -v cmd.exe 2>/dev/null || command -v cmd 2>/dev/null || echo /c/Windows/System32/cmd.exe)"
  if [[ ! -x "$cmd_exe" ]]; then
    echo "[run-dev-win] cmd.exe not found on PATH and /c/Windows/System32/cmd.exe missing" >&2
    rm -f "$vcvars_launcher"
    exit 1
  fi
  msvc_env_raw="$("$cmd_exe" //c "$vcvars_launcher_win" 2>&1 || true)"
  rm -f "$vcvars_launcher"
  # Strip lines that aren't key=value (vcvars banner, blank lines).
  msvc_env="$(printf '%s\n' "$msvc_env_raw" | grep -E '^[A-Za-z_][A-Za-z0-9_()]*=' || true)"
  if [[ -z "$msvc_env" ]]; then
    echo "[run-dev-win] failed to capture MSVC env from vcvars64.bat" >&2
    echo "[run-dev-win] cmd.exe used: $cmd_exe" >&2
    echo "[run-dev-win] launcher: $vcvars_launcher_win" >&2
    echo "[run-dev-win] --- cmd output (first 40 lines) ---" >&2
    printf '%s\n' "$msvc_env_raw" | head -n 40 >&2
    echo "[run-dev-win] --- end cmd output ---" >&2
    exit 1
  fi
  pre_vcvars_path="$PATH"
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
        # Prepend vcvars' PATH so MSVC tools win, but append the pre-vcvars
        # PATH so node, pnpm, git, etc. remain findable. vcvars64.bat ships a
        # MSVC-only PATH; without re-adding the original, downstream tools
        # (pnpm.cmd invoking node, etc.) blow up with "node is not recognized".
        export PATH="$new_path:$pre_vcvars_path"
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

# Windows SDK self-discovery fallback.
#
# vcvars64.bat can silently "succeed" while only setting up the MSVC half
# of the toolchain — when vswhere is missing from PATH at the time
# vcvars runs, or when the Windows SDK isn't registered in the way
# vcvarsall expects, it skips setting `WindowsSdkDir` / `WindowsSDKVersion`
# and only appends MSVC's own libs to `LIB`. The linker then fails with
# `LNK1181: cannot open input file 'kernel32.lib'` because the SDK's
# `um\x64\kernel32.lib` isn't on the search list.
#
# This block runs unconditionally (whether or not we just bootstrapped
# vcvars) and patches in the SDK paths if they're missing. Detects the
# latest installed SDK on disk under `Windows Kits\10\Lib` and appends
# both lib and include trees.
if [[ -z "${WindowsSdkDir:-}" || "${WindowsSDKVersion:-}" == "\\" || -z "${WindowsSDKVersion:-}" ]]; then
  sdk_root_unix="/c/Program Files (x86)/Windows Kits/10"
  if [[ -d "$sdk_root_unix/Lib" ]]; then
    sdk_version="$(ls -d "$sdk_root_unix"/Lib/*/ 2>/dev/null \
      | sort -V | tail -n1 \
      | sed 's|/$||; s|.*/||')"
    if [[ -n "$sdk_version" && -f "$sdk_root_unix/Lib/$sdk_version/um/x64/kernel32.lib" ]]; then
      sdk_root_win="$(cygpath -w "$sdk_root_unix")"
      export WindowsSdkDir="${sdk_root_win}\\"
      export WindowsSDKVersion="${sdk_version}\\"
      sdk_lib_um="${sdk_root_win}\\Lib\\${sdk_version}\\um\\x64"
      sdk_lib_ucrt="${sdk_root_win}\\Lib\\${sdk_version}\\ucrt\\x64"
      sdk_inc_shared="${sdk_root_win}\\Include\\${sdk_version}\\shared"
      sdk_inc_um="${sdk_root_win}\\Include\\${sdk_version}\\um"
      sdk_inc_ucrt="${sdk_root_win}\\Include\\${sdk_version}\\ucrt"
      sdk_inc_winrt="${sdk_root_win}\\Include\\${sdk_version}\\winrt"
      export LIB="${LIB:+$LIB;}${sdk_lib_um};${sdk_lib_ucrt}"
      export INCLUDE="${INCLUDE:+$INCLUDE;}${sdk_inc_shared};${sdk_inc_um};${sdk_inc_ucrt};${sdk_inc_winrt}"
      # Prepend the SDK bin dir to PATH so `rc.exe` (Windows Resource
      # Compiler) is findable. CMake-driven native crates (cef-dll-sys
      # via cmake-rs, whisper-rs-sys, etc.) invoke `rc` by bare name
      # during their try-compile probe; vcvars usually adds this dir
      # but doesn't when its SDK detection degraded.
      sdk_bin_unix="$sdk_root_unix/bin/$sdk_version/x64"
      if [[ -x "$sdk_bin_unix/rc.exe" ]]; then
        export PATH="$sdk_bin_unix:$PATH"
        echo "[run-dev-win] SDK bin dir (with rc.exe) prepended to PATH: $sdk_bin_unix"
      else
        echo "[run-dev-win] WARNING: rc.exe not found at $sdk_bin_unix — CMake-driven crates will fail" >&2
      fi
      echo "[run-dev-win] Windows SDK discovered manually (vcvars degraded): version ${sdk_version}"
    else
      echo "[run-dev-win] WARNING: Windows SDK version dir or kernel32.lib not found under $sdk_root_unix/Lib" >&2
      echo "[run-dev-win] linker will likely fail with LNK1181." >&2
    fi
  else
    echo "[run-dev-win] WARNING: Windows SDK not installed at $sdk_root_unix" >&2
    echo "[run-dev-win] Install via Visual Studio Build Tools and retry." >&2
  fi
fi

echo "[run-dev-win] LIB = ${LIB:-<unset>}"
echo "[run-dev-win] WindowsSdkDir = ${WindowsSdkDir:-<unset>}"
echo "[run-dev-win] WindowsSDKVersion = ${WindowsSDKVersion:-<unset>}"

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
  # WinGet (preferred on a fresh contributor machine).
  if winget_pnpm="$(find_winget_exe "pnpm.pnpm" "pnpm.exe")"; then
    printf '%s\n' "$winget_pnpm"
    return 0
  fi
  # npm-global install — `npm i -g pnpm` drops a shim under %APPDATA%\npm.
  # The shim is a `.cmd` on Windows; bash invokes .cmd via the same path.
  local appdata_unix=""
  if [[ -n "${APPDATA:-}" ]]; then
    appdata_unix="$(to_unix_path "$APPDATA" 2>/dev/null || true)"
  fi
  if [[ -z "$appdata_unix" && -n "${USERPROFILE:-}" ]]; then
    local userprofile_unix
    userprofile_unix="$(to_unix_path "$USERPROFILE" 2>/dev/null || true)"
    if [[ -n "$userprofile_unix" ]]; then
      appdata_unix="$userprofile_unix/AppData/Roaming"
    fi
  fi
  # Ordering matters: prefer the bare shebang shim (a `#!/bin/sh` script)
  # over `pnpm.cmd`. The .cmd shim invokes `node` through cmd.exe, which
  # ignores the bash-side PATH after vcvars rewriting and blows up with
  # `'"node"' is not recognized`. The bash shim execs node directly using
  # bash's PATH, which we've taken care to keep node on.
  #
  # NB: MSYS does NOT set the execute bit on .cmd files (only on .exe and
  # shebang-prefixed scripts), so we test with `-f` (regular file) rather
  # than `-x`.
  if [[ -n "$appdata_unix" ]]; then
    for candidate in \
        "$appdata_unix/npm/pnpm" \
        "$appdata_unix/npm/pnpm.cmd" \
        "$appdata_unix/npm/pnpm.exe"; do
      if [[ -f "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    done
  fi
  # Chocolatey shim — same pattern as find_ninja above.
  for choco_pnpm in \
      "/c/ProgramData/chocolatey/bin/pnpm.cmd" \
      "/c/ProgramData/chocolatey/bin/pnpm.exe"; do
    if [[ -f "$choco_pnpm" ]]; then
      printf '%s\n' "$choco_pnpm"
      return 0
    fi
  done
  return 1
}

find_ninja() {
  if command -v ninja >/dev/null 2>&1; then
    command -v ninja
    return 0
  fi
  # WinGet (preferred on a fresh contributor machine).
  if winget_ninja="$(find_winget_exe "Ninja-build.Ninja" "ninja.exe")"; then
    printf '%s\n' "$winget_ninja"
    return 0
  fi
  # Chocolatey shim — common on engineering desktops that pre-date WinGet.
  # `-f` rather than `-x` because MSYS leaves .cmd files unmarked-executable.
  for choco_ninja in \
      "/c/ProgramData/chocolatey/bin/ninja.exe" \
      "/c/ProgramData/chocolatey/bin/ninja.cmd" \
      "/c/ProgramData/chocolatey/lib/ninja/tools/ninja.exe"; do
    if [[ -f "$choco_ninja" ]]; then
      printf '%s\n' "$choco_ninja"
      return 0
    fi
  done
  # CMake's own bundled ninja, if a recent CMake install dropped one alongside.
  local bundled="/c/Program Files/CMake/bin/ninja.exe"
  if [[ -f "$bundled" ]]; then
    printf '%s\n' "$bundled"
    return 0
  fi
  return 1
}

# pnpm.cmd / the bare pnpm shim both ultimately `exec node ...`. When
# PowerShell launches pnpm which launches bash.exe, the inherited PATH
# does NOT reliably include Node.js — and vcvars wipes the rest. Probe
# the common Windows install locations and prepend whatever we find so
# downstream `exec node` calls in pnpm shims and Tauri scripts succeed.
find_nodejs_dir() {
  # 1) Already on PATH (unlikely if we got here, but cheap to check).
  if command -v node >/dev/null 2>&1 || command -v node.exe >/dev/null 2>&1; then
    dirname "$(command -v node 2>/dev/null || command -v node.exe)"
    return 0
  fi
  # 2) Standard installer locations.
  for nodejs_dir in \
      "/c/Program Files/nodejs" \
      "/c/Program Files (x86)/nodejs"; do
    if [[ -f "$nodejs_dir/node.exe" ]]; then
      printf '%s\n' "$nodejs_dir"
      return 0
    fi
  done
  # 3) nvm-for-windows: %LOCALAPPDATA%\nvm\v<version>. Pick the highest.
  if [[ -n "${LOCALAPPDATA:-}" ]]; then
    local nvm_root
    nvm_root="$(to_unix_path "$LOCALAPPDATA" 2>/dev/null || true)/nvm"
    if [[ -d "$nvm_root" ]]; then
      local nvm_pick
      nvm_pick="$(ls -d "$nvm_root"/v* 2>/dev/null | sort -V | tail -n1)"
      if [[ -n "$nvm_pick" && -f "$nvm_pick/node.exe" ]]; then
        printf '%s\n' "$nvm_pick"
        return 0
      fi
    fi
  fi
  # 4) Chocolatey shim.
  if [[ -f "/c/ProgramData/chocolatey/bin/node.exe" ]]; then
    printf '%s\n' "/c/ProgramData/chocolatey/bin"
    return 0
  fi
  return 1
}

NODEJS_DIR="$(find_nodejs_dir || true)"
if [[ -z "$NODEJS_DIR" ]]; then
  echo "[run-dev-win] node.exe not found on PATH or in common Windows install dirs." >&2
  echo "[run-dev-win] Install Node.js (https://nodejs.org/) and retry." >&2
  exit 1
fi
export PATH="$NODEJS_DIR:$PATH"
echo "[run-dev-win] nodejs dir prepended to PATH: $NODEJS_DIR"

# Same trick for cargo. Git Bash's /etc/profile.d scripts wipe the parent
# Windows PATH and re-install a MSYS-default one; rustup's
# `~/.cargo/bin` (or `$CARGO_HOME/bin`) doesn't survive that. We need
# cargo for the vendored tauri-cli install (`ensure-tauri-cli.sh`),
# `core:stage`, and `cargo tauri dev` itself.
find_cargo_dir() {
  if command -v cargo >/dev/null 2>&1 || command -v cargo.exe >/dev/null 2>&1; then
    dirname "$(command -v cargo 2>/dev/null || command -v cargo.exe)"
    return 0
  fi
  # 1) Honour CARGO_HOME (rustup, workspace .env conventions).
  if [[ -n "${CARGO_HOME:-}" ]]; then
    local ch
    ch="$(to_unix_path "$CARGO_HOME" 2>/dev/null || printf '%s' "$CARGO_HOME")"
    if [[ -f "$ch/bin/cargo.exe" ]]; then
      printf '%s\n' "$ch/bin"
      return 0
    fi
  fi
  # 2) Default rustup install at %USERPROFILE%\.cargo\bin.
  if [[ -n "${USERPROFILE:-}" ]]; then
    local up
    up="$(to_unix_path "$USERPROFILE" 2>/dev/null || true)"
    if [[ -n "$up" && -f "$up/.cargo/bin/cargo.exe" ]]; then
      printf '%s\n' "$up/.cargo/bin"
      return 0
    fi
  fi
  # 3) Same path via $HOME (Git Bash sometimes only sets HOME, not USERPROFILE).
  if [[ -n "${HOME:-}" && -f "$HOME/.cargo/bin/cargo.exe" ]]; then
    printf '%s\n' "$HOME/.cargo/bin"
    return 0
  fi
  return 1
}

CARGO_DIR="$(find_cargo_dir || true)"
if [[ -z "$CARGO_DIR" ]]; then
  echo "[run-dev-win] cargo.exe not found. Install Rust via rustup (https://rustup.rs/) and retry." >&2
  exit 1
fi
export PATH="$CARGO_DIR:$PATH"
echo "[run-dev-win] cargo dir prepended to PATH: $CARGO_DIR"

PNPM_EXE="$(find_pnpm || true)"
if [[ -z "$PNPM_EXE" ]]; then
  echo "[run-dev-win] pnpm not found. Install pnpm and retry."
  exit 1
fi
echo "[run-dev-win] pnpm resolved to: $PNPM_EXE"
echo "[run-dev-win] node on bash PATH:    $(command -v node 2>/dev/null || echo '<not found>')"
echo "[run-dev-win] node.exe on bash PATH: $(command -v node.exe 2>/dev/null || echo '<not found>')"

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

# ─────────────────────────────────────────────────────────────────────────────
# Stage the CEF runtime next to the dev OpenHuman.exe.
#
# `cargo tauri build` (release) copies CEF into the bundle automatically, but
# `cargo tauri dev` doesn't — the dev .exe lands at <target>/debug/OpenHuman.exe
# alone, and Windows can't find libcef.dll. The .exe panics during boot with
# `cef::library_loader::LibraryLoader::new` errors (or just refuses to launch
# with "libcef.dll not found"). Without this step every fresh contributor
# session hits the wall.
#
# We stage by copying (not symlinking) so the script runs without admin /
# Developer-Mode privileges. `cp -ru` only copies entries newer than the
# destination, so subsequent dev runs are essentially free.
# ─────────────────────────────────────────────────────────────────────────────
if [[ -n "${CEF_RUNTIME_PATH:-}" && -f "$CEF_RUNTIME_PATH/libcef.dll" ]]; then
  # The dev OpenHuman.exe is produced by the *Tauri shell* crate
  # (app/src-tauri/Cargo.toml), not the root core crate. When
  # CARGO_TARGET_DIR is set both workspaces share it; when unset, the
  # Tauri shell builds into app/src-tauri/target while the root crate
  # builds into target/. Stage CEF next to where OpenHuman.exe will
  # actually live so Windows' DLL search order finds libcef.dll
  # regardless of how the exe is launched (terminal, OAuth deep-link,
  # double-click, etc).
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    CEF_STAGE_DIR="$(to_unix_path "$CARGO_TARGET_DIR" 2>/dev/null || printf '%s' "$CARGO_TARGET_DIR")/debug"
  else
    CEF_STAGE_DIR="$REPO_ROOT/app/src-tauri/target/debug"
  fi
  mkdir -p "$CEF_STAGE_DIR"
  if [[ ! -f "$CEF_STAGE_DIR/libcef.dll" \
        || "$CEF_RUNTIME_PATH/libcef.dll" -nt "$CEF_STAGE_DIR/libcef.dll" ]]; then
    echo "[run-dev-win] staging CEF runtime → $CEF_STAGE_DIR (first run only — copies ~270MB)"
    cp -ru "$CEF_RUNTIME_PATH"/. "$CEF_STAGE_DIR/"
    echo "[run-dev-win] CEF runtime staged"
  else
    echo "[run-dev-win] CEF runtime already staged at $CEF_STAGE_DIR (libcef.dll up to date)"
  fi
else
  echo "[run-dev-win] WARNING: CEF_RUNTIME_PATH not set or libcef.dll missing — the dev exe will fail to load" >&2
  echo "[run-dev-win] expected: $CEF_PATH/<version>/cef_windows_x86_64/libcef.dll" >&2
fi

# Use the vendored tauri-cef CLI (via the pnpm tauri script) so the
# CEF runtime is correctly bundled. APPLE_SIGNING_IDENTITY is macOS-only
# and is intentionally omitted here.
#
# OPENHUMAN_DEV_PORT lets parallel worktree dev sessions avoid the
# hardcoded 1420 collision. Vite reads the same env var directly; the
# tauri-cli inline override patches tauri.conf.json's `devUrl` so the
# shell connects to the right Vite instance.
# Validate OPENHUMAN_DEV_PORT before interpolating into JSON — a stray
# space, alphabetic char, or out-of-range value would produce an invalid
# devUrl and tauri would refuse to start (or worse, drift from Vite's
# own numeric fallback). Trim whitespace, require pure digits in
# [1, 65535], fall back to 1420 with a warning otherwise.
raw_dev_port="${OPENHUMAN_DEV_PORT:-1420}"
raw_dev_port="${raw_dev_port//[[:space:]]/}"
if [[ "$raw_dev_port" =~ ^[0-9]+$ ]] && (( raw_dev_port >= 1 && raw_dev_port <= 65535 )); then
  DEV_PORT="$raw_dev_port"
else
  echo "[run-dev-win] WARNING: invalid OPENHUMAN_DEV_PORT='$raw_dev_port'; falling back to 1420" >&2
  DEV_PORT=1420
fi

if (( DEV_PORT != 1420 )); then
  echo "[run-dev-win] OPENHUMAN_DEV_PORT=$DEV_PORT — overriding tauri devUrl"
  "$PNPM_EXE" tauri dev -c "{\"build\":{\"devUrl\":\"http://localhost:$DEV_PORT\"}}"
else
  "$PNPM_EXE" tauri dev
fi
