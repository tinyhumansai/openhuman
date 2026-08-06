#!/usr/bin/env bash
# Validate the final, repacked AppImage bytes before release signing.
#
# This file is sourceable for fixture tests. When executed directly it extracts
# exactly one AppImage from a foreign working directory, validates the released
# sharun layout, and optionally performs a bounded Xvfb startup smoke.
#
# Usage: validate-appimage-runtime.sh <final.AppImage>
#
# Env:
#   APPIMAGE_RUNTIME_SMOKE — set to 1 to require the bounded Xvfb startup smoke;
#                            otherwise only static final-artifact checks run
#   APPIMAGE_RUNTIME_APPARMOR_USERNS — set to 1 on Ubuntu 24.04 CI to give
#                            Chromium's sandbox access to user namespaces for
#                            the smoke window: relaxes
#                            kernel.apparmor_restrict_unprivileged_userns (and
#                            restores it afterwards) and loads a temporary,
#                            per-executable AppArmor profile

set -euo pipefail

RUNTIME_VALIDATOR_SCRIPT_DIR="$(
  cd "$(dirname "${BASH_SOURCE[0]}")" && pwd
)"
# shellcheck source=strip-appimage-graphics-libs.sh
source "$RUNTIME_VALIDATOR_SCRIPT_DIR/strip-appimage-graphics-libs.sh"

runtime_validation_error() {
  echo "[appimage-runtime] ERROR: $*" >&2
  return 1
}

validate_extracted_appdir() {
  local appdir="$1"
  local lib_path="$appdir/shared/lib/lib.path"

  echo "[appimage-runtime] Validating extracted AppDir: $appdir"
  if [ -f "$lib_path" ]; then
    echo "[appimage-runtime] shared/lib/lib.path:"
    sed 's/^/[appimage-runtime]   /' "$lib_path"
  else
    echo "[appimage-runtime] shared/lib/lib.path: <missing>"
  fi

  if ! uses_sharun_launcher "$appdir"; then
    runtime_validation_error "released AppDir does not use a detectable sharun launcher"
    return 1
  fi

  local executable
  for executable in \
    "$appdir/AppRun" \
    "$appdir/sharun" \
    "$appdir/bin/OpenHuman" \
    "$appdir/shared/bin/OpenHuman"; do
    if ! is_executable_elf "$executable"; then
      runtime_validation_error "${executable#"$appdir"/} is not an executable ELF"
      return 1
    fi
  done

  local alias
  for alias in "$appdir/AppRun" "$appdir/bin/OpenHuman"; do
    if [ "$alias" -ef "$appdir/sharun" ] || cmp -s "$alias" "$appdir/sharun"; then
      continue
    fi
    runtime_validation_error "${alias#"$appdir"/} does not match sharun"
    return 1
  done

  validate_sharun_lib_path "$appdir" || return 1
  validate_appimage_required_libs "$appdir" || return 1

  local real_app="$appdir/shared/bin/OpenHuman"
  local needed
  if ! needed="$(patchelf --print-needed "$real_app")"; then
    runtime_validation_error \
      "could not read NEEDED entries from shared/bin/OpenHuman"
    return 1
  fi
  echo "[appimage-runtime] shared/bin/OpenHuman NEEDED entries:"
  if [ -n "$needed" ]; then
    printf '%s\n' "$needed" | sed 's/^/[appimage-runtime]   /'
  else
    echo "[appimage-runtime]   <none>"
  fi

  local expected_needed="${APPIMAGE_EXPECTED_NEEDED-libxdo.so.3 libcef.so}"
  local needed_name
  for needed_name in $expected_needed; do
    if ! printf '%s\n' "$needed" | grep -Fx "$needed_name" >/dev/null; then
      runtime_validation_error "shared/bin/OpenHuman is missing NEEDED entry '$needed_name'"
      return 1
    fi
  done

  local root elf rpath
  for root in \
    "$appdir/shared/bin" \
    "$appdir/shared/lib" \
    "$appdir/bin" \
    "$appdir/lib" \
    "$appdir/usr/bin" \
    "$appdir/usr/lib"; do
    [ -d "$root" ] || continue
    while IFS= read -r -d '' elf; do
      is_elf "$elf" || continue
      rpath="$(patchelf --print-rpath "$elf" 2>/dev/null || true)"
      case "$rpath" in
        *"/home/runner/"*|*"/__w/"*)
          runtime_validation_error \
            "${elf#"$appdir"/} retains forbidden build-runner RPATH '$rpath'"
          return 1
          ;;
      esac
    done < <(find "$root" -type f -print0)
  done
}

