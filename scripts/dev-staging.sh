#!/usr/bin/env bash
# Launch a local staging build of the Tauri app.
#
# Loads signing credentials from scripts/ci-secrets.json, imports the
# Developer ID certificate into a temporary keychain (like CI), and sets
# OPENHUMAN_APP_ENV=staging so the encrypted-file keyring backend is used.
#
# Usage:
#   bash scripts/dev-staging.sh
#   pnpm dev:staging

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SECRETS_FILE="$ROOT_DIR/scripts/ci-secrets.json"

if [[ ! -f "$SECRETS_FILE" ]]; then
  echo "[dev-staging] $SECRETS_FILE not found — cannot load signing credentials" >&2
  exit 1
fi

# Load secrets + vars from the CI secrets file
source "$SCRIPT_DIR/load-env-json.sh" "$SECRETS_FILE" '.secrets + .vars'

# Ensure staging env
export OPENHUMAN_APP_ENV=staging
export VITE_OPENHUMAN_APP_ENV=staging

# Point to staging API (ci-secrets.json has localhost for CI)
export BACKEND_URL=https://staging-api.tinyhumans.ai
export VITE_BACKEND_URL=https://staging-api.tinyhumans.ai

# Load the regular .env (secrets take precedence since they're already set)
source "$SCRIPT_DIR/load-dotenv.sh"

export CEF_PATH="$HOME/Library/Caches/tauri-cef"

# ── Temporary keychain for codesign (CI-style) ────────────────────────────────
KEYCHAIN_NAME="dev-staging.keychain-db"
KEYCHAIN_PATH="$HOME/Library/Keychains/$KEYCHAIN_NAME"
KEYCHAIN_PASSWORD="dev-staging-$(date +%s)"

cleanup_keychain() {
  security delete-keychain "$KEYCHAIN_PATH" 2>/dev/null || true
}
trap cleanup_keychain EXIT

# Remove stale keychain from a previous run
cleanup_keychain

echo "[dev-staging] creating temporary keychain for codesign..."
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"

# Import the Developer ID certificate
CERT_TMP=$(mktemp /tmp/dev-staging-cert.XXXXXX.p12)
echo "$APPLE_CERTIFICATE_BASE64" | base64 --decode > "$CERT_TMP"
security import "$CERT_TMP" \
  -k "$KEYCHAIN_PATH" \
  -P "$APPLE_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign \
  -T /usr/bin/security
rm -f "$CERT_TMP"

# Allow codesign to access the key without prompts
security set-key-partition-list -S "apple-tool:,apple:,codesign:" \
  -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH" >/dev/null 2>&1

# Prepend temporary keychain to the search list so codesign finds it
EXISTING_KEYCHAINS=$(security list-keychains -d user | tr -d '"' | tr '\n' ' ')
security list-keychains -d user -s "$KEYCHAIN_PATH" $EXISTING_KEYCHAINS

echo "[dev-staging] certificate imported into temporary keychain"

# ── Chromium safe storage & tauri-cli ─────────────────────────────────────────
bash "$SCRIPT_DIR/setup-chromium-safe-storage.sh"

cd "$ROOT_DIR/app"
pnpm tauri:ensure

echo "[dev-staging] APPLE_SIGNING_IDENTITY=$APPLE_SIGNING_IDENTITY"
echo "[dev-staging] OPENHUMAN_APP_ENV=$OPENHUMAN_APP_ENV"
echo "[dev-staging] building and running (no file watcher)..."

# Build the frontend first
pnpm run dev &
VITE_PID=$!

# Wait for vite to be ready
until curl -s http://localhost:1420 >/dev/null 2>&1; do sleep 0.5; done

# Build the .app bundle only (skip DMG) and run directly
cargo tauri build --debug --bundles app

# Kill vite
kill $VITE_PID 2>/dev/null

echo "[dev-staging] launching app bundle..."
exec "$ROOT_DIR/app/src-tauri/target/debug/bundle/macos/OpenHuman.app/Contents/MacOS/OpenHuman"
