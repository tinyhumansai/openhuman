#!/usr/bin/env bash
# library-fleet.sh — sweep the `fleet` scenario of library-profile across
# agent counts and gate the result against the 2 GB RAM / 2 vCPU server
# budget (100-1000 live agents). Companion scripts: library-bench.sh
# (per-scenario RSS/duration), library-cpu.sh (samply), library-heap.sh
# (dhat).
#
# Usage:
#   ./scripts/profile/library-fleet.sh [options]
#
# Options:
#   --agents "50,100,500"  Comma-separated agent-count sweep (default: "50,100,500")
#   --turns N              OPENHUMAN_PROFILE_TURNS per agent (default: 3)
#   --latency-ms N         OPENHUMAN_PROFILE_MOCK_LATENCY_MS (default: 200)
#   --workers N            OPENHUMAN_PROFILE_WORKER_THREADS, simulates the
#                          2 vCPU box (default: 2)
#   --repeat N             Fresh-process repeats per agent count (default: 3)
#   --target N             OPENHUMAN_PROFILE_TARGET_AGENTS (default: 1000)
#   --budget-mib N         OPENHUMAN_PROFILE_RAM_BUDGET_MIB (default: 2048)
#   --skip-build           Reuse the existing target/release binaries
#   --slim                 Build with --no-default-features (slim library recipe)
#   --out DIR              Output directory (default: target/profile/rust-library/fleet-<timestamp>)
#   --no-gate              Do not fail the exit code on fits==false (report only)
#   -h, --help             Show this help
#
# Examples:
#   ./scripts/profile/library-fleet.sh
#   ./scripts/profile/library-fleet.sh --agents 100 --latency-ms 200
#   ./scripts/profile/library-fleet.sh --agents "100,1000" --target 1000 --budget-mib 2048

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

AGENTS="50,100,500"
TURNS=3
LATENCY_MS=200
WORKERS=2
REPEAT=3
TARGET=1000
BUDGET_MIB=2048
SKIP_BUILD=0
SLIM=0
OUT_DIR=""
GATE=1

# Idle CPU is measured over a 10s parked window and should be ~flat
# regardless of agent count (idle agents cost ~zero CPU). 500ms of CPU
# across that window (5% of one core) is the "low" threshold used in the
# PASS/FAIL verdict line; see docs/library-benchmarking.md.
IDLE_CPU_MS_MAX=500

usage() {
    sed -n '2,27p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --agents)
            AGENTS="${2:?--agents requires a value}"; shift 2 ;;
        --turns)
            TURNS="${2:?--turns requires a value}"; shift 2 ;;
        --latency-ms)
            LATENCY_MS="${2:?--latency-ms requires a value}"; shift 2 ;;
        --workers)
            WORKERS="${2:?--workers requires a value}"; shift 2 ;;
        --repeat)
            REPEAT="${2:?--repeat requires a value}"; shift 2 ;;
        --target)
            TARGET="${2:?--target requires a value}"; shift 2 ;;
        --budget-mib)
            BUDGET_MIB="${2:?--budget-mib requires a value}"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --slim) SLIM=1; shift ;;
        --out)
            OUT_DIR="${2:?--out requires a value}"; shift 2 ;;
        --no-gate) GATE=0; shift ;;
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
    OUT_DIR="$REPO_ROOT/target/profile/rust-library/fleet-$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$OUT_DIR"

BIN="$REPO_ROOT/target/release/library-profile"

log() { echo "[library-fleet] $*" >&2; }

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

