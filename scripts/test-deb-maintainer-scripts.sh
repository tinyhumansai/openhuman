#!/usr/bin/env bash
# scripts/test-deb-maintainer-scripts.sh — regression guard for the Debian
# maintainer scripts that make the OpenHuman binary reachable from any shell
# (openhuman#5497). No CI lane executes shell packaging tests today, so run this
# locally / in review after touching app/src-tauri/{postinst,postrm} or the deb
# bundle config: bash scripts/test-deb-maintainer-scripts.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC_TAURI="$REPO_ROOT/app/src-tauri"
POSTINST="$SRC_TAURI/postinst"
POSTRM="$SRC_TAURI/postrm"
CONF="$SRC_TAURI/tauri.conf.json"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

# --- 1. The scripts exist and are valid POSIX sh -----------------------------
[ -f "$POSTINST" ] || fail "missing $POSTINST"
[ -f "$POSTRM" ] || fail "missing $POSTRM"
sh -n "$POSTINST" || fail "postinst is not valid POSIX sh"
sh -n "$POSTRM" || fail "postrm is not valid POSIX sh"

# --- 2. tauri.conf.json wires both scripts into the deb bundle ---------------
grep -q '"postInstallScript": "postinst"' "$CONF" \
  || fail "tauri.conf.json does not reference postinst via postInstallScript"
grep -q '"postRemoveScript": "postrm"' "$CONF" \
  || fail "tauri.conf.json does not reference postrm via postRemoveScript"
# The lowercase symlink only helps if the binary really installs as OpenHuman.
grep -q '"productName": "OpenHuman"' "$CONF" \
  || fail "productName changed — the postinst symlink target is now stale"

# --- 3. Behavioural test in a throwaway sandbox ------------------------------
# Point the scripts' hardcoded /usr paths at a temp root via the documented
# override env vars, so we exercise the real logic without touching the system.
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
export OPENHUMAN_DEB_BINARY="$SANDBOX/usr/bin/OpenHuman"
export OPENHUMAN_DEB_SYMLINK="$SANDBOX/usr/local/bin/openHuman"

mkdir -p "$(dirname "$OPENHUMAN_DEB_BINARY")"
printf '#!/bin/sh\nexit 0\n' >"$OPENHUMAN_DEB_BINARY"
chmod +x "$OPENHUMAN_DEB_BINARY"

# 3a. Fresh install creates the lowercase symlink pointing at the binary.
sh "$POSTINST" configure
[ -L "$OPENHUMAN_DEB_SYMLINK" ] || fail "postinst did not create the symlink"
[ "$(readlink "$OPENHUMAN_DEB_SYMLINK")" = "$OPENHUMAN_DEB_BINARY" ] \
  || fail "symlink points at the wrong target"

# 3b. Re-running (upgrade reconfigure) is idempotent.
sh "$POSTINST" configure
[ -L "$OPENHUMAN_DEB_SYMLINK" ] || fail "postinst not idempotent"

# 3c. Removal deletes our symlink.
sh "$POSTRM" remove
[ -e "$OPENHUMAN_DEB_SYMLINK" ] && fail "postrm did not remove the symlink"

# 3d. postrm on 'upgrade' must NOT remove the link (postinst recreates it).
sh "$POSTINST" configure
sh "$POSTRM" upgrade
[ -L "$OPENHUMAN_DEB_SYMLINK" ] || fail "postrm removed the symlink on upgrade"

# 3e. A user's own file at the link path is never clobbered or deleted.
rm -f "$OPENHUMAN_DEB_SYMLINK"
printf 'user data\n' >"$OPENHUMAN_DEB_SYMLINK"
sh "$POSTINST" configure
[ -L "$OPENHUMAN_DEB_SYMLINK" ] && fail "postinst clobbered a user's real file"
sh "$POSTRM" purge
[ -f "$OPENHUMAN_DEB_SYMLINK" ] || fail "postrm deleted a user's real file"

# 3f. A pre-existing symlink at the link path that points to a DIRECTORY must be
# replaced in place — never followed. `ln -sf` would create the launcher inside
# that directory (CWE-59); postinst must land the link at $LINK and write
# nothing into the target directory.
rm -f "$OPENHUMAN_DEB_SYMLINK"
DECOY_DIR="$SANDBOX/decoy-dir"
mkdir -p "$DECOY_DIR"
ln -s "$DECOY_DIR" "$OPENHUMAN_DEB_SYMLINK"
sh "$POSTINST" configure
[ -e "$DECOY_DIR/$(basename "$OPENHUMAN_DEB_BINARY")" ] && fail "postinst followed a symlink and wrote inside the target directory (CWE-59)"
[ "$(readlink "$OPENHUMAN_DEB_SYMLINK")" = "$OPENHUMAN_DEB_BINARY" ] || fail "postinst did not replace the directory symlink with the launcher link"

echo "PASS: deb maintainer scripts create/remove the openHuman symlink safely"
