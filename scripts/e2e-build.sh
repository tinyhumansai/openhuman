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

# Clean Rust build cache so frontend assets get re-embedded
cargo clean --manifest-path src-tauri/Cargo.toml

# Load .env for other vars (Telegram API keys, etc.)
source scripts/load-dotenv.sh

# Re-export mock URL in case load-dotenv.sh clobbered it
export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT:-18473}"

# Build .app only (skip DMG to avoid bundle_dmg.sh failures)
tauri build --bundles app --debug

echo "E2E build complete."
