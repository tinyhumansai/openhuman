#!/usr/bin/env bash
#
# Agent-scale benchmark: drive a real openhuman-core process at concurrency
# against a mocked LLM, sample its CPU/RSS, and report a leak verdict.
#
# This is the OUT-OF-PROCESS tier. It complements scripts/profile/, which
# embeds the core as a library and measures the current process. Here the core
# is a normally-built server binary reached over /rpc, so the numbers include
# the transport, serde and scheduler costs a library benchmark cannot see, and
# RSS is what the OS actually charges the shipped binary.
#
# It needs NO cargo test features and NO core code changes. Two facts make that
# work, and both are load-bearing:
#
#   * BACKEND_URL redirects managed inference. The core derives its inference
#     base and its backend base from the same value, so pointing it at the mock
#     captures chat completions, embeddings and telemetry in one move.
#   * A session token shaped `<a>.<b>.local` is stored WITHOUT the GET /auth/me
#     round-trip a remote JWT triggers, so no login and no auth mock is needed.
#     The driver seeds it before the load starts.
#
# Usage:
#   scripts/bench/run-agent-scale.sh [options]
#
#   --concurrency N     parallel in-flight turns (default 8)
#   --turns N           total turns in the measured window (default 300)
#   --duration-ms N     run for a wall-clock duration instead of a turn count
#   --warmup-turns N    turns to run and discard before measuring (default 10)
#   --tool-depth N      tool calls the mock drives per turn (default 1)
#   --latency-ms N      mock inference latency (default 40)
#   --jitter-ms N       jitter around that latency (default 20)
#   --reply-chars N     assistant reply size (default 240)
#   --fail-rate F       fraction of completions answered 500 (default 0)
#   --thread-mode M     fresh | per-worker | shared (default fresh)
#   --interval-ms N     resource sampling interval (default 250)
#   --tree              also sample descendant processes
#   --keep-workspace    do not delete the temp workspace on exit
#   --workspace DIR     reuse an existing populated workspace (implies --keep-workspace)
#   --memory-off        disable memory reads and writes (recall + learning)
#   --memory-writes-off disable only memory writes, keep recall reads (mutually
#                       exclusive with --memory-off)
#   --out-dir DIR       where to write artifacts (default target/bench/<stamp>)
#
# Exit status is the analyzer's: non-zero when a leak or drift check fails.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

CONCURRENCY=8
TURNS=300
DURATION_MS=""
WARMUP_TURNS=10
TOOL_DEPTH=1
LATENCY_MS=40
JITTER_MS=20
REPLY_CHARS=240
FAIL_RATE=0
THREAD_MODE=fresh
INTERVAL_MS=250
TREE=""
KEEP_WORKSPACE=""
REUSE_WORKSPACE=""
MEMORY_OFF=""
MEMORY_WRITES_OFF=""
OUT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --concurrency) CONCURRENCY="$2"; shift 2 ;;
    --turns) TURNS="$2"; shift 2 ;;
    --duration-ms) DURATION_MS="$2"; shift 2 ;;
    --warmup-turns) WARMUP_TURNS="$2"; shift 2 ;;
    --tool-depth) TOOL_DEPTH="$2"; shift 2 ;;
    --latency-ms) LATENCY_MS="$2"; shift 2 ;;
    --jitter-ms) JITTER_MS="$2"; shift 2 ;;
    --reply-chars) REPLY_CHARS="$2"; shift 2 ;;
    --fail-rate) FAIL_RATE="$2"; shift 2 ;;
    --thread-mode) THREAD_MODE="$2"; shift 2 ;;
    --interval-ms) INTERVAL_MS="$2"; shift 2 ;;
    --tree) TREE="--tree"; shift ;;
    --memory-off) MEMORY_OFF=1; shift ;;
    --memory-writes-off) MEMORY_WRITES_OFF=1; shift ;;
    --keep-workspace) KEEP_WORKSPACE=1; shift ;;
    --workspace) REUSE_WORKSPACE="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    -h|--help) sed -n '2,/^# Exit status/p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -n "$MEMORY_OFF" && -n "$MEMORY_WRITES_OFF" ]]; then
  echo "error: --memory-off and --memory-writes-off are mutually exclusive:" >&2
  echo "  --memory-off already disables writes, so combining them would emit a" >&2
  echo "  config with duplicate [memory] and [learning] tables, which the core" >&2
  echo "  rejects. Run with just --memory-off (both off) or just --memory-writes-off." >&2
  exit 2
fi

STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${OUT_DIR:-$REPO_ROOT/target/bench/$STAMP}"
mkdir -p "$OUT_DIR"

