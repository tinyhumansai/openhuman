#!/usr/bin/env bash
# bench-memory-retrieval.sh — benchmark memory retrieval quality and latency.
#
# Tests the core's memory query (semantic search) and tree file-based content
# retrieval against a set of benchmark queries. Measures wall-clock time per
# query, result count, and content quality.
#
# Usage:
#   ./scripts/bench-memory-retrieval.sh
#   ./scripts/bench-memory-retrieval.sh --verbose

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VERBOSE=0
[[ "${1:-}" == "--verbose" ]] && VERBOSE=1

CORE_BIN="$REPO_ROOT/target/debug/openhuman-core"
if [[ ! -x "$CORE_BIN" ]]; then
    echo "ERROR: Build openhuman-core first: cargo build --bin openhuman-core"
    exit 1
fi

WORKSPACE_DIR="${OPENHUMAN_WORKSPACE:-$HOME/.openhuman-staging}"
USERS_DIR="$WORKSPACE_DIR/users"

if [[ ! -d "$USERS_DIR" ]]; then
    echo "ERROR: Workspace users dir not found: $USERS_DIR"
    exit 1
fi

# Find first user workspace with memory_tree content
CONTENT_ROOT=$(find "$USERS_DIR" -path "*/workspace/memory_tree/content" -type d 2>/dev/null | head -1 || true)
if [[ -z "$CONTENT_ROOT" ]]; then
    echo "ERROR: No memory_tree content found under $USERS_DIR/"
    exit 1
fi
RESULTS_DIR="$REPO_ROOT/target/bench-memory"
mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
RESULTS_FILE="$RESULTS_DIR/retrieval-$TIMESTAMP.txt"

# Inventory
FILE_COUNT=$(find "$CONTENT_ROOT" -type f -name "*.md" 2>/dev/null | wc -l | tr -d ' ')
TOTAL_SIZE=$(du -sh "$CONTENT_ROOT" 2>/dev/null | cut -f1)
CHAT_COUNT=$(find "$CONTENT_ROOT/chat" -type f -name "*.md" 2>/dev/null | wc -l | tr -d ' ')
EPISODIC_COUNT=$(find "$CONTENT_ROOT/episodic" -type f -name "*.md" 2>/dev/null | wc -l | tr -d ' ')
RAW_COUNT=$(find "$CONTENT_ROOT/raw" -type f -name "*.md" 2>/dev/null | wc -l | tr -d ' ')
WIKI_COUNT=$(find "$CONTENT_ROOT/wiki" -type f -name "*.md" 2>/dev/null | wc -l | tr -d ' ')

cat <<EOF | tee "$RESULTS_FILE"
==============================================================
  Memory Retrieval Benchmark
  $(date)
==============================================================

Content root:    $CONTENT_ROOT
Total files:     $FILE_COUNT markdown files ($TOTAL_SIZE)
  chat:          $CHAT_COUNT files
  episodic:      $EPISODIC_COUNT files
  raw:           $RAW_COUNT files
  wiki:          $WIKI_COUNT files

--------------------------------------------------------------
  1. KV Memory Recall (semantic search via SQLite)
--------------------------------------------------------------
EOF

QUERIES=(
    "what are the most important things to work on"
    "what projects have I been discussing recently"
    "what are my preferences and settings"
    "what people have I interacted with"
    "what technical decisions have been made"
)

export OPENHUMAN_APP_ENV=staging
export OPENHUMAN_KEYRING_BACKEND=file

for query in "${QUERIES[@]}"; do
    echo "" | tee -a "$RESULTS_FILE"
    echo "Query: $query" | tee -a "$RESULTS_FILE"

    START_NS=$(python3 -c "import time; print(int(time.time()*1e9))")

    OUTPUT=$(RUST_LOG=error "$CORE_BIN" memory query -n global -q "$query" 2>&1) || true

    END_NS=$(python3 -c "import time; print(int(time.time()*1e9))")
    ELAPSED_MS=$(python3 -c "print(($END_NS - $START_NS) / 1_000_000)")

    RESULT_COUNT=$(echo "$OUTPUT" | grep -c '^\[' || true)
    FIRST_RESULT=$(echo "$OUTPUT" | grep '^\[' | head -1 | cut -c1-120)

    echo "  -> ${ELAPSED_MS}ms, $RESULT_COUNT results" | tee -a "$RESULTS_FILE"
    if [[ -n "$FIRST_RESULT" ]]; then
        echo "  -> Top: $FIRST_RESULT..." | tee -a "$RESULTS_FILE"
    fi

    if [[ $VERBOSE -eq 1 ]]; then
        echo "$OUTPUT" | head -20
    fi
