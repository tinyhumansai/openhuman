#!/usr/bin/env bash
# Regression test for scripts/release/tauri-signer.sh (#5658).
#
# The property under test is not "signing works" — it is that signing can never
# silently no-op. The macOS leg used to die on `cargo tauri`, and the Linux leg
# used to warn-and-return, leaving a .sig that covered pre-strip bytes. Both
# shapes must now fail.
#
# Deliberately dependency-free: a stub stands in for the Tauri CLI, so this runs
# on any runner without pnpm, node or a signing key.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PASS=0
FAIL=0

check() {
  if [ "$1" = "$2" ]; then
    echo "  ok   — $3"
    PASS=$((PASS + 1))
  else
    echo "  FAIL — $3 (got '$1', want '$2')" >&2
    FAIL=$((FAIL + 1))
  fi
}

# A throwaway repo tree so the helper's repo-root resolution has something to
# find, and so a stub CLI can sit where `pnpm install` would have put the real one.
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/repo/scripts/release" "$TMP/repo/app/node_modules/.bin"
cp "$SCRIPT_DIR/tauri-signer.sh" "$TMP/repo/scripts/release/tauri-signer.sh"

# shellcheck source=scripts/release/tauri-signer.sh
. "$TMP/repo/scripts/release/tauri-signer.sh"

STUB="$TMP/repo/app/node_modules/.bin/tauri"
BIN_DIR="$TMP/repo/app/node_modules/.bin"

# Records its argv, then behaves as STUB_MODE dictates.
write_stub() {
  cat > "$STUB" <<STUB_EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" > "$TMP/argv"
case "\${STUB_MODE:-ok}" in
  ok)      printf 'signature\n' > "\${!#}.sig" ;;
  nosig)   : ;;
  boom)    exit 3 ;;
esac
STUB_EOF
  chmod +x "$STUB"
}
write_stub

export TAURI_SIGNING_PRIVATE_KEY="dummy-private-key"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="dummy-password"

run() { # run <expected_rc> <label> [env assignments...]
  local want="$1" label="$2"; shift 2
  local rc=0
  set +e
  ( "$@" ) >/dev/null 2>&1
  rc=$?
  set -e
  check "$rc" "$want" "$label"
}

echo "[test-tauri-signer] happy path"
printf 'payload\n' > "$TMP/ok.tar.gz"
run 0 "signs when the npm CLI is present" tauri_signer_sign "$TMP/ok.tar.gz"
check "$([ -s "$TMP/ok.tar.gz.sig" ] && echo yes || echo no)" "yes" "writes a non-empty .sig"
# Pin the invocation contract: subcommand and both flags, in the form the npm
# CLI accepts. `cargo tauri ...` regressing back in would not match this.
check "$(cat "$TMP/argv")" \
  "signer sign --private-key dummy-private-key --password dummy-password $TMP/ok.tar.gz" \
  "invokes 'signer sign' with --private-key and --password"

echo "[test-tauri-signer] failure paths"
printf 'payload\n' > "$TMP/nokey.tar.gz"
run 1 "fails when TAURI_SIGNING_PRIVATE_KEY is unset" \
  env -u TAURI_SIGNING_PRIVATE_KEY bash -c \
  ". '$TMP/repo/scripts/release/tauri-signer.sh'; tauri_signer_sign '$TMP/nokey.tar.gz'"

run 1 "fails when the file to sign does not exist" tauri_signer_sign "$TMP/absent.tar.gz"

printf 'payload\n' > "$TMP/boom.tar.gz"
STUB_MODE=boom run 1 "fails when the signer exits non-zero" tauri_signer_sign "$TMP/boom.tar.gz"

printf 'payload\n' > "$TMP/nosig.tar.gz"
STUB_MODE=nosig run 1 "fails when the signer produces no .sig" tauri_signer_sign "$TMP/nosig.tar.gz"

echo "[test-tauri-signer] a stale signature is never left behind"
# The Linux regression exactly: bytes were rewritten, signing did not happen,
# and the previous .sig stayed on disk to be published.
printf 'payload\n' > "$TMP/stale.tar.gz"
printf 'SIGNATURE-OVER-OLD-BYTES\n' > "$TMP/stale.tar.gz.sig"
STUB_MODE=boom run 1 "fails when re-signing rewritten bytes fails" tauri_signer_sign "$TMP/stale.tar.gz"
check "$([ -e "$TMP/stale.tar.gz.sig" ] && echo present || echo removed)" "removed" \
  "removes the stale .sig instead of shipping it"

echo "[test-tauri-signer] no Tauri CLI available at all"
rm -f "$STUB"
if PATH=/usr/bin:/bin command -v pnpm >/dev/null 2>&1 || PATH=/usr/bin:/bin command -v cargo-tauri >/dev/null 2>&1; then
  echo "  skip — pnpm or cargo-tauri lives in /usr/bin:/bin here, cannot isolate"
else
  printf 'payload\n' > "$TMP/nocli.tar.gz"
  set +e
  ( PATH=/usr/bin:/bin; tauri_signer_sign "$TMP/nocli.tar.gz" ) >/dev/null 2>&1
  rc=$?
  set -e
  check "$rc" "1" "fails loudly rather than skipping when no CLI is installed"
fi
mkdir -p "$BIN_DIR"

echo
if [ "$FAIL" -ne 0 ]; then
  echo "[test-tauri-signer] $PASS passed, $FAIL FAILED" >&2
  exit 1
fi
echo "[test-tauri-signer] $PASS passed, 0 failed"
