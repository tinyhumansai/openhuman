#!/usr/bin/env bash
# library-cpu.sh — samply wrapper for a CPU profile of one library-profile
# scenario, following the recipe used in docs/resource-profiling-session-2026-07-21.md.
#
# Usage:
#   ./scripts/profile/library-cpu.sh <scenario> [-- <extra env VAR=value>...]
#   ./scripts/profile/library-cpu.sh --no-isolate <scenario>
#
# Options:
#   --no-isolate    Do not force OPENHUMAN_PROFILE_DISABLE_MEMORY_WRITES=1 /
#                   OPENHUMAN_PROFILE_FORCE_UTC=1 (default: isolated, matching
#                   the documented cold-path CPU recipe)
#   --skip-build    Reuse the existing target/release/library-profile binary
#   -h, --help      Show this help
#
# Extra environment variables can be passed after `--`, e.g.:
#   ./scripts/profile/library-cpu.sh long-agent -- OPENHUMAN_PROFILE_TURNS=50
#
# Output: target/profile/rust-library/<scenario>-cpu.json.gz
# View it with: samply load target/profile/rust-library/<scenario>-cpu.json.gz

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

ISOLATE=1
SKIP_BUILD=0
SCENARIO=""
EXTRA_ENV=()

usage() {
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-isolate) ISOLATE=0; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        -h|--help) usage 0 ;;
        --)
            shift
            EXTRA_ENV=("$@")
            break
            ;;
        *)
            if [[ -z "$SCENARIO" ]]; then
                SCENARIO="$1"
                shift
            else
                echo "ERROR: unexpected argument: $1" >&2
                usage 1
            fi
            ;;
    esac
done

if [[ -z "$SCENARIO" ]]; then
    echo "ERROR: scenario is required" >&2
    usage 1
fi

if ! command -v samply >/dev/null 2>&1; then
    echo "ERROR: samply is required. Install it with 'cargo install samply'." >&2
    exit 1
fi

BIN="$REPO_ROOT/target/release/library-profile"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    echo "[library-cpu] building library-profile (GGML_NATIVE=OFF)" >&2
    (
        cd "$REPO_ROOT"
        GGML_NATIVE=OFF cargo build --release --features rss-bench --bin library-profile
    )
fi

if [[ ! -x "$BIN" ]]; then
    echo "ERROR: $BIN not found or not executable. Build it or drop --skip-build." >&2
    exit 1
fi

OUT_DIR="$REPO_ROOT/target/profile/rust-library"
mkdir -p "$OUT_DIR"
OUT_FILE="$OUT_DIR/${SCENARIO}-cpu.json.gz"

ENV_ARGS=()
if [[ "$ISOLATE" -eq 1 ]]; then
    ENV_ARGS+=(OPENHUMAN_PROFILE_DISABLE_MEMORY_WRITES=1 OPENHUMAN_PROFILE_FORCE_UTC=1)
fi
ENV_ARGS+=("${EXTRA_ENV[@]+"${EXTRA_ENV[@]}"}")

echo "[library-cpu] recording scenario '$SCENARIO' -> $OUT_FILE" >&2
env "${ENV_ARGS[@]+"${ENV_ARGS[@]}"}" samply record \
    --save-only \
    --unstable-presymbolicate \
    --rate 1000 \
    --iteration-count 5 \
    --output "$OUT_FILE" \
    -- "$BIN" "$SCENARIO"

echo "[library-cpu] done: $OUT_FILE" >&2
echo "[library-cpu] view it with: samply load $OUT_FILE" >&2
