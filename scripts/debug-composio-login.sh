#!/usr/bin/env bash
#
# debug-composio-login.sh — Walk the Composio Google/Gmail OAuth
# handoff end-to-end against a live openhuman backend.
#
# This is the Rust-side counterpart to
#   backend-1/src/scripts/live-test-composio-gmail.ts
# and it hits the exact same endpoints that the new
# src/openhuman/composio/ module wraps in Rust.
#
# Flow:
#   1. GET  /agent-integrations/composio/toolkits
#        → verify that the target toolkit (default: gmail) is on the
#          backend allowlist.
#   2. GET  /agent-integrations/composio/connections
#        → list existing connections; skip OAuth if one is already
#          ACTIVE/CONNECTED.
#   3. POST /agent-integrations/composio/authorize  {toolkit}
#        → print the `connectUrl` for the user to open in a browser,
#          then poll /connections until the status flips to
#          ACTIVE/CONNECTED (or timeout).
#   4. GET  /agent-integrations/composio/tools?toolkits=<toolkit>
#        → print the first ~20 tool slugs discovered.
#   5. (optional) POST /agent-integrations/composio/execute
#        → run a read-only action like GMAIL_GET_PROFILE.
#
# Usage:
#   bash scripts/debug-composio-login.sh
#
# Environment variables (set in .env or export before running):
#   BACKEND_URL                — e.g. https://staging-api.alphahuman.xyz
#   JWT_TOKEN                  — bearer JWT for your test user
#   COMPOSIO_TOOLKIT           — toolkit slug (default: gmail)
#   COMPOSIO_EXECUTE_TOOL      — optional, e.g. GMAIL_GET_PROFILE
#   COMPOSIO_AUTH_TIMEOUT_SECS — OAuth poll timeout (default: 300)
#   COMPOSIO_POLL_INTERVAL_SECS — poll interval (default: 5)
#   COMPOSIO_OPEN_URL          — "1" to auto-open connectUrl via `open`
#
# Requirements: bash, curl, jq.

set -euo pipefail

# Track any temp files created by `call` so we can clean them up on
# abort (Ctrl+C, error exit, etc.) — otherwise a mid-flight interrupt
# leaves a dangling mktemp file behind.
TMP_FILES=()
cleanup_tmp_files() {
    for f in "${TMP_FILES[@]}"; do
        [ -n "$f" ] && rm -f "$f"
    done
}
trap cleanup_tmp_files EXIT INT TERM

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Load .env ────────────────────────────────────────────────────────
if [ -f "$REPO_ROOT/.env" ]; then
    # shellcheck disable=SC1091
    source "$SCRIPT_DIR/load-dotenv.sh" "$REPO_ROOT/.env"
fi

# ── Inputs ───────────────────────────────────────────────────────────
BACKEND_URL="${BACKEND_URL:-}"
JWT_TOKEN="${JWT_TOKEN:-}"
TOOLKIT="${COMPOSIO_TOOLKIT:-gmail}"
EXECUTE_TOOL="${COMPOSIO_EXECUTE_TOOL:-}"
AUTH_TIMEOUT_SECS="${COMPOSIO_AUTH_TIMEOUT_SECS:-300}"
POLL_INTERVAL_SECS="${COMPOSIO_POLL_INTERVAL_SECS:-5}"
OPEN_URL="${COMPOSIO_OPEN_URL:-0}"

if [ -z "$BACKEND_URL" ]; then
    echo "ERROR: BACKEND_URL not set. Add it to .env or export it." >&2
    exit 1
fi
if [ -z "$JWT_TOKEN" ]; then
    echo "ERROR: JWT_TOKEN not set. Add it to .env or export it." >&2
    exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
    echo "ERROR: jq is required (brew install jq / apt install jq)" >&2
    exit 1
fi

# Strip any trailing slash on BACKEND_URL so path joining is predictable.
BACKEND_URL="${BACKEND_URL%/}"

echo "╔════════════════════════════════════════════════════════╗"
echo "║  Composio Login Debug                                  ║"
echo "╠════════════════════════════════════════════════════════╣"
printf "║  Backend:       %s\n" "$BACKEND_URL"
printf "║  Toolkit:       %s\n" "$TOOLKIT"
printf "║  JWT:           %s...\n" "${JWT_TOKEN:0:20}"
if [ -n "$EXECUTE_TOOL" ]; then
    printf "║  Execute tool:  %s\n" "$EXECUTE_TOOL"
fi
echo "╚════════════════════════════════════════════════════════╝"
echo ""

AUTH_HEADER="Authorization: Bearer $JWT_TOKEN"

