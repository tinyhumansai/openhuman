#!/usr/bin/env bash
# Regression test for the issue #3224 hardening in
# strip-appimage-graphics-libs.sh:
#   1. sanitize_elf_rpaths        — strips /home/runner|/__w build-machine
#                                    RPATHs from bundled ELFs, rewriting to
#                                    $ORIGIN-relative.
#   2. validate_appimage_required_libs — hard-fails when anylinux.so,
#                                    libxdo.so.*, or libcef.so is missing from
#                                    a sharun AppDir.
#   3. validate-appimage-runtime.sh — checks the final extracted sharun layout
#                                    and the pre-signing validation seam.
#
# Linux-only: needs `patchelf` and a host ELF to mutate. Skips cleanly (exit 0)
# on macOS / any host without patchelf so it is a no-op on dev boxes and a real
# gate in CI (where build-desktop.yml already apt-installs patchelf).
#
# Usage: bash scripts/release/test-strip-appimage-rpaths.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$SCRIPT_DIR/strip-appimage-graphics-libs.sh"

skip() {
  echo "[test-rpaths] SKIP: $1"
  exit 0
}

[ -f "$TARGET" ] || { echo "[test-rpaths] FAIL: $TARGET not found" >&2; exit 1; }
command -v patchelf >/dev/null 2>&1 || skip "patchelf not installed (expected on non-Linux dev boxes)"

# Find a small host ELF we can copy + mutate.
HOST_ELF=""
for cand in /bin/true /usr/bin/true /bin/echo; do
  if [ -f "$cand" ] && [ "$(LC_ALL=C head -c 4 "$cand" 2>/dev/null || true)" = $'\177ELF' ]; then
    HOST_ELF="$cand"
    break
  fi
done
[ -n "$HOST_ELF" ] || skip "no host ELF available to build a fixture"

# Source the target so we can call its functions in isolation. The
# sourced-vs-executed guard at the bottom of the script keeps main() from
# running here.
# shellcheck source=/dev/null
source "$TARGET"

RUNTIME_VALIDATOR="$SCRIPT_DIR/validate-appimage-runtime.sh"
[ -f "$RUNTIME_VALIDATOR" ] \
  || { echo "[test-rpaths] FAIL: $RUNTIME_VALIDATOR not found" >&2; exit 1; }
# shellcheck source=/dev/null
source "$RUNTIME_VALIDATOR"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail() { echo "[test-rpaths] FAIL: $1" >&2; exit 1; }

make_sharun_appdir() {
  local appdir="$1"
  mkdir -p "$appdir/shared/bin" "$appdir/shared/lib/plugins" "$appdir/bin"

  cp "$HOST_ELF" "$appdir/sharun"
  printf 'Interpreter not found!' >> "$appdir/sharun"
  chmod +x "$appdir/sharun"
  ln "$appdir/sharun" "$appdir/AppRun"
  ln "$appdir/sharun" "$appdir/bin/OpenHuman"

  cp "$HOST_ELF" "$appdir/shared/bin/OpenHuman"
  chmod +x "$appdir/shared/bin/OpenHuman"
}

assert_lib_path_contents() {
  local fixture="$1"
  local expected="$2"
  local expected_file="$fixture/expected-lib.path"
  printf '%s\n' "$expected" >"$expected_file"
  cmp -s "$expected_file" "$fixture/shared/lib/lib.path" \
    || fail "unexpected normalized lib.path for ${fixture##*/}"
}

# --- Case 0: rewrite_sharun_lib_path emits sharun-native marker entries -----
REWRITE_SQUASHFS="$WORK/rewrite-squashfs"
make_sharun_appdir "$REWRITE_SQUASHFS"
printf '%s\n' "$WORK/squashfs-root/shared/lib" >"$REWRITE_SQUASHFS/shared/lib/lib.path"
rewrite_sharun_lib_path "$REWRITE_SQUASHFS" \
  || fail "squashfs-root shared/lib was not normalized"
assert_lib_path_contents "$REWRITE_SQUASHFS" '+'

REWRITE_STAGING="$WORK/rewrite-staging"
make_sharun_appdir "$REWRITE_STAGING"
printf '%s\n' "$WORK/appimage_deb/data/usr/lib" >"$REWRITE_STAGING/shared/lib/lib.path"
rewrite_sharun_lib_path "$REWRITE_STAGING" \
  || fail "appimage_deb data/usr/lib was not normalized"
assert_lib_path_contents "$REWRITE_STAGING" '+'

REWRITE_DESCENDANTS="$WORK/rewrite-descendants"
make_sharun_appdir "$REWRITE_DESCENDANTS"
printf '%s\n%s\n' \
  "$WORK/squashfs-root/shared/lib/plugins" \
  "$WORK/appimage_deb/data/usr/lib/plugins" \
  >"$REWRITE_DESCENDANTS/shared/lib/lib.path"
rewrite_sharun_lib_path "$REWRITE_DESCENDANTS" \
  || fail "AppDir library descendants were not normalized"
assert_lib_path_contents "$REWRITE_DESCENDANTS" '+/plugins'

REWRITE_MIXED="$WORK/rewrite-mixed"
make_sharun_appdir "$REWRITE_MIXED"
printf '%s\n' '+' '+/plugins' '+' '/opt/unrelated/lib' \
  >"$REWRITE_MIXED/shared/lib/lib.path"
rewrite_sharun_lib_path "$REWRITE_MIXED" \
  || fail "mixed marker entries were not normalized"
assert_lib_path_contents "$REWRITE_MIXED" "$(printf '+\n+/plugins')"

cp "$REWRITE_MIXED/shared/lib/lib.path" "$REWRITE_MIXED/lib.path.before"
if rewrite_sharun_lib_path "$REWRITE_MIXED"; then
  fail "rewrite_sharun_lib_path was not idempotent"
fi
cmp -s "$REWRITE_MIXED/lib.path.before" "$REWRITE_MIXED/shared/lib/lib.path" \
  || fail "idempotent rewrite changed lib.path bytes"

REWRITE_INVALID="$WORK/rewrite-invalid"
make_sharun_appdir "$REWRITE_INVALID"
printf '%s\n' '/opt/unrelated/lib' >"$REWRITE_INVALID/shared/lib/lib.path"
if ( rewrite_sharun_lib_path "$REWRITE_INVALID" ) 2>"$WORK/rewrite.err"; then
  fail "rewrite accepted a lib.path with no AppDir library root"
fi
grep -F "no valid AppDir library entries" "$WORK/rewrite.err" >/dev/null \
  || fail "rewrite failure did not explain why lib.path was rejected"
assert_lib_path_contents "$REWRITE_INVALID" '/opt/unrelated/lib'

REWRITE_BARE_RELATIVE="$WORK/rewrite-bare-relative"
make_sharun_appdir "$REWRITE_BARE_RELATIVE"
printf '%s\n' '+' 'shared/lib' \
  >"$REWRITE_BARE_RELATIVE/shared/lib/lib.path"