# Ports chosen to avoid both the core's default 7788 (so a dev core can keep
# running) and the LOCAL_AI_PORTS set the core routes around.
MOCK_PORT="${BENCH_MOCK_PORT:-18700}"
CORE_PORT="${BENCH_CORE_PORT:-17788}"
CORE_TOKEN="bench-$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')"

CORE_BIN="$REPO_ROOT/target/release/openhuman-core"
if [[ ! -x "$CORE_BIN" ]]; then
  echo "error: $CORE_BIN not found." >&2
  echo "Build it first:" >&2
  echo "  cargo build --release --bin openhuman-core \\" >&2
  echo "    --no-default-features --features \"\$(bash scripts/ci/product-features.sh)\"" >&2
  exit 1
fi

# The workspace goes on real disk, under the artifacts directory — NOT in /tmp.
#
# On a host where /tmp is tmpfs (common, and the case on the box this was
# written on) that choice is not cosmetic, it corrupts the experiment twice
# over. tmpfs pages ARE memory, so every byte the core writes to its workspace
# is charged against the machine's RAM while the benchmark is trying to
# attribute RAM to the core. And it does not merely skew the numbers: a
# sustained run fills the mount, at which point the core starts failing with
# "Failed to write auth profile lock owner" and SQLite "disk I/O error", and
# throughput collapses to zero — a failure that reads like a leak-induced
# meltdown rather than a full disk.
#
# --workspace reuses an existing (already populated) workspace instead of
# starting empty. That turns the harness into a controlled experiment: run once
# to accumulate state, then point a FRESH core process at the result. If
# per-turn latency starts where the previous run ended, the cost is a function
# of accumulated DATA (an O(N) path over stored state); if it starts low again,
# the cost was a function of process uptime (an in-process leak). The two have
# entirely different fixes, and nothing else in this harness separates them.
if [[ -n "$REUSE_WORKSPACE" ]]; then
  if [[ ! -d "$REUSE_WORKSPACE" ]]; then
    echo "error: --workspace $REUSE_WORKSPACE does not exist" >&2
    exit 1
  fi
  WORKSPACE="$(cd "$REUSE_WORKSPACE" && pwd)"
  # Never delete a workspace the caller supplied.
  KEEP_WORKSPACE=1
  echo "==> reusing workspace: $WORKSPACE"
else
  WORKSPACE="$OUT_DIR/workspace"
fi
mkdir -p "$WORKSPACE"

WORKSPACE_FS="$(findmnt -no FSTYPE --target "$WORKSPACE" 2>/dev/null || echo unknown)"
if [[ "$WORKSPACE_FS" == "tmpfs" || "$WORKSPACE_FS" == "ramfs" ]]; then
  echo "error: the benchmark workspace is on $WORKSPACE_FS ($WORKSPACE)." >&2
  echo "  A RAM-backed filesystem charges the core's disk writes against machine" >&2
  echo "  memory, which invalidates the memory measurement, and fills up mid-run." >&2
  echo "  Use --out-dir to place artifacts on a disk-backed filesystem." >&2
  exit 1
fi
if [[ "$WORKSPACE_FS" == "unknown" ]]; then
  echo "error: could not determine the filesystem backing $WORKSPACE (findmnt missing or failed)." >&2
  echo "  The guard exists to keep the run off tmpfs/ramfs; refusing to proceed blind." >&2
  exit 1
fi

WORKSPACE_AVAIL_MIB="$(df -Pm "$WORKSPACE" 2>/dev/null | awk 'NR==2 {print $4}')"
if [[ -n "$WORKSPACE_AVAIL_MIB" && "$WORKSPACE_AVAIL_MIB" -lt 2048 ]]; then
  echo "warning: only ${WORKSPACE_AVAIL_MIB} MiB free at $WORKSPACE." >&2
  echo "  A sustained run writes memory chunks and embeddings continuously; if the" >&2
  echo "  filesystem fills, turns start failing and the run measures that instead." >&2
fi

# The core enforces a daily managed-inference spend limit, $10 by default, and
# prices the mock's reported token usage against it. A sustained run blows
# through that in a few hundred turns, after which EVERY remaining turn fails
# instantly with "cost budget exceeded" — which does not look like a broken
# benchmark, it looks like enormous throughput and a memory curve driven
# entirely by error handling.
#
# The limits are raised rather than the check disabled (`[cost] enabled = false`)
# so the budget check still runs on every turn and its cost stays in the
# measurement. We are not benchmarking billing, but we should not silently
# remove work the product does per turn either.
#
# Written to both candidate locations because the resolver accepts
# `<workspace>/config.toml` and `<workspace>/../config.toml` depending on layout.
BENCH_CONFIG=$(cat <<'TOML'
[cost]
enabled = true
daily_limit_usd = 1000000.0
monthly_limit_usd = 1000000.0
TOML
)

