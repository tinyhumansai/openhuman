#!/usr/bin/env bash
# Pre-seeds the "Chromium Safe Storage" keychain entry with a permissive
# ACL so CEF/Chromium reads it without prompting.
#
# Idempotent: if the entry already exists, leaves the encryption key alone
# (so existing cookies/IndexedDB stay decryptable) and only re-applies the
# permissive ACL via partition-list.
set -euo pipefail

SVC="Chromium Safe Storage"
ACCT="Chromium"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-generic-password -s "$SVC" -a "$ACCT" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "[chromium-safe-storage] entry exists — leaving key intact, refreshing ACL"
  # Permissive partition list: any binary can read.
  security set-generic-password-partition-list \
    -S "apple-tool:,apple:,unsigned:" \
    -s "$SVC" \
    -a "$ACCT" \
    -k "" \
    "$KEYCHAIN" >/dev/null 2>&1 || true
else
  echo "[chromium-safe-storage] entry missing — seeding with random key + permissive ACL"
  KEY=$(openssl rand -base64 16)
  security add-generic-password \
    -s "$SVC" \
    -a "$ACCT" \
    -w "$KEY" \
    -A \
    "$KEYCHAIN"
fi

echo "[chromium-safe-storage] done"