if ( rewrite_sharun_lib_path "$REWRITE_BARE_RELATIVE" ) \
  2>"$REWRITE_BARE_RELATIVE/error.log"; then
  fail "rewrite accepted a mixed lib.path containing a bare-relative entry"
fi
grep -F "malformed entry: 'shared/lib'" \
  "$REWRITE_BARE_RELATIVE/error.log" >/dev/null \
  || fail "rewrite bare-relative error did not identify 'shared/lib'"
assert_lib_path_contents "$REWRITE_BARE_RELATIVE" "$(printf '+\nshared/lib')"

REWRITE_MARKER_INJECTION="$WORK/rewrite-marker-injection"
make_sharun_appdir "$REWRITE_MARKER_INJECTION"
mkdir -p "$REWRITE_MARKER_INJECTION/shared/lib/plugins+extra"
printf '%s\n' '+/plugins+extra' \
  >"$REWRITE_MARKER_INJECTION/shared/lib/lib.path"
if ( rewrite_sharun_lib_path "$REWRITE_MARKER_INJECTION" ) \
  2>"$REWRITE_MARKER_INJECTION/error.log"; then
  fail "rewrite accepted an extra marker in a canonical suffix"
fi
grep -F "malformed entry: '+/plugins+extra'" \
  "$REWRITE_MARKER_INJECTION/error.log" >/dev/null \
  || fail "rewrite marker-injection error did not identify '+/plugins+extra'"

REWRITE_DERIVED_MARKER="$WORK/rewrite-derived-marker"
make_sharun_appdir "$REWRITE_DERIVED_MARKER"
derived_marker_path="$WORK/squashfs-root/shared/lib/plugins+extra"
printf '%s\n' "$derived_marker_path" \
  >"$REWRITE_DERIVED_MARKER/shared/lib/lib.path"
if ( rewrite_sharun_lib_path "$REWRITE_DERIVED_MARKER" ) \
  2>"$REWRITE_DERIVED_MARKER/error.log"; then
  fail "rewrite accepted an extra marker in a derived suffix"
fi
grep -F "malformed entry: '$derived_marker_path'" \
  "$REWRITE_DERIVED_MARKER/error.log" >/dev/null \
  || fail "derived marker-injection error did not identify '$derived_marker_path'"
echo "[test-rpaths] ok: sharun lib.path normalization is canonical and idempotent"

# --- Case 0b: validate_sharun_lib_path rejects unsafe loader entries --------
assert_lib_path_rejected() {
  local label="$1"
  local value="$2"
  local expected_entry="$3"
  local fixture="$WORK/reject-$label"
  make_sharun_appdir "$fixture"
  printf '%s\n' "$value" >"$fixture/shared/lib/lib.path"

  if ( validate_sharun_lib_path "$fixture" ) 2>"$fixture/error.log"; then
    fail "validate_sharun_lib_path accepted $label"
  fi
  grep -F "invalid sharun lib.path entry '$expected_entry':" \
    "$fixture/error.log" >/dev/null \
    || fail "$label error did not identify offending entry '$expected_entry'"
}

assert_lib_path_rejected bare-relative 'shared/lib' 'shared/lib'
assert_lib_path_rejected runner-absolute \
  '/home/runner/work/openhuman/squashfs-root/shared/lib' \
  '/home/runner/work/openhuman/squashfs-root/shared/lib'
assert_lib_path_rejected actions-absolute \
  '/__w/openhuman/openhuman/appimage_deb/data/usr/lib' \
  '/__w/openhuman/openhuman/appimage_deb/data/usr/lib'
assert_lib_path_rejected parent-traversal '+/../outside' '+/../outside'
assert_lib_path_rejected nested-traversal \
  '+/plugins/../../outside' '+/plugins/../../outside'
assert_lib_path_rejected missing-directory '+/missing' '+/missing'
assert_lib_path_rejected loader-separator \
  '+/plugins:shared/lib' '+/plugins:shared/lib'
assert_lib_path_rejected leading-whitespace ' +' ' +'
assert_lib_path_rejected trailing-whitespace '+ ' '+ '

VALID_ROOT="$WORK/valid-root"
make_sharun_appdir "$VALID_ROOT"
printf '%s\n' '+' >"$VALID_ROOT/shared/lib/lib.path"
validate_sharun_lib_path "$VALID_ROOT" \
  || fail "validate_sharun_lib_path rejected canonical root marker"

VALID_DESCENDANT="$WORK/valid-descendant"
make_sharun_appdir "$VALID_DESCENDANT"
printf '%s\n' '+' '+/plugins' >"$VALID_DESCENDANT/shared/lib/lib.path"
cp "$VALID_DESCENDANT/shared/lib/lib.path" "$VALID_DESCENDANT/lib.path.before"
validate_sharun_lib_path "$VALID_DESCENDANT" \
  || fail "validate_sharun_lib_path rejected canonical descendant marker"
validate_sharun_lib_path "$VALID_DESCENDANT" \
  || fail "validate_sharun_lib_path rejected a second validation pass"
cmp -s \
  "$VALID_DESCENDANT/lib.path.before" \
  "$VALID_DESCENDANT/shared/lib/lib.path" \
  || fail "validate_sharun_lib_path mutated a normalized file"

MARKER_INJECTION="$WORK/marker-injection"
make_sharun_appdir "$MARKER_INJECTION"
mkdir -p "$MARKER_INJECTION/shared/lib/plugins+extra"
printf '%s\n' '+/plugins+extra' >"$MARKER_INJECTION/shared/lib/lib.path"
if ( validate_sharun_lib_path "$MARKER_INJECTION" ) \
  2>"$MARKER_INJECTION/error.log"; then
  fail "validate_sharun_lib_path accepted an extra marker in a suffix"
fi
grep -F "invalid sharun lib.path entry '+/plugins+extra':" \
  "$MARKER_INJECTION/error.log" >/dev/null \
  || fail "marker-injection error did not identify '+/plugins+extra'"

ESCAPE_DIR="$WORK/escape-symlink"
make_sharun_appdir "$ESCAPE_DIR"
mkdir -p "$WORK/outside"
ln -s "$WORK/outside" "$ESCAPE_DIR/shared/lib/escape"
printf '%s\n' '+/escape' >"$ESCAPE_DIR/shared/lib/lib.path"
if ( validate_sharun_lib_path "$ESCAPE_DIR" ) 2>"$ESCAPE_DIR/error.log"; then
  fail "validate_sharun_lib_path accepted a symlink escaping shared/lib"
fi
grep -F "invalid sharun lib.path entry '+/escape':" \
  "$ESCAPE_DIR/error.log" >/dev/null \
  || fail "symlink escape error did not identify offending entry '+/escape'"

RELEASED_LAYOUT="$WORK/released-layout"
make_sharun_appdir "$RELEASED_LAYOUT"
printf '%s\n' '+' >"$RELEASED_LAYOUT/shared/lib/lib.path"
[ "$RELEASED_LAYOUT/AppRun" -ef "$RELEASED_LAYOUT/sharun" ] \
  || fail "AppRun fixture is not hard-linked to sharun"
