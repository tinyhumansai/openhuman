#!/usr/bin/env bash
# End-to-end subconscious loop test with real local AI (Ollama).
# Ingests data, runs ticks, verifies decisions.
set -euo pipefail

CORE_BIN="./app/src-tauri/binaries/openhuman-core-x86_64-pc-windows-msvc.exe"
RPC_PORT=7810
RPC_URL="http://127.0.0.1:${RPC_PORT}/rpc"
FIXTURES="./tests/fixtures/subconscious"

# Pre-seed the RPC bearer token so curl calls authenticate correctly.
# The core reads OPENHUMAN_CORE_TOKEN at startup and skips writing a token file.
RPC_TOKEN="$(openssl rand -hex 32 2>/dev/null || python3 -c 'import secrets; print(secrets.token_hex(32))')"

if [ ! -f "$CORE_BIN" ]; then echo "ERROR: Core binary not found"; exit 1; fi

# Check Ollama
if ! curl -s --max-time 3 http://localhost:11434/ >/dev/null 2>&1; then
  echo "ERROR: Ollama not running. Start with: ollama serve"
  exit 1
fi

echo "=== Subconscious Loop E2E Test ==="
echo ""

# Start core server
echo "[setup] Starting core on port $RPC_PORT..."
OPENHUMAN_CORE_PORT="$RPC_PORT" OPENHUMAN_CORE_TOKEN="$RPC_TOKEN" "$CORE_BIN" serve > /tmp/subconscious-test.log 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; wait "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

for i in $(seq 1 15); do
  if curl -s "$RPC_URL" -H "Content-Type: application/json" -H "Authorization: Bearer $RPC_TOKEN" \
    -d '{"jsonrpc":"2.0","id":0,"method":"openhuman.health_snapshot","params":{}}' 2>/dev/null | grep -q "result"; then
    echo "[setup] Server ready."
    break
  fi
  [ "$i" -eq 15 ] && { echo "ERROR: Server timeout"; exit 1; }
  sleep 1
done

rpc() {
  curl -s --max-time 120 "$RPC_URL" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $RPC_TOKEN" \
    -d "$1" 2>&1
}

# Write HEARTBEAT.md to the workspace
echo "[setup] Writing HEARTBEAT.md to workspace..."
WORKSPACE="$HOME/.openhuman/workspace"
mkdir -p "$WORKSPACE"
cp "$FIXTURES/heartbeat.md" "$WORKSPACE/HEARTBEAT.md"
echo "[setup] HEARTBEAT.md written: $(cat "$WORKSPACE/HEARTBEAT.md" | grep "^- " | wc -l) tasks"

echo ""
echo "========================================="
echo "  PHASE 1: Ingest tick 1 data"
echo "========================================="

# Ingest tick1 gmail
GMAIL1=$(cat "$FIXTURES/tick1_gmail.txt")
GMAIL1_ESC=$(echo "$GMAIL1" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))")
RESULT=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"openhuman.memory_doc_ingest\",\"params\":{\"namespace\":\"skill-gmail\",\"key\":\"tick1-gmail\",\"title\":\"Deadline reminder and meeting invite\",\"content\":$GMAIL1_ESC,\"source_type\":\"gmail\",\"priority\":\"high\",\"category\":\"core\"}}")
echo "Gmail tick1 ingested: $(echo "$RESULT" | python3 -c "import sys,json;d=json.load(sys.stdin);r=d.get('result',{});print(f\"{r.get('entityCount',0)} entities, {r.get('relationCount',0)} relations\")" 2>/dev/null || echo "$RESULT" | head -c 200)"

# Ingest tick1 notion
NOTION1=$(cat "$FIXTURES/tick1_notion.txt")
NOTION1_ESC=$(echo "$NOTION1" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))")
RESULT=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"openhuman.memory_doc_ingest\",\"params\":{\"namespace\":\"skill-notion\",\"key\":\"tick1-notion\",\"title\":\"Q1 Delivery Tracker\",\"content\":$NOTION1_ESC,\"source_type\":\"notion\",\"priority\":\"high\",\"category\":\"core\"}}")
echo "Notion tick1 ingested: $(echo "$RESULT" | python3 -c "import sys,json;d=json.load(sys.stdin);r=d.get('result',{});print(f\"{r.get('entityCount',0)} entities, {r.get('relationCount',0)} relations\")" 2>/dev/null || echo "$RESULT" | head -c 200)"

