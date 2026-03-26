#!/usr/bin/env bash
#
# Run E2E Telegram integration flow tests only.
#
# Starts Appium, cleans app caches, runs the telegram-flow spec,
# then tears everything down. Each flow script is self-contained so
# specs don't pollute each other's Redux Persist state.
#
# Usage:
#   ./scripts/e2e-telegram.sh
#   APPIUM_PORT=4723 ./scripts/e2e-telegram.sh
#
set -euo pipefail

APPIUM_PORT="${APPIUM_PORT:-4723}"
E2E_MOCK_PORT="${E2E_MOCK_PORT:-18473}"
SPEC="test/e2e/specs/telegram-flow.spec.ts"

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"
export BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"

# Clean cached app data for a fresh state — Redux Persist would otherwise
# remember the JWT from a previous run and skip the login flow.
echo "Cleaning cached app data..."
rm -rf ~/Library/WebKit/com.openhuman.app
rm -rf ~/Library/Caches/com.openhuman.app
rm -rf "$HOME/Library/Application Support/com.openhuman.app"

# Verify the frontend dist has the mock server URL baked in.
DIST_JS="$(ls dist/assets/index-*.js 2>/dev/null | head -1)"
if [ -z "$DIST_JS" ]; then
  echo "ERROR: No frontend bundle found at dist/assets/index-*.js." >&2
  echo "       Run 'yarn test:e2e:build' to build the app before running E2E tests." >&2
  exit 1
fi
if ! grep -q "127.0.0.1:${E2E_MOCK_PORT}" "$DIST_JS"; then
  echo "ERROR: frontend bundle does NOT contain mock server URL (127.0.0.1:${E2E_MOCK_PORT})." >&2
  echo "       Run 'yarn test:e2e:build' to rebuild with the mock URL." >&2
  exit 1
fi
echo "Verified: frontend bundle contains mock server URL."

# --- Resolve Node 24 via nvm ---------------------------------------------------
export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
# shellcheck source=/dev/null
[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"

NODE24="$(nvm which 24 2>/dev/null || true)"
if [ -z "$NODE24" ] || [ ! -x "$NODE24" ]; then
  echo "ERROR: Node 24 is required for appium v3. Install it with: nvm install 24" >&2
  exit 1
fi

APPIUM_BIN="$(dirname "$NODE24")/appium"
if [ ! -x "$APPIUM_BIN" ]; then
  echo "ERROR: appium not found at $APPIUM_BIN. Install it with: nvm use 24 && npm i -g appium" >&2
  exit 1
fi

# --- Start Appium in the background -------------------------------------------
APPIUM_LOG="/tmp/appium-e2e-telegram.log"
NODE_VER=$("$NODE24" --version)
echo "Starting Appium on port $APPIUM_PORT (Node $NODE_VER)..."
echo "  Appium logs: $APPIUM_LOG"
"$NODE24" "$APPIUM_BIN" --port "$APPIUM_PORT" --relaxed-security > "$APPIUM_LOG" 2>&1 &
APPIUM_PID=$!

cleanup() {
  echo "Stopping Appium (pid $APPIUM_PID)..."
  kill "$APPIUM_PID" 2>/dev/null || true
  wait "$APPIUM_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Wait for Appium to be ready
for i in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:$APPIUM_PORT/status" >/dev/null 2>&1; then
    echo "Appium is ready."
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo "ERROR: Appium did not start within 30 seconds." >&2
    exit 1
  fi
  sleep 1
done

# --- Run WebDriverIO ----------------------------------------------------------
echo "Running E2E Telegram integration flow tests ($SPEC)..."
npx wdio run test/wdio.conf.ts --spec "$SPEC"