[ "$RELEASED_LAYOUT/bin/OpenHuman" -ef "$RELEASED_LAYOUT/sharun" ] \
  || fail "bin/OpenHuman fixture is not hard-linked to sharun"
is_executable_elf "$RELEASED_LAYOUT/AppRun" \
  || fail "released-style AppRun fixture is not ELF"
uses_sharun_launcher "$RELEASED_LAYOUT" \
  || fail "released-style hard-linked ELF launcher was not detected"
validate_sharun_lib_path "$RELEASED_LAYOUT" \
  || fail "released-style AppDir has an invalid canonical lib.path"

CALLER="$WORK/caller"
mkdir -p "$CALLER"
(
  cd "$CALLER"
  expanded="$(
    sed "s|+|$RELEASED_LAYOUT/shared/lib|g" \
      "$RELEASED_LAYOUT/shared/lib/lib.path" |
      paste -sd ':' -
  )"
  [ "$expanded" = "$RELEASED_LAYOUT/shared/lib" ] \
    || fail "sharun marker did not expand beneath the fixture AppDir"
)

released_apprun_inode="$(ls -di "$RELEASED_LAYOUT/AppRun")"
if patch_apprun_sharun_cwd "$RELEASED_LAYOUT"; then
  fail "patch_apprun_sharun_cwd modified a released-style ELF AppRun"
fi
[ "$released_apprun_inode" = "$(ls -di "$RELEASED_LAYOUT/AppRun")" ] \
  || fail "patch_apprun_sharun_cwd changed the released-style ELF AppRun inode"
echo "[test-rpaths] ok: sharun lib.path validation is fail-closed for the released ELF launcher"

# --- Case 0c: final extracted runtime layout is validated before signing ----
make_runtime_appdir() {
  local appdir="$1"
  make_sharun_appdir "$appdir"
  printf '%s\n' '+' >"$appdir/shared/lib/lib.path"

  cp "$HOST_ELF" "$appdir/shared/lib/anylinux.so"
  cp "$HOST_ELF" "$appdir/shared/lib/libxdo.so.3"
  cp "$HOST_ELF" "$appdir/shared/lib/libcef.so"

  local elf
  for elf in \
    "$appdir/AppRun" \
    "$appdir/sharun" \
    "$appdir/bin/OpenHuman" \
    "$appdir/shared/bin/OpenHuman" \
    "$appdir/shared/lib/anylinux.so" \
    "$appdir/shared/lib/libxdo.so.3" \
    "$appdir/shared/lib/libcef.so"; do
    patchelf --remove-rpath "$elf"
  done
}

assert_runtime_layout_rejected() {
  local label="$1"
  local expected="$2"
  shift 2
  local fixture="$WORK/runtime-$label"
  make_runtime_appdir "$fixture"
  "$@" "$fixture"
  if ( APPIMAGE_EXPECTED_NEEDED="" validate_extracted_appdir "$fixture" ) \
    >"$fixture/output.log" 2>&1; then
    fail "validate_extracted_appdir accepted $label"
  fi
  grep -F "$expected" "$fixture/output.log" >/dev/null \
    || fail "$label failure did not report '$expected'"
}

remove_anylinux() { rm -f "$1/shared/lib/anylinux.so"; }
remove_libxdo() { rm -f "$1/shared/lib/libxdo.so.3"; }
remove_libcef() { rm -f "$1/shared/lib/libcef.so"; }
remove_real_app() { rm -f "$1/shared/bin/OpenHuman"; }
replace_real_app_with_text() {
  rm -f "$1/shared/bin/OpenHuman"
  printf '%s\n' 'not an ELF' >"$1/shared/bin/OpenHuman"
  chmod +x "$1/shared/bin/OpenHuman"
}
replace_apprun() {
  rm -f "$1/AppRun"
  cp "$HOST_ELF" "$1/AppRun"
  chmod +x "$1/AppRun"
}
replace_bin_launcher() {
  rm -f "$1/bin/OpenHuman"
  cp "$HOST_ELF" "$1/bin/OpenHuman"
  chmod +x "$1/bin/OpenHuman"
}
inject_runner_rpath() {
  patchelf --set-rpath \
    '/home/runner/work/openhuman/openhuman/shared/lib' \
    "$1/shared/lib/libcef.so"
}
inject_actions_rpath() {
  patchelf --set-rpath \
    '/__w/openhuman/openhuman/shared/lib' \
    "$1/shared/lib/libcef.so"
}

RUNTIME_COMPLETE="$WORK/runtime-complete"
make_runtime_appdir "$RUNTIME_COMPLETE"
APPIMAGE_EXPECTED_NEEDED="" validate_extracted_appdir "$RUNTIME_COMPLETE" \
  || fail "validate_extracted_appdir rejected a complete runtime fixture"

RUNTIME_EQUIVALENT="$WORK/runtime-equivalent-launchers"
make_runtime_appdir "$RUNTIME_EQUIVALENT"
rm "$RUNTIME_EQUIVALENT/AppRun" "$RUNTIME_EQUIVALENT/bin/OpenHuman"
cp "$RUNTIME_EQUIVALENT/sharun" "$RUNTIME_EQUIVALENT/AppRun"
cp "$RUNTIME_EQUIVALENT/sharun" "$RUNTIME_EQUIVALENT/bin/OpenHuman"
chmod +x "$RUNTIME_EQUIVALENT/AppRun" "$RUNTIME_EQUIVALENT/bin/OpenHuman"
APPIMAGE_EXPECTED_NEEDED="" validate_extracted_appdir "$RUNTIME_EQUIVALENT" \
  || fail "validate_extracted_appdir rejected byte-equivalent launchers"

FAKE_APPIMAGE="$WORK/final-fixture.AppImage"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  '[ "$1" = "--appimage-extract" ] || exit 9' \
  'cp -R "$EXTRACT_SOURCE" squashfs-root' \
  >"$FAKE_APPIMAGE"
chmod +x "$FAKE_APPIMAGE"
export EXTRACT_SOURCE="$RUNTIME_COMPLETE"
APPIMAGE_EXPECTED_NEEDED="" validate_final_appimage "$FAKE_APPIMAGE" \
  || fail "validate_final_appimage rejected a complete extracted fixture"

if APPIMAGE_EXPECTED_NEEDED="" \
  "$RUNTIME_VALIDATOR" "$FAKE_APPIMAGE" \
  >"$WORK/direct-validator.log" 2>&1; then
  fail "direct validator allowed a fixture override to disable production NEEDED checks"
fi
grep -F "missing NEEDED entry 'libxdo.so.3'" \
  "$WORK/direct-validator.log" >/dev/null \
  || fail "direct validator did not enforce the production NEEDED contract"

RUNTIME_BAD_LIBPATH="$WORK/runtime-bad-libpath"
cp -R "$RUNTIME_COMPLETE" "$RUNTIME_BAD_LIBPATH"
printf '%s\n' 'shared/lib' >"$RUNTIME_BAD_LIBPATH/shared/lib/lib.path"
if ( APPIMAGE_EXPECTED_NEEDED="" \
  validate_extracted_appdir "$RUNTIME_BAD_LIBPATH" ) \
  >"$RUNTIME_BAD_LIBPATH/conditional.log" 2>&1; then
  fail "conditional validate_extracted_appdir suppressed lib.path validation failure"
