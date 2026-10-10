---
description: >-
  Measured footprint: how many agents fit in one process, cold start, binary
  size and the dependency limits that keep it small.
icon: gauge
---

# Performance and footprint

OpenHuman's Rust core runs in-process as a library, not as one OS process per agent. Most of the density numbers on this page come from that one decision. A fixed bootstrap cost (allocator warm-up, code paging, registries, detectors) is paid once per process and shared by every agent inside it.

The original measurements below were made on Apple Silicon macOS with a `--release` build and a deterministic mock inference provider (the `rss-bench` feature), not real network calls. Treat the absolute numbers as numbers for that machine. The ratios (density, marginal cost, binary delta) are the parts worth generalizing. The scripts to reproduce them are listed at the end.

## Fleet: how many agents fit in one process

`library-fleet.sh` runs N concurrent live agents in a single process. Mock inference takes 200 ms, so idle time behaves like a real network wait instead of a busy loop. The script reports the marginal RSS per additional agent once the fixed base is paid. That number decides how many agents fit in a box.

| N agents | Marginal KiB/agent | Settled MiB | Idle CPU ms/10s | Threads | FDs |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 50 | 1,985 | 223 | 3 | 71 | 420 |
| 100 | 1,866 | 356 | 3 | 123 | 820 |
| 500 | 1,770 | 1,393 | 3 | 211 | 3,220 |

500 agents fit in one process at 1,770 KiB (about 1.73 MiB) marginal cost each. Idle CPU stays flat as N grows, which matters because an agent that is not mid-turn should not spend cycles. Thread count grows by about 0.35 per agent. Watch that line before pushing past 500 in production.

Thousands of agents on one box is a goal, not a measured result. The marginal cost settles across 50, 100 and 500 agents instead of rising, which makes the goal plausible.

## In-process vs one process per agent

`library-instances.sh` spawns N independent `library-profile` processes instead of N agents in one process. Each instance used about 47.8 to 48.2 MiB, flat across N = 10, 25 and 50. That puts roughly 42 instances in a 2 GiB box by summed RSS. This is an upper bound, because macOS has no PSS-equivalent metric to divide out shared pages.

One process per agent pays the 30 to 50 MiB fixed base every time. The in-process model pays it once. At the measured marginal cost, one process is roughly 25 times denser than one process per agent for the same memory budget.

## Cold start

| Scenario | Median settled RSS | Median duration |
| --- | ---: | ---: |
| `agent-turn` (cold, one turn, no delegation) | 47.6 MiB | 102 ms |
| `cold-phases` (nine bootstrap phases: config load, registry init, agent build, memory construction, first turn) | 51.2 MiB | 476 ms |

The `cold-phases` scenario was removed from the profile binary, so that row cannot be re-run from this tree. Everything else on this page reproduces with the commands below.

A cold agent turn answers in about 102 ms. The full nine-phase bootstrap, which a process pays once, takes 476 ms. After that, a warmed turn in the same process costs 0.5 to 1.9 MiB, not the 26 to 31 MiB a first turn retains. Most of a cold turn's cost is executable code paging in for the first time, not per-turn allocation.

## Slim build footprint

A `--no-default-features --features rss-bench` build, which drops every optional domain, settles at about 15.2 MiB of private physical memory and roughly 42 MiB RSS. The rest of that RSS is reclaimable executable text and allocator high-water retention, not live data. A deep attribution pass found about 3.2 MiB of live heap and 18.7 MiB of resident executable text inside the 42 MiB.

## Binary size by feature set

Cargo feature gates control what compiles in. Two starting points matter: Contrib (`[features] default` in `crates/openhuman-core/Cargo.toml`, what a bare `cargo check` builds) and Product (`scripts/ci/product-features.txt`, what the desktop app ships).

| Build | Features | Unstripped | Stripped |
| --- | --- | ---: | ---: |
| Default (all gates) | Contrib/Product superset | 115.9 MiB | n/a |
| library-minimal | `skills,flows` | ~81.1 MiB | ~60.4 MiB |
| Pure slim | none | 68.4 MiB | 51.0 MiB |