# --memory-off is the CONTROL for the memory subsystem.
#
# Per-turn cost that grows with accumulated data is hard to attribute from a
# single run, because the agent both writes memory and recalls over it every
# turn. Turning the writes and the post-turn learning hooks off gives a
# comparison run whose only difference is that accumulation: if per-turn latency
# stops climbing, the growth is the memory path; if it still climbs, it is not.
#
# There is no single `[memory] enabled` switch — three independent producers run
# per turn, so all three have to be named:
#   * `memory.auto_save`     the UnifiedMemory document write (markdown sidecar,
#                            memory_docs row, and re-inserted vector chunks)
#   * `[learning]` hooks     episodic capture and chat→tree, which default ON
#                            *independently* of `learning.enabled`
#   * embeddings             `provider = "none"` selects the no-op embedder;
#                            an unknown string here hard-errors the session
# Agent turns still complete with all of this off — every path degrades rather
# than failing.
if [[ -n "$MEMORY_OFF" ]]; then
  BENCH_CONFIG="$BENCH_CONFIG$(cat <<'TOML'

[memory]
auto_save = false
embedding_provider = "none"

[learning]
enabled = false
episodic_capture_enabled = false
chat_to_tree_enabled = false
stm_recall_enabled = false
tool_memory_capture_enabled = false
goals_enrichment_enabled = false

[memory_tree]
spacy_enabled = false
TOML
)"
fi

# --memory-writes-off stops memory WRITES while leaving recall reads intact.
#
# `--memory-off` turns off reads and writes together, so on its own it cannot
# say which of the two is the expensive one — and they have different fixes. Run
# this against an already-populated workspace (`--workspace`): recall still
# scans the whole namespace every turn, but nothing new is stored. If throughput
# stays at the --memory-off-less level, the cost is in the read path; if it
# recovers, write contention on the shared connection was the problem.
if [[ -n "$MEMORY_WRITES_OFF" ]]; then
  BENCH_CONFIG="$BENCH_CONFIG$(cat <<'TOML'

[memory]
auto_save = false

[learning]
enabled = false
episodic_capture_enabled = false
chat_to_tree_enabled = false
TOML
)"
fi
mkdir -p "$WORKSPACE/workspace"
printf '%s\n' "$BENCH_CONFIG" >"$WORKSPACE/config.toml"
printf '%s\n' "$BENCH_CONFIG" >"$WORKSPACE/workspace/config.toml"

MOCK_PID=""
CORE_PID=""
SAMPLER_PID=""

cleanup() {
  local status=$?
  set +e
  [[ -n "$SAMPLER_PID" ]] && kill "$SAMPLER_PID" 2>/dev/null
  if [[ -n "$CORE_PID" ]]; then
    kill "$CORE_PID" 2>/dev/null
    # Give it a moment to flush and close cleanly before forcing.
    for _ in $(seq 1 20); do kill -0 "$CORE_PID" 2>/dev/null || break; sleep 0.25; done
    kill -9 "$CORE_PID" 2>/dev/null
  fi
  [[ -n "$MOCK_PID" ]] && kill "$MOCK_PID" 2>/dev/null
  # Record how much the run wrote before removing it — a workspace that grows
  # without bound is its own finding, and it is invisible once deleted.
  if [[ -d "$WORKSPACE" ]]; then
    du -sm "$WORKSPACE" 2>/dev/null | awk '{print "workspace on disk: " $1 " MiB"}' >&2
  fi
  if [[ -z "$KEEP_WORKSPACE" && -d "$WORKSPACE" ]]; then
    rm -rf "$WORKSPACE"
  elif [[ -n "$KEEP_WORKSPACE" ]]; then
    echo "workspace kept at $WORKSPACE" >&2
  fi
  exit $status
}
trap cleanup EXIT INT TERM

echo "==> artifacts: $OUT_DIR"
echo "==> workspace: $WORKSPACE"

# ---------------------------------------------------------------- mock LLM
echo "==> starting mock LLM on :$MOCK_PORT"
node "$REPO_ROOT/scripts/bench/mock-llm.mjs" \
  --port "$MOCK_PORT" \
  --latency-ms "$LATENCY_MS" \
  --jitter-ms "$JITTER_MS" \
  --tool-depth "$TOOL_DEPTH" \
  --reply-chars "$REPLY_CHARS" \
  --fail-rate "$FAIL_RATE" \
  >"$OUT_DIR/mock-llm.log" 2>&1 &
