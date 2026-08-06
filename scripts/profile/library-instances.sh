#!/usr/bin/env bash
# library-instances.sh — multi-instance fuzz driver: spawn N independent
# `library-profile` processes (one live instance each, held alive at settled
# state via OPENHUMAN_PROFILE_HOLD_SECS) and measure per-INSTANCE cost and
# box survivability under the "N processes" deployment model, as opposed to
# the "N agents in one process" model that `library-fleet.sh` measures.
#
# Where library-fleet.sh answers "how many agents fit in one process", this
# answers "how many independent processes/containers fit on one box" — the
# opencompany per-tenant-instance question. Companion scripts: library-bench.sh
# (per-scenario RSS/duration), library-fleet.sh (one-process fleet sweep).
#
# Usage:
#   ./scripts/profile/library-instances.sh [options]
#
# Options:
#   --instances "10,25,50"  Comma-separated instance-count sweep (default: "10,25,50")
#   --hold-secs N            Seconds each instance holds alive at settled state
#                            after finishing its workload (default: 30)
#   --scenario NAME          library-profile scenario to run per instance
#                            (default: agent-turn)
#   --stagger-ms N           Delay between spawning consecutive instances, in
#                            milliseconds (default: 100)
#   --skip-build             Reuse the existing target/release binary
#   --slim                   Build with --no-default-features (slim library recipe)
#   --max-instances N        Hard safety cap on any swept N (default: 200).
#                            Each held instance settles ~47 MiB RSS on a
#                            default build, so sum-RSS scales roughly
#                            linearly: 1000 instances (at default settings)
#                            is on the order of 1000 x 47 MiB ~= 47 GB
#                            aggregate RSS. Raising this cap is an explicit
#                            "yes, I mean to spawn that many processes" — it
#                            will not happen implicitly.
#   --out DIR                Output directory (default:
#                            target/profile/rust-library/instances-<timestamp>)
#   --gate                   Exit nonzero if any instance failed (nonzero exit
#                            or missing/invalid JSON). Default: report only,
#                            always exit 0 unless argument/build errors occur.
#   -h, --help                Show this help
#
# Examples:
#   ./scripts/profile/library-instances.sh
#   ./scripts/profile/library-instances.sh --instances "10,50" --hold-secs 30
#   ./scripts/profile/library-instances.sh --instances "500" --max-instances 500 --gate

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

INSTANCES="10,25,50"
HOLD_SECS=30
SCENARIO="agent-turn"
STAGGER_MS=100
SKIP_BUILD=0
SLIM=0
MAX_INSTANCES=200
OUT_DIR=""
GATE=0

# Default-build settled RSS per held instance, used only for the safety-cap
# explanation message and the summary.md extrapolation fallback when PSS is
# unavailable (macOS). This is a rule-of-thumb constant, not a measurement —
# the script's own measured settled RSS supersedes it once real data exists.
ASSUMED_PER_INSTANCE_RSS_MIB=47
BUDGET_MIB=2048

usage() {
    sed -n '2,38p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --instances)
            INSTANCES="${2:?--instances requires a value}"; shift 2 ;;
        --hold-secs)
            HOLD_SECS="${2:?--hold-secs requires a value}"; shift 2 ;;
        --scenario)
            SCENARIO="${2:?--scenario requires a value}"; shift 2 ;;
        --stagger-ms)
            STAGGER_MS="${2:?--stagger-ms requires a value}"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --slim) SLIM=1; shift ;;
        --max-instances)
            MAX_INSTANCES="${2:?--max-instances requires a value}"; shift 2 ;;
        --out)
            OUT_DIR="${2:?--out requires a value}"; shift 2 ;;
        --gate) GATE=1; shift ;;
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
    OUT_DIR="$REPO_ROOT/target/profile/rust-library/instances-$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$OUT_DIR"

BIN="$REPO_ROOT/target/release/library-profile"

log() { echo "[library-instances] $*" >&2; }

# --- safety cap -------------------------------------------------------------

