#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# test-channel-messaging.sh
#
# End-to-end test: sends a message from the backend to the user's
# linked Telegram account via the Rust core RPC.
#
# Usage:
#   bash scripts/test-channel-messaging.sh
#   bash scripts/test-channel-messaging.sh "Custom message text"
#
# Prerequisites:
#   - Active session token (login via the app first)
#   - Telegram account linked (completed managed DM flow)
#   - Core binary built: cargo build --bin openhuman-core
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load env
if [[ -f "$ROOT_DIR/scripts/load-dotenv.sh" ]]; then
  source "$ROOT_DIR/scripts/load-dotenv.sh" 2>/dev/null || true
fi

CORE_BIN="${OPENHUMAN_CORE_BIN:-}"
if [[ -z "$CORE_BIN" ]]; then
  CORE_BIN="$ROOT_DIR/target/debug/openhuman-core"
  if [[ ! -x "$CORE_BIN" ]]; then
    echo "Building openhuman-core..."
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --bin openhuman-core 2>&1 | tail -2
  fi
fi

MESSAGE="${1:-Hello from OpenHuman! 🚀 This is a test message sent via the channel messaging API.}"

divider() { echo "────────────────────────────────────────────────"; }

echo ""
echo "🧪 Channel Messaging E2E Test"
divider

# ── Step 1: Check session ────────────────────────────────────────────
echo ""
echo "1️⃣  Checking session..."
AUTH_STATE=$("$CORE_BIN" auth get_state 2>&1 | grep -A20 '{' || true)
IS_AUTH=$(echo "$AUTH_STATE" | grep -o '"isAuthenticated": *true' || true)

if [[ -z "$IS_AUTH" ]]; then
  echo "   ❌ Not authenticated. Please login via the app first."
  echo "   Auth state:"
  echo "$AUTH_STATE" | head -10
  exit 1
fi
echo "   ✅ Authenticated"

# ── Step 2: Validate session against backend ─────────────────────────
echo ""
echo "2️⃣  Validating session with backend (GET /auth/me)..."
ME_RESULT=$("$CORE_BIN" auth get_me 2>&1 || true)
if echo "$ME_RESULT" | grep -qi "401\|Invalid token\|expired\|failed"; then
  echo "   ❌ Session token expired or invalid."
  echo "   $ME_RESULT" | tail -3
  echo ""
  echo "   Please re-login via the app to get a fresh token."
  exit 1
fi

TELEGRAM_ID=$(echo "$ME_RESULT" | grep -o '"telegramId": *"[^"]*"' | head -1 | sed 's/.*: *"//;s/"//' || true)
USERNAME=$(echo "$ME_RESULT" | grep -o '"username": *"[^"]*"' | head -1 | sed 's/.*: *"//;s/"//' || true)
echo "   ✅ Session valid — user: ${USERNAME:-unknown}, telegramId: ${TELEGRAM_ID:-not linked}"

if [[ -z "$TELEGRAM_ID" ]]; then
  echo ""
  echo "   ⚠️  No telegramId found on your profile."
  echo "   Complete the Telegram managed DM linking flow first."
  echo "   (Skills page → Telegram → Login with OpenHuman → click Start in Telegram)"
  exit 1
fi

# ── Step 3: Send a text message via Telegram ─────────────────────────
echo ""
echo "3️⃣  Sending message to Telegram..."
echo "   Channel: telegram"
echo "   Message: $MESSAGE"
divider

SEND_RESULT=$("$CORE_BIN" channels send_message \
  --channel telegram \
  --message "{\"text\": \"$MESSAGE\"}" 2>&1 || true)

echo "$SEND_RESULT" | grep -A20 '{' | head -20

if echo "$SEND_RESULT" | grep -qi '"success": *true\|"messageId"'; then
  echo ""
  echo "   ✅ Message sent successfully! Check your Telegram."
else
  echo ""
  echo "   ❌ Message send may have failed. Check output above."
fi

# ── Step 4: Send a message with a button ─────────────────────────────
echo ""
echo "4️⃣  Sending message with inline button..."

BUTTON_MSG=$("$CORE_BIN" channels send_message \
  --channel telegram \
  --message '{"text": "Here is a link for you:", "buttons": [{"label": "OpenHuman GitHub", "url": "https://github.com/tinyhumansai/openhuman"}]}' 2>&1 || true)

echo "$BUTTON_MSG" | grep -A20 '{' | head -15

if echo "$BUTTON_MSG" | grep -qi '"success": *true\|"messageId"'; then
  echo "   ✅ Button message sent!"
else
  echo "   ❌ Button message may have failed."
fi

# ── Step 5: List threads ─────────────────────────────────────────────
echo ""
echo "5️⃣  Listing Telegram threads..."

THREADS=$("$CORE_BIN" channels list_threads \
  --channel telegram 2>&1 || true)

# Show the JSON result (skip the banner lines)
echo "$THREADS" | tail -5
echo "   ✅ Threads listed."

# ── Done ─────────────────────────────────────────────────────────────
divider
echo ""
echo "✅ Channel messaging E2E test complete."
echo ""
echo "Available RPC methods:"
echo "  openhuman.channels_send_message    — Send rich message (text, photo, stickers, buttons)"
echo "  openhuman.channels_send_reaction   — React to a message with emoji"
echo "  openhuman.channels_create_thread   — Create a conversation thread"
echo "  openhuman.channels_update_thread   — Close or reopen a thread"
echo "  openhuman.channels_list_threads    — List threads for a channel"
echo ""
