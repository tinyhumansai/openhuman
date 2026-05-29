#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$APP_DIR/.." && pwd)"
cd "$APP_DIR"

RUST_HOST_TRIPLE="${RUST_HOST_TRIPLE:-$(rustc -vV | awk '/^host: / { print $2 }')}"
E2E_WEB_CORE_TARGET_DIR="${E2E_WEB_CORE_TARGET_DIR:-$REPO_ROOT/target/e2e-web-${RUST_HOST_TRIPLE}}"
E2E_MOCK_PORT="${E2E_MOCK_PORT:-18473}"
OPENHUMAN_CORE_PORT="${OPENHUMAN_CORE_PORT:-17788}"
E2E_WEB_PORT="${E2E_WEB_PORT:-4173}"
PW_CORE_RPC_TOKEN="${PW_CORE_RPC_TOKEN:-openhuman-playwright-token}"
PW_CORE_RPC_URL="http://127.0.0.1:${OPENHUMAN_CORE_PORT}/rpc"
PW_BASE_URL="http://127.0.0.1:${E2E_WEB_PORT}"

OPENHUMAN_WORKSPACE="${OPENHUMAN_WORKSPACE:-$(mktemp -d)}"
CREATED_TEMP_WORKSPACE=""
if [ ! -d "${OPENHUMAN_WORKSPACE}" ] || [[ "${OPENHUMAN_WORKSPACE}" == /tmp/* ]]; then
  CREATED_TEMP_WORKSPACE="$OPENHUMAN_WORKSPACE"
fi
export OPENHUMAN_WORKSPACE
export OPENHUMAN_KEYRING_BACKEND="${OPENHUMAN_KEYRING_BACKEND:-file}"

MOCK_PID=""
CORE_PID=""
WEB_PID=""

cleanup() {
  local status=$?
  set +e
  if [ -n "$WEB_PID" ]; then
    kill "$WEB_PID" 2>/dev/null || true
    wait "$WEB_PID" 2>/dev/null || true
  fi
  if [ -n "$CORE_PID" ]; then
    kill "$CORE_PID" 2>/dev/null || true
    wait "$CORE_PID" 2>/dev/null || true
  fi
  if [ -n "$MOCK_PID" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
  if [ -n "$CREATED_TEMP_WORKSPACE" ]; then
    rm -rf "$CREATED_TEMP_WORKSPACE"
  fi
  return "$status"
}
trap cleanup EXIT

wait_for_http() {
  local url="$1"
  local name="$2"
  for _ in $(seq 1 90); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: ${name} did not become ready at ${url}" >&2
  return 1
}

wait_for_rpc_auth() {
  local rpc_url="$1"
  local token="$2"
  for _ in $(seq 1 30); do
    if curl -fsS "$rpc_url" \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer $token" \
      -d '{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}' >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: authenticated RPC probe failed for ${rpc_url}" >&2
  return 1
}

mkdir -p "$OPENHUMAN_WORKSPACE"
cat > "$OPENHUMAN_WORKSPACE/config.toml" <<EOF
api_url = "http://127.0.0.1:${E2E_MOCK_PORT}"
primary_cloud = "p_e2e_mock"
default_model = "e2e-mock-model"
chat_provider = "e2e:e2e-mock-model"
reasoning_provider = "e2e:e2e-mock-model"
agentic_provider = "e2e:e2e-mock-model"
coding_provider = "e2e:e2e-mock-model"

[[cloud_providers]]
id = "p_e2e_mock"
slug = "e2e"
label = "E2E Mock"
endpoint = "http://127.0.0.1:${E2E_MOCK_PORT}/openai/v1"
auth_style = "none"
default_model = "e2e-mock-model"
EOF

node "$REPO_ROOT/scripts/mock-api-server.mjs" --port "$E2E_MOCK_PORT" >"$OPENHUMAN_WORKSPACE/mock.log" 2>&1 &
MOCK_PID=$!
wait_for_http "http://127.0.0.1:${E2E_MOCK_PORT}/__admin/health" "mock backend"

OPENHUMAN_CORE_BIN="$E2E_WEB_CORE_TARGET_DIR/debug/openhuman-core"
if [ ! -x "$OPENHUMAN_CORE_BIN" ]; then
  echo "ERROR: standalone core binary is missing at $OPENHUMAN_CORE_BIN. Run pnpm test:e2e:web:build first." >&2
  exit 1
fi

export OPENHUMAN_CORE_TOKEN="$PW_CORE_RPC_TOKEN"
export OPENHUMAN_TELEGRAM_BOT_API_BASE="http://127.0.0.1:${E2E_MOCK_PORT}"

"$OPENHUMAN_CORE_BIN" run --host 127.0.0.1 --port "$OPENHUMAN_CORE_PORT" \
  >"$OPENHUMAN_WORKSPACE/core.log" 2>&1 &
CORE_PID=$!
wait_for_http "http://127.0.0.1:${OPENHUMAN_CORE_PORT}/health" "standalone core"
wait_for_rpc_auth "$PW_CORE_RPC_URL" "$PW_CORE_RPC_TOKEN"

python3 -m http.server "$E2E_WEB_PORT" --bind 127.0.0.1 --directory "$APP_DIR/dist-web" \
  >"$OPENHUMAN_WORKSPACE/web.log" 2>&1 &
WEB_PID=$!
wait_for_http "$PW_BASE_URL" "web host"

export PW_BASE_URL
export PW_CORE_RPC_URL
export PW_CORE_RPC_TOKEN

pnpm exec playwright test "$@"