# ── Helper: call backend and split body/status ──────────────────────
# Usage:  call METHOD PATH [json-body]
# Exports RESP_BODY and RESP_CODE after return.
call() {
    local method="$1" path="$2" body="${3:-}"
    local url="${BACKEND_URL}${path}"
    local tmp
    tmp="$(mktemp)"
    TMP_FILES+=("$tmp")
    if [ -n "$body" ]; then
        RESP_CODE=$(curl -sS -X "$method" "$url" \
            -H "$AUTH_HEADER" \
            -H "Content-Type: application/json" \
            --data "$body" \
            -o "$tmp" -w "%{http_code}" || echo "000")
    else
        RESP_CODE=$(curl -sS -X "$method" "$url" \
            -H "$AUTH_HEADER" \
            -o "$tmp" -w "%{http_code}" || echo "000")
    fi
    RESP_BODY="$(cat "$tmp")"
    rm -f "$tmp"
}

envelope_data() {
    # Extract `.data` from a `{success, data, error}` envelope; fall
    # back to the raw body if not enveloped.
    local body="$1"
    echo "$body" | jq -c '.data // .' 2>/dev/null || echo "$body"
}

envelope_error() {
    local body="$1"
    echo "$body" | jq -r '.error // empty' 2>/dev/null || true
}

require_success() {
    local step="$1"
    if [ "$RESP_CODE" != "200" ] && [ "$RESP_CODE" != "201" ]; then
        echo "  ✗ $step failed (HTTP $RESP_CODE)" >&2
        echo "    body: $RESP_BODY" >&2
        exit 1
    fi
}

# ── Step 1: list toolkits ────────────────────────────────────────────
echo "--- Step 1: GET /agent-integrations/composio/toolkits ---"
call GET "/agent-integrations/composio/toolkits"
require_success "list_toolkits"

TOOLKITS_JSON="$(envelope_data "$RESP_BODY")"
TOOLKITS_LIST="$(echo "$TOOLKITS_JSON" | jq -r '.toolkits[]?' 2>/dev/null || true)"
echo "  enabled toolkits:"
if [ -z "$TOOLKITS_LIST" ]; then
    echo "    (none)"
else
    echo "$TOOLKITS_LIST" | sed 's/^/    - /'
fi

if ! echo "$TOOLKITS_LIST" | grep -iqx "$TOOLKIT"; then
    echo "  ✗ toolkit '$TOOLKIT' is NOT in the backend allowlist." >&2
    echo "    Add it via COMPOSIO_ENABLED_TOOLKITS on the backend and retry." >&2
    exit 1
fi
echo "  ✓ $TOOLKIT is on the allowlist"
echo ""

# ── Step 2: list existing connections ───────────────────────────────
echo "--- Step 2: GET /agent-integrations/composio/connections ---"
call GET "/agent-integrations/composio/connections"
require_success "list_connections"

CONNECTIONS_JSON="$(envelope_data "$RESP_BODY")"
echo "$CONNECTIONS_JSON" | jq -r '.connections[]? | "  - \(.toolkit) [\(.status)] id=\(.id)"' 2>/dev/null || true

ACTIVE_ID="$(echo "$CONNECTIONS_JSON" | jq -r --arg tk "$TOOLKIT" \
    '.connections[]? | select((.toolkit|ascii_downcase) == ($tk|ascii_downcase)) | select(.status == "ACTIVE" or .status == "CONNECTED") | .id' \
    2>/dev/null | head -n1)"

if [ -n "$ACTIVE_ID" ]; then
    echo "  ✓ existing $TOOLKIT connection is ACTIVE (id=$ACTIVE_ID) — skipping OAuth"
    echo ""
