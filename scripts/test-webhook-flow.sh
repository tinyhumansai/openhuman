#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -f "$ROOT_DIR/.env" ]]; then
  # shellcheck disable=SC1091
  eval "$(bash "$ROOT_DIR/scripts/load-dotenv.sh" "$ROOT_DIR/.env")"
fi

CORE_HOST="${OPENHUMAN_CORE_HOST:-127.0.0.1}"
CORE_PORT="${OPENHUMAN_CORE_PORT:-7788}"
CORE_RPC_URL="${CORE_RPC_URL:-http://${CORE_HOST}:${CORE_PORT}/rpc}"

# Resolve the core RPC bearer token.  Resolution order:
#   1. OPENHUMAN_CORE_TOKEN env var (set by caller or via .env / docker / cloud).
#      Always wins — explicit operator configuration.
#   2. Whichever of the two on-disk token files is FRESHEST (newest mtime):
#        a. config-style core.token file in workspace dir (written by
#           standalone `openhuman core run`)
#        b. Debug-only e2e token file written by the Tauri shell at
#           ${TMPDIR:-/tmp}/openhuman-e2e-rpc-token (mode 0600). Only present
#           in debug builds — release builds do not write it. The Tauri shell
#           hands its bearer to the in-process core in-memory; no env-var or
#           ps-readable surface exists.
#
#      Freshest-wins matters because both files commonly coexist on dev
#      machines (a previous standalone run leaves $workspace/core.token
#      behind; the live debug Tauri app writes a new e2e token on each
#      launch). Static "workspace first" precedence would let a stale
#      standalone token shadow the live debug bearer.
_file_mtime() {
  # Print the modification time (epoch seconds) of $1 to stdout.
  # Falls back to 0 if stat is missing or the file does not exist.
  local f="$1"
  if [[ ! -f "$f" ]]; then
    echo 0
    return
  fi
  # macOS (BSD stat) uses -f %m; Linux (GNU stat) uses -c %Y.
  if stat -f %m "$f" >/dev/null 2>&1; then
    stat -f %m "$f"
  elif stat -c %Y "$f" >/dev/null 2>&1; then
    stat -c %Y "$f"
  else
    echo 0
  fi
}

_resolve_rpc_token() {
  if [[ -n "${OPENHUMAN_CORE_TOKEN:-}" ]]; then
    echo "core-token source: OPENHUMAN_CORE_TOKEN env var" >&2
    echo "$OPENHUMAN_CORE_TOKEN"
    return
  fi

  local workspace="${OPENHUMAN_WORKSPACE:-$HOME/.openhuman}"
  local workspace_token_file="$workspace/core.token"
  local e2e_token_file="${TMPDIR:-/tmp}/openhuman-e2e-rpc-token"
  e2e_token_file="${e2e_token_file%/}"

  local workspace_mtime
  local e2e_mtime
  workspace_mtime="$(_file_mtime "$workspace_token_file")"
  e2e_mtime="$(_file_mtime "$e2e_token_file")"

  if [[ "$workspace_mtime" -eq 0 && "$e2e_mtime" -eq 0 ]]; then
    echo "ERROR: core RPC token not found. Options:" >&2
    echo "  1. Set OPENHUMAN_CORE_TOKEN=<token> before running this script" >&2
    echo "  2. Start the core standalone: openhuman core run  (writes $workspace_token_file)" >&2
    echo "  3. Run the OpenHuman app in debug mode (writes $e2e_token_file)" >&2
    exit 1
  fi

  # Pick whichever exists. If both exist, prefer the freshest by mtime —
  # that's the live debug bearer, not the stale standalone leftover.
  if [[ "$e2e_mtime" -gt "$workspace_mtime" ]]; then
    echo "core-token source: debug e2e file ($e2e_token_file, mtime=$e2e_mtime)" >&2
    cat "$e2e_token_file"
  else
    echo "core-token source: workspace file ($workspace_token_file, mtime=$workspace_mtime)" >&2
    cat "$workspace_token_file"
  fi
}
RPC_TOKEN="$(_resolve_rpc_token)"
KEEP_TUNNEL=0
TUNNEL_NAME="echo-debug-$(date +%s)"
HOOK_PATH="/echo-test"
HOOK_METHOD="POST"
PAYLOAD='{"message":"hello from scripts/test-webhook-flow.sh","source":"local-curl"}'

usage() {
  cat <<EOF
Usage: scripts/test-webhook-flow.sh [options]

Creates a backend webhook tunnel, registers the built-in core echo target,
triggers the webhook with curl, prints the captured core log entry, and
deletes the tunnel unless told to keep it.

Options:
  --keep                 Keep the backend tunnel and local echo registration
  --name <name>          Tunnel name override
  --path <path>          Request path suffix to send after /webhooks/ingress/<uuid>
  --method <method>      HTTP method to send (default: POST)
  --payload <json>       Raw JSON payload string to send
  -h, --help             Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep)
      KEEP_TUNNEL=1
      shift
      ;;
    --name)
      TUNNEL_NAME="${2:?missing value for --name}"
      shift 2
      ;;
    --path)
      HOOK_PATH="${2:?missing value for --path}"
      shift 2
      ;;
    --method)
      HOOK_METHOD="${2:?missing value for --method}"
      shift 2
      ;;
    --payload)
      PAYLOAD="${2:?missing value for --payload}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq is required" >&2
  exit 1
fi