smoke_extracted_apprun() {
  local appdir="$1"
  local foreign_cwd="$2"
  local log_file="$3"

  command -v timeout >/dev/null 2>&1 \
    || { runtime_validation_error "timeout is required for the AppImage smoke"; return 1; }
  command -v xvfb-run >/dev/null 2>&1 \
    || { runtime_validation_error "xvfb-run is required for the AppImage smoke"; return 1; }

  echo "[appimage-runtime] Smoking AppRun from AppDir: $appdir"
  echo "[appimage-runtime] Smoke caller CWD: $foreign_cwd"

  local temp_root
  temp_root="$(dirname "$foreign_cwd")"
  local smoke_home="$temp_root/home"
  local smoke_config="$temp_root/config"
  local smoke_data="$temp_root/data"
  local smoke_cache="$temp_root/cache"
  mkdir -p \
    "$foreign_cwd" \
    "$smoke_home" \
    "$smoke_config" \
    "$smoke_data" \
    "$smoke_cache"

  local -a unset_args=()
  local secret_name
  for secret_name in \
    GITHUB_TOKEN \
    GH_TOKEN \
    OPENHUMAN_CEF_NO_SANDBOX \
    TAURI_SIGNING_PRIVATE_KEY \
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
    SENTRY_AUTH_TOKEN; do
    unset_args+=(-u "$secret_name")
  done
  while IFS= read -r secret_name; do
    [ -n "$secret_name" ] && unset_args+=(-u "$secret_name")
  done < <(compgen -v OPENAI_ || true)

  # ── D-Bus session bus for the smoke window ─────────────────────────────────
  #
  # A real desktop user always launches the AppImage inside a session that has a
  # D-Bus session bus; a CI runner does not. `tauri-plugin-single-instance` calls
  # `zbus::blocking::connection::Builder::session().unwrap()` in its `setup()`,
  # so with no usable bus it panics before any window exists:
  #
  #   thread 'main' panicked at plugins/single-instance/src/platform_impl/linux.rs:57
  #   called `Result::unwrap()` on an `Err` value:
  #     Address("unsupported transport 'disabled'")
  #
  # ...which is exactly how the Release Production ubuntu smoke failed (exit 101,
  # actions/runs/30393949588). Chromium logs the same underlying condition as
  # "Failed to connect to the bus: Could not parse server address".
  #
  # `app/scripts/e2e-run-session.sh` already solves this for the desktop e2e
  # runner by starting a bus with `dbus-launch`; the AppImage smoke never got the
  # same treatment. Wrapping in `dbus-run-session` is the modern equivalent and
  # needs no explicit teardown — the bus dies with the wrapped command, so the
  # 15s `timeout` still bounds everything.
  #
  # Echo the inherited value first: whether it arrives as `disabled`, `disabled:`
  # or unset changes which code path the app takes, and the failing log gave us
  # no way to tell.
  echo "[appimage-runtime] Inherited DBUS_SESSION_BUS_ADDRESS=${DBUS_SESSION_BUS_ADDRESS:-<unset>}"

  # Always drop the inherited value: the wrapper below supplies a real one, and
  # if no wrapper is available a poisoned `disabled` must not reach the app —
  # with the variable absent the app's own `can_register_single_instance_plugin()`
  # probe falls back to inspecting `$XDG_RUNTIME_DIR/bus`, whereas the literal
  # string `disabled` is precisely what zbus refuses to parse.
  unset_args+=(-u DBUS_SESSION_BUS_ADDRESS)

  local -a dbus_wrapper=()
  if command -v dbus-run-session >/dev/null 2>&1; then
    dbus_wrapper=(dbus-run-session --)
    echo "[appimage-runtime] Providing a session bus via dbus-run-session"
  elif command -v dbus-launch >/dev/null 2>&1; then
    # Older images ship dbus-launch (from dbus-x11) but not dbus-run-session.
    dbus_wrapper=(dbus-launch --exit-with-session)
    echo "[appimage-runtime] Providing a session bus via dbus-launch"
  else
    echo "[appimage-runtime] WARNING: neither dbus-run-session nor dbus-launch found;" \
      "smoking without a session bus (single-instance will be skipped)"
  fi

  local status
  if (
    cd "$foreign_cwd"
    timeout --signal=TERM --kill-after=5s 15s \
      xvfb-run -a --server-args="-screen 0 1280x960x24" \
      env "${unset_args[@]}" \
        HOME="$smoke_home" \
        XDG_CONFIG_HOME="$smoke_config" \
        XDG_DATA_HOME="$smoke_data" \
        XDG_CACHE_HOME="$smoke_cache" \
        OPENHUMAN_CEF_PREWARM=0 \
        OPENHUMAN_DISABLE_GPU=1 \
        ${dbus_wrapper[@]+"${dbus_wrapper[@]}"} \
        "$appdir/AppRun"
  ) >"$log_file" 2>&1; then
    status=0
  else
    status=$?
  fi

  local forbidden=0
  if grep -Eiq \
    'anylinux\.so.*cannot be preloaded|cannot be preloaded.*anylinux\.so' \
    "$log_file"; then
    forbidden=1
  fi
  if grep -Eiq \
    'libxdo\.so\.3.*cannot open shared object file|cannot open shared object file.*libxdo\.so\.3' \
    "$log_file"; then
    forbidden=1
  fi
  if grep -Eiq \
    'libcef\.so.*cannot open shared object file|cannot open shared object file.*libcef\.so' \
    "$log_file"; then
    forbidden=1
  fi
  if grep -Eiq 'error while loading shared libraries' "$log_file"; then
    forbidden=1
  fi

  # Name this failure explicitly. It exits 101 like any other Rust panic, and the
  # generic "status 101" message sent the last investigation looking at the
  # non-fatal `libcef.so => not found` ldd lines instead of the actual cause.
  if grep -Eq "unsupported transport 'disabled'|single-instance/src/platform_impl/linux\.rs" \
    "$log_file"; then
    forbidden=1
    echo "[appimage-runtime] Detected the single-instance D-Bus panic — the smoke" \
      "environment has no usable session bus. Check the dbus-run-session wrapper" \
      "above and that 'dbus'/'dbus-x11' are installed on the runner." >&2
  fi

  # The desktop process is expected to remain alive until timeout ends the
  # startup window. An earlier clean exit is a failure as well as loader output.
  if [ "$forbidden" -ne 0 ] || [ "$status" -ne 124 ]; then
    echo "[appimage-runtime] AppImage startup smoke failed (status $status):" >&2
    sed 's/^/[appimage-runtime]   /' "$log_file" >&2
    return 1
  fi

  grep -Ei 'loader|loading|cef|startup|started|ready' "$log_file" \
    | sed 's/^/[appimage-runtime]   /' \
    || true
  echo "[appimage-runtime] Application remained alive for the 15-second startup window"
}

