# OpenHuman resource profiling session

Date: 2026-07-21  
Platform: Apple Silicon macOS 26.5.1  
Worktree: `worktrees/tauri-resource-profiler`  
Primary question: What CPU and RAM does OpenHuman consume, which components account for it, and what would it take to use the Rust core as an efficient embedded library?

## Executive summary

The desktop application's large footprint is primarily outside the Rust core. The full Tauri/CEF process family measured about 1.2-1.4 GiB depending on CEF prewarming and spaCy. Disabling CEF prewarming saved about 86 MiB, and disabling spaCy after that saved another 146 MiB.

The Rust core is much smaller:

- A warmed one-agent roster used 38.7 MiB RSS with default features and 35.5 MiB in a slim build.
- Growing from one to eight warmed agents cost only about 0.40 MiB per additional agent.
- Ingesting 100 representative chat messages retained about 9.3 MiB and completed in about 2.25 seconds.
- A cold chat turn that spawned two real subagents increased RSS by about 26-31 MiB, depending on feature selection and run conditions.
- That subagent increase is overwhelmingly a first-use cost. After one complete warm-up turn, another equivalent turn added only 0.52 MiB with persistence disabled or 1.84 MiB with normal memory capture enabled.

There is no single Rust module holding 45 MiB of live data. In the clean slim-build snapshot at approximately 42 MiB total RSS:

- only 15.2 MiB was private physical footprint;
- only 3.18 MiB was active heap allocation;
- 18.7 MiB was resident executable code from the OpenHuman binary;
- the malloc zones retained 9.4 MiB despite only about 3.2 MiB being live, indicating substantial allocator high-water retention/fragmentation.

The most actionable module-level findings are:

1. Parent and child agents each initialize full memory/SQLite infrastructure.
2. TinyCortex's multilingual PII `RegexSet` and its regex caches dominate the identified live Rust heap growth during normal memory capture.
3. The first turn touches about 15 MiB of previously nonresident OpenHuman executable code.
4. Built-in agent TOML parsing, agent construction, unified-memory construction, and SQLite initialization dominate cold-path CPU.
5. Compile-time feature selection reduces binary size dramatically but reduces live RSS only moderately.

## Scope and methodology

The session used three progressively narrower boundaries:

1. The complete Tauri desktop process family, including CEF and helper processes.
2. The standalone Rust core with no Tauri host.
3. Direct Rust library paths for agent construction, memory ingestion, chat orchestration, and subagent delegation.

All Rust measurements used release builds. Network inference was replaced by a deterministic provider behind the default-off `rss-bench` feature. The subagent scenario still used the real `LongLivedSession`, built-in agent registry, agent builder, `spawn_parallel_agents` tool, two researcher agents, memory infrastructure, prompt enforcement, and post-turn hooks.

RSS was sampled every 5 ms during measured workloads. On macOS, the profiler obtains:

- current RSS from `proc_pid_rusage`;
- peak RSS from `getrusage`;
- thread count from `proc_pidinfo`;
- binary size from the running executable.

macOS does not expose Linux `/proc`-style PSS and private clean/dirty page fields through the same interface, so those JSON fields remain zero. `vmmap`, `heap`, `malloc_history`, Instruments Allocations, and Samply were used for deeper attribution.

Unless stated otherwise, summarized Rust results are medians from five fresh processes. CPU samples from `/usr/bin/time` are representative runs rather than five-run medians.

## Desktop/Tauri measurements

These measurements aggregate the desktop process family rather than only the Rust process.

| Configuration                  |    Mean RAM |           Change |
| ------------------------------ | ----------: | ---------------: |
| Default CEF prewarm and spaCy  | 1,439.8 MiB |         baseline |
| CEF prewarm disabled           | 1,353.5 MiB |        -86.3 MiB |
| CEF prewarm and spaCy disabled | 1,207.6 MiB | -232.2 MiB total |

The clearest desktop optimizations are therefore:

- initialize spaCy lazily only when memory-tree operations require it;
- avoid permanent CEF prewarming, or make the prewarm process short-lived;
- keep accessibility snapshots and `osascript`-based probes event-driven and rate-limited rather than continuously polling.

These desktop results motivated isolating the Rust core: most of the shipped application's memory is not explained by core agent objects.

## Bare Rust agent roster

