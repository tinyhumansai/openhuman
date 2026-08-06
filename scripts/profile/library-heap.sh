#!/usr/bin/env bash
# library-heap.sh — builds the rss-bench-dhat variant of library-profile and
# runs one scenario under dhat heap profiling.
#
# Note: dhat instrumentation perturbs RSS/timing measurements. Use
# library-bench.sh / library-cpu.sh for RSS and CPU numbers; use this script
# only for live-heap attribution (allocation sites, retained bytes).
#
# This build REPLACES target/release/library-profile with the dhat variant.
# A later `library-bench.sh --skip-build` would pick it up; the bench script
# detects the dhat marker in the output JSON and refuses, but the clean fix
# is to rerun library-bench.sh without --skip-build afterwards.
#
# Usage:
#   ./scripts/profile/library-heap.sh <scenario> [-- <extra env VAR=value>...]
#
# Options:
#   --skip-build    Reuse the existing target/release/library-profile binary
#   -h, --help      Show this help
#
# Output: target/profile/rust-library/dhat-<scenario>.json
# View it at: https://nnethercote.github.io/dh_view/dh_view.html

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SKIP_BUILD=0
SCENARIO=""
EXTRA_ENV=()

usage() {
    sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
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

BIN="$REPO_ROOT/target/release/library-profile"
OUT_DIR="$REPO_ROOT/target/profile/rust-library"
mkdir -p "$OUT_DIR"
OUT_FILE="$OUT_DIR/dhat-${SCENARIO}.json"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    echo "[library-heap] building library-profile with rss-bench-dhat (GGML_NATIVE=OFF)" >&2
    (
        cd "$REPO_ROOT"
        GGML_NATIVE=OFF cargo build --release --features rss-bench-dhat --bin library-profile
    )
fi

if [[ ! -x "$BIN" ]]; then
    echo "ERROR: $BIN not found or not executable. Build it or drop --skip-build." >&2
    exit 1
fi

echo "[library-heap] running scenario '$SCENARIO' under dhat" >&2
echo "[library-heap] WARNING: dhat instrumentation perturbs RSS and timing; do not compare these numbers against library-bench.sh output" >&2

env OPENHUMAN_PROFILE_DHAT_OUT="$OUT_FILE" "${EXTRA_ENV[@]+"${EXTRA_ENV[@]}"}" "$BIN" "$SCENARIO" >/dev/null

if [[ ! -f "$OUT_FILE" ]]; then
    echo "ERROR: expected dhat output was not written: $OUT_FILE" >&2
    exit 1
fi

echo "[library-heap] done: $OUT_FILE" >&2
echo "[library-heap] view it at https://nnethercote.github.io/dh_view/dh_view.html (load the JSON file)" >&2