check_cap() {
    local n="$1"
    if [[ "$n" -gt "$MAX_INSTANCES" ]]; then
        local naive_gib
        naive_gib=$(jq -n --argjson n "$n" --argjson mib "$ASSUMED_PER_INSTANCE_RSS_MIB" '(($n * $mib) / 1024 * 100 | round) / 100')
        echo "ERROR: requested $n instances exceeds --max-instances cap ($MAX_INSTANCES)." >&2
        echo "       RAM math: each held instance settles ~${ASSUMED_PER_INSTANCE_RSS_MIB} MiB RSS on a" >&2
        echo "       default build (many-processes model pays that base N times, unlike the" >&2
        echo "       one-process fleet model, which amortizes it once). $n instances is roughly" >&2
        echo "       $n x ${ASSUMED_PER_INSTANCE_RSS_MIB} MiB ~= ${naive_gib} GiB of aggregate sum-RSS — likely more" >&2
        echo "       than this machine has. Pass --max-instances $n (or higher) to confirm you" >&2
        echo "       intend to spawn that many processes." >&2
        exit 1
    fi
}

# --- build -------------------------------------------------------------------

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

# One quick, short-lived run before the sweep: confirms the binary emits
# valid schema JSON and is not the dhat-instrumented build (library-heap.sh
# clobbers target/release/library-profile with rss-bench-dhat, whose
# allocator perturbs RSS in a way that would corrupt every number below).
probe_binary() {
    log "probe run: validating '$BIN $SCENARIO' before the sweep"
    local probe_file="$OUT_DIR/probe.json"
    local probe_log="$OUT_DIR/probe.log"

    OPENHUMAN_PROFILE_HOLD_SECS=0 "$BIN" "$SCENARIO" >"$probe_file" 2>"$probe_log"

    if ! jq empty "$probe_file" >/dev/null 2>&1; then
        echo "ERROR: probe run did not produce valid JSON: $probe_file" >&2
        exit 1
    fi
    if [[ "$(jq -r '.dhat // false' "$probe_file")" == "true" ]]; then
        echo "ERROR: $BIN was built with rss-bench-dhat (library-heap.sh clobbered it)." >&2
        echo "       Re-run without --skip-build to rebuild the plain rss-bench binary." >&2
        exit 1
    fi
    log "probe OK: settled rss_kib=$(jq -r '.settled.rss_kib' "$probe_file")"
}

# --- spawn + sample ----------------------------------------------------------

sleep_ms() {
    local ms="$1"
    [[ "$ms" -le 0 ]] && return 0
    local secs
    secs=$(awk -v ms="$ms" 'BEGIN { printf "%.3f", ms / 1000 }')
    sleep "$secs"
}

# Best-effort per-process attribution via the macOS `footprint` tool, run
# against a handful of pids for one sweep point only (it is slow and
# invasive; ignore any failure — this is a bonus artifact, not load-bearing).
capture_footprint_sample() {
    local point_dir="$1"; shift
    local pids=("$@")

    if ! command -v footprint >/dev/null 2>&1; then
        log "footprint tool not present, skipping footprint sample"
        return 0
    fi

    local sampled=0
    local pid
    for pid in "${pids[@]}"; do
        [[ "$sampled" -ge 3 ]] && break
        footprint "$pid" >"$point_dir/footprint-pid$pid.txt" 2>&1 || true
        sampled=$((sampled + 1))
    done
    log "footprint: sampled $sampled pid(s) (best-effort, see footprint-pid*.txt)"
}

# Samples aggregate sum-RSS + live count every 2s via `ps -o pid=,rss=` until
# every pid in the list has exited, appending to samples.csv. Records the
# peak sum-RSS observed and takes one `vm_stat` system-memory snapshot at
# that peak.
sample_while_holding() {
    local point_dir="$1"; shift
    local -a pids=("$@")

    local samples_csv="$point_dir/samples.csv"
    echo "elapsed_s,sum_rss_kib,live_count" >"$samples_csv"

    local pid_csv
    pid_csv=$(IFS=,; echo "${pids[*]}")

    local start_ts
    start_ts=$(date +%s)
    local peak_sum=0

    while true; do
        local ps_out
        ps_out=$(ps -o pid=,rss= -p "$pid_csv" 2>/dev/null || true)

        local live_count sum_rss
        if [[ -z "$ps_out" ]]; then
            live_count=0
            sum_rss=0
        else
            read -r live_count sum_rss <<<"$(echo "$ps_out" | awk '{s += $2; c += 1} END { print c + 0, s + 0 }')"
        fi

        local elapsed=$(( $(date +%s) - start_ts ))
        echo "$elapsed,$sum_rss,$live_count" >>"$samples_csv"

        if [[ "$sum_rss" -gt "$peak_sum" ]]; then
            peak_sum="$sum_rss"
            vm_stat >"$point_dir/vm_stat-peak.txt" 2>/dev/null || true
        fi

        if [[ "$live_count" -eq 0 ]]; then
            break
        fi
        sleep 2
    done

    echo "$peak_sum" >"$point_dir/.peak_sum_rss_kib"
}