done

cat <<EOF | tee -a "$RESULTS_FILE"

--------------------------------------------------------------
  2. Tree Content File Walk (direct file scan)
--------------------------------------------------------------
EOF

# Benchmark file-based tree walking: grep across content files
TREE_QUERIES=(
    "OpenHuman"
    "memory"
    "important"
    "project"
    "preference"
)

for pattern in "${TREE_QUERIES[@]}"; do
    echo "" | tee -a "$RESULTS_FILE"
    echo "Pattern: $pattern" | tee -a "$RESULTS_FILE"

    START_NS=$(python3 -c "import time; print(int(time.time()*1e9))")

    HIT_COUNT=$(grep -rl "$pattern" "$CONTENT_ROOT" 2>/dev/null | wc -l | tr -d ' ')
    SAMPLE=$(grep -rl "$pattern" "$CONTENT_ROOT" 2>/dev/null | head -3 | while read f; do
        basename "$f" | tr '\n' ' '
    done)

    END_NS=$(python3 -c "import time; print(int(time.time()*1e9))")
    ELAPSED_MS=$(python3 -c "print(($END_NS - $START_NS) / 1_000_000)")

    echo "  -> ${ELAPSED_MS}ms, $HIT_COUNT files matched" | tee -a "$RESULTS_FILE"
    if [[ -n "$SAMPLE" ]]; then
        echo "  -> Sample: $SAMPLE" | tee -a "$RESULTS_FILE"
    fi
done

cat <<EOF | tee -a "$RESULTS_FILE"

--------------------------------------------------------------
  3. RPC Memory Tree API (structured retrieval)
--------------------------------------------------------------
EOF

# Test the tree RPC APIs
RPC_METHODS=(
    "openhuman.memory_tree_search_entities::{\"query\":\"OpenHuman project\",\"limit\":5}"
    "openhuman.memory_tree_query_source::{\"source_kind\":\"chat\",\"limit\":5}"
    "openhuman.memory_tree_query_source::{\"source_kind\":\"episodic\",\"limit\":5}"
)

for spec in "${RPC_METHODS[@]}"; do
    METHOD="${spec%%::*}"
    PARAMS="${spec##*::}"

    echo "" | tee -a "$RESULTS_FILE"
    echo "RPC: $METHOD" | tee -a "$RESULTS_FILE"
    echo "  params: $PARAMS" | tee -a "$RESULTS_FILE"

    START_NS=$(python3 -c "import time; print(int(time.time()*1e9))")

    OUTPUT=$(RUST_LOG=error "$CORE_BIN" call --method "$METHOD" --params "$PARAMS" 2>&1) || true

    END_NS=$(python3 -c "import time; print(int(time.time()*1e9))")
    ELAPSED_MS=$(python3 -c "print(($END_NS - $START_NS) / 1_000_000)")

    # Extract hit count from JSON
    HITS=$(echo "$OUTPUT" | python3 -c "
import sys,json
raw = sys.stdin.read()
last_line = [l for l in raw.split('\n') if '{' in l]
d = json.loads(last_line[-1]) if last_line else {}
print(d.get('result',{}).get('total',0))
" 2>/dev/null || echo "parse-error")

    echo "  -> ${ELAPSED_MS}ms, hits=$HITS" | tee -a "$RESULTS_FILE"

    if [[ $VERBOSE -eq 1 ]]; then
        echo "$OUTPUT" | tail -5
    fi
done

cat <<EOF | tee -a "$RESULTS_FILE"

==============================================================
  Summary
==============================================================
Results saved to: $RESULTS_FILE
EOF