# Ubuntu 23.10+ (the hosted ubuntu-24.04 runner included) sets
# `kernel.apparmor_restrict_unprivileged_userns=1`, which denies unprivileged
# user-namespace creation to every *unconfined* process on the box. Chromium's
# zygote needs one, so CEF aborts during startup with
# `FATAL:zygote_host_impl_linux.cc] No usable sandbox!` and the smoke sees exit
# 133 (SIGTRAP) instead of the expected 124 (still alive at the timeout).
#
# `install_smoke_userns_profile` below is Chromium's second documented remedy —
# a per-executable AppArmor profile carrying `userns,`. It loads successfully
# but never takes effect here, because the profile attaches by execve path and
# the sharun launcher runs the app through the AppDir's bundled dynamic loader
# rather than exec'ing `shared/bin/OpenHuman` directly (Chromium then re-execs
# `/proc/self/exe` for the zygote). Toggling the sysctl is Chromium's first
# documented remedy and is path-independent, so it cannot miss the way the
# profile attachment does. The AppArmor profile is retained alongside it: it is
# harmless, and it keeps the narrower per-executable grant in place for hosts
# where the sysctl is unavailable.
#
# The original value is restored after the smoke window so the runner is left
# exactly as it was found.
SMOKE_USERNS_SYSCTL="kernel.apparmor_restrict_unprivileged_userns"

smoke_userns_sysctl_value() {
  sysctl -n "$SMOKE_USERNS_SYSCTL" 2>/dev/null || true
}