else
    # ── Step 3: authorize ───────────────────────────────────────────
    echo ""
    echo "--- Step 3: POST /agent-integrations/composio/authorize ---"
    call POST "/agent-integrations/composio/authorize" "{\"toolkit\":\"$TOOLKIT\"}"
    require_success "authorize"

    AUTH_JSON="$(envelope_data "$RESP_BODY")"
    CONNECT_URL="$(echo "$AUTH_JSON" | jq -r '.connectUrl // empty')"
    CONNECTION_ID="$(echo "$AUTH_JSON" | jq -r '.connectionId // empty')"

    if [ -z "$CONNECT_URL" ]; then
        echo "  ✗ authorize response did not include connectUrl" >&2
        echo "    body: $RESP_BODY" >&2
        exit 1
    fi

    echo "  connectionId: $CONNECTION_ID"
    echo "  connectUrl:   $CONNECT_URL"
    echo ""
    echo "  >>> OPEN THIS URL IN A BROWSER TO COMPLETE GOOGLE OAUTH:"
    echo "      $CONNECT_URL"
    echo ""

    # Optionally auto-open on macOS.
    if [ "$OPEN_URL" = "1" ] && command -v open >/dev/null 2>&1; then
        open "$CONNECT_URL" >/dev/null 2>&1 || true
    fi

    echo "  polling /connections until $TOOLKIT becomes ACTIVE..."
    echo "    timeout=${AUTH_TIMEOUT_SECS}s interval=${POLL_INTERVAL_SECS}s"

    START_TS=$(date +%s)
    TICK=0
    while :; do
        TICK=$((TICK + 1))
        call GET "/agent-integrations/composio/connections"
        if [ "$RESP_CODE" = "200" ]; then
            CONNECTIONS_JSON="$(envelope_data "$RESP_BODY")"
            STATUS="$(echo "$CONNECTIONS_JSON" | jq -r --arg tk "$TOOLKIT" --arg cid "$CONNECTION_ID" \
                '.connections[]? | select(.id == $cid or (.toolkit|ascii_downcase) == ($tk|ascii_downcase)) | .status' \
                2>/dev/null | head -n1)"
            printf "    [tick %d] status=%s\n" "$TICK" "${STATUS:-<missing>}"
            if [ "$STATUS" = "ACTIVE" ] || [ "$STATUS" = "CONNECTED" ]; then
                ACTIVE_ID="$CONNECTION_ID"
                echo "  ✓ connection became ACTIVE (id=$ACTIVE_ID)"
                break
            fi
        else
            echo "    [tick $TICK] poll HTTP $RESP_CODE — $(envelope_error "$RESP_BODY")"
        fi

        NOW_TS=$(date +%s)
        if [ $((NOW_TS - START_TS)) -ge "$AUTH_TIMEOUT_SECS" ]; then
            echo "  ✗ timed out after ${AUTH_TIMEOUT_SECS}s waiting for OAuth to complete" >&2
            exit 1
        fi
        sleep "$POLL_INTERVAL_SECS"
    done
    echo ""
fi

# ── Step 4: list tools for the toolkit ──────────────────────────────
echo "--- Step 4: GET /agent-integrations/composio/tools?toolkits=$TOOLKIT ---"
call GET "/agent-integrations/composio/tools?toolkits=$TOOLKIT"
require_success "list_tools"

TOOLS_JSON="$(envelope_data "$RESP_BODY")"
TOOL_COUNT="$(echo "$TOOLS_JSON" | jq -r '.tools | length' 2>/dev/null || echo 0)"
echo "  found $TOOL_COUNT tool(s) for $TOOLKIT"
echo "$TOOLS_JSON" | jq -r '.tools[0:20][] | "    - \(.function.name)"' 2>/dev/null || true
if [ "$TOOL_COUNT" -gt 20 ]; then
    echo "    … (+$((TOOL_COUNT - 20)) more)"
fi
echo ""

# ── Step 5: optional execute ────────────────────────────────────────
if [ -n "$EXECUTE_TOOL" ]; then
    echo "--- Step 5: POST /agent-integrations/composio/execute ($EXECUTE_TOOL) ---"
    call POST "/agent-integrations/composio/execute" \
        "{\"tool\":\"$EXECUTE_TOOL\",\"arguments\":{}}"
    require_success "execute"

    EXEC_JSON="$(envelope_data "$RESP_BODY")"
    SUCCESSFUL="$(echo "$EXEC_JSON" | jq -r '.successful // false')"
    COST="$(echo "$EXEC_JSON" | jq -r '.costUsd // 0')"
    ERR="$(echo "$EXEC_JSON" | jq -r '.error // empty')"

    printf "  successful: %s\n" "$SUCCESSFUL"
    printf "  costUsd:    %s\n" "$COST"
    if [ -n "$ERR" ]; then
        printf "  error:      %s\n" "$ERR"
    fi
    echo "  data preview:"
    echo "$EXEC_JSON" | jq -C '.data' 2>/dev/null | head -n 20 || echo "$EXEC_JSON"
    echo ""

    if [ "$SUCCESSFUL" != "true" ]; then
        echo "  ✗ $EXECUTE_TOOL reported successful=false" >&2
        exit 1
    fi
else
    echo "--- Step 5: SKIPPED — set COMPOSIO_EXECUTE_TOOL=GMAIL_GET_PROFILE to exercise execute ---"
    echo ""
fi

echo "=== Done ==="
echo "  toolkit:      $TOOLKIT"
echo "  connectionId: ${ACTIVE_ID:-<none>}"