run_sweep_point() {
    local n="$1"
    local do_footprint="$2"
    local point_dir="$OUT_DIR/n$n"
    mkdir -p "$point_dir"

    log "spawning $n instance(s) of scenario '$SCENARIO' (hold=${HOLD_SECS}s, stagger=${STAGGER_MS}ms)"

    local -a pids=()
    local i
    for ((i = 1; i <= n; i++)); do
        local out_file="$point_dir/proc-$i.json"
        local log_file="$point_dir/proc-$i.log"

        env "OPENHUMAN_PROFILE_HOLD_SECS=$HOLD_SECS" "$BIN" "$SCENARIO" \
            >"$out_file" 2>"$log_file" &
        pids+=("$!")

        if [[ "$i" -lt "$n" ]]; then
            sleep_ms "$STAGGER_MS"
        fi
    done

    if [[ "$do_footprint" -eq 1 ]]; then
        capture_footprint_sample "$point_dir" "${pids[@]}"
    fi

    sample_while_holding "$point_dir" "${pids[@]}"

    local ok=0 failed=0
    local pid_index=0
    for pid in "${pids[@]}"; do
        pid_index=$((pid_index + 1))
        local out_file="$point_dir/proc-$pid_index.json"
        local status=0
        wait "$pid" || status=$?

        if [[ "$status" -eq 0 && -s "$out_file" ]] && jq empty "$out_file" >/dev/null 2>&1; then
            ok=$((ok + 1))
        else
            failed=$((failed + 1))
            log "WARN: instance $pid_index (pid $pid, n=$n) failed (exit=$status)"
        fi
    done

    echo "$ok" >"$point_dir/.ok_count"
    echo "$failed" >"$point_dir/.failed_count"
    log "n=$n done: ok=$ok failed=$failed"
}

# --- aggregate ----------------------------------------------------------------