The `skills,flows` recipe in [`docs/library-minimal-recipe.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/library-minimal-recipe.md) is the supported embed target for a headless host. It keeps SKILL.md execution and saved-workflow runs, drops voice, web3, media, MCP and the desktop automation stack, and ends up about 30% smaller than the default build. Most of that gain is binary size and code-paging surface. It moves settled RSS by only about 3 to 5 MiB per scenario, because RSS depends more on initialization and allocator behavior than on linked code size.

Note that the smallest figure here, 15.2 MiB, is private memory in a slim running process. It is not a binary size. The smallest binary is 51 MiB stripped with nothing enabled.

## Feature gates and loadable modules

Cargo features (`media`, `skills`, `flows`, `mcp`, `channels`, `http-server`, `scheduler-gate`, `file-logging`, `modules` and more) decide what compiles in. Beyond that, several domains ship as loadable native `cdylib` modules that are not linked into the core binary at all.

The compiled registry pins fourteen records across twelve module names, because `tinyruntime` ships as a router plus two language providers. The modules are `tinycomputer`, `tinysearch`, `tinydocs`, `tinywallet`, `tinyjuice`, `tinyvoice`, `tinyruntime` (with its Node and Python providers), `tinymcp`, `tinyconnectors`, `tinybox`, `tinychannels` and `tinyhosts`. Each sits behind a small `*-bus` contract crate. A module loads into the same process and shares its privileges, so the admission checks (ABI, manifest, dependency, digest) matter more than for an ordinary dependency. See [Loadable modules](loadable-modules.md).

## The dependency-floor ratchet

`scripts/kernel-floor.sh` measures the dependency graph of the `flows` profile (`--no-default-features --features flows`, the surface a second host would embed) on three counts: packages, unique crate names and native (C/C++) builds. `scripts/kernel-floor.limits` holds the ceiling for each. The ratchet only moves down. A change that grows the graph must lower those numbers in the same PR, and raising one needs a written justification. Read the current ceiling from the file, because it changes. The ratchet stops the dependency floor from creeping back up after a gating effort sheds crates.

## How to reproduce

Every number above comes from the driver scripts in the [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) repository, under `profile/scripts/`. They are built around `profile/src/bin/library_profile/main.rs` there (the scenarios) and the `library-profile` and `rss-bench` binaries, compiled against a vendored checkout of this repository. The benchmarks run from these scripts, not in CI. Run the commands below from an openhuman-benchmarks checkout, except the dependency-floor ratchet, which runs here.

```bash
# RSS/duration medians across fresh processes, all scenarios
./profile/scripts/library-bench.sh

# Same, against the slim (--no-default-features) recipe
./profile/scripts/library-bench.sh --slim

# Fleet sweep + the 2 GB / 2 vCPU budget gate
./profile/scripts/library-fleet.sh --agents "50,100,500" --target 1000 --budget-mib 2048

# Many-processes counterpart to the fleet sweep
./profile/scripts/library-instances.sh --instances "10,25,50" --hold-secs 30

# Dependency-floor ratchet
scripts/kernel-floor.sh flows
```

[`profile/scripts/README.md`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/scripts/README.md) documents the remaining scripts: `library-cpu.sh` for CPU profiling with samply and `library-heap.sh` for live-heap attribution with dhat. Full methodology, caveats and the per-scenario breakdown are in
[`profile/docs/library-benchmarking.md`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md) and
[`docs/library-minimal-recipe.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/library-minimal-recipe.md).

## Measurement conditions

These numbers were gathered on macOS. It has no local cgroup memory limit and no `/proc/<pid>/smaps_rollup`, so there is no true PSS (proportional shared memory) reading. RSS overcounts shared pages, and the error grows with agent count. The fleet and instances numbers project from measured marginal cost. They are not a live test of surviving an OOM kill at N agents on a 2 GB / 2 vCPU Linux box. Every scenario also replaces network inference with a mock provider at fixed latency, so turn timings measure orchestration overhead, not real model latency. The Linux cgroup measurements in the following section use a different host and harness.

For token cost instead of process footprint, see [Smart token compression](../features/token-compression.md). It is the other half of "cheap": it controls how much of what the harness assembles reaches the model.

## Linux runtime-owned agents (Medulla integration)

