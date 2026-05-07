#!/usr/bin/env bash
#
# memory-tree-progress.sh — live progress monitor for the memory_tree pipeline.
#
# Polls the workspace SQLite DB and prints a one-line snapshot every INTERVAL
# seconds: extract jobs done/pending, summaries per level, the currently
# claimed job (if any), recent throughput, and a rolling cloud-LLM round-trip
# estimate scraped from the core log. Exits cleanly when there is nothing left
# to do (no `ready`/`running` jobs other than the daily digest dedupe).
#
# Optionally triggers a fresh `flush_now` first so the seal cascade picks up
# whatever is currently buffered without waiting for the 50k-token threshold.
#
# Usage:
#   scripts/memory-tree-progress.sh                   # monitor only
#   scripts/memory-tree-progress.sh --flush           # flush_now then monitor
#   scripts/memory-tree-progress.sh --interval 10     # change tick (default 5s)
#   scripts/memory-tree-progress.sh --log /tmp/x.log  # override log path
#   scripts/memory-tree-progress.sh --once            # one snapshot, then exit
#
# Env:
#   OPENHUMAN_WORKSPACE  — workspace dir (default: derive from active_user.toml)
#   CORE_BIN             — path to openhuman-core (default: target/debug/openhuman-core)
#   CORE_LOG             — core log to scrape for round-trip times (default: /tmp/oh-core.log)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

INTERVAL=5
DO_FLUSH=0
ONCE=0
CORE_BIN="${CORE_BIN:-target/debug/openhuman-core}"
CORE_LOG="${CORE_LOG:-/tmp/oh-core.log}"

while [ $# -gt 0 ]; do
    case "$1" in
        --flush) DO_FLUSH=1; shift ;;
        --interval) INTERVAL="$2"; shift 2 ;;
        --log) CORE_LOG="$2"; shift 2 ;;
        --once) ONCE=1; shift ;;
        -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

# ── Resolve workspace + DB path ─────────────────────────────────────────────

if [ -z "${OPENHUMAN_WORKSPACE:-}" ]; then
    DEFAULT_DIR="$HOME/.openhuman-staging"
    [ -d "$DEFAULT_DIR" ] || DEFAULT_DIR="$HOME/.openhuman"
    ACTIVE_USER_FILE="$DEFAULT_DIR/active_user.toml"
    if [ -f "$ACTIVE_USER_FILE" ]; then
        USER_ID=$(awk -F'"' '/user_id/ {print $2; exit}' "$ACTIVE_USER_FILE")
        OPENHUMAN_WORKSPACE="$DEFAULT_DIR/users/$USER_ID/workspace"
    fi
fi
DB="${OPENHUMAN_WORKSPACE:-}/memory_tree/chunks.db"
if [ ! -f "$DB" ]; then
    echo "memory_tree DB not found at: $DB" >&2
    echo "Set OPENHUMAN_WORKSPACE to override." >&2
    exit 1
fi

echo "workspace: $OPENHUMAN_WORKSPACE"
echo "db:        $DB"
echo "log:       $CORE_LOG"
echo

# ── Optional initial flush ──────────────────────────────────────────────────

if [ "$DO_FLUSH" = 1 ]; then
    if [ ! -x "$CORE_BIN" ]; then
        echo "core binary not found: $CORE_BIN — build with 'cargo build --bin openhuman-core'" >&2
        exit 1
    fi
    echo "→ triggering memory_tree.flush_now"
    "$CORE_BIN" call --method openhuman.memory_tree_flush_now --params '{}' 2>&1 \
        | grep -E '"enqueued"|"stale_buffers"|memory_tree::read' || true
    echo
fi

# ── Snapshot helper ─────────────────────────────────────────────────────────

q() { sqlite3 "$DB" "$@"; }

# Track previous done counts so we can show throughput per tick.
PREV_EXTRACT_DONE=0
PREV_SUMMARIES=0
START_TS=$(date +%s)