MOCK_PID=$!

for _ in $(seq 1 50); do
  if curl -fsS "http://127.0.0.1:$MOCK_PORT/health" >/dev/null 2>&1; then break; fi
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$MOCK_PORT/health" >/dev/null || {
  echo "error: mock LLM did not become healthy; see $OUT_DIR/mock-llm.log" >&2
  exit 1
}

# ---------------------------------------------------------------- core
echo "==> starting openhuman-core on :$CORE_PORT"
# BACKEND_URL is the whole redirect: it feeds both the inference base and the
# backend base, so every outbound call lands on the mock.
#
# OPENHUMAN_APPROVAL_GATE=0 is not a convenience. The gate is ON by default and
# parks interactive chat turns pending a human decision, with a 10-minute TTL
# that resolves to Deny. Left on, every benchmark turn would block on a prompt
# nobody is there to answer, and the run would measure a queue of parked turns
# rather than agent throughput.
#
# `env -i` clears the environment so a developer's own OPENHUMAN_* or BACKEND_URL
# settings cannot silently redirect the run at their real account or backend.
env -i \
  PATH="$PATH" HOME="$WORKSPACE" \
  OPENHUMAN_WORKSPACE="$WORKSPACE" \
  OPENHUMAN_ACTION_DIR="$WORKSPACE/projects" \
  OPENHUMAN_CORE_HOST=127.0.0.1 \
  OPENHUMAN_CORE_PORT="$CORE_PORT" \
  OPENHUMAN_CORE_TOKEN="$CORE_TOKEN" \
  BACKEND_URL="http://127.0.0.1:$MOCK_PORT" \
  OPENHUMAN_APPROVAL_GATE=0 \
  RUST_LOG="${RUST_LOG:-warn}" \
  "$CORE_BIN" serve >"$OUT_DIR/core.log" 2>&1 &
CORE_PID=$!

for _ in $(seq 1 150); do
  if curl -fsS "http://127.0.0.1:$CORE_PORT/health" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$CORE_PID" 2>/dev/null; then
    echo "error: core exited during startup; see $OUT_DIR/core.log" >&2
    tail -30 "$OUT_DIR/core.log" >&2
    exit 1
  fi
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$CORE_PORT/health" >/dev/null || {
  echo "error: core did not become healthy; see $OUT_DIR/core.log" >&2
  tail -30 "$OUT_DIR/core.log" >&2
  exit 1
}
echo "==> core pid $CORE_PID healthy"

# ---------------------------------------------------------------- sample + load
# The sampler starts before the driver so the series covers warm-up too; the
# analyzer drops that head via --warmup-frac.
echo "==> sampling every ${INTERVAL_MS}ms"
node "$REPO_ROOT/scripts/bench/sampler.mjs" \
  --pid "$CORE_PID" --interval-ms "$INTERVAL_MS" $TREE \
  >"$OUT_DIR/samples.jsonl" 2>"$OUT_DIR/sampler.log" &
SAMPLER_PID=$!

DRIVER_ARGS=(
  --core-url "http://127.0.0.1:$CORE_PORT"
  --token "$CORE_TOKEN"
  --concurrency "$CONCURRENCY"
  --thread-mode "$THREAD_MODE"
  --warmup-turns "$WARMUP_TURNS"
  --out "$OUT_DIR/driver.json"
  --turns-out "$OUT_DIR/turns.jsonl"
)
if [[ -n "$DURATION_MS" ]]; then
  DRIVER_ARGS+=(--duration-ms "$DURATION_MS")
else
  DRIVER_ARGS+=(--turns "$TURNS")
fi

# Workspace size before and after the load. The agent persists memory chunks and
# embeddings every turn, so this grows even in `fresh` thread mode — which means
# RSS growth is not automatically a leak, it may be an index tracking data that
# genuinely accumulated. Recording both lets the report say which.
WORKSPACE_MIB_BEFORE="$(du -sm "$WORKSPACE" 2>/dev/null | awk '{print $1}')"

echo "==> running load"
DRIVER_STATUS=0
node "$REPO_ROOT/scripts/bench/driver.mjs" "${DRIVER_ARGS[@]}" \
  >"$OUT_DIR/driver.stdout" 2>"$OUT_DIR/driver.log" || DRIVER_STATUS=$?

WORKSPACE_MIB_AFTER="$(du -sm "$WORKSPACE" 2>/dev/null | awk '{print $1}')"