relax_smoke_userns_restriction() {
  local previous_file="$1"
  local current
  current="$(smoke_userns_sysctl_value)"

  if [ -z "$current" ]; then
    echo "[appimage-runtime] $SMOKE_USERNS_SYSCTL is absent; unprivileged user namespaces are already unrestricted"
    return 0
  fi
  if [ "$current" = "0" ]; then
    echo "[appimage-runtime] $SMOKE_USERNS_SYSCTL is already 0; leaving it unchanged"
    return 0
  fi

  command -v sudo >/dev/null 2>&1 \
    || { runtime_validation_error "sudo is required to relax $SMOKE_USERNS_SYSCTL for the AppImage smoke"; return 1; }
  sudo --non-interactive sysctl -q -w "$SMOKE_USERNS_SYSCTL=0" \
    || { runtime_validation_error "could not relax $SMOKE_USERNS_SYSCTL for the AppImage smoke"; return 1; }

  printf '%s\n' "$current" >"$previous_file"
  echo "[appimage-runtime] Relaxed $SMOKE_USERNS_SYSCTL: $current -> 0 for the smoke window"
}

restore_smoke_userns_restriction() {
  local previous_file="$1"
  [ -s "$previous_file" ] || return 0

  local previous
  previous="$(cat "$previous_file")"
  rm -f "$previous_file"
  sudo --non-interactive sysctl -q -w "$SMOKE_USERNS_SYSCTL=$previous" \
    || { runtime_validation_error "could not restore $SMOKE_USERNS_SYSCTL to $previous"; return 1; }
  echo "[appimage-runtime] Restored $SMOKE_USERNS_SYSCTL to $previous"
}

install_smoke_userns_profile() {
  local appdir="$1"
  local profile_file="$2"
  local executable="$appdir/shared/bin/OpenHuman"

  [ -x "$executable" ] \
    || { runtime_validation_error "AppArmor target is not executable: $executable"; return 1; }
  command -v sudo >/dev/null 2>&1 \
    || { runtime_validation_error "sudo is required for the AppImage smoke AppArmor profile"; return 1; }
  command -v apparmor_parser >/dev/null 2>&1 \
    || { runtime_validation_error "apparmor_parser is required for the AppImage smoke AppArmor profile"; return 1; }

  local profile_name="openhuman_appimage_runtime_smoke_$$"
  printf '%s\n' \
    'abi <abi/4.0>,' \
    'include <tunables/global>' \
    '' \
    "profile $profile_name \"$executable\" flags=(unconfined) {" \
    '  userns,' \
    '}' \
    >"$profile_file"

  sudo --non-interactive apparmor_parser --replace "$profile_file" >/dev/null \
    || { runtime_validation_error "could not load the AppImage smoke AppArmor profile"; return 1; }
  echo "[appimage-runtime] Loaded temporary AppArmor userns profile for: $executable"
}

remove_smoke_userns_profile() {
  local profile_file="$1"
  sudo --non-interactive apparmor_parser --remove "$profile_file" >/dev/null \
    || { runtime_validation_error "could not remove the AppImage smoke AppArmor profile"; return 1; }
  echo "[appimage-runtime] Removed temporary AppArmor userns profile: $profile_file"
}