run_sweep_point() {
    local n="$1"
    local point_dir="$OUT_DIR/fleet-$n"
    mkdir -p "$point_dir"

    log "running fleet scenario with $n agents x$REPEAT (fresh process each run)"

    local i
    for ((i = 1; i <= REPEAT; i++)); do
        local run_file="$point_dir/run-$i.json"

        env \
            "OPENHUMAN_PROFILE_AGENTS=$n" \
            "OPENHUMAN_PROFILE_TURNS=$TURNS" \
            "OPENHUMAN_PROFILE_MOCK_LATENCY_MS=$LATENCY_MS" \
            "OPENHUMAN_PROFILE_WORKER_THREADS=$WORKERS" \
            "OPENHUMAN_PROFILE_TARGET_AGENTS=$TARGET" \
            "OPENHUMAN_PROFILE_RAM_BUDGET_MIB=$BUDGET_MIB" \
            "$BIN" fleet >"$run_file"

        if ! jq empty "$run_file" >/dev/null 2>&1; then
            echo "ERROR: run $i for agents=$n did not produce valid JSON: $run_file" >&2
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
# field across the run files for one sweep point, as a JSON object on stdout.
aggregate_field() {
    local point_dir="$1"
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
    " "$point_dir"/run-*.json
}

# fits is a boolean per run; report it as "fits" only when every repeat
# agreed it fits (a single unlucky repeat should not hide a marginal case).
aggregate_fits() {
    local point_dir="$1"
    jq -s '
        [ .[] | .budget.fits // false ] as $vals |
        ($vals | length) as $n |
        ($vals | map(select(. == true)) | length) as $true_n |
        { all_fit: ($n > 0 and $true_n == $n), true_n: $true_n, n: $n }
    ' "$point_dir"/run-*.json
}

aggregate_point() {
    local n="$1"
    local point_dir="$OUT_DIR/fleet-$n"

    local marginal settled_rss idle_cpu threads open_fds p50 p95 p99 projected fits
    marginal=$(aggregate_field "$point_dir" ".marginal_rss_kib_per_agent")
    settled_rss=$(aggregate_field "$point_dir" ".settled.rss_kib")
    idle_cpu=$(aggregate_field "$point_dir" ".idle_cpu_ms")
    threads=$(aggregate_field "$point_dir" ".settled.threads")
    open_fds=$(aggregate_field "$point_dir" ".settled.open_fds")
    p50=$(aggregate_field "$point_dir" ".turn_latency_ms.p50")
    p95=$(aggregate_field "$point_dir" ".turn_latency_ms.p95")
    p99=$(aggregate_field "$point_dir" ".turn_latency_ms.p99")
    projected=$(aggregate_field "$point_dir" ".budget.projected_rss_mib_at_target")
    fits=$(aggregate_fits "$point_dir")

    jq -n \
        --argjson agents "$n" \
        --argjson marginal_rss_kib_per_agent "$marginal" \
        --argjson settled_rss_kib "$settled_rss" \
        --argjson idle_cpu_ms "$idle_cpu" \
        --argjson threads "$threads" \
        --argjson open_fds "$open_fds" \
        --argjson p50 "$p50" \
        --argjson p95 "$p95" \
        --argjson p99 "$p99" \
        --argjson projected_rss_mib_at_target "$projected" \
        --argjson fits "$fits" \
        '{
            agents: $agents,
            marginal_rss_kib_per_agent: $marginal_rss_kib_per_agent,
            settled_rss_kib: $settled_rss_kib,
            idle_cpu_ms: $idle_cpu_ms,
            threads: $threads,
            open_fds: $open_fds,
            turn_latency_ms: { p50: $p50, p95: $p95, p99: $p99 },
            projected_rss_mib_at_target: $projected_rss_mib_at_target,
            fits: $fits
        }'
}

kib_to_mib() {
    # $1: kib value (may be null/"null"). Prints "n/a" for null.
    local kib="$1"
    if [[ "$kib" == "null" || -z "$kib" ]]; then
        echo "n/a"
        return
    fi
    jq -n --argjson kib "$kib" '($kib / 1024 * 100 | round) / 100'
}

num_or_na() {
    local v="$1"
    [[ "$v" == "null" || -z "$v" ]] && echo "n/a" || echo "$v"
}

write_summary() {
    local summary_json="$OUT_DIR/summary.json"
    local summary_md="$OUT_DIR/summary.md"

    jq -s \
        --argjson turns "$TURNS" \
        --argjson latency_ms "$LATENCY_MS" \
        --argjson workers "$WORKERS" \
        --argjson repeat "$REPEAT" \
        --argjson target "$TARGET" \
        --argjson budget_mib "$BUDGET_MIB" \
        --arg build "$([[ "$SLIM" -eq 1 ]] && echo "slim" || echo "default")" \
        '{
            generated_at: (now | todate),
            build: $build,
            config: {
                turns: $turns,
                mock_latency_ms: $latency_ms,
                worker_threads: $workers,
                repeat: $repeat,
                target_agents: $target,
                ram_budget_mib: $budget_mib
            },
            sweep: .
        }' \
        "$OUT_DIR"/*.point.json >"$summary_json"

    local any_fail=0

    {
        echo "# Fleet benchmark summary"
        echo
        echo "Build: \`$([[ "$SLIM" -eq 1 ]] && echo "slim" || echo "default")\`  "
        echo "Repeats per agent count: ${REPEAT}  "
        echo "Turns/agent: ${TURNS}, mock latency: ${LATENCY_MS}ms, worker threads: ${WORKERS}  "
        echo "Target: ${TARGET} agents, budget: ${BUDGET_MIB} MiB  "
        echo "Generated: $(date)"
        echo
        echo "| N | marginal KiB/agent | settled MiB | idle CPU ms/10s | threads | fds | p95 ms | projected MiB @ target | fits |"
        echo "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | :---: |"

        local f
        for f in "$OUT_DIR"/*.point.json; do
            local n marginal settled idle threads fds p95 projected fits_all
            n=$(jq -r '.agents' "$f")
            marginal=$(num_or_na "$(jq -r '.marginal_rss_kib_per_agent.median' "$f")")
            settled=$(kib_to_mib "$(jq -r '.settled_rss_kib.median' "$f")")
            idle=$(num_or_na "$(jq -r '.idle_cpu_ms.median' "$f")")
            threads=$(num_or_na "$(jq -r '.threads.median' "$f")")
            fds=$(num_or_na "$(jq -r '.open_fds.median' "$f")")
            p95=$(num_or_na "$(jq -r '.turn_latency_ms.p95.median' "$f")")
            projected=$(num_or_na "$(jq -r '.projected_rss_mib_at_target.median' "$f")")
            fits_all=$(jq -r '.fits.all_fit' "$f")
            echo "| $n | $marginal | $settled | $idle | $threads | $fds | $p95 | $projected | $fits_all |"
        done

        echo
        echo "## Verdict"
        echo

        for f in "$OUT_DIR"/*.point.json; do
            local n fits_all idle_median verdict reasons
            n=$(jq -r '.agents' "$f")
            fits_all=$(jq -r '.fits.all_fit' "$f")
            idle_median=$(jq -r '.idle_cpu_ms.median // "null"' "$f")

            reasons=""
            verdict="PASS"
            if [[ "$fits_all" != "true" ]]; then
                verdict="FAIL"
                reasons="${reasons}projected RSS at target does not fit ${BUDGET_MIB} MiB budget; "
                any_fail=1
            fi
            local idle_over
            idle_over=0
            if [[ "$idle_median" != "null" ]]; then
                idle_over=$(jq -n --argjson v "$idle_median" --argjson max "$IDLE_CPU_MS_MAX" 'if $v > $max then 1 else 0 end')
            fi
            if [[ "$idle_over" -eq 1 ]]; then
                verdict="FAIL"
                reasons="${reasons}idle CPU ${idle_median}ms/10s exceeds ${IDLE_CPU_MS_MAX}ms threshold; "
            fi

            if [[ "$verdict" == "PASS" ]]; then
                echo "- **N=$n: PASS** — fits budget, idle CPU low."
            else
                echo "- **N=$n: FAIL** — ${reasons}"
            fi
        done
    } >"$summary_md"

    if [[ "$any_fail" -eq 1 ]]; then
        return 1
    fi
    return 0
}

main() {
    build_binaries

    if [[ ! -x "$BIN" ]]; then
        echo "ERROR: $BIN not found or not executable. Build it or drop --skip-build." >&2
        exit 1
    fi

    IFS=',' read -r -a agent_list <<<"$AGENTS"

    local n
    for n in "${agent_list[@]}"; do
        run_sweep_point "$n"
    done

    for n in "${agent_list[@]}"; do
        aggregate_point "$n" >"$OUT_DIR/fleet-$n.point.json"
    done

    local gate_status=0
    write_summary || gate_status=1

    log "results: $OUT_DIR"
    cat "$OUT_DIR/summary.md"

    if [[ "$GATE" -eq 1 && "$gate_status" -ne 0 ]]; then
        log "GATE FAILED: at least one swept N does not fit the ${BUDGET_MIB} MiB / ${TARGET}-agent budget"
        exit 1
    fi
}

main "$@"