fi

RUNTIME_FINAL_INVALID="$WORK/runtime-final-invalid"
cp -R "$RUNTIME_COMPLETE" "$RUNTIME_FINAL_INVALID"
rm -f "$RUNTIME_FINAL_INVALID/shared/lib/anylinux.so"
if ( EXTRACT_SOURCE="$RUNTIME_FINAL_INVALID" APPIMAGE_EXPECTED_NEEDED="" \
  validate_final_appimage "$FAKE_APPIMAGE" ) \
  >"$WORK/conditional-final.log" 2>&1; then
  fail "conditional validate_final_appimage suppressed extracted-layout failure"
fi

REAL_PATCHELF="$(command -v patchelf)"
PATCHELF_FAIL_BIN="$WORK/patchelf-fail-bin"
mkdir -p "$PATCHELF_FAIL_BIN"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'if [ "$1" = "--print-needed" ]; then exit 42; fi' \
  'exec "$REAL_PATCHELF" "$@"' \
  >"$PATCHELF_FAIL_BIN/patchelf"
chmod +x "$PATCHELF_FAIL_BIN/patchelf"
export REAL_PATCHELF
if ( PATH="$PATCHELF_FAIL_BIN:$PATH" APPIMAGE_EXPECTED_NEEDED="" \
  validate_extracted_appdir "$RUNTIME_COMPLETE" ) \
  >"$WORK/patchelf-failure.log" 2>&1; then
  fail "conditional validate_extracted_appdir suppressed patchelf failure"
fi

SMOKE_BIN="$WORK/smoke-bin"
mkdir -p "$SMOKE_BIN"
SMOKE_RECORD="$WORK/smoke-command-record"
SMOKE_PROBE="$WORK/smoke-probe-record"
REAL_ENV="$(command -v env)"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'printf "timeout-cwd=%s\n" "$PWD" >>"$SMOKE_RECORD"' \
  'while [ "$#" -gt 0 ]; do' \
  '  case "$1" in' \
  '    --signal=*|--kill-after=*|15s) shift ;;' \
  '    *) break ;;' \
  '  esac' \
  'done' \
  '"$@"' \
  '[ -n "${SMOKE_TIMEOUT_MESSAGE:-}" ] && printf "%s\n" "$SMOKE_TIMEOUT_MESSAGE" >&2' \
  'exit "${SMOKE_TIMEOUT_STATUS:-124}"' \
  >"$SMOKE_BIN/timeout"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'printf "xvfb-invoked\n" >>"$SMOKE_RECORD"' \
  'while [ "$#" -gt 0 ]; do' \
  '  case "$1" in' \
  '    -a|--server-args=*) shift ;;' \
  '    *) break ;;' \
  '  esac' \
  'done' \
  'exec "$@"' \
  >"$SMOKE_BIN/xvfb-run"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'args=("$@")' \
  'index=0' \
  'while [ "$index" -lt "${#args[@]}" ]; do' \
  '  arg="${args[$index]}"' \
  '  case "$arg" in' \
  '    -u)' \
  '      next=$((index + 1))' \
  '      printf "env-unset=%s\n" "${args[$next]}" >>"$SMOKE_RECORD"' \
  '      index=$((index + 2))' \
  '      ;;' \
  '    HOME=*|XDG_CONFIG_HOME=*|XDG_DATA_HOME=*|XDG_CACHE_HOME=*)' \
  '      printf "env-assignment=%s\n" "$arg" >>"$SMOKE_RECORD"' \
  '      index=$((index + 1))' \
  '      ;;' \
  '    OPENHUMAN_CEF_PREWARM=*|OPENHUMAN_DISABLE_GPU=*) index=$((index + 1)) ;;' \
  '    *) break ;;' \
  '  esac' \
  'done' \
  'exec "$REAL_ENV" "${args[@]}"' \
  >"$SMOKE_BIN/env"
chmod +x "$SMOKE_BIN/timeout" "$SMOKE_BIN/xvfb-run" "$SMOKE_BIN/env"
SMOKE_APPDIR="$WORK/smoke-appdir"
mkdir -p "$SMOKE_APPDIR"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'printf "apprun-cwd=%s\n" "$PWD" >>"$SMOKE_PROBE"' \
  'printf "home=%s\n" "$HOME" >>"$SMOKE_PROBE"' \
  'printf "config=%s\n" "$XDG_CONFIG_HOME" >>"$SMOKE_PROBE"' \
  'printf "data=%s\n" "$XDG_DATA_HOME" >>"$SMOKE_PROBE"' \
  'printf "cache=%s\n" "$XDG_CACHE_HOME" >>"$SMOKE_PROBE"' \
  '[ -z "${GITHUB_TOKEN+x}" ] || exit 71' \
  '[ -z "${GH_TOKEN+x}" ] || exit 72' \
  '[ -z "${TAURI_SIGNING_PRIVATE_KEY+x}" ] || exit 73' \
  '[ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD+x}" ] || exit 74' \
  '[ -z "${SENTRY_AUTH_TOKEN+x}" ] || exit 75' \
  '[ -z "${OPENAI_API_KEY+x}" ] || exit 76' \
  'printf "probe-executed\n" >>"$SMOKE_PROBE"' \
  >"$SMOKE_APPDIR/AppRun"
chmod +x "$SMOKE_APPDIR/AppRun"
export SMOKE_RECORD SMOKE_PROBE REAL_ENV
SMOKE_ROOT="$WORK/smoke-runtime"
mkdir -p "$SMOKE_ROOT/foreign-cwd"
GITHUB_TOKEN='secret-github-value' \
GH_TOKEN='secret-gh-value' \
TAURI_SIGNING_PRIVATE_KEY='secret-signing-value' \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD='secret-password-value' \
SENTRY_AUTH_TOKEN='secret-sentry-value' \
OPENAI_API_KEY='secret-openai-value' \
PATH="$SMOKE_BIN:$PATH" smoke_extracted_apprun \
  "$SMOKE_APPDIR" \
  "$SMOKE_ROOT/foreign-cwd" \
  "$SMOKE_ROOT/success.log" \
  || fail "smoke_extracted_apprun did not accept timeout status 124"
grep -Fx "timeout-cwd=$SMOKE_ROOT/foreign-cwd" "$SMOKE_RECORD" >/dev/null \
  || fail "smoke did not invoke timeout from the foreign CWD"
grep -Fx "xvfb-invoked" "$SMOKE_RECORD" >/dev/null \
  || fail "smoke did not invoke xvfb-run"
for secret_name in \
  GITHUB_TOKEN \
  GH_TOKEN \
  OPENHUMAN_CEF_NO_SANDBOX \
  TAURI_SIGNING_PRIVATE_KEY \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
  SENTRY_AUTH_TOKEN \
  OPENAI_API_KEY; do
  grep -Fx "env-unset=$secret_name" "$SMOKE_RECORD" >/dev/null \
    || fail "smoke env did not unset $secret_name"
