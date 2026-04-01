#!/usr/bin/env bash
#
# debug-notion-live.sh — Debug Notion skill with a live backend + JWT.
#
# Loads environment from .env (BACKEND_URL, JWT_TOKEN, etc.)
#
# Tests the full OAuth proxy chain that the Notion skill uses:
#   1. Raw HTTP call to backend proxy endpoint
#   2. Skill startup with BACKEND_URL + session token
#   3. Tool call that uses oauth.fetch (proxied through backend)
#
# Usage:
#   bash scripts/debug-notion-live.sh
#
# Environment variables (set in .env or override via export):
#   BACKEND_URL   — staging or prod backend
#   JWT_TOKEN     — session JWT
#   CREDENTIAL_ID — Notion OAuth credential ID
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load .env
if [ -f "$REPO_ROOT/.env" ]; then
    source "$SCRIPT_DIR/load-dotenv.sh" "$REPO_ROOT/.env"
fi

BACKEND_URL="${BACKEND_URL:-}"
JWT_TOKEN="${JWT_TOKEN:-}"
CREDENTIAL_ID="${CREDENTIAL_ID:-}"

# Read credential ID from oauth_credential.json if not set
if [ -z "$CREDENTIAL_ID" ]; then
    CRED_FILE="$HOME/.openhuman/skills_data/notion/oauth_credential.json"
    if [ -f "$CRED_FILE" ]; then
        CREDENTIAL_ID=$(python3 -c "import json; print(json.load(open('$CRED_FILE')).get('credentialId',''))" 2>/dev/null || echo "")
    fi
fi

if [ -z "$BACKEND_URL" ]; then
    echo "ERROR: BACKEND_URL not set. Add it to .env or export it."
    exit 1
fi

if [ -z "$JWT_TOKEN" ]; then
    echo "ERROR: JWT_TOKEN not set. Add it to .env or export it."
    exit 1
fi

echo "╔════════════════════════════════════════════════════════╗"
echo "║  Notion Skill Live Debug                               ║"
echo "╠════════════════════════════════════════════════════════╣"
echo "║  Backend:       $BACKEND_URL"
echo "║  Credential ID: ${CREDENTIAL_ID:-<not found>}"
echo "║  JWT:           ${JWT_TOKEN:0:20}..."
echo "╚════════════════════════════════════════════════════════╝"
echo ""

# ── Step 1: Check backend health ──
echo "--- Step 1: Backend Health Check ---"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BACKEND_URL/settings" -H "Authorization: Bearer $JWT_TOKEN" 2>/dev/null || echo "000")
echo "  GET /settings → HTTP $HTTP_CODE"

if [ "$HTTP_CODE" = "000" ] || [ "$HTTP_CODE" = "502" ] || [ "$HTTP_CODE" = "503" ]; then
    echo "  ✗ Backend is DOWN (HTTP $HTTP_CODE)"
    echo ""
    echo "  The backend at $BACKEND_URL is unreachable."
    echo "  The Notion skill uses oauth.fetch() which proxies through:"
    echo "    $BACKEND_URL/proxy/by-id/$CREDENTIAL_ID/{path}"
    echo ""
    echo "  Fix: Bring the backend online, then re-run this script."
    exit 1
fi

if [ "$HTTP_CODE" = "401" ]; then
    echo "  ✗ JWT is invalid or expired (HTTP 401)"
    echo "  Get a fresh JWT and set JWT_TOKEN in .env"
    exit 1
fi

echo "  ✓ Backend reachable (HTTP $HTTP_CODE)"

# ── Step 2: Raw proxy call ──
if [ -n "$CREDENTIAL_ID" ]; then
    echo ""
    echo "--- Step 2: Raw OAuth Proxy Call ---"
    echo "  Testing: GET $BACKEND_URL/proxy/by-id/$CREDENTIAL_ID/v1/users?page_size=1"
    PROXY_RESP=$(curl -s -w "\n__HTTP_CODE__:%{http_code}" \
        "$BACKEND_URL/proxy/by-id/$CREDENTIAL_ID/v1/users?page_size=1" \
        -H "Authorization: Bearer $JWT_TOKEN" \
        -H "Content-Type: application/json" 2>/dev/null || echo "__HTTP_CODE__:000")

    PROXY_BODY=$(echo "$PROXY_RESP" | sed '/__HTTP_CODE__/d')
    PROXY_CODE=$(echo "$PROXY_RESP" | grep "__HTTP_CODE__" | cut -d: -f2)

    echo "  HTTP $PROXY_CODE"
    if [ "$PROXY_CODE" = "200" ]; then
        echo "  ✓ Notion API accessible via proxy"
        echo "  Response: ${PROXY_BODY:0:200}..."
    else
        echo "  ✗ Proxy returned HTTP $PROXY_CODE"
        echo "  Response: $PROXY_BODY"
    fi
else
    echo ""
    echo "--- Step 2: SKIPPED (no CREDENTIAL_ID) ---"
fi

# ── Step 3: Test via Rust runtime ──
echo ""
echo "--- Step 3: Skill Runtime Test (with live backend) ---"

export SKILL_DEBUG_ID=notion
export SKILL_DEBUG_TOOL=sync-status
export RUST_LOG="${RUST_LOG:-info}"

cargo test --test skills_debug_e2e skill_full_lifecycle -- --nocapture 2>&1 | \
    grep -E "(✓|✗|·|---|====|Text:|Result:)" | head -40

echo ""
echo "--- Step 4: Notion Live Test (real data dir) ---"
echo ""
cargo test --test skills_notion_live -- --nocapture 2>&1 | \
    grep -E "(✓|✗|---|Step|Backend|OAuth|HTTP|status|connected|workspace|totals|Result:|is_error|Done|COMPLETE)" | head -30

echo ""
echo "=== Done ==="
