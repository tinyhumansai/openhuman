#!/usr/bin/env bash
#
# Build the .app bundle for E2E tests with the mock server URL baked in.
#
# This does a cargo clean first to ensure the frontend assets are re-embedded
# (Cargo's incremental build won't detect changes to dist/).
#
set -euo pipefail

# Source Cargo environment
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT:-18473}"

echo "Building E2E app bundle with VITE_BACKEND_URL=$VITE_BACKEND_URL"

if [ -z "${E2E_SKIP_CARGO_CLEAN:-}" ]; then
  cargo clean --manifest-path src-tauri/Cargo.toml
else
  echo "Skipping cargo clean (E2E_SKIP_CARGO_CLEAN is set)."
fi

if [ -f .env ]; then
  # shellcheck source=/dev/null
  source scripts/load-dotenv.sh
else
  echo "No .env file — skipping load-dotenv (optional for CI)."
fi

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT:-18473}"

# Stage rust-core sidecar for bundle.externalBin (see src-tauri/tauri.conf.json).
node scripts/stage-core-sidecar.mjs

# Use npx so CI does not require a global Tauri CLI
npx tauri build --bundles app --debug

echo "E2E build complete."
