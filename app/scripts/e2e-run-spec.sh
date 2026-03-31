#!/usr/bin/env bash
#
# Run a single WebDriverIO E2E spec.
#
# - macOS: Appium mac2 driver (started locally, port 4723)
# - Linux: tauri-driver (started locally, port 4444)
#
# Usage:
#   ./app/scripts/e2e-run-spec.sh test/e2e/specs/login-flow.spec.ts [log-suffix]
#
set -euo pipefail

SPEC="${1:?spec path required}"
LOG_SUFFIX="${2:-$(basename "$SPEC" .spec.ts)}"

E2E_MOCK_PORT="${E2E_MOCK_PORT:-18473}"
OS="$(uname)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$APP_DIR/.." && pwd)"
cd "$APP_DIR"

CREATED_TEMP_WORKSPACE=""
DRIVER_PID=""

if [ -z "${OPENHUMAN_WORKSPACE:-}" ]; then
  OPENHUMAN_WORKSPACE="$(mktemp -d)"
  CREATED_TEMP_WORKSPACE="$OPENHUMAN_WORKSPACE"
  export OPENHUMAN_WORKSPACE
  echo "Using temporary OPENHUMAN_WORKSPACE: $OPENHUMAN_WORKSPACE"
else
  echo "Using OPENHUMAN_WORKSPACE from environment: $OPENHUMAN_WORKSPACE"
fi

if [ "${OPENHUMAN_SERVICE_MOCK:-0}" = "1" ] && [ -z "${OPENHUMAN_SERVICE_MOCK_STATE_FILE:-}" ]; then
  OPENHUMAN_SERVICE_MOCK_STATE_FILE="$OPENHUMAN_WORKSPACE/service-mock-state.json"
  export OPENHUMAN_SERVICE_MOCK_STATE_FILE
  echo "Using OPENHUMAN_SERVICE_MOCK_STATE_FILE: $OPENHUMAN_SERVICE_MOCK_STATE_FILE"
fi

cleanup() {
  if [ -n "$DRIVER_PID" ]; then
    echo "Stopping driver (pid $DRIVER_PID)..."
    kill "$DRIVER_PID" 2>/dev/null || true
    wait "$DRIVER_PID" 2>/dev/null || true
  fi
  if [ -n "$CREATED_TEMP_WORKSPACE" ]; then
    rm -rf "$CREATED_TEMP_WORKSPACE"
  fi
}
trap cleanup EXIT

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"
export BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"

echo "Cleaning cached app data..."
if [ "$OS" = "Darwin" ]; then
  rm -rf ~/Library/WebKit/com.openhuman.app
  rm -rf ~/Library/Caches/com.openhuman.app
  rm -rf "$HOME/Library/Application Support/com.openhuman.app"
else
  rm -rf "$HOME/.local/share/com.openhuman.app" 2>/dev/null || true
  rm -rf "$HOME/.cache/com.openhuman.app" 2>/dev/null || true
  rm -rf "$HOME/.config/com.openhuman.app" 2>/dev/null || true
fi

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

if [ "$OS" = "Linux" ]; then
  # ---------------------------------------------------------------------------
  # Linux: start tauri-driver
  # ---------------------------------------------------------------------------
  TAURI_DRIVER_PORT="${TAURI_DRIVER_PORT:-4444}"
  DRIVER_LOG="/tmp/tauri-driver-e2e-${LOG_SUFFIX}.log"

  TAURI_DRIVER_BIN="$(command -v tauri-driver 2>/dev/null || true)"
  if [ -z "${TAURI_DRIVER_BIN:-}" ] || [ ! -x "$TAURI_DRIVER_BIN" ]; then
    # Try cargo bin path
    TAURI_DRIVER_BIN="$HOME/.cargo/bin/tauri-driver"
  fi
  if [ ! -x "$TAURI_DRIVER_BIN" ]; then
    echo "ERROR: tauri-driver not found. Install with: cargo install tauri-driver" >&2
    exit 1
  fi

  echo "Starting tauri-driver on port $TAURI_DRIVER_PORT..."
  echo "  Driver logs: $DRIVER_LOG"
  "$TAURI_DRIVER_BIN" --port "$TAURI_DRIVER_PORT" > "$DRIVER_LOG" 2>&1 &
  DRIVER_PID=$!

  for i in $(seq 1 15); do
    if curl -sf "http://127.0.0.1:$TAURI_DRIVER_PORT/status" >/dev/null 2>&1; then
      echo "tauri-driver is ready."
      break
    fi
    if [ "$i" -eq 15 ]; then
      echo "ERROR: tauri-driver did not start within 15 seconds." >&2
      cat "$DRIVER_LOG" >&2
      exit 1
    fi
    sleep 1
  done
else
  # ---------------------------------------------------------------------------
  # macOS: start Appium
  # ---------------------------------------------------------------------------
  APPIUM_PORT="${APPIUM_PORT:-4723}"
  # shellcheck source=/dev/null
  source "$SCRIPT_DIR/e2e-resolve-node-appium.sh"

  APPIUM_LOG="/tmp/appium-e2e-${LOG_SUFFIX}.log"
  NODE_VER=$("$NODE24" --version)
  echo "Starting Appium on port $APPIUM_PORT (Node $NODE_VER)..."
  echo "  Appium logs: $APPIUM_LOG"
  "$APPIUM_BIN" --port "$APPIUM_PORT" --relaxed-security > "$APPIUM_LOG" 2>&1 &
  DRIVER_PID=$!

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
fi

echo "Running E2E spec ($SPEC)..."
npx wdio run test/wdio.conf.ts --spec "$SPEC"
