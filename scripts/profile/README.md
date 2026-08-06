# `scripts/profile/`

Reproducible benchmarking scripts for the OpenHuman Rust core as an embedded
library (no RPC server), built around the `library-profile` and `rss-bench`
binaries (see `src/bin/library_profile/main.rs`). Full write-up:
[`docs/library-benchmarking.md`](../../docs/library-benchmarking.md). Prior
findings: [`docs/resource-profiling-session-2026-07-21.md`](../../docs/resource-profiling-session-2026-07-21.md).

Five driver scripts: `library-bench.sh` (per-scenario RSS/duration),
`library-cpu.sh` (samply), `library-heap.sh` (dhat), `library-fleet.sh`
(fleet-scale sweep + budget gate), and `library-instances.sh` (multi-process
instance sweep).

## Scripts

### `library-bench.sh` — RSS/duration benchmark

Builds `library-profile` + `rss-bench`, runs each scenario N times as a fresh
process, and aggregates median/min/max duration, settled RSS, retained delta,
and peak delta into `summary.json` + `summary.md`.

```bash
./scripts/profile/library-bench.sh                              # all 7 scenarios, default build, 5 repeats
./scripts/profile/library-bench.sh --slim --repeat 7             # slim (no-default-features) build
./scripts/profile/library-bench.sh --scenarios "long-agent,subagents" --turns 50 --warm
```

Results land in `target/profile/rust-library/bench-<timestamp>/` (or `--out DIR`).

### `library-cpu.sh` — CPU profile via samply

Wraps `samply record` around one scenario, isolated from persistence/timezone
noise by default (matching the documented cold-path CPU recipe).

```bash
./scripts/profile/library-cpu.sh subagents
./scripts/profile/library-cpu.sh long-agent -- OPENHUMAN_PROFILE_TURNS=50
samply load target/profile/rust-library/subagents-cpu.json.gz
```

### `library-heap.sh` — live heap attribution via dhat

Builds the `rss-bench-dhat` variant and runs one scenario under dhat. RSS and
timing numbers from this build are perturbed by instrumentation; use it only
for allocation-site/retained-bytes attribution, not for RSS comparisons.

```bash
./scripts/profile/library-heap.sh memory-ingest
# open https://nnethercote.github.io/dh_view/dh_view.html and load
# target/profile/rust-library/dhat-memory-ingest.json
```

### `library-fleet.sh` — fleet sweep + 2 GB / 2 vCPU budget gate

Builds `library-profile` + `rss-bench`, sweeps the `fleet` scenario (N
concurrent live agents with latency-realistic mock inference) across a list
of agent counts, aggregates medians per N, and gates on whether the
projected footprint at the target agent count fits the RAM budget.

```bash
./scripts/profile/library-fleet.sh --agents 100 --latency-ms 200
./scripts/profile/library-fleet.sh --agents "50,100,500" --target 1000 --budget-mib 2048
```

Results land in `target/profile/rust-library/fleet-<timestamp>/` (or
`--out DIR`). Exits nonzero if any swept N reports `fits: false` (use
`--no-gate` to report only). See
[`docs/library-benchmarking.md`](../../docs/library-benchmarking.md#the-2-gb--2-vcpu-server-budget)
for the budget math.

### `library-instances.sh` — multi-instance (many-processes) sweep

Spawns N independent `library-profile` processes (each a live instance held
alive via `OPENHUMAN_PROFILE_HOLD_SECS`), staggered on startup, and measures
**per-process** cost and box survivability — the opencompany "N independent
processes/containers" deployment model, as opposed to `library-fleet.sh`'s
"N agents in one process" model. Samples aggregate sum-RSS + live count every
2s while instances hold, captures a `vm_stat` snapshot at peak (and a
best-effort `footprint` sample if that macOS tool is present), then
aggregates per swept N: launched/ok counts, median settled RSS per instance,
mean and peak aggregate sum-RSS, and — on Linux, where it's meaningful —
summed PSS.

```bash
./scripts/profile/library-instances.sh --instances "10,50" --hold-secs 30
./scripts/profile/library-instances.sh --instances "100,500" --max-instances 500 --gate
```

Results land in `target/profile/rust-library/instances-<timestamp>/` (or
`--out DIR`). Refuses to spawn more than `--max-instances` (default 200)
without an explicit raise — see the script's `--help` for the RAM math. Exits
nonzero with `--gate` if any instance failed to complete cleanly (nonzero
exit or missing/invalid JSON); default is report-only. See
[`docs/library-benchmarking.md`](../../docs/library-benchmarking.md#fleet-one-process-vs-instances-many-processes)
for the fleet-vs-instances framing.

### `library-pool-gate.sh` — runtime pool regression gate (#5106)

Drives the `skill-run` scenario with K parallel skill runs and asserts the DoD:
with the shared runtime pool ON, the process tree grows by ~one pooled worker,
**not** K interpreters. The scenario hard-asserts `tree.child_count <= max_workers`
for K > 1 (nonzero exit); this script runs it and reports a pooled-vs-unpooled
comparison. A regression that reintroduces per-run forking fails the gate.

```bash
./scripts/profile/library-pool-gate.sh                     # K=8, max_workers=1
./scripts/profile/library-pool-gate.sh --concurrency 16 --skip-build
```

Exits 0 = pass, 1 = regression. SKIPs (exit 0) when no system `node` is on
`PATH` (the scenario must never download an interpreter), so node-less CI
runners don't false-fail.

## Quick start

```bash
# 1. Baseline RSS/duration across all scenarios
./scripts/profile/library-bench.sh

# 2. CPU attribution for the slowest/most interesting scenario
./scripts/profile/library-cpu.sh subagents

# 3. If a scenario's RSS looks off, drill into live heap
./scripts/profile/library-heap.sh subagents
```

All scripts require `jq` for JSON parsing/aggregation; `library-cpu.sh` also
requires `samply` (`cargo install samply`). Build commands use
`GGML_NATIVE=OFF` to work around the Apple Silicon whisper-rs/llama.cpp NEON
fp16 build issue.
