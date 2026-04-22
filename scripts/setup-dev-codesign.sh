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
P12="$TMPDIR_CERT/openhuman-dev.p12"
P12_PASS="${OPENHUMAN_P12_PASS:-openhuman-dev}"

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
cat > "$TMPDIR_CERT/openssl.conf" <<EOF
[ req ]
distinguished_name = req_distinguished_name
prompt = no
x509_extensions = v3_ca

[ req_distinguished_name ]
CN = $IDENTITY

[ v3_ca ]
basicConstraints = CA:FALSE
keyUsage = digitalSignature,nonRepudiation,keyEncipherment,dataEncipherment
extendedKeyUsage = codeSigning
EOF

openssl req \
  -newkey rsa:2048 \
  -nodes \
  -keyout "$KEY" \
  -x509 \
  -days 3650 \
  -out "$CERT" \
  -config "$TMPDIR_CERT/openssl.conf" \
  2>/dev/null

# ── Bundle to PKCS12 ─────────────────────────────────────────────────────────
# `-legacy` keeps PKCS12 MAC/encryption compatible with macOS `security` tool
# which does not yet support OpenSSL 3.x defaults (SHA256 MAC / AES-256-CBC).
# Older OpenSSL/LibreSSL (including the macOS-bundled LibreSSL) do not know
# about `-legacy`, so probe for support before adding it.
PKCS12_LEGACY_ARGS=()
if openssl pkcs12 -help 2>&1 | grep -q -- '-legacy'; then
  PKCS12_LEGACY_ARGS=(-legacy)
fi

openssl pkcs12 \
  -legacy \
  -export \
  "${PKCS12_LEGACY_ARGS[@]}" \
  -out "$P12" \
  -inkey "$KEY" \
  -in "$CERT" \
  -passout "pass:$P12_PASS"

# ── Import into login Keychain ───────────────────────────────────────────────
security import "$P12" \
  -k "$KEYCHAIN" \
  -P "$P12_PASS" \
  -T /usr/bin/codesign \
  -T /usr/bin/security

# ── Trust for code signing ───────────────────────────────────────────────────
# Note: we add both basic and codeSign trust.
security add-trusted-cert \
  -r trustRoot \
  -p basic \
  -p codeSign \
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