# Let the sampler capture the post-load tail. Memory that is only released once
# work stops shows up here, and so does a process that keeps burning CPU after
# the last turn — which is itself a finding.
echo "==> settling"
sleep 3
kill "$SAMPLER_PID" 2>/dev/null || true
wait "$SAMPLER_PID" 2>/dev/null || true
SAMPLER_PID=""

curl -fsS "http://127.0.0.1:$MOCK_PORT/__bench/stats" >"$OUT_DIR/mock-stats.json" 2>/dev/null || true

if [[ $DRIVER_STATUS -ne 0 ]]; then
  echo "error: driver failed (exit $DRIVER_STATUS); see $OUT_DIR/driver.log" >&2
  tail -20 "$OUT_DIR/driver.log" >&2
  exit $DRIVER_STATUS
fi

# ---------------------------------------------------------------- cross-check
# A turn can return 200 while the inference call behind it silently degraded —
# the RPC succeeds, the agent answers with an error string, and the run looks
# green having measured nothing. Comparing what the driver thinks it ran against
# what the mock was actually asked for is the cheapest way to catch that, and
# without it a misconfigured BACKEND_URL would produce a confident, meaningless
# "no leak detected".
echo "==> verifying the mock actually served the load"
node -e '
const fs = require("node:fs");
const [statsPath, driverPath] = process.argv.slice(1);
let stats, driver;
try {
  stats = JSON.parse(fs.readFileSync(statsPath, "utf8"));
  driver = JSON.parse(fs.readFileSync(driverPath, "utf8"));
} catch (err) {
  console.error(`  could not cross-check: ${err.message}`);
  process.exit(1);
}
const turns = driver.turnsOk ?? 0;
const failed = driver.turnsFailed ?? 0;
const total = turns + failed;
console.error(
  `  driver: ${turns} ok turns | mock: ${stats.completions} completions, ` +
  `${stats.toolCallsEmitted} tool calls, ${stats.embeddings} embeddings, ` +
  `${stats.telemetry} telemetry`,
);

// A run where most turns errored is not a measurement of agent work — it is a
// measurement of the error path, and its memory curve says nothing about a
// leak in normal operation. This check exists because a run that failed 96% of
// its turns on an exhausted cost budget still produced a confident verdict:
// the failures were fast, so throughput looked high and nothing else complained.
const FAILURE_BUDGET = 0.05;
if (total > 0 && failed / total > FAILURE_BUDGET) {
  console.error(
    `  ERROR: ${failed}/${total} turns failed ` +
    `(${((failed / total) * 100).toFixed(1)}%), over the ${FAILURE_BUDGET * 100}% budget. ` +
    `This run measured the failure path, not agent work. Distinct errors:`,
  );
  for (const [msg, count] of Object.entries(driver.errors ?? {}).slice(0, 5)) {
    console.error(`    ${count} x ${msg.slice(0, 180)}`);
  }
  process.exit(1);
}
if (failed > 0) {
  console.error(`  note: ${failed}/${total} turns failed, within the tolerated budget.`);
}
if (stats.unknownRoutes > 0) {
  console.error(
    `  WARNING: the core called ${stats.unknownRoutes} route(s) the mock does not ` +
    `implement (see mock-llm.log). Those calls failed, so some path ran degraded.`,
  );
}
if (turns > 0 && stats.completions < turns) {
  console.error(
    `  ERROR: ${turns} turns reported ok but the mock served only ` +
    `${stats.completions} completions. Turns are not reaching the mocked LLM, ` +
    `so these numbers do not describe agent work.`,
  );
  process.exit(1);
}
' "$OUT_DIR/mock-stats.json" "$OUT_DIR/driver.json" || {
  echo "error: cross-check failed — refusing to report a verdict on this run" >&2
  exit 1
}

# ---------------------------------------------------------------- analyze
echo "==> analyzing"
ANALYZE_STATUS=0
node "$REPO_ROOT/scripts/bench/analyze.mjs" \
  --samples "$OUT_DIR/samples.jsonl" \
  --driver "$OUT_DIR/driver.json" \
  --turns "$OUT_DIR/turns.jsonl" \
  --workspace-mib-before "${WORKSPACE_MIB_BEFORE:-0}" \
  --workspace-mib-after "${WORKSPACE_MIB_AFTER:-0}" \
  --out "$OUT_DIR/report.json" \
  >/dev/null || ANALYZE_STATUS=$?

echo "==> artifacts in $OUT_DIR"
exit $ANALYZE_STATUS
