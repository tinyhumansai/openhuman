#!/usr/bin/env bash
# Regression test for the issue #3224 hardening in
# strip-appimage-graphics-libs.sh:
#   1. sanitize_elf_rpaths        — strips /home/runner|/__w build-machine
#                                    RPATHs from bundled ELFs, rewriting to
#                                    $ORIGIN-relative.
#   2. validate_appimage_required_libs — hard-fails when libxdo.so.* is missing
#                                    from a sharun AppDir.
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

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail() { echo "[test-rpaths] FAIL: $1" >&2; exit 1; }

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

# --- Case 2: validate_appimage_required_libs guards libxdo -------------------
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

# 2a — libxdo absent → must exit non-zero.
if ( validate_appimage_required_libs "$GUARD_DIR" ) 2>/dev/null; then
  fail "validate_appimage_required_libs passed despite missing libxdo.so.*"
fi
echo "[test-rpaths] ok: guard fails when libxdo.so.* is absent"

# 2b — libxdo present → must pass.
: > "$GUARD_DIR/shared/lib/libxdo.so.3"
if ! ( validate_appimage_required_libs "$GUARD_DIR" ) 2>/dev/null; then
  fail "validate_appimage_required_libs failed despite libxdo.so.3 present"
fi
echo "[test-rpaths] ok: guard passes when libxdo.so.3 is present"

echo "[test-rpaths] PASS"