The existing `rss-bench` binary constructs real OpenHuman agents without Tauri and measures stable RSS in fresh child processes.

### Default feature build

| Roster   |            Median RSS | Threads | Binary size |
| -------- | --------------------: | ------: | ----------: |
| 1 agent  | 39,616 KiB / 38.7 MiB |      22 |   115.9 MiB |
| 8 agents | 42,480 KiB / 41.5 MiB |      24 |   115.9 MiB |

The seven additional agents added 2,864 KiB in total, or approximately 409 KiB per agent. The fixed runtime and linked-code cost is much larger than the marginal agent object cost.

### Slim feature build

The slim binaries were built with:

```bash
GGML_NATIVE=OFF cargo build --release \
  --no-default-features --features rss-bench \
  --bin rss-bench --bin library-profile
```

| Roster   |            Median RSS | Binary size |
| -------- | --------------------: | ----------: |
| 1 agent  | 36,368 KiB / 35.5 MiB |    68.4 MiB |
| 8 agents | 39,584 KiB / 38.7 MiB |    68.4 MiB |

Feature selection reduced the profiling binary by about 40%, but reduced one-agent RSS by only 3.2 MiB. Compile-time gates are very valuable for download and embedding size, but they are not sufficient by themselves to minimize the active working set.

## Rust-only memory ingestion

The `memory-ingest` scenario creates an isolated workspace, disables local inference, Python, spaCy, and embeddings, then sends 100 representative chat messages through the real canonicalization, ingestion, admission, persistence, and memory-queue drain paths.

### Default features

| Metric                 |                   Median |
| ---------------------- | -----------------------: |
| Duration               |                 2,251 ms |
| Baseline RSS           |    16,624 KiB / 16.2 MiB |
| Settled RSS            |    26,144 KiB / 25.5 MiB |
| Retained/peak increase |     9,536 KiB / 9.31 MiB |
| Throughput             | about 44 messages/second |

### Slim features

| Metric                 |                Median |
| ---------------------- | --------------------: |
| Duration               |              2,313 ms |
| Baseline RSS           | 15,536 KiB / 15.2 MiB |
| Settled RSS            | 24,320 KiB / 23.8 MiB |
| Retained/peak increase |  8,784 KiB / 8.58 MiB |

The feature change reduced settled RSS by about 1.8 MiB and had no meaningful throughput benefit.

A representative default-feature `/usr/bin/time -l` run reported 0.18 seconds user CPU and 0.69 seconds system CPU. This should be treated as an upper bound: sampling RSS every 5 ms adds macOS process-inspection system calls, and the workload also performs real temporary-workspace I/O.

## Rust-only subagent chat

The `subagents` scenario uses a deterministic local provider but otherwise follows the real orchestration path:

1. Build a long-lived subconscious session.
2. Submit a promoted chat message.
3. Call the real `spawn_parallel_agents` tool.
4. Spawn two real `researcher` agents with separate ownership scopes.
5. Verify that both child prompts were executed.
6. Measure baseline, peak, and settled RSS.

### Cold default-feature result

| Metric                 |                Median |
| ---------------------- | --------------------: |
| Duration               |                163 ms |
| Baseline RSS           | 18,192 KiB / 17.8 MiB |
| Settled RSS            | 49,712 KiB / 48.5 MiB |
| Retained/peak increase | 31,520 KiB / 30.8 MiB |

A representative process used 0.04 seconds user CPU and 0.06 seconds system CPU. Model and network latency are deliberately excluded.

### Cold slim-feature result

| Metric                 |                Median |
| ---------------------- | --------------------: |
| Duration               |                158 ms |
| Baseline RSS           | 17,072 KiB / 16.7 MiB |
| Settled RSS            | 43,392 KiB / 42.4 MiB |
| Retained/peak increase | 26,304 KiB / 25.7 MiB |

The slim build saved about 6.2 MiB of settled RSS in this richer workload.

## Deep memory attribution

### Clean normal snapshot

A non-instrumented slim-build run with normal memory capture settled at 43,104 KiB RSS, or 42.1 MiB.