Measured on 2026-10-10 at OpenHuman commit `b783b39e78`, using the release
`openhuman-embed` example `linux_fleet` with default features disabled. This
exercises one `Runtime` and N `AgentSpec`s, with two Tokio workers, ephemeral
session storage, and a loopback HTTP chat-completions mock. Each agent
advertises the builtin `shell` tool; the mock returns text without executing it.
Conversation memory, local model runtimes, and session dual writes are off.
The host is x86_64 Ubuntu, Linux 7.0.0-31-generic, on an Intel Core i7-14700F.
Other builds were running on the host, so these are shared-host measurements.

Each run starts in a fresh cgroup with `memory.max = 2147483648` and
`cpu.max = 200000 100000` (2 GiB and a two-CPU quota). Three fresh processes
were measured at each size with the default swap limit; the table gives medians.
RSS is sampled after every agent completes one concurrent turn, with all agent handles retained. Marginal
RSS is `(RSS after turns - RSS after Runtime::build) / N`, without allocator
trimming. The cgroup peak includes the HTTP mock and the Python measurement
wrapper. Mock request recording is disabled, so retained HTTP request history
is excluded. RSS and cgroup accounting differ because shared file pages may be
charged outside the new cgroup.

| Concurrent agents | Process RSS after turns | Marginal RSS/agent | Cgroup peak | Fleet wall time | Turn p95 |
| --- | --- | --- | --- | --- | --- |
| 50 | 270.21 MiB | 4.702 MiB | 237.32 MiB | 1,527 ms | 1,462 ms |
| 100 | 437.38 MiB | 4.028 MiB | 417.89 MiB | 3,537 ms | 3,370 ms |
| 500 | 1,712.97 MiB | 3.355 MiB | 1,760.32 MiB | 20,784 ms | 19,450 ms |

The 500-agent median is within twice the earlier macOS/mock 1,770 KiB
figure (1.728515625 MiB per agent, doubled to 3.45703125 MiB, or about
3.46 MiB); the 50- and 100-agent medians miss that target. The
500-agent runs ranged from 3.341 to 3.585 MiB per agent. These are different
hosts and harness entry points, so the table is a capacity measurement rather
than a controlled comparison of the platforms. This run also incorporates the
upstream embed lifecycle-event and dynamic-agent APIs; results are higher than
the earlier prerequisite-branch measurements recorded before that base update.

Runtime boot took 259–263 ms. The first turn in each fresh process took
49.9–87.4 ms, before launching the concurrent fleet. Both are within twice the
earlier 476 ms bootstrap and 102 ms first-turn figures. “First turn” includes
session/model/tool initialization, but excludes compiling and loading the
executable; the OS page cache was warm.

A separate 500-agent run with **swap disabled** (`MemorySwapMax=0`) completed
in 19,090 ms, with 3.541 MiB marginal RSS per agent and a
1,884.52 MiB cgroup peak. Its marginal RSS exceeds the approximately
3.46 MiB target. The integration ticket used a nominal 1.77 MiB baseline
and 3.54 MiB doubled target; this run also narrowly exceeds that nominal target.
Every run recorded zero `max`, `oom`, and `oom_kill` memory events.
This does not establish capacity for real providers, tool subprocesses, MCP
servers, or 1,000 simultaneously active turns. Measure those workloads before
sizing a production fleet.

To reproduce from the OpenHuman repository root:

```sh
cargo build -p openhuman-embed --locked --release --no-default-features --example linux_fleet
systemd-run --user --scope -p MemoryMax=2G -p CPUQuota=200% --quiet \
  python3 crates/openhuman-embed/examples/linux_fleet_cgroup.py 500
```

Use `50` or `100` for the other sizes and repeat each command in a fresh scope
three times to reproduce the main table medians. Leave swap at its default
limit for those runs. To reproduce the separate no-swap 500-agent run:

```sh
systemd-run --user --scope -p MemoryMax=2G -p MemorySwapMax=0 -p CPUQuota=200% --quiet \
  python3 crates/openhuman-embed/examples/linux_fleet_cgroup.py 500
```

The JSON output records process RSS, bootstrap/turn timings, cgroup limits,
peak memory, and OOM counters. The raw runs are checked in as
[`docs/benchmarks/medulla-embed-linux.json`](../../docs/benchmarks/medulla-embed-linux.json).
