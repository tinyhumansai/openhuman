#!/usr/bin/env bash
# One-time setup: create a stable local code-signing certificate for the
# openhuman-core sidecar. Run this once per development machine.
#
# Why: macOS TCC identifies unsigned binaries by content hash (Mach-O UUID).
# Every `yarn core:stage` recompiles the sidecar, changing its hash, so TCC
# no longer matches the old grant. Signing with a stable certificate causes
# TCC to use the certificate identity instead — grants persist across rebuilds.
#
# After running this script:
#   1. yarn core:stage        (signs the sidecar with the new cert)
#   2. In OpenHuman → Request Permissions (removes old stale TCC entry,
#      registers current binary)
#   3. Grant in System Settings → Refresh Status
#   From this point the grant survives future `yarn core:stage` runs.

set -euo pipefail

IDENTITY="OpenHuman Dev Signer"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
TMPDIR_CERT=$(mktemp -d)
KEY="$TMPDIR_CERT/openhuman-dev.key"
CERT="$TMPDIR_CERT/openhuman-dev.crt"
KEY_DER="$TMPDIR_CERT/openhuman-dev-key.der"

cleanup() {
  rm -rf "$TMPDIR_CERT"
}
trap cleanup EXIT

# ── Check if already set up ──────────────────────────────────────────────────
if security find-identity -v -p codesigning 2>/dev/null | grep -q "$IDENTITY"; then
  echo "[setup-dev-codesign] Certificate \"$IDENTITY\" already exists — nothing to do."
  echo "[setup-dev-codesign] Run 'yarn core:stage' to sign the sidecar."
  exit 0
fi

echo "[setup-dev-codesign] Creating self-signed code-signing certificate: \"$IDENTITY\""

# ── Generate key + self-signed certificate ───────────────────────────────────
openssl req \
  -newkey rsa:2048 \
  -nodes \
  -keyout "$KEY" \
  -x509 \
  -days 3650 \
  -out "$CERT" \
  -subj "/CN=$IDENTITY" \
  2>/dev/null

# ── Bundle to PKCS12 ─────────────────────────────────────────────────────────
openssl pkcs12 \
  -export \
  -out "$P12" \
  -inkey "$KEY" \
  -in "$CERT" \
  -passout "pass:$P12_PASS" \
  2>/dev/null

# ── Import into login Keychain ───────────────────────────────────────────────
security import "$P12" \
  -k "$KEYCHAIN" \
  -P "$P12_PASS" \
  -T /usr/bin/codesign \
  -T /usr/bin/security

# ── Trust for code signing ───────────────────────────────────────────────────
security add-trusted-cert \
  -d \
  -r trustRoot \
  -k "$KEYCHAIN" \
  "$CERT"

echo ""
echo "[setup-dev-codesign] Done. Certificate \"$IDENTITY\" added to login Keychain."
echo ""
echo "Next steps:"
echo "  1. yarn core:stage          — rebuilds and signs the sidecar"
echo "  2. In OpenHuman click 'Request Permissions' to register the signed binary"
echo "  3. Grant in System Settings → Privacy & Security → Accessibility"
echo "  4. Click 'Refresh Status'"
echo ""
echo "After this, accessibility grants will survive future 'yarn core:stage' runs."