| VM/heap measurement                   |        Result |
| ------------------------------------- | ------------: |
| Total RSS                             |      42.1 MiB |
| Private physical footprint            |      15.2 MiB |
| Approximate clean/file-backed portion |      26.9 MiB |
| Live heap allocations                 |      3.18 MiB |
| Resident malloc regions               | about 9.4 MiB |
| Resident stacks                       |      0.97 MiB |
| Resident OpenHuman executable text    |      18.7 MiB |

These categories overlap and must not be added together. For example, active heap and stacks are part of the private footprint, while executable text is part of clean/file-backed RSS.

The key interpretation is that RSS is not equivalent to private heap. The process reports roughly 42 MiB RSS while holding only about 3.2 MiB of live heap allocations.

### First-use executable paging

Before the chat turn, only about 3.3 MiB of the profiling executable's `__TEXT` segment was resident. After the turn, 18.7 MiB was resident. The first turn therefore faulted in approximately 15.4 MiB of OpenHuman's own executable code.

CoreFoundation, Foundation, ICU, Security, CoreAudio, and other macOS framework `__TEXT` residency was effectively identical in the controlled baseline and post-turn snapshots. The increase came from the OpenHuman executable, not from a single newly loaded macOS framework.

Executable pages are clean, file-backed, reclaimable under pressure, and shareable between identical processes. They count toward RSS but are not the same as permanently retained private data.

### Allocator retention

The normal snapshot had about 3.2 MiB of active allocations inside approximately 9.4 MiB of resident malloc pages. Roughly 6 MiB was therefore allocator slack, fragmentation, size-class pages, or high-water retention after the burst of first-turn allocations.

This does not prove a leak. A repeated-turn plateau test is more meaningful than expecting macOS RSS to immediately return to its pre-turn value.

### Memory capture and PII cost

A controlled profiler switch disabled both:

- `memory.auto_save`;
- `learning.episodic_capture_enabled` and its archivist hook.

With those writes disabled, the median cold-turn increase fell from about 25.9 MiB to 22.0 MiB, saving approximately 3.8 MiB.

Stack-logged allocations identified TinyCortex's multilingual PII sanitizer as the dominant live Rust allocation family during normal memory capture. Important allocations originated from:

- `tinycortex::memory::store::safety::pii::SCREEN`;
- the combined `RegexSet` NFA;
- per-thread hybrid-DFA regex caches;
- calls through `sanitize_text`, document upsert, FTS5 episodic insertion, autosave, and the archivist hook.

The `SCREEN` implementation is described as a cheap prefilter, but its large multilingual, Unicode-aware combined automaton is not cheap in retained memory. A byte-oriented candidate scan followed by targeted regex evaluation is a promising replacement.

### Prompt-injection detector

With memory writes disabled, the next visible regex allocation family came from `prompt_injection::detector::DETECTION_RULE_SET`. It was materially smaller than the TinyCortex PII machinery, on the order of a few hundred KiB in this workload.

This is not currently a first-priority RAM optimization, but its scratch/cache strategy should be considered if agent turns become highly parallel.

### Timezone control

`current_datetime_line` normally calls `iana_time_zone`, which uses CoreFoundation on macOS. Forcing the profiling build to use UTC saved only about 0.5-0.6 MiB in the isolated subagent scenario. This is measurable but not a primary explanation for the 42-49 MiB working set.

## Cold-path CPU attribution

A symbolized Samply profile was recorded across repeated no-network, no-memory-write, UTC-controlled runs. Inclusive percentages overlap because a sample contributes to every parent frame in its call stack.

| Cold-path component                    | Approximate inclusive CPU |
| -------------------------------------- | ------------------------: |
| Built-in agent registry initialization |                       17% |
| `LongLivedSession::build_agent`        |                       16% |
| `Agent::from_config_for_agent`         |                       15% |
| Built-in agent TOML parsing/loading    |                       13% |
| Unified-memory construction            |                       10% |
| SQLite/unified-memory initialization   |                        7% |
| Actual TinyAgents turn runner          |                        5% |

The profile also showed `Config::load_or_init`, config serialization/migrations, filesystem synchronization, tool-policy cloning, prompt-injection initialization, and runtime task scheduling.

The built-in registry is initialized before the profiler's RSS baseline, so its CPU appears in the whole-process profile but its already-resident pages are part of the baseline rather than the measured turn delta.

## Warmed-process control

The most important experiment prewarmed one complete two-subagent turn, dropped that warm-up session, then measured a new session performing the same workload in the same process.