done
grep -Fx "env-assignment=HOME=$SMOKE_ROOT/home" "$SMOKE_RECORD" >/dev/null \
  || fail "smoke HOME was not isolated"
grep -Fx "env-assignment=XDG_CONFIG_HOME=$SMOKE_ROOT/config" "$SMOKE_RECORD" >/dev/null \
  || fail "smoke XDG_CONFIG_HOME was not isolated"
grep -Fx "env-assignment=XDG_DATA_HOME=$SMOKE_ROOT/data" "$SMOKE_RECORD" >/dev/null \
  || fail "smoke XDG_DATA_HOME was not isolated"
grep -Fx "env-assignment=XDG_CACHE_HOME=$SMOKE_ROOT/cache" "$SMOKE_RECORD" >/dev/null \
  || fail "smoke XDG_CACHE_HOME was not isolated"
grep -Fx "apprun-cwd=$SMOKE_ROOT/foreign-cwd" "$SMOKE_PROBE" >/dev/null \
  || fail "smoke AppRun did not execute from the foreign CWD"
grep -Fx "probe-executed" "$SMOKE_PROBE" >/dev/null \
  || fail "smoke AppRun probe did not execute"
if grep -F 'secret-' "$SMOKE_RECORD" "$SMOKE_PROBE" >/dev/null; then
  fail "smoke command records exposed a secret value"
fi
if ( PATH="$SMOKE_BIN:$PATH" SMOKE_TIMEOUT_STATUS=0 \
  smoke_extracted_apprun \
    "$SMOKE_APPDIR" \
    "$SMOKE_ROOT/foreign-cwd" \
    "$SMOKE_ROOT/early-exit.log" ) >/dev/null 2>&1; then
  fail "smoke_extracted_apprun accepted an immediate clean exit"
fi
if ( PATH="$SMOKE_BIN:$PATH" \
  SMOKE_TIMEOUT_MESSAGE='libcef.so: cannot open shared object file' \
  smoke_extracted_apprun \
    "$SMOKE_APPDIR" \
    "$SMOKE_ROOT/foreign-cwd" \
    "$SMOKE_ROOT/loader-error.log" ) >/dev/null 2>&1; then
  fail "smoke_extracted_apprun accepted a forbidden loader diagnostic"
fi
(
  set +e
  PATH="$SMOKE_BIN:$PATH" smoke_extracted_apprun \
    "$SMOKE_APPDIR" \
    "$SMOKE_ROOT/foreign-cwd" \
    "$SMOKE_ROOT/errexit-disabled.log" \
    || exit 81
  case "$-" in
    *e*) exit 82 ;;
  esac
) || fail "smoke_extracted_apprun changed a sourced caller's disabled errexit state"

APPARMOR_BIN="$WORK/apparmor-bin"
APPARMOR_RECORD="$WORK/apparmor-command-record"
APPARMOR_PROFILE="$WORK/openhuman-appimage-smoke.profile"
APPARMOR_PROFILE_SNAPSHOT="$WORK/openhuman-appimage-smoke.profile.loaded"
mkdir -p "$APPARMOR_BIN"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' \
  >"$APPARMOR_BIN/apparmor_parser"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'printf "%s\n" "$*" >>"$APPARMOR_RECORD"' \
  'if [ "$2" = "apparmor_parser" ] && [ "$3" = "--replace" ]; then' \
  '  cp "$4" "$APPARMOR_PROFILE_SNAPSHOT"' \
  'fi' \
  >"$APPARMOR_BIN/sudo"
# Stand in for a restricted Ubuntu 24.04 host so the sysctl branch is exercised
# deterministically on any development machine, including ones with no sysctl
# key of that name at all.
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'if [ "${1:-}" = "-n" ]; then' \
  '  printf "%s\n" "1"' \
  '  exit 0' \
  'fi' \
  'exit 0' \
  >"$APPARMOR_BIN/sysctl"
chmod +x \
  "$APPARMOR_BIN/apparmor_parser" \
  "$APPARMOR_BIN/sudo" \
  "$APPARMOR_BIN/sysctl"
export APPARMOR_RECORD APPARMOR_PROFILE_SNAPSHOT
USERNS_SYSCTL="kernel.apparmor_restrict_unprivileged_userns"
PATH="$APPARMOR_BIN:$PATH" install_smoke_userns_profile \
  "$RUNTIME_COMPLETE" "$APPARMOR_PROFILE" \
  || fail "install_smoke_userns_profile rejected the fixture AppDir"
grep -F "\"$RUNTIME_COMPLETE/shared/bin/OpenHuman\"" \
  "$APPARMOR_PROFILE_SNAPSHOT" >/dev/null \
  || fail "AppArmor profile did not attach to the extracted real executable"
grep -Fx "  userns," "$APPARMOR_PROFILE_SNAPSHOT" >/dev/null \
  || fail "AppArmor profile did not preserve Chromium's userns sandbox"
grep -Fx -- \
  "--non-interactive apparmor_parser --replace $APPARMOR_PROFILE" \
  "$APPARMOR_RECORD" >/dev/null \
  || fail "AppArmor profile was not loaded through non-interactive sudo"
PATH="$APPARMOR_BIN:$PATH" remove_smoke_userns_profile "$APPARMOR_PROFILE" \
  || fail "remove_smoke_userns_profile rejected the loaded profile"
grep -Fx -- \
  "--non-interactive apparmor_parser --remove $APPARMOR_PROFILE" \
  "$APPARMOR_RECORD" >/dev/null \
  || fail "AppArmor profile was not removed after the smoke"

: >"$APPARMOR_RECORD"
set +e
(
  smoke_extracted_apprun() {
    printf '%s\n' "smoke" >>"$APPARMOR_RECORD"
    return 37
  }
  PATH="$APPARMOR_BIN:$PATH" smoke_extracted_apprun_with_userns \
    "$RUNTIME_COMPLETE" \
    "$SMOKE_ROOT/foreign-cwd" \
    "$SMOKE_ROOT/apparmor-failure.log" \
    "$APPARMOR_PROFILE"
)
apparmor_smoke_status=$?
set -e
[ "$apparmor_smoke_status" -eq 37 ] \
  || fail "AppArmor wrapper did not preserve smoke failure status 37"
printf '%s\n' \
  "--non-interactive sysctl -q -w $USERNS_SYSCTL=0" \
  "--non-interactive apparmor_parser --replace $APPARMOR_PROFILE" \
  "smoke" \
  "--non-interactive apparmor_parser --remove $APPARMOR_PROFILE" \
  "--non-interactive sysctl -q -w $USERNS_SYSCTL=1" \
  >"$WORK/apparmor-expected-order"
cmp -s "$WORK/apparmor-expected-order" "$APPARMOR_RECORD" \
  || fail "AppArmor profile was not loaded before and removed after a failing smoke"
[ ! -e "$APPARMOR_PROFILE.sysctl" ] \
  || fail "userns sysctl state file survived a failing smoke"

