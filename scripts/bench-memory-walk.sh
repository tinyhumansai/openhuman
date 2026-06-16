#!/usr/bin/env bash
# bench-memory-walk.sh — benchmark memory tree walking and retrieval performance.
#
# Calls the core CLI with a set of test queries against the memory tree,
# measures latency per query, and reports summary statistics.
#
# Usage:
#   ./scripts/bench-memory-walk.sh                                 # defaults
#   ./scripts/bench-memory-walk.sh --query "what is X?"            # single query
#   ./scripts/bench-memory-walk.sh --content-root /path/to/tree    # custom root
#   ./scripts/bench-memory-walk.sh --max-turns 20                  # more turns
#   ./scripts/bench-memory-walk.sh --model "deepseek:deepseek-chat"
#   ./scripts/bench-memory-walk.sh --verbose                       # show full output

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
CONTENT_ROOT="${OPENHUMAN_MEMORY_CONTENT_ROOT:-$HOME/.openhuman-staging/users/69d9cb73e61f755583c3671f/workspace/memory_tree/content}"
MAX_TURNS=12
NAMESPACE="default"
MODEL=""
VERBOSE=0
CUSTOM_QUERY=""

# Default benchmark queries covering different retrieval patterns
DEFAULT_QUERIES=(
    "What projects am I working on?"
    "What did I discuss in my most recent conversations?"
    "What are my preferences and settings?"
    "Find any mentions of GitHub or pull requests"
    "What people have I interacted with recently?"
)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --content-root) CONTENT_ROOT="$2"; shift 2 ;;
        --max-turns)    MAX_TURNS="$2"; shift 2 ;;
        --namespace)    NAMESPACE="$2"; shift 2 ;;
        --model)        MODEL="$2"; shift 2 ;;
        --query)        CUSTOM_QUERY="$2"; shift 2 ;;
        --verbose)      VERBOSE=1; shift ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --content-root PATH   Memory tree content root (default: staging)"
            echo "  --max-turns N         Max LLM turns per query (default: 12)"
            echo "  --namespace NS        Memory namespace (default: 'default')"
            echo "  --model MODEL         Provider:model override"
            echo "  --query TEXT          Run a single custom query instead of defaults"
            echo "  --verbose             Show full tool output"
            echo "  -h, --help            Show this help"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Verify content root exists
if [[ ! -d "$CONTENT_ROOT" ]]; then
    echo "ERROR: Content root does not exist: $CONTENT_ROOT"
    echo "Set OPENHUMAN_MEMORY_CONTENT_ROOT or use --content-root"
    exit 1
fi

# Count files in the tree
FILE_COUNT=$(find "$CONTENT_ROOT" -type f -name "*.md" 2>/dev/null | wc -l | tr -d ' ')
DIR_COUNT=$(find "$CONTENT_ROOT" -type d 2>/dev/null | wc -l | tr -d ' ')
TOTAL_SIZE=$(du -sh "$CONTENT_ROOT" 2>/dev/null | cut -f1)

echo "=============================================="
echo "  Memory Tree Walk Benchmark"
echo "=============================================="
echo ""
echo "Content root:  $CONTENT_ROOT"
echo "Files:         $FILE_COUNT markdown files"
echo "Directories:   $DIR_COUNT"
echo "Total size:    $TOTAL_SIZE"
echo "Max turns:     $MAX_TURNS"
echo "Namespace:     $NAMESPACE"
if [[ -n "$MODEL" ]]; then
    echo "Model:         $MODEL"
fi
echo ""

# Build the queries array
if [[ -n "$CUSTOM_QUERY" ]]; then
    QUERIES=("$CUSTOM_QUERY")
else
    QUERIES=("${DEFAULT_QUERIES[@]}")
fi

# Check if the core binary exists
CORE_BIN="$REPO_ROOT/target/debug/openhuman-core"
if [[ ! -x "$CORE_BIN" ]]; then
    echo "Building openhuman-core..."
    cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --bin openhuman-core 2>&1 | tail -3
    echo ""
fi

# Results storage
RESULTS_DIR="$REPO_ROOT/target/bench-memory"
mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
RESULTS_FILE="$RESULTS_DIR/bench-$TIMESTAMP.jsonl"

echo "----------------------------------------------"
echo "  Running ${#QUERIES[@]} queries"
echo "----------------------------------------------"
echo ""

TOTAL_START=$(date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))")
PASS=0
FAIL=0

for i in "${!QUERIES[@]}"; do
    query="${QUERIES[$i]}"
    idx=$((i + 1))
    echo "[$idx/${#QUERIES[@]}] $query"

    QUERY_START=$(date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))")

    # Call the core CLI with the memory_smart_walk RPC
    # Use python3 to safely build the JSON payload and avoid query injection
    RPC_PAYLOAD=$(python3 -c "
import json, sys
payload = {
    'jsonrpc': '2.0',
    'id': 'bench-$idx',
    'method': 'openhuman.memory_smart_walk',
    'params': {
        'query': sys.argv[1],
        'namespace': sys.argv[2],
        'max_turns': $MAX_TURNS
    }
}
print(json.dumps(payload))
" "$query" "$NAMESPACE")

    # Use the CLI's rpc subcommand if available, otherwise use the tool directly
    if [[ $VERBOSE -eq 1 ]]; then
        OUTPUT=$("$CORE_BIN" rpc --stdin <<< "$RPC_PAYLOAD" 2>&1) || true
        echo "$OUTPUT"
    else
        OUTPUT=$("$CORE_BIN" rpc --stdin <<< "$RPC_PAYLOAD" 2>/dev/null) || true
    fi

    QUERY_END=$(date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))")
    ELAPSED_MS=$(( (QUERY_END - QUERY_START) / 1000000 ))

    if echo "$OUTPUT" | grep -q '"result"'; then
        PASS=$((PASS + 1))
        STATUS="OK"
    else
        FAIL=$((FAIL + 1))
        STATUS="FAIL"
    fi

    echo "   -> ${STATUS} in ${ELAPSED_MS}ms"

    # Log to JSONL — use python3 to safely encode the query string
    python3 -c "
import json, sys
record = {
    'query': sys.argv[1],
    'elapsed_ms': $ELAPSED_MS,
    'status': sys.argv[2],
    'timestamp': sys.argv[3]
}
print(json.dumps(record))
" "$query" "$STATUS" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$RESULTS_FILE"
    echo ""
done

TOTAL_END=$(date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))")
TOTAL_MS=$(( (TOTAL_END - TOTAL_START) / 1000000 ))

echo "=============================================="
echo "  Summary"
echo "=============================================="
echo ""
echo "Total queries:    ${#QUERIES[@]}"
echo "Passed:           $PASS"
echo "Failed:           $FAIL"
echo "Total time:       ${TOTAL_MS}ms"
echo "Results saved to: $RESULTS_FILE"
echo ""

if [[ $FAIL -gt 0 ]]; then
    echo "WARNING: $FAIL queries failed. Run with --verbose to see errors."
    exit 1
fi