| Subsequent equivalent turn          | Median duration |     Median added RSS |
| ----------------------------------- | --------------: | -------------------: |
| Persistence disabled and UTC forced |           46 ms |   528 KiB / 0.52 MiB |
| Normal memory capture               |           60 ms | 1,888 KiB / 1.84 MiB |

One normal-memory repetition was an 8.3 MiB outlier, consistent with asynchronous persistence or allocator behavior; the other four were between about 1.4 and 1.9 MiB.

This result changes the interpretation of the cold 26-31 MiB increase. It is overwhelmingly initialization, code paging, global regex/cache construction, and allocator high-water behavior. It is not a linear 26 MiB cost per chat turn.

## Library-design implications

OpenHuman is feasible as a Rust library, but the current construction path behaves like an application bootstrap rather than a lightweight per-instance library API.

### Share services between agents

`Agent::build_session_agent_inner` constructs session memory and obtains a SQLite connection for the agent. Parent and child agents should instead receive shared instance services such as:

```text
OpenHumanLibrary
  Arc<ConfigSnapshot>
  Arc<AgentDefinitionRegistry>
  Arc<ToolCatalog>
  Arc<MemoryServices>
  Arc<ProviderRegistry>
  Arc<EventBus>
```

Subagents should borrow or clone these `Arc` handles rather than rebuilding configuration, memory stores, schema state, providers, and tool catalogs.

### Offer explicit warm-up

A library API should expose an optional warm-up method that initializes predictable first-use costs:

- built-in agent definitions;
- prompt and PII detectors;
- memory schemas and connection pools;
- tool catalogs and policy snapshots;
- provider routing;
- commonly used prompt fragments.

This lets latency-sensitive hosts choose between low startup work and predictable first-message latency.

### Separate runtime and compile-time slimness

`DomainSet` is useful for runtime surface selection, but runtime-disabled code remains linked. Library consumers need documented compile-time feature recipes or a dedicated library/harness feature set so unrelated domains never enter the binary.

### Avoid mandatory globals

The profiling harness currently has to initialize a global event bus, a global built-in registry, environment-derived workspace selection, and a global provider override. Instance-owned state would make it safer to embed multiple OpenHuman instances in one process and would improve deterministic testing.

## Recommended optimization order

1. **Share unified-memory and SQLite services between parent and child agents.** This is the clearest architectural duplication in the cold path.
2. **Replace the TinyCortex PII `RegexSet` screen with a lightweight candidate scan.** Preserve the strict regex/checksum validation after a candidate is found.
3. **Add a warmed repeated-turn benchmark to CI or the local profiling suite.** Track both cold-start and steady-state behavior so one does not obscure the other.
4. **Provide a supported library-minimal feature recipe.** Measure binary size, cold code paging, and steady RSS for that exact recipe.
5. **Reuse temporary prompt, tool-schema, and sanitizer buffers.** Reduce burst allocation and malloc-zone fragmentation.
6. **Benchmark an alternate allocator only in the profiling binary.** This can reveal how much of the 6 MiB malloc slack is allocator-specific, but the library should not impose a global allocator on consumers.
7. **Add per-phase checkpoints.** Measure config load, agent build, memory construction, prompt render, delegation, child execution, merge, hooks, and teardown separately.

## Functional observation

In the deterministic `LongLivedSession` workload, both parallel child agents executed, but the session returned the last subagent result rather than performing the prepared parent synthesis response. Direct `Agent::turn` tests cover synthesis behavior elsewhere, so the long-lived-session boundary deserves a focused correctness audit. This is separate from the resource findings but was exposed by the same harness.

## Reproducing the measurements

Build the default-feature profiling binaries:

```bash
GGML_NATIVE=OFF cargo build --release --features rss-bench \
  --bin rss-bench --bin library-profile
```

Build the slim versions:

```bash
GGML_NATIVE=OFF cargo build --release \
  --no-default-features --features rss-bench \
  --bin rss-bench --bin library-profile
```

Run the bare roster benchmark:

```bash
target/release/rss-bench --repeat 5 \
  --out target/profile/rust-library/bare-agents.json
```

Run stateful library workloads:

```bash
target/release/library-profile memory-ingest
target/release/library-profile subagents
```