APPARMOR_TERM_RECORD="$WORK/apparmor-term-command-record"
APPARMOR_TERM_MARKER="$WORK/apparmor-term-smoke-started"
APPARMOR_TERM_RUNNER="$WORK/apparmor-term-runner.sh"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  '# shellcheck source=/dev/null' \
  'source "$RUNTIME_VALIDATOR"' \
  'smoke_extracted_apprun() {' \
  '  printf "%s\n" "smoke-start" >>"$APPARMOR_RECORD"' \
  '  : >"$APPARMOR_TERM_MARKER"' \
  '  while :; do sleep 1; done' \
  '}' \
  'smoke_extracted_apprun_with_userns \' \
  '  "$RUNTIME_COMPLETE" \' \
  '  "$SMOKE_ROOT/foreign-cwd" \' \
  '  "$SMOKE_ROOT/apparmor-term.log" \' \
  '  "$APPARMOR_PROFILE"' \
  >"$APPARMOR_TERM_RUNNER"
chmod +x "$APPARMOR_TERM_RUNNER"
export \
  APPARMOR_TERM_MARKER \
  RUNTIME_COMPLETE \
  RUNTIME_VALIDATOR \
  SMOKE_ROOT \
  APPARMOR_PROFILE
APPARMOR_RECORD="$APPARMOR_TERM_RECORD" \
PATH="$APPARMOR_BIN:$PATH" \
  bash "$APPARMOR_TERM_RUNNER" &
apparmor_term_pid=$!
for _ in $(seq 1 100); do
  if [ -s "$APPARMOR_TERM_RECORD" ] && [ -f "$APPARMOR_TERM_MARKER" ]; then
    break
  fi
  sleep 0.05
done
[ -s "$APPARMOR_TERM_RECORD" ] && [ -f "$APPARMOR_TERM_MARKER" ] || {
  kill -TERM "$apparmor_term_pid" 2>/dev/null || true
  fail "AppArmor TERM fixture did not reach the smoke"
}
kill -TERM "$apparmor_term_pid"
set +e
wait "$apparmor_term_pid"
apparmor_term_status=$?
set -e
[ "$apparmor_term_status" -eq 143 ] \
  || fail "AppArmor TERM fixture exited $apparmor_term_status instead of 143"
printf '%s\n' \
  "--non-interactive sysctl -q -w $USERNS_SYSCTL=0" \
  "--non-interactive apparmor_parser --replace $APPARMOR_PROFILE" \
  "smoke-start" \
  "--non-interactive apparmor_parser --remove $APPARMOR_PROFILE" \
  "--non-interactive sysctl -q -w $USERNS_SYSCTL=1" \
  >"$WORK/apparmor-term-expected-order"
cmp -s "$WORK/apparmor-term-expected-order" "$APPARMOR_TERM_RECORD" \
  || fail "AppArmor profile was not removed exactly once after TERM"
[ ! -e "$APPARMOR_PROFILE.sysctl" ] \
  || fail "userns sysctl state file survived a TERM-interrupted smoke"
echo "[test-rpaths] ok: CI smoke grants userns only to the extracted executable"

# The sysctl toggle must be a no-op on hosts that are already permissive or
# that have no AppArmor userns restriction at all, so the release smoke never
# needs sudo outside the restricted-Ubuntu case it exists for.
USERNS_NOOP_BIN="$WORK/userns-noop-bin"
USERNS_NOOP_RECORD="$WORK/userns-noop-command-record"
USERNS_NOOP_STATE="$WORK/userns-noop.sysctl"
mkdir -p "$USERNS_NOOP_BIN"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'printf "%s\n" "$*" >>"$USERNS_NOOP_RECORD"' \
  >"$USERNS_NOOP_BIN/sudo"
chmod +x "$USERNS_NOOP_BIN/sudo"
export USERNS_NOOP_RECORD
for permissive_value in 0 ""; do
  : >"$USERNS_NOOP_RECORD"
  rm -f "$USERNS_NOOP_STATE"
  if [ -n "$permissive_value" ]; then
    printf '%s\n' \
      '#!/usr/bin/env bash' \
      '[ "${1:-}" = "-n" ] && printf "%s\n" "0"' \
      'exit 0' \
      >"$USERNS_NOOP_BIN/sysctl"
  else
    # No such key on this host: sysctl exits non-zero and prints nothing.
    printf '%s\n' '#!/usr/bin/env bash' 'exit 1' \
      >"$USERNS_NOOP_BIN/sysctl"
  fi
  chmod +x "$USERNS_NOOP_BIN/sysctl"
  PATH="$USERNS_NOOP_BIN:$PATH" \
    relax_smoke_userns_restriction "$USERNS_NOOP_STATE" >/dev/null \
    || fail "relax_smoke_userns_restriction failed on a permissive host"
  [ ! -s "$USERNS_NOOP_RECORD" ] \
    || fail "relax_smoke_userns_restriction used sudo on a permissive host"
  [ ! -e "$USERNS_NOOP_STATE" ] \
    || fail "relax_smoke_userns_restriction recorded state it never changed"
  PATH="$USERNS_NOOP_BIN:$PATH" \
    restore_smoke_userns_restriction "$USERNS_NOOP_STATE" >/dev/null \
    || fail "restore_smoke_userns_restriction failed without recorded state"
  [ ! -s "$USERNS_NOOP_RECORD" ] \
    || fail "restore_smoke_userns_restriction used sudo without recorded state"
done
echo "[test-rpaths] ok: userns sysctl toggle is a no-op on permissive hosts"

assert_runtime_layout_rejected missing-anylinux \
  "missing anylinux.so" remove_anylinux
assert_runtime_layout_rejected missing-libxdo \
  "missing libxdo.so.*" remove_libxdo
assert_runtime_layout_rejected missing-libcef \
  "missing libcef.so" remove_libcef
assert_runtime_layout_rejected missing-real-app \
  "shared/bin/OpenHuman is not an executable ELF" remove_real_app
assert_runtime_layout_rejected text-real-app \
  "shared/bin/OpenHuman is not an executable ELF" replace_real_app_with_text
assert_runtime_layout_rejected mismatched-apprun \
  "AppRun does not match sharun" replace_apprun
assert_runtime_layout_rejected mismatched-bin-launcher \
  "bin/OpenHuman does not match sharun" replace_bin_launcher
assert_runtime_layout_rejected runner-rpath \
  "/home/runner/work/openhuman/openhuman/shared/lib" inject_runner_rpath
assert_runtime_layout_rejected actions-rpath \
  "/__w/openhuman/openhuman/shared/lib" inject_actions_rpath

RUNTIME_NEEDED="$WORK/runtime-needed"
make_runtime_appdir "$RUNTIME_NEEDED"
if ( APPIMAGE_EXPECTED_NEEDED="libxdo.so.3" \
  validate_extracted_appdir "$RUNTIME_NEEDED" ) \
  >"$RUNTIME_NEEDED/missing-xdo.log" 2>&1; then
  fail "validate_extracted_appdir accepted a missing libxdo NEEDED entry"
