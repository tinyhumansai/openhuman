#!/usr/bin/env bash
#
# debug-notion-live.sh — Debug Notion skill with a live backend + JWT.
#
# Tests the full OAuth proxy chain that the Notion skill uses:
#   1. Raw HTTP call to backend proxy endpoint
#   2. Skill startup with BACKEND_URL + session token
#   3. Tool call that uses oauth.fetch (proxied through backend)
#
# Usage:
#   bash scripts/debug-notion-live.sh
#
# Environment overrides:
#   BACKEND_URL   — staging or prod backend (default: staging)
#   JWT_TOKEN     — session JWT
#   CREDENTIAL_ID — Notion OAuth credential ID
#
set -euo pipefail

BACKEND_URL="${BACKEND_URL:-https://staging-api.alphahuman.xyz}"
JWT_TOKEN="${JWT_TOKEN:-eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VySWQiOiI2OWMzNDNlNjlmMmFjZmI4MzE2YTAzZGYiLCJpYXQiOjE3NzQ1Njc2NzcsImV4cCI6MTc3NzE1OTY3N30.hFB3ogBY9fwwhjWxrWqd1BDAawIJeMhPWtWYBd7pNwg}"
CREDENTIAL_ID="${CREDENTIAL_ID:-69cafd0b103bd070232d3223}"

echo "╔════════════════════════════════════════════════════════╗"
echo "║  Notion Skill Live Debug                               ║"
echo "╠════════════════════════════════════════════════════════╣"
echo "║  Backend:       $BACKEND_URL"
echo "║  Credential ID: $CREDENTIAL_ID"
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
    echo "  The staging backend at $BACKEND_URL is unreachable."
    echo "  This is the root cause — the Notion skill uses oauth.fetch() which"
    echo "  proxies requests through: $BACKEND_URL/proxy/by-id/$CREDENTIAL_ID/{path}"
    echo ""
    echo "  When the backend is down:"
    echo "    → oauth.fetch returns error"
    echo "    → bootstrap.js interprets as 401/403"
    echo "    → auto-clears credential"
    echo "    → sets connection_status='error'"
    echo "    → auth_status='not_authenticated'"
    echo ""
    echo "  Fix: Bring the staging backend online, then re-run this script."
    exit 1
fi

if [ "$HTTP_CODE" = "401" ]; then
    echo "  ✗ JWT is invalid or expired (HTTP 401)"
    echo "  Get a fresh JWT from the staging backend."
    exit 1
fi

echo "  ✓ Backend reachable (HTTP $HTTP_CODE)"

# ── Step 2: Raw proxy call ──
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
    echo ""
    echo "  Possible issues:"
    echo "    - Credential $CREDENTIAL_ID may be revoked/expired on the backend"
    echo "    - The Notion integration token may have been disconnected"
    echo "    - The backend proxy route may not be configured"
fi

# ── Step 3: Test via Rust runtime ──
echo ""
echo "--- Step 3: Skill Runtime Test (with live backend) ---"
echo "  Running: cargo test --test skills_debug_e2e skill_full_lifecycle"
echo "  BACKEND_URL=$BACKEND_URL"
echo ""

export BACKEND_URL
export SKILL_DEBUG_ID=notion
export SKILL_DEBUG_TOOL=sync-status
export RUST_LOG=info

# Run the test and capture output
cargo test --test skills_debug_e2e skill_full_lifecycle -- --nocapture 2>&1 | \
    grep -E "(✓|✗|·|---|====|Text:|Result:)" | head -40

echo ""
echo "--- Step 4: Test via HTTP JSON-RPC ---"
echo ""
cargo test --test skills_rpc_e2e -- --nocapture 2>&1 | \
    grep -E "(---|Result:|Status:|tools:|skill)" | head -20

echo ""
echo "=== Done ==="