Measure a warmed subagent turn:

```bash
OPENHUMAN_PROFILE_PREWARM_SUBAGENTS=1 \
  target/release/library-profile subagents
```

Isolate orchestration from persistence and local timezone initialization:

```bash
OPENHUMAN_PROFILE_DISABLE_MEMORY_WRITES=1 \
OPENHUMAN_PROFILE_FORCE_UTC=1 \
  target/release/library-profile subagents
```

Hold a process at its baseline or settled state for `vmmap`, `heap`, or `malloc_history`:

```bash
OPENHUMAN_PROFILE_HOLD_BEFORE_SECS=120 \
  target/release/library-profile subagents

OPENHUMAN_PROFILE_HOLD_SECS=120 \
  target/release/library-profile subagents
```

Example live inspection:

```bash
vmmap -summary <pid>
heap -sH <pid>

MallocStackLogging=1 OPENHUMAN_PROFILE_HOLD_SECS=120 \
  target/release/library-profile subagents
malloc_history <pid> -allBySize
```

Record a symbolized CPU profile:

```bash
OPENHUMAN_PROFILE_DISABLE_MEMORY_WRITES=1 \
OPENHUMAN_PROFILE_FORCE_UTC=1 \
samply record --save-only --unstable-presymbolicate \
  --rate 1000 --iteration-count 5 \
  --output target/profile/rust-library/subagents-cpu.json.gz \
  -- target/release/library-profile subagents
```

## Artifacts and code added during the session

- `src/bin/library_profile/main.rs`: hermetic memory-ingestion and subagent workloads, peak RSS sampler, isolation controls, warm-up control, and debugger hold points.
- `src/openhuman/platform/proc_metrics/mod.rs`: macOS RSS, peak RSS, thread-count, and binary-size sampling.
- `src/openhuman/inference/provider/factory.rs`: deterministic provider override enabled under the default-off profiling feature.
- `src/openhuman/inference/provider/ops/provider_factory.rs`: routed-provider support for the same profiling override.
- `src/openhuman/agent/prompts/render_helpers.rs`: profiling-only UTC control under `rss-bench`.
- `target/profile/rust-library/`: ignored JSON, Instruments, and Samply artifacts from local runs.

None of the profiling binaries or provider overrides enter normal shipped builds because they require the default-off `rss-bench` feature.

## Validation completed

- Default-feature release builds of `rss-bench` and `library-profile`.
- Slim release builds using `--no-default-features --features rss-bench`.
- Five-run fresh-process repetitions for the main scenarios.
- macOS process-metric unit test.
- Existing datetime prompt test with `rss-bench` enabled.
- `cargo fmt --check`.
- `git diff --check`.

Repository warnings observed during builds were pre-existing unused-import/dead-code and future-incompatibility warnings; no new build failure was introduced by the profiling harness.

## Bottom line

OpenHuman's Rust core is not holding 45 MiB of agent objects. The steady process is mostly executable working set plus runtime/allocator pages, with a small live heap. The cold first turn is expensive because it initializes and touches a broad application-oriented path. Once warmed, subagent turns are inexpensive and appear to plateau rather than grow linearly.

The best route to an efficient library is therefore not micro-optimizing every agent struct. It is narrowing and sharing the initialization graph: reuse memory and SQLite services, avoid rebuilding agent infrastructure for children, simplify the PII prefilter, expose an explicit warm-up lifecycle, and provide a compile-time library-minimal profile.

---

## Addendum: library benchmarking session (2026-07-22)

The follow-up session turned the manual investigation above into a permanent
benchmark environment, executed two of the recommended optimizations, and
answered the deployment-density question. Full detail lives in
[`library-benchmarking.md`](library-benchmarking.md); this addendum records
the deltas against this document.

### What was built

- **Ten hermetic scenarios** in `library-profile` (was two): `agent-turn`,
  `long-agent`, `workflow`, `subconscious`, `cold-phases`, `fleet`,
  `skill-run`, `subagent-storm` joined `memory-ingest`/`subagents`. All
  offline, mock-provider, JSON schema v2 with per-phase/per-turn checkpoints.
- **Five driver scripts** under `scripts/profile/`: `library-bench.sh`
  (medians + summary), `library-fleet.sh` (agent-count sweep with a
  2 GB/2 vCPU pass/fail gate), `library-instances.sh` (many-processes model),
  `library-cpu.sh` (samply), `library-heap.sh` (dhat).
