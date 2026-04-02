#!/usr/bin/env bash
#
# debug-notion-sync-memory.sh — Run the Notion live test with memory verification.
#
# Tests the full flow:  skill start → sync → memory persistence → verify documents
#
# Prerequisites:
#   - .env with BACKEND_URL, JWT_TOKEN, CREDENTIAL_ID, SKILLS_DATA_DIR
#   - OAuth credential at $SKILLS_DATA_DIR/notion/oauth_credential.json
#   - openhuman-skills repo available (auto-detected or via SKILL_DEBUG_DIR)
#
# Usage:
#   bash scripts/debug-notion-sync-memory.sh
#
# Environment variables (set in .env or export before running):
#   BACKEND_URL     — backend API URL (e.g. https://staging-api.alphahuman.xyz)
#   JWT_TOKEN       — session JWT for OAuth proxy
#   CREDENTIAL_ID   — OAuth credential ID for the proxy
#   SKILLS_DATA_DIR — path to skills data dir (contains notion/ subdir)
#   RUST_LOG        — Rust log filter (default: info)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load .env
if [ -f "$REPO_ROOT/.env" ]; then
    source "$SCRIPT_DIR/load-dotenv.sh" "$REPO_ROOT/.env"
fi

export RUN_LIVE_NOTION=1
export RUST_LOG="${RUST_LOG:-info}"

echo "========================================"
echo "  Notion Sync + Memory Verification"
echo "========================================"
echo "  BACKEND_URL:     ${BACKEND_URL:-<not set>}"
echo "  JWT_TOKEN:       ${JWT_TOKEN:+<set, ${#JWT_TOKEN} bytes>}"
echo "  CREDENTIAL_ID:   ${CREDENTIAL_ID:-<not set>}"
echo "  SKILLS_DATA_DIR: ${SKILLS_DATA_DIR:-<not set>}"
echo "  RUST_LOG:        ${RUST_LOG}"
echo ""

# Verify required vars
for var in BACKEND_URL JWT_TOKEN CREDENTIAL_ID SKILLS_DATA_DIR; do
    if [ -z "${!var:-}" ]; then
        echo "ERROR: $var is not set. Add it to .env or export it."
        exit 1
    fi
done

# Verify OAuth credential exists
CRED_FILE="$SKILLS_DATA_DIR/notion/oauth_credential.json"
if [ -f "$CRED_FILE" ]; then
    echo "  OAuth credential: present ($(wc -c < "$CRED_FILE" | tr -d ' ') bytes)"
else
    echo "  WARNING: $CRED_FILE not found"
    echo "  The skill will start without OAuth — API calls will fail"
fi
echo ""

cd "$REPO_ROOT"

echo "--- Running Notion live test with memory verification ---"
echo ""

cargo test --test skills_notion_live -- --nocapture notion_live_with_real_data 2>&1

echo ""
echo "========================================"
echo "  DONE"
echo "========================================"
