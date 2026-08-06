# Library benchmarking environment

## Purpose

"opencompany" wants to embed the OpenHuman Rust core as a library: no always-on
RPC server, no Tauri shell, just the core linked in-process and driven
directly. That changes what "resource usage" means. There is no single steady
process to profile; there are per-use-case workloads (a long-running agent
loop, a delegated multi-agent turn, a saved workflow run, a background
subconscious pass, a memory ingest, a bare embed) that each have their own
startup cost, steady-state footprint, and growth curve.

This document describes the benchmark environment built to measure that: a
pinned `library-profile` binary with eight scenarios, four driver scripts
under `scripts/profile/`, and the comparison point the team cares about
(ZeroClaw). It builds on the manual investigation in
[`docs/resource-profiling-session-2026-07-21.md`](resource-profiling-session-2026-07-21.md);
read that document for the deep memory/CPU attribution work. This document is
about running repeatable benchmarks, not re-deriving those findings.

## The eight scenarios

All scenarios run in `target/release/library-profile <scenario>`, replace
network inference with a deterministic provider (`rss-bench` feature), and
print one pretty-printed JSON result object to stdout (diagnostics go to
stderr). Each models a distinct embedding use case:

| Scenario | Models |
| --- | --- |
| `memory-ingest` | Canonicalizing and ingesting a batch of chat messages through the real extraction/admission/tree-queue pipeline. |
| `subagents` | A delegation turn: an orchestrator session spawns real subagents via `spawn_parallel_agents` and merges their findings. |
| `agent-turn` | The minimal embed case: one agent, one turn, no delegation, no workflow. The smallest useful "hello world" for a host that just wants a single reply. |
| `long-agent` | A long-running agent loop (`OPENHUMAN_PROFILE_TURNS`, default 25) in one process, to see whether RSS plateaus or grows per turn. |
| `workflow` | A saved automation run (`flows_create` + `flows_run`), representing the flows/automation embedding path rather than ad hoc chat. |
| `subconscious` | A background subconscious turn (the always-on reflective pass), distinct from an interactive chat turn. |
| `cold-phases` | Bootstrap attribution: per-phase checkpoints (config load, registry init, agent build, memory construction, first turn) so cold-start cost can be attributed to a phase instead of one lump sum. |
| `fleet` | N concurrent live agents with latency-realistic mock inference — the "100-1000 agents in a 2 GB / 2 vCPU server" question. See [below](#the-2-gb--2-vcpu-server-budget). |

## How to run

Six scripts under `scripts/profile/` (each has `-h`/`--help`):

- **`library-bench.sh`** — the primary RSS/duration benchmark. Builds the
  binaries, runs each scenario N fresh-process repeats (default 5), and
  aggregates median/min/max into `summary.json` + `summary.md`.

  ```bash
  ./scripts/profile/library-bench.sh                     # default build, all scenarios
  ./scripts/profile/library-bench.sh --slim               # --no-default-features recipe
  ./scripts/profile/library-bench.sh --scenarios "long-agent,subagents" --turns 50 --warm
  ```

- **`library-cpu.sh`** — a `samply` wrapper for one scenario's CPU profile,
  isolated from persistence/timezone noise by default.

  ```bash
  ./scripts/profile/library-cpu.sh subagents
  samply load target/profile/rust-library/subagents-cpu.json.gz
  ```

- **`library-heap.sh`** — builds the `rss-bench-dhat` variant and runs a
  scenario under dhat for live-heap attribution (allocation sites, retained
  bytes). RSS/timing under dhat are perturbed; don't compare those numbers to
  `library-bench.sh` output.

  ```bash
  ./scripts/profile/library-heap.sh memory-ingest
  # load target/profile/rust-library/dhat-memory-ingest.json at
  # https://nnethercote.github.io/dh_view/dh_view.html
  ```

- **`library-fleet.sh`** — sweeps the `fleet` scenario across a list of agent
  counts and gates the result against the 2 GB / 2 vCPU server budget (see
  below).

  ```bash
  ./scripts/profile/library-fleet.sh --agents 100 --latency-ms 200
  ./scripts/profile/library-fleet.sh --agents "50,100,500" --target 1000 --budget-mib 2048
  ```

- **`library-instances.sh`** — sweeps N independent *processes* (not agents
  in one process) of a scenario, held alive via `OPENHUMAN_PROFILE_HOLD_SECS`,
  and measures per-instance/aggregate cost — the many-processes counterpart
  to `library-fleet.sh`'s one-process model (see
  [below](#fleet-one-process-vs-instances-many-processes)).

  ```bash
  ./scripts/profile/library-instances.sh --instances "10,25,50" --hold-secs 30
  ```

- **`library-pool-gate.sh`** — the runtime-pool regression gate (#5106). Runs
  `skill-run` with K parallel skill runs and asserts the process tree grows by
  ~one pooled worker, not K interpreters; reports pooled vs unpooled.

  ```bash
  ./scripts/profile/library-pool-gate.sh --concurrency 8 --workers 1
  ```

### Default vs slim builds

Default-feature builds link every compile-time domain gate (`voice`, `web3`,
`media`, `meet`, `skills`, `flows`, `mcp`, `tui`) — the byte-identical desktop
recipe. The slim recipe drops everything not required by the harness:

```bash
GGML_NATIVE=OFF cargo build --release \
  --no-default-features --features rss-bench \
  --bin library-profile --bin rss-bench
```

`--slim` on `library-bench.sh` builds this recipe. Per the prior session,
compile-time gates shrink the binary substantially but only move settled RSS
by a few MiB — most of the RSS story is initialization and allocator
behavior, not linked code size.

### Useful env knobs

| Variable | Effect |
| --- | --- |
| `OPENHUMAN_PROFILE_TURNS` | Turn count for `long-agent` (default 25). |
| `OPENHUMAN_PROFILE_PREWARM_SUBAGENTS=1` | Run one warm-up turn before measuring (`subagents`/`subconscious`), isolating first-use cost from steady state. |
| `OPENHUMAN_PROFILE_DISABLE_MEMORY_WRITES=1` | Disable `memory.auto_save` and episodic capture, isolating orchestration from persistence. |
| `OPENHUMAN_PROFILE_FORCE_UTC=1` | Skip `iana_time_zone`/CoreFoundation timezone resolution. |
| `OPENHUMAN_PROFILE_HOLD_SECS` / `HOLD_BEFORE_SECS` | Pause the process at settled/baseline state for external inspection (`vmmap`, `heap`, `malloc_history`, Instruments). |
| `OPENHUMAN_PROFILE_DHAT_OUT` | Output path for dhat JSON (set by `library-heap.sh`). |
| `OPENHUMAN_PROFILE_SKILL_RUN_CONCURRENCY` | `skill-run`: number of parallel `code_executor` turns (K), each spawning a `node_exec` job (default 1). |
| `OPENHUMAN_PROFILE_SKILL_RUN_POOL` | `skill-run`: `off` disables the shared runtime pool (legacy per-call spawn; tree then shows ~K resident `node` children). Default on. |
| `OPENHUMAN_PROFILE_SKILL_RUN_POOL_WORKERS` | `skill-run`: pool size W when pooling is on (default 1). The scenario asserts `child_count <= W` for K > 1 — the #5106 regression gate. |

## Metrics and interpretation

Every run reports `baseline`/`settled`/`peak_rss_kib` (macOS: `proc_pid_rusage`
RSS, `getrusage` peak, `proc_pidinfo` thread count), `retained_delta_kib`
(settled minus baseline), `peak_delta_kib`, and `duration_ms`. `long-agent`
additionally reports `checkpoints[]` so first-turn vs. last-turn growth is
visible directly.

**RSS is not private heap.** The prior session's deep attribution found a
~42 MiB slim-build snapshot broken down as roughly 15.2 MiB private physical
footprint, 3.18 MiB live heap, 18.7 MiB resident executable text, and ~9.4 MiB
of resident-but-mostly-inactive malloc pages (allocator high-water
retention). See
[`docs/resource-profiling-session-2026-07-21.md`](resource-profiling-session-2026-07-21.md#deep-memory-attribution)
for the full breakdown, the executable-paging finding (a cold turn faults in
~15 MiB of previously nonresident OpenHuman code), and the warmed-process
control showing steady-state turns cost ~0.5-1.9 MiB once warm rather than
the ~26-31 MiB a cold turn costs. Use `library-bench.sh` for the RSS/duration
headline numbers, `library-cpu.sh` when CPU attribution is the question, and
`library-heap.sh` only when RSS numbers need live-allocation attribution
(accepting the dhat perturbation).

## The ZeroClaw comparison

ZeroClaw self-reports idling under 5 MiB RAM; the "7.8-12 MiB under load"
figure sometimes quoted alongside it has no locatable primary source, and even
the idle figure is vendor marketing with no third-party verification (see
[`docs/harness-comparison-2026-07-22.md`](harness-comparison-2026-07-22.md)).
OpenHuman's Rust core currently settles around 35-50 MiB depending on
scenario and feature set (see the baseline table below).

Treat this as a **north star, not an apples-to-apples benchmark**. ZeroClaw's
scope and feature set differ substantially from the OpenHuman core: OpenHuman
links a full agent/memory/tool/orchestration stack (SQLite-backed unified
memory, TinyCortex PII detection, prompt-injection detection, a builtin-agent
registry, tool catalogs, provider routing) that a narrower harness may not
carry at all. A closer gap is a meaningful signal that the initialization
graph is leaner; it is not evidence of feature parity, and a wider gap is not
automatically a regression if it comes from carrying more capability. Every
`library-bench.sh` summary includes a labeled comparison row/note for exactly
this reason: visible, but explicitly called out as external.

## The 2 GB / 2 vCPU server budget

"opencompany" wants a single server to host 100-1000 live agents inside a 2 GB
RAM / 2 vCPU box. That is a budget question, not a per-scenario RSS question:
2048 MiB / 1000 agents is roughly 2 MiB per agent all-in, but the fixed
per-process base (allocator high water, code paging, registries, detectors —
the same ~30 MiB every scenario above pays once) amortizes across however many
agents share the process. What actually determines whether 1000 agents fit is
the **marginal** cost per additional agent once that base is paid, not the
per-agent average. `library-fleet.sh` runs the `fleet` scenario (N concurrent
live agents, latency-realistic mock inference so idle time looks like real
network waits rather than a busy loop) across a sweep of N and reports that
marginal cost directly (`marginal_rss_kib_per_agent`), alongside idle CPU over
a parked 10s window, thread count, and open FD count — all of which should
stay roughly flat as N grows if per-agent state is cheap and idle agents cost
~zero CPU.

Working targets: marginal cost ≤ 1.5 MiB/agent, threads and FDs flat (not
linear) in N, and idle CPU low regardless of N — an agent that isn't mid-turn
should not be spending cycles. `OPENHUMAN_PROFILE_WORKER_THREADS=2` pins the
scenario's tokio runtime to 2 worker threads to simulate the 2 vCPU box rather
than scaling with the host's actual core count. The `budget` block in each
run's JSON (`target_agents`, `ram_budget_mib`, `projected_rss_mib_at_target`,
`fits`) projects the swept marginal cost out to the real target (default 1000
agents / 2048 MiB); `library-fleet.sh` aggregates medians per N into
`summary.md` and exits nonzero if any swept N projects `fits: false`, making
it usable as a CI-style regression gate (`--no-gate` to disable).

**Caveats, stated plainly:** these numbers are gathered on macOS, which has no
cgroup memory limit to enforce or observe locally — the budget check is a
projection from measured marginal cost, not a live "did it actually get
OOM-killed at N agents" test. macOS also lacks Linux's `/proc/<pid>/smaps_rollup`,
which would give true PSS (proportional shared memory) instead of RSS; RSS
overcounts shared pages (executable text, shared library mappings) in a way
that matters more as agent count grows and more of the process footprint is
genuinely shared. Treat the macOS numbers as an approximation of the target
Linux server, not a substitute for it. The JSON schema already has a Linux
path — `proc_metrics` reads `/proc/<pid>/status` and `/proc/<pid>/stat` on
Linux — so true validation should eventually mean running the same
`library-profile fleet` binary on a cgroup-limited Linux box (matching the 2
vCPU / 2 GB target) rather than trusting the macOS projection alone.

### Fleet (one process) vs instances (many processes)

The budget section above measures one deployment shape: N agents sharing a
single process. But "opencompany" may instead run OpenHuman as **N
independent processes or containers** — one per tenant — rather than N
agents inside one process. Those are different cost models and the fleet
number does not answer the second one.

- **Fleet (`library-fleet.sh`)** pays the ~30-50 MiB fixed base (allocator
  high water, code paging, registries, detectors) **once**, and amortizes it
  across however many agents share that process. Marginal cost per agent is
  what matters, and it can be well under 1 MiB once the base is paid.
- **Instances (`library-instances.sh`)** pays that same fixed base **N
  times**, once per process — minus whatever the OS actually shares across
  processes (resident executable text, shared library mappings). Summed RSS
  across instances therefore **double-counts** those shared pages; it is an
  upper bound, not the true footprint. True per-instance marginal cost is
  better read from summed PSS (Linux only — macOS has no PSS-equivalent
  metric), which divides shared pages across the processes that share them.

`library-instances.sh` spawns N held `library-profile` processes staggered
on startup, samples aggregate sum-RSS every 2s while they hold at settled
state, and reports median settled RSS/instance, mean and peak aggregate
sum-RSS, and summed PSS when available, plus a labeled 2 GB-box
extrapolation estimate:

```bash
./scripts/profile/library-instances.sh --instances "10,25,50" --hold-secs 30
```

**This is still a macOS proxy, not container validation.** True validation
means running the same binary under real `cgroup` memory limits (e.g.
`docker run --memory=2g`) on a Linux host and observing whether it survives
or gets OOM-killed at the target instance count — not projecting from local
sum-RSS. That is follow-up work, and it belongs on a Linux box: this repo's
own `openhuman-core` Docker build is currently blocked on Apple Silicon (the
`whisper-rs-sys`/whisper.cpp NEON fp16 intrinsics fail to compile under
arm64-Linux emulation with GCC 12 — see the umbrella repo's root `CLAUDE.md`
gotchas and `docs/resource-profiling-session-2026-07-21.md`). The path
around that blocker is either building for `linux/amd64` under emulation (the
whisper AVX path has no NEON bug) or running the validation on a native Linux
host rather than macOS Docker Desktop.

## Profiling escalation path

Start cheap, escalate only as needed:

1. **`library-bench.sh`** — RSS/duration medians across fresh processes. Answers "did this change move the needle" for most changes.
2. **`library-cpu.sh` (samply)** — symbolized CPU profile when a scenario is slower than expected, or to attribute cold-path CPU to a specific phase (registry init, agent build, memory construction, SQLite init, TinyAgents turn runner were the top contributors in the prior session).
3. **`library-heap.sh` (dhat)** — live-heap allocation sites and retained bytes when RSS is high but the cause isn't obvious from CPU alone (e.g. the TinyCortex PII `RegexSet` finding came from stack-logged allocation attribution, not CPU sampling).
4. **Instruments / `vmmap` / `heap` / `malloc_history`** — deepest macOS-native attribution, using the `OPENHUMAN_PROFILE_HOLD_SECS` / `HOLD_BEFORE_SECS` hooks to pause the process at baseline or settled state:

   ```bash
   OPENHUMAN_PROFILE_HOLD_SECS=120 target/release/library-profile subagents &
   vmmap -summary <pid>
   heap -sH <pid>

   MallocStackLogging=1 OPENHUMAN_PROFILE_HOLD_SECS=120 \
     target/release/library-profile subagents &
   malloc_history <pid> -allBySize
   ```

   This is what surfaced the PII-sanitizer regex cache and the first-turn
   executable-paging finding in the prior session; reach for it only once
   `library-bench.sh`/`library-cpu.sh`/`library-heap.sh` have narrowed the
   question to a specific scenario and metric.

## Current baseline numbers

From the 2026-07-21 profiling session (medians over five fresh processes
unless noted; see that document for methodology and caveats):

| Scenario | Build | Median settled RSS | Median retained Δ |
| --- | --- | ---: | ---: |
| 1-agent roster | default | 38.7 MiB | - |
| 8-agent roster | default | 41.5 MiB | +2.8 MiB total (~0.40 MiB/agent) |
| 1-agent roster | slim | 35.5 MiB | - |
| 8-agent roster | slim | 38.7 MiB | +3.2 MiB total |
| `memory-ingest` (100 msgs) | default | 25.5 MiB | 9.31 MiB |
| `memory-ingest` (100 msgs) | slim | 23.8 MiB | 8.58 MiB |
| `subagents` (cold, 2 children) | default | 48.5 MiB | 30.8 MiB |
| `subagents` (cold, 2 children) | slim | 42.4 MiB | 25.7 MiB |
| `subagents` (warmed repeat, persistence off) | default | - | 0.52 MiB |
| `subagents` (warmed repeat, normal capture) | default | - | 1.84 MiB |
| ZeroClaw (external, idle, self-reported/unverified) | - | < 5 MiB | - |

First full `library-bench.sh` run of the new scenarios (default build, 5
fresh-process repeats, 2026-07-21, Apple Silicon macOS):

| Scenario | Build | Median settled RSS | Median retained Δ | Median duration |
| --- | --- | ---: | ---: | ---: |
| `agent-turn` (cold, 1 turn) | default | 47.6 MiB | 29.5 MiB | 102 ms |
| `subconscious` (cold, no delegation) | default | 47.9 MiB | 29.8 MiB | 138 ms |
| `subagents` (cold, 2 children) | default | 48.0 MiB | 29.9 MiB | 142 ms |
| `workflow` (`flows_create` + `flows_run`) | default | 50.9 MiB | 29.9 MiB | 110 ms |
| `long-agent` (25 warmed turns) | default | 65.8 MiB | 18.5 MiB | 1,361 ms |
| `cold-phases` (9 bootstrap phases) | default | 51.2 MiB | 36.5 MiB | 476 ms |
| `memory-ingest` (100 msgs) | default | 25.8 MiB | 9.3 MiB | 2,099 ms |

Notable structure behind these medians:

- The `long-agent` per-turn series plateaus: typical turns add 30-150 KiB,
  and the 25-turn total (~16.8 MiB first-to-last) is dominated by two async
  persistence/compaction bursts of 6-8 MiB each, matching the prior session's
  warmed-repeat outlier observation. Steady-state growth is not linear.
- Cold `agent-turn`, `subconscious`, `subagents`, and `workflow` all retain
  approximately the same ~29-30 MiB, confirming the cost is shared bootstrap
  (code paging, registries, detectors, allocator high water), not the
  specific workload on top of it.
- A dhat run of `agent-turn` measured 33.4 MB total allocated across 135,756
  blocks, but only 5.0 MB peak live heap and 3.1 MB live at exit, again
  showing RSS is mostly not live heap data.

### Fleet, instances, and runtime baselines (2026-07-22, post-PII-prefilter)

`library-fleet.sh` sweep (default build, 3 repeats, 3 turns/agent, 200 ms
mock latency, 2 worker threads, target 1000 agents / 2048 MiB):

| N agents | Marginal KiB/agent | Settled MiB | Idle CPU ms/10s | Threads | fds | p95 turn ms | Projected MiB @1000 | Fits |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | :---: |
| 50 | 1,985 | 223 | 3 | 71 | 420 | 2,848 | 1,956 | yes |
| 100 | 1,866 | 356 | 3 | 123 | 820 | 5,402 | 1,840 | yes |
| 500 | 1,770 | 1,393 | 3 | 211 | 3,220 | 25,484 | 1,747 | yes |

`library-instances.sh` (many-processes model, `agent-turn` held 20 s):
~47.8-48.2 MiB per instance flat at N=10/25/50 → roughly 42 instances per
2 GiB by sum-RSS (upper bound; macOS has no PSS). The one-process fleet
model is ~25x denser than the per-process model.

Runtime/subagent scenarios: `skill-run` measures a real `node` child at
~72-75 MB RSS (~121 MB process tree vs ~51 MB self) — the basis of the
runtime-pooling issue (tinyhumansai/openhuman#5106); `subagent-storm` shows
~0.78 MiB marginal per additional parallel subagent (K=8→32 cross-width).

Runtime pool (#5106): with the shared pool on (the default), a batch of K
concurrent `skill-run` turns shares a bounded set of warm `node` workers, so
the process tree grows by ~one pooled worker instead of K interpreters.
Measured at **K=8** (`max_workers=1`, system node v24): pooled tree
`child_count=1` at **~202 MiB**, vs unpooled `child_count=8` at **~690 MiB**
(eight `node` children of ~73–75 MB each) — a ~490 MiB / 3.4× reduction that
grows with K. Compare the two regimes directly:

```bash
# Legacy: K interpreters resident at peak.
OPENHUMAN_PROFILE_SKILL_RUN_CONCURRENCY=8 OPENHUMAN_PROFILE_SKILL_RUN_POOL=off \
  target/release/library-profile skill-run
# Pooled: child_count stays at the pool size (asserted), not K.
OPENHUMAN_PROFILE_SKILL_RUN_CONCURRENCY=8 OPENHUMAN_PROFILE_SKILL_RUN_POOL_WORKERS=1 \
  target/release/library-profile skill-run
```

The pool is configured in `[runtime_pool]` (master switch + per-language
`node`/`python` `max_workers`, `idle_ttl_secs`, `recycle_after_jobs`,
`max_queue_depth`); `enabled = false` reverts every caller to the legacy
per-call spawn.

Watch-items from the sweep: thread count grows ~0.35/agent (needs
attribution + cap before real 1000-agent runs), and p95 latency at N=500 on
2 workers shows CPU saturation is the load constraint, not memory.

## See also

- [`docs/resource-profiling-session-2026-07-21.md`](resource-profiling-session-2026-07-21.md) — the full manual investigation (deep attribution, cold-path CPU, library-design implications, recommended optimization order).
- [`scripts/profile/README.md`](../scripts/profile/README.md) — script quick reference.
- `src/bin/library_profile/main.rs` — the scenario implementations.
