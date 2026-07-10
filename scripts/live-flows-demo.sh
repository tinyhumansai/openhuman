#!/usr/bin/env bash
#
# Live demo of the flows agents + Opus/Sonnet demo workflow against a real
# backend. Drives the whole arc end-to-end:
#   flows_discover → flows_build → flows_create → flows_run
# and prints each step's output + the managed tier each agent node used.
#
# This runs the #[ignore]d `live_flows_demo_discover_build_save_run` test, which
# needs real credentials. It reads them from your `.env` (via load-dotenv.sh) or
# the ambient environment:
#   OPENHUMAN_LIVE_API_URL   backend origin (e.g. https://api.example.com)
#   OPENHUMAN_LIVE_TOKEN     a valid user session JWT
#   OPENHUMAN_LIVE_USER_ID   the user id owning that session
# Optional:
#   OPENHUMAN_LIVE_FLOWS_TOPIC  research topic to run the demo flow on
#
# Usage:
#   scripts/live-flows-demo.sh [path/to/.env]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load .env (default: repo-root .env) into the environment if present. Missing
# file is non-fatal — the vars may already be exported in the shell.
ENV_FILE="${1:-$ROOT_DIR/.env}"
if [[ -f "$ENV_FILE" ]]; then
  # shellcheck source=/dev/null
  source "$SCRIPT_DIR/load-dotenv.sh" "$ENV_FILE"
fi

: "${OPENHUMAN_LIVE_API_URL:?set OPENHUMAN_LIVE_API_URL (backend origin)}"
: "${OPENHUMAN_LIVE_TOKEN:?set OPENHUMAN_LIVE_TOKEN (user session JWT)}"
: "${OPENHUMAN_LIVE_USER_ID:?set OPENHUMAN_LIVE_USER_ID (user id)}"

echo "[live-flows-demo] backend=$OPENHUMAN_LIVE_API_URL user=$OPENHUMAN_LIVE_USER_ID"

# macOS Apple-Silicon whisper-rs/llama.cpp build workaround (see AGENTS.md).
export GGML_NATIVE="${GGML_NATIVE:-OFF}"

cd "$ROOT_DIR"
exec cargo test --manifest-path Cargo.toml --test live_flows_demo_e2e -- --ignored --nocapture