- **Professional profilers wired in**: dhat behind the default-off
  `rss-bench-dhat` feature; samply scripted; `proc_metrics` extended with
  CPU-time, fd counts, and a descendant process-tree sampler (`tree.rs`).

### Headline results (Apple Silicon, default build unless noted)

| Question | Answer |
| --- | --- |
| Cold turn cost, any shape (chat/subconscious/delegation/workflow) | ~29-30 MiB retained — shared bootstrap, not workload |
| Warmed long-agent growth | 30-150 KiB/turn plateau; occasional 6-8 MiB async persistence bursts |
| Live heap vs RSS (dhat, agent-turn) | 33.4 MB total allocated, 5.0 MB peak live, 3.1 MB at exit |
| Fleet marginal per in-process agent | ~1.7-2.0 MiB (N=50/100/500 sweep) |
| Idle CPU, parked fleet | ~3 ms per 10 s at every N |
| **1000 agents in 2 GiB (one process)** | **PASS — projected ~1747 MiB @ 1000; real 500-agent run settled 1393 MiB** |
| Same workload as N processes | ~48 MiB/instance flat → only ~42 instances per 2 GiB; one-process model is ~25x denser |
| True cost of a JS skill run | node child ~72-75 MB RSS (tree ~121 MB vs ~51 MB self) |
| Marginal per parallel subagent (storm K=8→32) | ~0.78 MiB |
| Library-minimal build (`--no-default-features --features skills,flows`) | 81 MiB binary (60 stripped) vs 116; RSS -3.5 to -4.7 MiB |

### Changes landed

- **PII prefilter replaced upstream** (recommendation 2 above):
  tinycortex#119 swaps the resident 17-pattern `RegexSet` + per-thread DFA
  caches for a single-pass byte candidate scan gating lazily-compiled
  per-class regexes; superset-verified against the old set, no common-path
  heap regression, wins scale with thread/agent concurrency. Merged with
  #120 (unrelated rustdoc fix); submodule bumped.
- **Library-minimal recipe** documented and measured
  ([`library-minimal-recipe.md`](library-minimal-recipe.md)); ranked
  follow-up sheds identified (an `inference` gate for whisper/GGML first).
- **Harness comparison** ([`harness-comparison-2026-07-22.md`](harness-comparison-2026-07-22.md)):
  the scope-matched peer (Hermes, Python) self-reports ~10x our RSS; the
  ZeroClaw "7.8-12 MiB under load" figure has no locatable primary source and
  is now flagged unverified wherever cited. Our in-process ~2 MiB marginal
  scaling has no equivalent among the surveyed harnesses.

### New watch-items surfaced by the fleet sweep

1. **Thread growth ~0.35/agent** (71 @ N=50 → 211 @ N=500, →~420 projected at
   1000). Attribute (SQLite? blocking pool?) and cap before 1000-agent runs.
2. **CPU, not memory, is the load constraint on 2 workers**: p95 turn latency
   25.5 s at N=500 under 200 ms mock latency. Scheduling/backpressure design
   matters more than RAM once the fleet is dense.
3. **Interpreter children break the budget**: pooling/sharing the node and
   python runtimes is filed as tinyhumansai/openhuman#5106 — without it,
   ~25 concurrent JS skill runs exhaust the whole 2 GiB box.

### Updated optimization order

1. Shared services between parent/child agents (unchanged, still first; the
   fleet benchmark is its regression instrument).
2. ~~PII prefilter~~ — done, merged upstream.
3. Runtime pooling for node/python (#5106) — new, promoted to near-top by the
   skill-run measurement.
4. Thread-growth attribution and cap (new).
5. Warm-up API, allocator experiment, per-phase checkpoints in CI — the
   checkpoints now exist (`cold-phases`); CI wiring remains.
6. `inference` compile gate (whisper/GGML) for the library-minimal profile.

### Next: the live desktop app

The same rigor now needs to reach the shipped Tauri/CEF app (the ~1.2-1.4 GiB
family this document opened with). The prepared brief for that session —
scenarios, reusable assets, gates — is
[`tauri-live-profiling-brief.md`](tauri-live-profiling-brief.md).
