#!/usr/bin/env bash

set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "$APP_DIR/.." && pwd)"
cd "$APP_DIR"

RUSTC_BIN="$(command -v rustc)"
CARGO_BIN="${CARGO_BIN:-$(dirname "$RUSTC_BIN")/cargo}"
if [ ! -x "$CARGO_BIN" ]; then
  CARGO_BIN="$(command -v cargo)"
fi

RUST_HOST_TRIPLE="${RUST_HOST_TRIPLE:-$("$RUSTC_BIN" -vV | awk '/^host: / { print $2 }')}"
E2E_WEB_CORE_TARGET_DIR="${E2E_WEB_CORE_TARGET_DIR:-$REPO_ROOT/target/e2e-web-${RUST_HOST_TRIPLE}}"

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT:-18473}"
export VITE_OPENHUMAN_TARGET="web"
export VITE_OPENHUMAN_E2E_DEFAULT_CORE_MODE="cloud"
export VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD="true"
export VITE_OPENHUMAN_CORE_RPC_URL="http://127.0.0.1:${OPENHUMAN_CORE_PORT:-17788}/rpc"
export VITE_CHAT_ATTACHMENTS="true"

if [ -f "$REPO_ROOT/.env" ]; then
  # shellcheck source=/dev/null
  source "$REPO_ROOT/scripts/load-dotenv.sh"
fi

echo "Building web E2E bundle with backend ${VITE_BACKEND_URL}"
pnpm run build:web
echo "Building standalone openhuman-core for web E2E into ${E2E_WEB_CORE_TARGET_DIR}..."
CARGO_TARGET_DIR="$E2E_WEB_CORE_TARGET_DIR" bash "$REPO_ROOT/scripts/ci-cancel-aware.sh" "$CARGO_BIN" build --manifest-path "$REPO_ROOT/Cargo.toml" --bin openhuman-core