snapshot() {
    local now ts ext_done ext_ready ext_run sums_l1 sums_l2 sums_l3 sums_l0 \
          chunks_pending chunks_admitted chunks_buffered \
          running_kind running_started_ms running_age_s \
          rt_recent_avg eta_min

    now=$(date +%s)
    ts=$(date '+%H:%M:%S')

    ext_done=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_jobs WHERE kind='extract_chunk' AND status='done';")
    ext_ready=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_jobs WHERE kind='extract_chunk' AND status='ready';")
    ext_run=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_jobs WHERE kind='extract_chunk' AND status='running';")

    sums_l0=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_summaries WHERE level=0;")
    sums_l1=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_summaries WHERE level=1;")
    sums_l2=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_summaries WHERE level=2;")
    sums_l3=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_summaries WHERE level>=3;")

    chunks_pending=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_chunks WHERE lifecycle_status='pending_extraction';")
    chunks_admitted=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_chunks WHERE lifecycle_status='admitted';")
    chunks_buffered=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_chunks WHERE lifecycle_status='buffered';")

    # Currently-claimed work (any kind), with age in seconds.
    local now_ms; now_ms=$((now * 1000))
    running_row=$(q "SELECT kind, COALESCE(started_at_ms,0) FROM mem_tree_jobs WHERE status='running' ORDER BY started_at_ms ASC LIMIT 1;")
    if [ -n "$running_row" ]; then
        running_kind=$(echo "$running_row" | cut -d'|' -f1)
        running_started_ms=$(echo "$running_row" | cut -d'|' -f2)
        if [ "$running_started_ms" -gt 0 ] 2>/dev/null; then
            running_age_s=$(( (now_ms - running_started_ms) / 1000 ))
        else
            running_age_s="?"
        fi
    else
        running_kind="-"
        running_age_s="-"
    fi

    # Throughput since last tick.
    local d_extract=$((ext_done - PREV_EXTRACT_DONE))
    local d_sums=$(( (sums_l0 + sums_l1 + sums_l2 + sums_l3) - PREV_SUMMARIES ))
    PREV_EXTRACT_DONE=$ext_done
    PREV_SUMMARIES=$((sums_l0 + sums_l1 + sums_l2 + sums_l3))

    # Rolling round-trip estimate from the last few cloud responses.
    rt_recent_avg="?"
    if [ -f "$CORE_LOG" ]; then
        rt_recent_avg=$(awk '
            /\[memory_tree::chat::cloud\] kind=/ {
                split($1, a, ":"); start = a[1]*3600 + a[2]*60 + a[3]
            }
            /\[memory_tree::chat::cloud\] response/ {
                split($1, a, ":"); end = a[1]*3600 + a[2]*60 + a[3]
                if (start > 0) { sum += (end - start); n++ }
            }
            END { if (n>0) printf "%.1fs", sum/n; else printf "?" }
        ' "$CORE_LOG" | tail -c 16)
    fi

    eta_min="?"
    if [ "$d_extract" -gt 0 ] 2>/dev/null; then
        # ETA based on jobs/tick * INTERVAL seconds.
        local secs_per=$(( INTERVAL / d_extract ))
        [ "$secs_per" -lt 1 ] && secs_per=1
        eta_min=$(( ext_ready * secs_per / 60 ))m
    fi

    # NOTE: source-tree leaves are L1+ (raw chunks are the L0 leaves of the
    # tree but aren't represented in `mem_tree_summaries`); the L0 row in
    # `mem_tree_summaries` is only populated by global-tree daily digests.
    # We surface it here as `digest=` so the bucket name doesn't mislead.
    printf "%s  extract: done=%d pending=%d run=%d (+%d/tick eta~%s)  summaries L1=%d L2=%d L3+=%d digest=%d (+%d)  chunks: pend=%d adm=%d buf=%d  running=%s/%ss  cloud_avg=%s\n" \
        "$ts" "$ext_done" "$ext_ready" "$ext_run" "$d_extract" "$eta_min" \
        "$sums_l1" "$sums_l2" "$sums_l3" "$sums_l0" "$d_sums" \
        "$chunks_pending" "$chunks_admitted" "$chunks_buffered" \
        "$running_kind" "$running_age_s" "$rt_recent_avg"

    # Done-condition: nothing pending or running across all kinds (digest_daily
    # rows are dedupe-suppressed steady state, ignore them).
    local active_other
    active_other=$(q "SELECT COALESCE(COUNT(*),0) FROM mem_tree_jobs \
                      WHERE status IN ('ready','running') \
                      AND kind <> 'digest_daily';")
    [ "$active_other" = "0" ]
}

# ── Loop ────────────────────────────────────────────────────────────────────

if [ "$ONCE" = 1 ]; then
    snapshot || true
    exit 0
fi

trap 'echo; echo "interrupted."; exit 0' INT

while true; do
    if snapshot; then
        echo "→ pipeline idle (no ready/running jobs). exiting."
        exit 0
    fi
    sleep "$INTERVAL"
done