fi
grep -F "missing NEEDED entry 'libxdo.so.3'" \
  "$RUNTIME_NEEDED/missing-xdo.log" >/dev/null \
  || fail "missing libxdo NEEDED diagnostic was not specific"

patchelf --add-needed libxdo.so.3 "$RUNTIME_NEEDED/shared/bin/OpenHuman"
if ( APPIMAGE_EXPECTED_NEEDED="libcef.so" \
  validate_extracted_appdir "$RUNTIME_NEEDED" ) \
  >"$RUNTIME_NEEDED/missing-cef.log" 2>&1; then
  fail "validate_extracted_appdir accepted a missing libcef NEEDED entry"
fi
grep -F "missing NEEDED entry 'libcef.so'" \
  "$RUNTIME_NEEDED/missing-cef.log" >/dev/null \
  || fail "missing libcef NEEDED diagnostic was not specific"

patchelf --add-needed libcef.so "$RUNTIME_NEEDED/shared/bin/OpenHuman"
APPIMAGE_EXPECTED_NEEDED="libxdo.so.3 libcef.so" \
  validate_extracted_appdir "$RUNTIME_NEEDED" \
  || fail "validate_extracted_appdir rejected complete NEEDED entries"

VALIDATOR_RECORD="$WORK/runtime-validator-record"
runtime_validator_stub() {
  [ -f "$1" ] || return 10
  [ "${#MODIFIED_PATHS[@]}" -eq 0 ] || return 11
  printf '%s\n' "$1" >"$VALIDATOR_RECORD"
}
REBUILT_FIXTURE="$WORK/rebuilt.AppImage"
: >"$REBUILT_FIXTURE"
MODIFIED_PATHS=()
APPIMAGE_RUNTIME_VALIDATOR=runtime_validator_stub \
  validate_rebuilt_appimage "$REBUILT_FIXTURE" \
  || fail "validate_rebuilt_appimage did not forward to its configured command"
[ "$(cat "$VALIDATOR_RECORD")" = "$REBUILT_FIXTURE" ] \
  || fail "validate_rebuilt_appimage forwarded the wrong artifact path"
runtime_validator_failure_stub() { return 37; }
if APPIMAGE_RUNTIME_VALIDATOR=runtime_validator_failure_stub \
  validate_rebuilt_appimage "$REBUILT_FIXTURE"; then
  fail "conditional validate_rebuilt_appimage suppressed validator failure"
fi

CLEANUP_IMAGE="$WORK/cleanup-invalid.AppImage"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  '[ "$1" = "--appimage-extract" ] || exit 9' \
  'mkdir -p squashfs-root/shared/lib' \
  >"$CLEANUP_IMAGE"
chmod +x "$CLEANUP_IMAGE"
CLEANUP_WORKDIR="$WORK/cleanup-invalid-workdir"
CLEANUP_LOG="$WORK/cleanup-invalid.log"
set +e
TARGET="$TARGET" \
CLEANUP_IMAGE="$CLEANUP_IMAGE" \
CLEANUP_WORKDIR="$CLEANUP_WORKDIR" \
  bash -c '
    set -euo pipefail
    # shellcheck source=/dev/null
    source "$TARGET"
    mktemp() {
      if [ "${1:-}" = "-d" ]; then
        mkdir -p "$CLEANUP_WORKDIR"
        printf "%s\n" "$CLEANUP_WORKDIR"
      else
        command mktemp "$@"
      fi
    }
    ensure_sharun_interpreter() { return 1; }
    rewrite_sharun_lib_path() { return 1; }
    patch_apprun_sharun_cwd() { return 1; }
    sanitize_elf_rpaths() { return 1; }
    validate_sharun_lib_path() {
      echo "intentional lib.path validation failure" >&2
      return 47
    }
    validate_appimage_required_libs() { return 0; }
    validate_rebuilt_appimage() { return 0; }
    strip_one_appimage "$CLEANUP_IMAGE"
  ' >"$CLEANUP_LOG" 2>&1
cleanup_status=$?
set -e
[ "$cleanup_status" -ne 0 ] \
  || fail "strip_one_appimage accepted a failing lib.path validation"
grep -F "intentional lib.path validation failure" "$CLEANUP_LOG" >/dev/null \
  || fail "strip_one_appimage hid the original lib.path validation error"
[ ! -e "$CLEANUP_WORKDIR" ] \
  || fail "strip_one_appimage leaked its workdir after lib.path validation failed"

NOOP_IMAGE="$WORK/noop-final.AppImage"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  '[ "$1" = "--appimage-extract" ] || exit 9' \
  'mkdir -p squashfs-root' \
  >"$NOOP_IMAGE"
chmod +x "$NOOP_IMAGE"
NOOP_IMAGE_BEFORE="$(sha256sum "$NOOP_IMAGE" | awk '{print $1}')"
NOOP_VALIDATOR_RECORD="$WORK/noop-validator-record"
noop_runtime_validator_stub() {
  [ -f "$1" ] || return 20
  [ "${#MODIFIED_PATHS[@]}" -eq 0 ] || return 21
  printf '%s\n' "$1" >>"$NOOP_VALIDATOR_RECORD"
}
MODIFIED_PATHS=()
APPIMAGE_RUNTIME_VALIDATOR=noop_runtime_validator_stub \
  strip_one_appimage "$NOOP_IMAGE" \
  || fail "strip_one_appimage rejected a valid no-change final artifact"
[ "$(wc -l <"$NOOP_VALIDATOR_RECORD" | tr -d ' ')" -eq 1 ] \
  || fail "no-change AppImage was not validated exactly once"
[ "$(cat "$NOOP_VALIDATOR_RECORD")" = "$(realpath "$NOOP_IMAGE")" ] \
  || fail "no-change validation did not inspect the original final AppImage"
[ "${#MODIFIED_PATHS[@]}" -eq 0 ] \
  || fail "no-change AppImage was incorrectly registered for re-signing"
[ "$NOOP_IMAGE_BEFORE" = "$(sha256sum "$NOOP_IMAGE" | awk '{print $1}')" ] \
  || fail "no-change AppImage bytes were unexpectedly replaced"

mv_line="$(grep -nF 'mv "$rebuilt" "$original"' "$TARGET" | cut -d: -f1)"
validate_line="$(
  grep -nF 'validate_rebuilt_appimage "$original"' "$TARGET" \
    | tail -n 1 \
    | cut -d: -f1
)"
modified_line="$(grep -nF 'MODIFIED_PATHS+=("$original")' "$TARGET" | cut -d: -f1)"
[ -n "$mv_line" ] && [ -n "$validate_line" ] && [ -n "$modified_line" ] \
  || fail "post-repack validation ordering statements are missing"
[ "$mv_line" -lt "$validate_line" ] && [ "$validate_line" -lt "$modified_line" ] \
  || fail "final artifact validation does not run after mv and before signing registration"
echo "[test-rpaths] ok: final runtime layout and pre-signing handoff are fail-closed"

