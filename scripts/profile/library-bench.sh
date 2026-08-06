#!/usr/bin/env bash
# library-bench.sh — reproducible RSS/duration benchmark for the OpenHuman
# core as an embedded library, using the `library-profile` binary.
#
# Runs each scenario N times as a FRESH process (no state shared across
# repeats), captures each run's JSON, and aggregates medians/min/max into a
# markdown + JSON summary. Companion scripts: library-cpu.sh (samply),
# library-heap.sh (dhat).
#
# Usage:
#   ./scripts/profile/library-bench.sh [options]
#
# Options:
#   --slim                Build with --no-default-features (slim library recipe)
#   --repeat N             Fresh-process repeats per scenario (default: 5)
#   --scenarios "a,b,c"    Comma-separated scenario list (default: all seven)
#   --turns N               OPENHUMAN_PROFILE_TURNS for long-agent (default binary default: 25)
#   --skip-build           Reuse the existing target/release binaries
#   --warm                 Also run PREWARM_SUBAGENTS=1 variants for subagents + subconscious
#   --out DIR              Output directory (default: target/profile/rust-library/bench-<timestamp>)
#   -h, --help             Show this help
#
# Examples:
#   ./scripts/profile/library-bench.sh
#   ./scripts/profile/library-bench.sh --slim --repeat 7
#   ./scripts/profile/library-bench.sh --scenarios "long-agent,subagents" --turns 50 --warm

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

ALL_SCENARIOS="memory-ingest,subagents,agent-turn,long-agent,workflow,subconscious,cold-phases"
WARM_ELIGIBLE=("subagents" "subconscious")

SLIM=0
REPEAT=5
SCENARIOS="$ALL_SCENARIOS"
TURNS=""
SKIP_BUILD=0
WARM=0
OUT_DIR=""

usage() {
    sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --slim) SLIM=1; shift ;;
        --repeat)
            REPEAT="${2:?--repeat requires a value}"; shift 2 ;;
        --scenarios)
            SCENARIOS="${2:?--scenarios requires a value}"; shift 2 ;;
        --turns)
            TURNS="${2:?--turns requires a value}"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --warm) WARM=1; shift ;;
        --out)
            OUT_DIR="${2:?--out requires a value}"; shift 2 ;;
        -h|--help) usage 0 ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            usage 1 ;;
    esac
done

if ! command -v jq >/dev/null 2>&1; then
    echo "ERROR: jq is required (aggregation over run JSON). Install it (e.g. 'brew install jq')." >&2
    exit 1
fi

if [[ -z "$OUT_DIR" ]]; then
    OUT_DIR="$REPO_ROOT/target/profile/rust-library/bench-$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$OUT_DIR"

BIN="$REPO_ROOT/target/release/library-profile"

log() { echo "[library-bench] $*" >&2; }

build_binaries() {
    if [[ "$SKIP_BUILD" -eq 1 ]]; then
        log "skipping build (--skip-build)"
        return
    fi
    local feature_args=(--features rss-bench)
    if [[ "$SLIM" -eq 1 ]]; then
        feature_args=(--no-default-features --features rss-bench)
        log "building slim library-profile + rss-bench (GGML_NATIVE=OFF)"
    else
        log "building default-feature library-profile + rss-bench (GGML_NATIVE=OFF)"
    fi
    (
        cd "$REPO_ROOT"
        GGML_NATIVE=OFF cargo build --release \
            "${feature_args[@]}" \
            --bin library-profile --bin rss-bench
    )
}