rpc_call() {
  local method="$1"
  local params="${2:-{}}"
  curl -fsS "$CORE_RPC_URL" \
    -H 'Content-Type: application/json' \
    -H "Authorization: Bearer $RPC_TOKEN" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"${method}\",\"params\":${params}}"
}

json_string() {
  jq -Rn --arg value "$1" '$value'
}

echo "=== Webhook Flow Test ==="
echo "Core RPC: $CORE_RPC_URL"

curl -fsS "${CORE_RPC_URL%/rpc}/health" >/dev/null

SESSION_TOKEN="$(
  rpc_call "openhuman.auth_get_session_token" \
  | jq -r '.result.result.token // empty'
)"

if [[ -z "$SESSION_TOKEN" ]]; then
  echo "ERROR: no stored session token in the local core. Log into the app first." >&2
  exit 1
fi

BACKEND_URL="$(
  rpc_call "openhuman.config_resolve_api_url" \
  | jq -r '.result.api_url // empty'
)"

if [[ -z "$BACKEND_URL" ]]; then
  echo "ERROR: could not resolve backend API URL from the local core." >&2
  exit 1
fi

echo "Backend: $BACKEND_URL"
echo "Tunnel name: $TUNNEL_NAME"

CREATE_BODY="$(jq -n --arg name "$TUNNEL_NAME" '{name: $name, description: "Live webhook echo flow test"}')"
CREATE_RESP="$(
  curl -fsS "${BACKEND_URL%/}/webhooks/core" \
    -H 'Content-Type: application/json' \
    -H "Authorization: Bearer $SESSION_TOKEN" \
    -d "$CREATE_BODY"
)"

TUNNEL_ID="$(echo "$CREATE_RESP" | jq -r '.data.id // .data._id // empty')"
TUNNEL_UUID="$(echo "$CREATE_RESP" | jq -r '.data.uuid // empty')"
TUNNEL_NAME_ACTUAL="$(echo "$CREATE_RESP" | jq -r '.data.name // empty')"

if [[ -z "$TUNNEL_ID" || -z "$TUNNEL_UUID" ]]; then
  echo "ERROR: failed to create tunnel" >&2
  echo "$CREATE_RESP" | jq .
  exit 1
fi

cleanup() {
  if [[ "$KEEP_TUNNEL" -eq 1 ]]; then
    echo "Keeping tunnel $TUNNEL_UUID"
    return
  fi

  echo "Cleaning up local echo registration..."
  rpc_call "openhuman.webhooks_unregister_echo" \
    "$(jq -n --arg tunnel_uuid "$TUNNEL_UUID" '{tunnel_uuid: $tunnel_uuid}')" >/dev/null || true

  echo "Deleting backend tunnel..."
  curl -fsS -X DELETE "${BACKEND_URL%/}/webhooks/core/${TUNNEL_ID}" \
    -H "Authorization: Bearer $SESSION_TOKEN" >/dev/null || true
}

trap cleanup EXIT

echo "Created tunnel: $TUNNEL_NAME_ACTUAL ($TUNNEL_UUID)"

REGISTER_PARAMS="$(
  jq -n \
    --arg tunnel_uuid "$TUNNEL_UUID" \
    --arg tunnel_name "$TUNNEL_NAME_ACTUAL" \
    --arg backend_tunnel_id "$TUNNEL_ID" \
    '{tunnel_uuid: $tunnel_uuid, tunnel_name: $tunnel_name, backend_tunnel_id: $backend_tunnel_id}'
)"
rpc_call "openhuman.webhooks_register_echo" "$REGISTER_PARAMS" >/dev/null

WEBHOOK_URL="${BACKEND_URL%/}/webhooks/ingress/${TUNNEL_UUID}${HOOK_PATH}"
echo "Triggering: ${HOOK_METHOD} ${WEBHOOK_URL}"

RESPONSE_BODY_FILE="$(mktemp)"
HTTP_STATUS="$(
  curl -sS -o "$RESPONSE_BODY_FILE" -w '%{http_code}' \
    -X "$HOOK_METHOD" \
    "$WEBHOOK_URL?source=local-curl&script=test-webhook-flow" \
    -H 'Content-Type: application/json' \
    -H 'X-OpenHuman-Debug: webhook-flow-script' \
    -d "$PAYLOAD"
)"

echo "Webhook HTTP status: $HTTP_STATUS"
echo "Response body:"
cat "$RESPONSE_BODY_FILE" | jq . || cat "$RESPONSE_BODY_FILE"

if [[ "$HTTP_STATUS" != "200" ]]; then
  if jq -e '.error == "No active client connection for this tunnel"' "$RESPONSE_BODY_FILE" >/dev/null 2>&1; then
    echo "ERROR: backend tunnel exists, but there is no active local relay connection for this tunnel." >&2
    echo "Open the desktop app and make sure the runtime is connected to the backend before running this script." >&2
  else
    echo "ERROR: webhook did not return 200" >&2
  fi
  rm -f "$RESPONSE_BODY_FILE"
  exit 1
fi

rm -f "$RESPONSE_BODY_FILE"

sleep 1

echo "Latest captured log:"
rpc_call "openhuman.webhooks_list_logs" '{"limit":1}' \
  | jq '.result.result.logs[0]'

echo "Latest registrations:"
rpc_call "openhuman.webhooks_list_registrations" \
  | jq '.result.result.registrations'

echo "Done."