kib_to_mib() {
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

aggregate_point() {
    local n="$1"
    local point_dir="$OUT_DIR/n$n"

    local ok failed peak_sum_rss_kib
    ok=$(cat "$point_dir/.ok_count" 2>/dev/null || echo 0)
    failed=$(cat "$point_dir/.failed_count" 2>/dev/null || echo 0)
    peak_sum_rss_kib=$(cat "$point_dir/.peak_sum_rss_kib" 2>/dev/null || echo 0)

    local -a valid_files=()
    local f
    for f in "$point_dir"/proc-*.json; do
        [[ -e "$f" ]] || continue
        if jq empty "$f" >/dev/null 2>&1; then
            valid_files+=("$f")
        fi
    done

    local settled_rss_median pss_max pss_sum_kib
    if [[ "${#valid_files[@]}" -gt 0 ]]; then
        settled_rss_median=$(jq -s '
            [ .[] | .settled.rss_kib | select(. != null) ] as $vals |
            ($vals | sort) as $sorted |
            ($sorted | length) as $n |
            (if $n == 0 then null else $sorted[(($n - 1) / 2 | floor)] end)
        ' "${valid_files[@]}")
        pss_max=$(jq -s '[ .[] | .settled.pss_kib // 0 ] | max' "${valid_files[@]}")
        pss_sum_kib=$(jq -s '[ .[] | .settled.pss_kib // 0 ] | add' "${valid_files[@]}")
    else
        settled_rss_median=null
        pss_max=0
        pss_sum_kib=0
    fi

    local mean_sum_rss_kib_per_instance
    if [[ "$ok" -gt 0 ]]; then
        mean_sum_rss_kib_per_instance=$(jq -n --argjson total "$peak_sum_rss_kib" --argjson ok "$ok" '$total / $ok')
    else
        mean_sum_rss_kib_per_instance=null
    fi

    local pss_available
    pss_available=$(jq -n --argjson v "$pss_max" 'if $v > 0 then true else false end')

    jq -n \
        --argjson instances "$n" \
        --argjson launched "$n" \
        --argjson ok "$ok" \
        --argjson failed "$failed" \
        --argjson peak_sum_rss_kib "$peak_sum_rss_kib" \
        --argjson mean_sum_rss_kib_per_instance "$mean_sum_rss_kib_per_instance" \
        --argjson settled_rss_kib_median "$settled_rss_median" \
        --argjson pss_available "$pss_available" \
        --argjson pss_sum_kib "$pss_sum_kib" \
        '{
            instances: $instances,
            launched: $launched,
            ok: $ok,
            failed: $failed,
            peak_sum_rss_kib: $peak_sum_rss_kib,
            mean_sum_rss_kib_per_instance: $mean_sum_rss_kib_per_instance,
            settled_rss_kib_median: $settled_rss_kib_median,
            pss_available: $pss_available,
            pss_sum_kib: (if $pss_available then $pss_sum_kib else null end)
        }'
}

write_summary() {
    local summary_json="$OUT_DIR/summary.json"
    local summary_md="$OUT_DIR/summary.md"

    jq -s \
        --argjson hold_secs "$HOLD_SECS" \
        --arg scenario "$SCENARIO" \
        --argjson stagger_ms "$STAGGER_MS" \
        --arg build "$([[ "$SLIM" -eq 1 ]] && echo "slim" || echo "default")" \
        '{
            generated_at: (now | todate),
            build: $build,
            config: { scenario: $scenario, hold_secs: $hold_secs, stagger_ms: $stagger_ms },
            sweep: .
        }' \
        "$OUT_DIR"/*.point.json >"$summary_json"

    local any_fail=0
    local last_point_file=""

    {
        echo "# Multi-instance (many-processes) benchmark summary"
        echo
        echo "Build: \`$([[ "$SLIM" -eq 1 ]] && echo "slim" || echo "default")\`  "
        echo "Scenario: \`${SCENARIO}\`, hold: ${HOLD_SECS}s, stagger: ${STAGGER_MS}ms  "
        echo "Generated: $(date)"
        echo
        echo "This measures the **many-processes** deployment model: N independent"
        echo "\`library-profile\` processes, each a live held instance, as opposed to"
        echo "\`library-fleet.sh\`'s **one-process** model (N agents inside a single"
        echo "process). **Correctness note:** summed RSS below double-counts shared"
        echo "clean pages — all instances share one binary's resident executable text"
        echo "and any shared library mappings — so sum-RSS is an **upper bound** on"
        echo "real physical memory use, not the true cost. On Linux, summed PSS"
        echo "(proportional set size, from each instance's \`settled.pss_kib\`) divides"
        echo "shared pages across the processes that share them and is the honest"
        echo "number; it is surfaced below whenever the field is nonzero. On macOS"
        echo "there is no PSS equivalent, so only sum-RSS is available and the table"
        echo "says so explicitly."
        echo
        echo "| N | ok/launched | median settled RSS/instance (MiB) | mean sum-RSS/instance (MiB) | peak aggregate sum-RSS (MiB) | summed PSS (MiB) |"
        echo "| ---: | :---: | ---: | ---: | ---: | ---: |"

        local pf
        for pf in "$OUT_DIR"/*.point.json; do
            [[ -e "$pf" ]] || continue
            last_point_file="$pf"
            local n ok launched settled_rss mean_sum peak_sum pss_available pss_sum pss_cell
            n=$(jq -r '.instances' "$pf")
            ok=$(jq -r '.ok' "$pf")
            launched=$(jq -r '.launched' "$pf")
            settled_rss=$(kib_to_mib "$(jq -r '.settled_rss_kib_median' "$pf")")
            mean_sum=$(kib_to_mib "$(jq -r '.mean_sum_rss_kib_per_instance' "$pf")")
            peak_sum=$(kib_to_mib "$(jq -r '.peak_sum_rss_kib' "$pf")")
            pss_available=$(jq -r '.pss_available' "$pf")
            if [[ "$pss_available" == "true" ]]; then
                pss_sum=$(kib_to_mib "$(jq -r '.pss_sum_kib' "$pf")")
                pss_cell="$pss_sum"
            else
                pss_cell="n/a (macOS)"
            fi

            if [[ "$ok" != "$launched" ]]; then
                any_fail=1
            fi

            echo "| $n | $ok/$launched | $settled_rss | $mean_sum | $peak_sum | $pss_cell |"
        done

        echo
        echo "## Verdict (2 GB box extrapolation, estimate)"
        echo

        if [[ -n "$last_point_file" ]]; then
            local pss_available per_instance_mib per_instance_label fits_estimate
            pss_available=$(jq -r '.pss_available' "$last_point_file")
            if [[ "$pss_available" == "true" ]]; then
                per_instance_mib=$(kib_to_mib "$(jq -r '(.pss_sum_kib / .ok)' "$last_point_file")")
                per_instance_label="summed PSS/instance (honest, shared pages divided across sharers)"
            else
                per_instance_mib=$(kib_to_mib "$(jq -r '.mean_sum_rss_kib_per_instance' "$last_point_file")")
                per_instance_label="mean sum-RSS/instance (macOS, no PSS — this OVERSTATES true cost by double-counting shared pages)"
            fi

            if [[ "$per_instance_mib" != "n/a" ]]; then
                fits_estimate=$(jq -n --argjson budget "$BUDGET_MIB" --argjson per "$per_instance_mib" '($budget / $per) | floor')
                echo "Using the largest swept N's ${per_instance_label}: **${per_instance_mib} MiB/instance**."
                echo
                echo "\`instances_that_fit ~= ${BUDGET_MIB} / ${per_instance_mib} ~= ${fits_estimate}\` — a rough"
                echo "**estimate**, not a measured limit. It assumes every additional instance"
                echo "costs the same as the ones already measured (no host-level contention,"
                echo "no cgroup memory limit enforced here)."
                if [[ "$pss_available" == "true" ]]; then
                    echo "PSS was available (Linux), so this divides shared pages across the"
                    echo "processes that share them rather than double-counting them — still an"
                    echo "estimate, but not skewed in a known direction the way the macOS"
                    echo "sum-RSS fallback is."
                else
                    echo "No PSS was available (macOS), so this uses sum-RSS as a stand-in,"
                    echo "which double-counts shared pages (executable text, shared library"
                    echo "mappings) across instances — treat this estimate as"
                    echo "conservative-in-the-wrong-direction (fewer instances would actually fit"
                    echo "than the sum-RSS math implies is safe, since real physical use is lower"
                    echo "than sum-RSS thanks to shared pages, but this is also unverified against"
                    echo "an actual memory-limited box)."
                fi
            else
                echo "No successful instances at the largest swept N — cannot extrapolate."
                any_fail=1
            fi
        else
            echo "No sweep points completed — nothing to extrapolate."
            any_fail=1
        fi
    } >"$summary_md"

    if [[ "$any_fail" -eq 1 ]]; then
        return 1
    fi
    return 0
}

main() {
    IFS=',' read -r -a instance_list <<<"$INSTANCES"

    local n
    for n in "${instance_list[@]}"; do
        check_cap "$n"
    done

    build_binaries

    if [[ ! -x "$BIN" ]]; then
        echo "ERROR: $BIN not found or not executable. Build it or drop --skip-build." >&2
        exit 1
    fi

    probe_binary

    local first_n="${instance_list[0]}"
    for n in "${instance_list[@]}"; do
        local do_footprint=0
        [[ "$n" == "$first_n" ]] && do_footprint=1
        run_sweep_point "$n" "$do_footprint"
    done

    for n in "${instance_list[@]}"; do
        aggregate_point "$n" >"$OUT_DIR/n$n.point.json"
    done

    local gate_status=0
    write_summary || gate_status=1

    log "results: $OUT_DIR"
    cat "$OUT_DIR/summary.md"

    if [[ "$GATE" -eq 1 && "$gate_status" -ne 0 ]]; then
        log "GATE FAILED: at least one instance failed to complete cleanly"
        exit 1
    fi
}

main "$@"