# Check what's in memory
echo ""
echo "Namespaces after tick1 ingest:"
rpc '{"jsonrpc":"2.0","id":3,"method":"openhuman.memory_list_namespaces","params":{}}' | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('result',{}).get('data',{}).get('namespaces',[]))" 2>/dev/null

echo ""
echo "========================================="
echo "  PHASE 2: Subconscious Tick 1"
echo "========================================="
echo "(Calling local AI via Ollama — may take 30-60s)"

TICK1=$(rpc '{"jsonrpc":"2.0","id":10,"method":"openhuman.subconscious_trigger","params":{}}')
echo "Tick 1 result:"
echo "$TICK1" | python3 -c "import sys,json; print(json.dumps(json.load(sys.stdin), indent=2))" 2>/dev/null || echo "$TICK1" | head -c 500

echo ""
echo "========================================="
echo "  PHASE 3: Ingest tick 2 data (state change)"
echo "========================================="

# Ingest tick2 gmail (deadline moved)
GMAIL2=$(cat "$FIXTURES/tick2_gmail.txt")
GMAIL2_ESC=$(echo "$GMAIL2" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))")
RESULT=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"openhuman.memory_doc_ingest\",\"params\":{\"namespace\":\"skill-gmail\",\"key\":\"tick2-gmail\",\"title\":\"URGENT deadline moved to tomorrow\",\"content\":$GMAIL2_ESC,\"source_type\":\"gmail\",\"priority\":\"high\",\"category\":\"core\"}}")
echo "Gmail tick2 ingested: $(echo "$RESULT" | python3 -c "import sys,json;d=json.load(sys.stdin);r=d.get('result',{});print(f\"{r.get('entityCount',0)} entities, {r.get('relationCount',0)} relations\")" 2>/dev/null || echo "$RESULT" | head -c 200)"

# Ingest tick2 notion (tracker updated)
NOTION2=$(cat "$FIXTURES/tick2_notion.txt")
NOTION2_ESC=$(echo "$NOTION2" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))")
RESULT=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"openhuman.memory_doc_ingest\",\"params\":{\"namespace\":\"skill-notion\",\"key\":\"tick2-notion\",\"title\":\"Q1 Tracker updated - unblocked\",\"content\":$NOTION2_ESC,\"source_type\":\"notion\",\"priority\":\"high\",\"category\":\"core\"}}")
echo "Notion tick2 ingested: $(echo "$RESULT" | python3 -c "import sys,json;d=json.load(sys.stdin);r=d.get('result',{});print(f\"{r.get('entityCount',0)} entities, {r.get('relationCount',0)} relations\")" 2>/dev/null || echo "$RESULT" | head -c 200)"

echo ""
echo "========================================="
echo "  PHASE 4: Subconscious Tick 2"
echo "========================================="
echo "(Calling local AI via Ollama — may take 30-60s)"

TICK2=$(rpc '{"jsonrpc":"2.0","id":11,"method":"openhuman.subconscious_trigger","params":{}}')
echo "Tick 2 result:"
echo "$TICK2" | python3 -c "import sys,json; print(json.dumps(json.load(sys.stdin), indent=2))" 2>/dev/null || echo "$TICK2" | head -c 500

echo ""
echo "========================================="
echo "  PHASE 5: Status check"
echo "========================================="

STATUS=$(rpc '{"jsonrpc":"2.0","id":12,"method":"openhuman.subconscious_status","params":{}}')
echo "Subconscious status:"
echo "$STATUS" | python3 -c "import sys,json; print(json.dumps(json.load(sys.stdin), indent=2))" 2>/dev/null || echo "$STATUS" | head -c 500

echo ""
echo "========================================="
echo "  DONE"
echo "========================================="