run_scenario() {
    local scenario="$1"
    local variant="$2" # "" or "warm"
    local label="$scenario"
    [[ -n "$variant" ]] && label="${scenario}-${variant}"

    local scenario_dir="$OUT_DIR/$label"
    mkdir -p "$scenario_dir"

    log "running scenario '$label' x$REPEAT (fresh process each run)"

    local i
    for ((i = 1; i <= REPEAT; i++)); do
        local run_file="$scenario_dir/run-$i.json"
        local env_args=()
        if [[ "$scenario" == "long-agent" && -n "$TURNS" ]]; then
            env_args+=(env "OPENHUMAN_PROFILE_TURNS=$TURNS")
        fi
        if [[ "$variant" == "warm" ]]; then
            env_args+=(env "OPENHUMAN_PROFILE_PREWARM_SUBAGENTS=1")
        fi

        if [[ ${#env_args[@]} -gt 0 ]]; then
            "${env_args[@]}" "$BIN" "$scenario" >"$run_file"
        else
            "$BIN" "$scenario" >"$run_file"
        fi

        if ! jq empty "$run_file" >/dev/null 2>&1; then
            echo "ERROR: run $i for scenario '$label' did not produce valid JSON: $run_file" >&2
            exit 1
        fi

        # library-heap.sh clobbers target/release/library-profile with the
        # rss-bench-dhat build, whose allocator perturbs RSS/timing. The dhat
        # binary marks its output, so refuse to benchmark it.
        if [[ "$(jq -r '.dhat // false' "$run_file")" == "true" ]]; then
            echo "ERROR: $BIN was built with rss-bench-dhat (library-heap.sh clobbered it)." >&2
            echo "       Re-run without --skip-build to rebuild the plain rss-bench binary." >&2
            exit 1
        fi
    done
}

# Emit the median (lower-middle for even N), min, and max of a jq numeric
# field across the run files for one scenario, as a JSON object on stdout.
aggregate_field() {
    local scenario_dir="$1"
    local jq_path="$2"
    jq -s "
        [ .[] | $jq_path | select(. != null) ] as \$vals |
        (\$vals | sort) as \$sorted |
        (\$sorted | length) as \$n |
        {
            median: (if \$n == 0 then null else \$sorted[(( \$n - 1) / 2 | floor)] end),
            min: (\$sorted[0] // null),
            max: (\$sorted[-1] // null),
            n: \$n
        }
    " "$scenario_dir"/run-*.json
}

aggregate_scenario() {
    local label="$1"
    local scenario_dir="$OUT_DIR/$label"

    local duration settled_rss retained_delta peak_delta
    duration=$(aggregate_field "$scenario_dir" ".duration_ms")
    settled_rss=$(aggregate_field "$scenario_dir" ".settled.rss_kib")
    retained_delta=$(aggregate_field "$scenario_dir" ".retained_delta_kib")
    peak_delta=$(aggregate_field "$scenario_dir" ".peak_delta_kib")

    local turn_growth="null"
    if [[ "$label" == long-agent* ]]; then
        # first-turn vs last-turn checkpoint rss delta (steady-state growth).
        turn_growth=$(jq -s '
            [ .[] |
              (.checkpoints // []) as $cps |
              if ($cps | length) >= 2 then
                ($cps[-1].rss_kib - $cps[0].rss_kib)
              else empty end
            ] as $vals |
            ($vals | sort) as $sorted |
            ($sorted | length) as $n |
            if $n == 0 then null
            else $sorted[((($n - 1) / 2) | floor)]
            end
        ' "$scenario_dir"/run-*.json)
    fi

    jq -n \
        --arg scenario "$label" \
        --argjson duration "$duration" \
        --argjson settled_rss "$settled_rss" \
        --argjson retained_delta "$retained_delta" \
        --argjson peak_delta "$peak_delta" \
        --argjson turn_growth_kib "$turn_growth" \
        '{
            scenario: $scenario,
            duration_ms: $duration,
            settled_rss_kib: $settled_rss,
            retained_delta_kib: $retained_delta,
            peak_delta_kib: $peak_delta,
            long_agent_turn_growth_kib: $turn_growth_kib
        }'
}

kib_to_mib() {
    # $1: kib value (may be null). Prints "n/a" for null.
    local kib="$1"
    if [[ "$kib" == "null" || -z "$kib" ]]; then
        echo "n/a"
        return
    fi
    jq -n --argjson kib "$kib" '($kib / 1024 * 100 | round) / 100'
}

write_summary() {
    local summary_json="$OUT_DIR/summary.json"
    local summary_md="$OUT_DIR/summary.md"

    jq -s '{ generated_at: (now | todate), build: ($ENV.LIBRARY_BENCH_BUILD // "default"), repeat: ($ENV.LIBRARY_BENCH_REPEAT | tonumber), scenarios: . }' \
        "$OUT_DIR"/*.scenario.json >"$summary_json"

    {
        echo "# Library benchmark summary"
        echo
        echo "Build: \`${LIBRARY_BENCH_BUILD}\`  "
        echo "Repeats per scenario: ${LIBRARY_BENCH_REPEAT}  "
        echo "Generated: $(date)"
        echo
        echo "| Scenario | Median settled RSS (MiB) | Median retained Δ (MiB) | Median peak Δ (MiB) | Median duration (ms) |"
        echo "| --- | ---: | ---: | ---: | ---: |"
        local f
        for f in "$OUT_DIR"/*.scenario.json; do
            local scenario settled retained peak duration
            scenario=$(jq -r '.scenario' "$f")
            settled=$(kib_to_mib "$(jq -r '.settled_rss_kib.median' "$f")")
            retained=$(kib_to_mib "$(jq -r '.retained_delta_kib.median' "$f")")
            peak=$(kib_to_mib "$(jq -r '.peak_delta_kib.median' "$f")")
            duration=$(jq -r '.duration_ms.median // "n/a"' "$f")
            echo "| $scenario | $settled | $retained | $peak | $duration |"
        done
        echo
        local long_growth_file
        for long_growth_file in "$OUT_DIR"/long-agent*.scenario.json; do
            [[ -e "$long_growth_file" ]] || continue
            local growth
            growth=$(jq -r '.long_agent_turn_growth_kib // "n/a"' "$long_growth_file")
            if [[ "$growth" != "n/a" && "$growth" != "null" ]]; then
                local growth_mib
                growth_mib=$(kib_to_mib "$growth")
                echo "**$(jq -r '.scenario' "$long_growth_file") steady-state growth** (first-turn to last-turn checkpoint delta, median across runs): ${growth_mib} MiB."
                echo
            fi
        done
        cat <<'EOF'
## External comparison point (not apples-to-apples)

ZeroClaw self-reports (unverified) idling under 5 MiB RAM and roughly 8-12 MiB
under load. OpenHuman's Rust core currently settles around 35-50 MiB depending
on scenario and feature set (see docs/library-benchmarking.md and
docs/resource-profiling-session-2026-07-21.md for scope/caveats). Treat this as
a north star, not a like-for-like comparison: ZeroClaw's feature surface and
scope differ substantially from the OpenHuman core.
EOF
    } >"$summary_md"
}

main() {
    build_binaries

    if [[ ! -x "$BIN" ]]; then
        echo "ERROR: $BIN not found or not executable. Build it or drop --skip-build." >&2
        exit 1
    fi

    IFS=',' read -r -a scenario_list <<<"$SCENARIOS"

    for scenario in "${scenario_list[@]}"; do
        run_scenario "$scenario" ""
        if [[ "$WARM" -eq 1 ]]; then
            for eligible in "${WARM_ELIGIBLE[@]}"; do
                if [[ "$scenario" == "$eligible" ]]; then
                    run_scenario "$scenario" "warm"
                fi
            done
        fi
    done

    # Build one *.scenario.json per label for the summary step.
    for scenario_dir in "$OUT_DIR"/*/; do
        [[ -d "$scenario_dir" ]] || continue
        local_label="$(basename "$scenario_dir")"
        aggregate_scenario "$local_label" >"$OUT_DIR/${local_label}.scenario.json"
    done

    export LIBRARY_BENCH_BUILD
    LIBRARY_BENCH_BUILD=$([[ "$SLIM" -eq 1 ]] && echo "slim" || echo "default")
    export LIBRARY_BENCH_REPEAT="$REPEAT"

    write_summary

    log "results: $OUT_DIR"
    cat "$OUT_DIR/summary.md"
}

main "$@"