smoke_extracted_apprun_with_userns() {
  local appdir="$1"
  local foreign_cwd="$2"
  local log_file="$3"
  local profile_file="$4"
  local profile_loaded=0
  # Derived rather than passed so the four-argument contract is unchanged.
  local userns_sysctl_file="$profile_file.sysctl"
  local previous_hup_trap previous_int_trap previous_term_trap
  previous_hup_trap="$(trap -p HUP)"
  previous_int_trap="$(trap -p INT)"
  previous_term_trap="$(trap -p TERM)"

  restore_smoke_signal_traps() {
    trap - HUP INT TERM
    [ -z "$previous_hup_trap" ] || eval "$previous_hup_trap"
    [ -z "$previous_int_trap" ] || eval "$previous_int_trap"
    [ -z "$previous_term_trap" ] || eval "$previous_term_trap"
  }

  cleanup_smoke_userns_profile() {
    if [ "$profile_loaded" -eq 1 ]; then
      profile_loaded=0
      remove_smoke_userns_profile "$profile_file" \
        || echo "[appimage-runtime] ERROR: AppArmor cleanup failed after smoke interruption" >&2
    fi
    restore_smoke_userns_restriction "$userns_sysctl_file" \
      || echo "[appimage-runtime] ERROR: $SMOKE_USERNS_SYSCTL restore failed after smoke interruption" >&2
  }

  interrupt_smoke_with_userns() {
    local exit_status="$1"
    trap - HUP INT TERM
    cleanup_smoke_userns_profile
    exit "$exit_status"
  }
  trap 'interrupt_smoke_with_userns 129' HUP
  trap 'interrupt_smoke_with_userns 130' INT
  trap 'interrupt_smoke_with_userns 143' TERM

  rm -f "$userns_sysctl_file"
  if ! relax_smoke_userns_restriction "$userns_sysctl_file"; then
    restore_smoke_signal_traps
    return 1
  fi

  if ! install_smoke_userns_profile "$appdir" "$profile_file"; then
    restore_smoke_userns_restriction "$userns_sysctl_file" \
      || echo "[appimage-runtime] ERROR: $SMOKE_USERNS_SYSCTL restore failed after AppArmor setup failure" >&2
    restore_smoke_signal_traps
    return 1
  fi
  profile_loaded=1

  local smoke_status
  if smoke_extracted_apprun "$appdir" "$foreign_cwd" "$log_file"; then
    smoke_status=0
  else
    smoke_status=$?
  fi

  profile_loaded=0
  local remove_status
  if remove_smoke_userns_profile "$profile_file"; then
    remove_status=0
  else
    remove_status=$?
  fi

  local restore_status
  if restore_smoke_userns_restriction "$userns_sysctl_file"; then
    restore_status=0
  else
    restore_status=$?
  fi

  restore_smoke_signal_traps
  # Preserve the original smoke failure even if cleanup also fails.
  [ "$smoke_status" -eq 0 ] || return "$smoke_status"
  [ "$remove_status" -eq 0 ] || return "$remove_status"
  return "$restore_status"
}

validate_final_appimage() (
  local image="$1"
  if ! image="$(realpath "$image")"; then
    runtime_validation_error "could not resolve AppImage path: $1"
    return 1
  fi
  if [ ! -f "$image" ] || [ ! -x "$image" ]; then
    runtime_validation_error "AppImage must be an executable regular file: $image"
    return 1
  fi
  command -v patchelf >/dev/null 2>&1 \
    || { runtime_validation_error "patchelf is required for final AppImage validation"; return 1; }

  local temp_root
  temp_root="$(mktemp -d)"
  trap 'rm -rf -- "$temp_root"' EXIT
  local foreign_cwd="$temp_root/foreign-cwd"
  local extraction_log="$temp_root/extraction.log"
  local smoke_log="$temp_root/smoke.log"
  mkdir -p "$foreign_cwd"

  if ! (
    cd "$foreign_cwd"
    "$image" --appimage-extract
  ) >"$extraction_log" 2>&1; then
    echo "[appimage-runtime] AppImage extraction failed:" >&2
    sed 's/^/[appimage-runtime]   /' "$extraction_log" >&2
    return 1
  fi

  local appdir="$foreign_cwd/squashfs-root"
  if [ ! -d "$appdir" ]; then
    runtime_validation_error "extraction did not create $appdir"
    return 1
  fi
  validate_extracted_appdir "$appdir" || return 1

  if [ "${APPIMAGE_RUNTIME_SMOKE:-0}" = "1" ]; then
    if [ "${APPIMAGE_RUNTIME_APPARMOR_USERNS:-0}" = "1" ]; then
      smoke_extracted_apprun_with_userns \
        "$appdir" \
        "$foreign_cwd" \
        "$smoke_log" \
        "$temp_root/openhuman-appimage-smoke.profile" \
        || return 1
    else
      smoke_extracted_apprun "$appdir" "$foreign_cwd" "$smoke_log" || return 1
    fi
  else
    echo "[appimage-runtime] Static validation complete; executable smoke disabled for this architecture"
  fi
)

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  [ "$#" -eq 1 ] || {
    echo "Usage: $0 <final.AppImage>" >&2
    exit 2
  }
  APPIMAGE_EXPECTED_NEEDED="libxdo.so.3 libcef.so" \
    validate_final_appimage "$1"
fi