# --- Case 1: sanitize_elf_rpaths strips a CI build-machine RPATH ------------
APPDIR="$WORK/squashfs-root"
mkdir -p "$APPDIR/usr/lib" "$APPDIR/shared/lib"

# An ELF that mimics libcef.so with an absolute build-runner RUNPATH plus a
# legitimate $ORIGIN entry — only the absolute one should be dropped.
FAKE_CEF="$APPDIR/usr/lib/libcef.so"
cp "$HOST_ELF" "$FAKE_CEF"
patchelf --set-rpath '/home/runner/.cache/tauri-cef/x/shared/lib:$ORIGIN/../shared/lib' "$FAKE_CEF"

# A clean ELF whose RPATH must be left byte-for-byte untouched (idempotency).
CLEAN_SO="$APPDIR/shared/lib/libclean.so"
cp "$HOST_ELF" "$CLEAN_SO"
patchelf --set-rpath '$ORIGIN' "$CLEAN_SO"
clean_before="$(patchelf --print-rpath "$CLEAN_SO")"

if ! sanitize_elf_rpaths "$APPDIR"; then
  fail "sanitize_elf_rpaths reported no change but a CI RPATH was present"
fi

cef_after="$(patchelf --print-rpath "$FAKE_CEF")"
case "$cef_after" in
  *"/home/runner/"*|*"/__w/"*) fail "CI RPATH survived: '$cef_after'" ;;
esac
case "$cef_after" in
  *'$ORIGIN'*) ;;
  *) fail "expected an \$ORIGIN-relative RPATH after sanitize, got: '$cef_after'" ;;
esac
echo "[test-rpaths] ok: libcef.so RPATH rewritten to '$cef_after'"

clean_after="$(patchelf --print-rpath "$CLEAN_SO")"
[ "$clean_before" = "$clean_after" ] || fail "clean ELF RPATH was mutated: '$clean_before' -> '$clean_after'"
echo "[test-rpaths] ok: clean ELF left untouched ('$clean_after')"

# Idempotency: a second pass must find nothing to do.
if sanitize_elf_rpaths "$APPDIR"; then
  fail "sanitize_elf_rpaths was not idempotent — rewrote on a clean second pass"
fi
echo "[test-rpaths] ok: second pass is a no-op (idempotent)"

# --- Case 1b: pure-absolute CI RPATH → depth-aware fallback ------------------
# This is the issue #3224 scenario: libcef.so carries ONLY an absolute
# build-machine RPATH, no surviving $ORIGIN entry. The fallback MUST reach the
# bundle's top-level shared/lib from usr/lib, i.e. `$ORIGIN/../../shared/lib`
# (two hops), NOT the naive `$ORIGIN/../shared/lib` which resolves to
# usr/shared/lib.
ABS_DIR="$WORK/abs"
mkdir -p "$ABS_DIR/usr/lib"
ABS_CEF="$ABS_DIR/usr/lib/libcef.so"
cp "$HOST_ELF" "$ABS_CEF"
patchelf --set-rpath '/home/runner/.cache/tauri-cef/x/shared/lib' "$ABS_CEF"
sanitize_elf_rpaths "$ABS_DIR" || fail "sanitize_elf_rpaths reported no change on a pure-absolute RPATH"
abs_after="$(patchelf --print-rpath "$ABS_CEF")"
[ "$abs_after" = '$ORIGIN:$ORIGIN/../../shared/lib' ] \
  || fail "usr/lib fallback wrong: expected '\$ORIGIN:\$ORIGIN/../../shared/lib', got '$abs_after'"
echo "[test-rpaths] ok: usr/lib pure-absolute RPATH → depth-aware fallback '$abs_after'"

# A lib one level deep (lib/) needs only one hop.
mkdir -p "$ABS_DIR/lib"
ABS_LIB="$ABS_DIR/lib/libfoo.so"
cp "$HOST_ELF" "$ABS_LIB"
patchelf --set-rpath '/__w/openhuman/openhuman/shared/lib' "$ABS_LIB"
sanitize_elf_rpaths "$ABS_DIR" >/dev/null || true
lib_after="$(patchelf --print-rpath "$ABS_LIB")"
[ "$lib_after" = '$ORIGIN:$ORIGIN/../shared/lib' ] \
  || fail "lib/ fallback wrong: expected '\$ORIGIN:\$ORIGIN/../shared/lib', got '$lib_after'"
echo "[test-rpaths] ok: lib/ pure-absolute RPATH → depth-aware fallback '$lib_after'"

# --- Case 2: validate_appimage_required_libs guards runtime libraries --------
# Build a minimal sharun AppDir. uses_sharun_launcher only inspects *ELF* entry
# binaries (is_executable_elf) and greps them for the literal
# "Interpreter not found!" string, mirroring the real sharun launcher binary.
# So the fixture must be an executable ELF with that marker appended (a shell
# AppRun would be skipped as non-ELF and the guard would no-op).
GUARD_DIR="$WORK/guard"
mkdir -p "$GUARD_DIR/shared/lib"
cp "$HOST_ELF" "$GUARD_DIR/sharun"
printf 'Interpreter not found!' >> "$GUARD_DIR/sharun"
chmod +x "$GUARD_DIR/sharun"
# Sanity: the fixture must actually register as a sharun launcher, else the
# guard's early return would make this case vacuously pass.
uses_sharun_launcher "$GUARD_DIR" || fail "fixture did not register as sharun launcher"

# 2a — the sharun preload library absent → must exit non-zero.
if ( validate_appimage_required_libs "$GUARD_DIR" ) 2>/dev/null; then
  fail "validate_appimage_required_libs passed despite missing anylinux.so"
fi
echo "[test-rpaths] ok: guard fails when anylinux.so is absent"

: > "$GUARD_DIR/shared/lib/anylinux.so"

# 2b — libxdo absent → must exit non-zero.
if ( validate_appimage_required_libs "$GUARD_DIR" ) 2>/dev/null; then
  fail "validate_appimage_required_libs passed despite missing libxdo.so.*"
fi
echo "[test-rpaths] ok: guard fails when libxdo.so.* is absent"

# 2c — libxdo present but libcef absent → must still fail.
: > "$GUARD_DIR/shared/lib/libxdo.so.3"
if ( validate_appimage_required_libs "$GUARD_DIR" ) 2>/dev/null; then
  fail "validate_appimage_required_libs passed despite missing libcef.so"
fi
echo "[test-rpaths] ok: guard fails when libcef.so is absent"

# 2d — all required runtime libraries present → must pass.
mkdir -p "$GUARD_DIR/usr/lib"
: > "$GUARD_DIR/shared/lib/libxdo.so.3"
: > "$GUARD_DIR/usr/lib/libcef.so"
if ! ( validate_appimage_required_libs "$GUARD_DIR" ) 2>/dev/null; then
  fail "validate_appimage_required_libs failed despite libxdo.so.3 and libcef.so being present"
fi
echo "[test-rpaths] ok: guard passes when libxdo.so.3 and libcef.so are present"

echo "[test-rpaths] PASS"
