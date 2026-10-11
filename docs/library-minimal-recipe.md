# Library-minimal feature recipe

> **Note.** The bench bins (`library-profile`, `rss-bench`) referenced below
> moved to the openhuman-benchmarks repository (#6944). The `openhuman-cli`
> `rss-bench` / `rss-bench-dhat` gates are gone; the core keeps its own
> `rss-bench` hook for that repository.

A **supported, measured** compile-time feature recipe for embedding the OpenHuman
Rust core as a library in "opencompany" — headless, no RPC server, no Tauri
shell, targeting 100-1000 live agents in a 2 GB RAM / 2 vCPU box.

It follows the repo's existing slim convention (`cargo build --no-default-features
--features "<explicit list>"`, see `crates/openhuman-core/README.md` "Feature flags" and the policy comments above `[features]` in `crates/openhuman-core/Cargo.toml`) and keeps
only the domains the opencompany use cases actually exercise: **agent turns,
subagent delegation, memory ingest, workflow (flows) runs, and python/js skill
execution.**

## The build command

OpenCompany's hosted-backend recipe:

```bash
cargo build --release \
  -p openhuman-tinyhumans \
  --no-default-features --features "skills,flows"
```

Use the same feature selection in the embedding application's manifest:

```toml
[dependencies]
openhuman-tinyhumans = { git = "https://github.com/tinyhumansai/openhuman", default-features = false, features = ["skills", "flows"] }
```

`openhuman_tinyhumans::RuntimeBuilder` installs the SDK-backed transport and
registers the hosted controllers when it builds the runtime. This connects the
API key to managed inference, integrations, channel relay and cloud voice when
their feature and runtime gates are enabled.

- To build the profiling harness against the same recipe, use the `profile/`
  crate in [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks), which vendors this repository:

  ```bash
  cargo build --release --manifest-path profile/Cargo.toml \
    --no-default-features --features "rss-bench,skills,flows" \
    --bin library-profile --bin rss-bench
  ```

There is **no** `library-minimal` meta-feature in `Cargo.toml`, on purpose — see
[Why no alias](#why-no-cargotoml-alias) below.

Usage against this recipe is the two-step API: one `Runtime` per process,
then agents on it. `DomainSet` families are registered at runtime build time
and agents can only narrow them, so include `mcp` / `skills` in both the
Cargo features **and** the runtime's `DomainSet` (the builder's default does)
if any agent will declare servers or skills.

See the runnable [two_agents example](../crates/openhuman-embed/examples/two_agents.rs), region `two_agents`. It uses the public facade and asserts its behavior against an offline stub.

### Choosing the backend layer

Use `openhuman_tinyhumans::RuntimeBuilder` for a host that uses the hosted
TinyHumans backend. The builder forwards the embedding options and binds the
transport before startup, as shown in its [compiled quick-start
example](../crates/openhuman-tinyhumans/src/lib.rs).

Use `openhuman_embed::Runtime::builder()` for a standalone host with its own
inference provider and local tools. Agent turns, memory operations (on a CortexDB engine or an installed host engine) and
enabled skills and flows remain available through their configured providers.
An API key on that builder stores a credential; the host must also supply a
backend transport to reach TinyHumans. Backend requests otherwise return
`BACKEND_UNAVAILABLE:`, and the hosted `billing`, `team`, `referral` and
`announcements` controllers are absent.

For a connected host that manages its own RPC catalog, set
`.hosted_controllers(false)` on the TinyHumans builder to skip hosted controller
registration. `.domains(DomainSet::embedded())` keeps the `hosted` domain gate
off, so it also excludes those controllers from dispatch. The backend transport
still serves agent and integration requests in either configuration.

## Keep / drop table

The table below records the July 2026 recipe, before the backend extraction in
#6362. **Contrib** refers to the recorded `[features] default` set; **Product**
refers to the recorded desktop feature set. Use
`crates/openhuman-core/README.md` and `scripts/ci/product-features.txt` for the
current sets. The dashes reflect gates outside the recorded set.

In that contributor set, `voice`, `web3`, `meet` and `tui` were default-OFF.
The Decision column records the domains needed by the headless library host.

| Gate                 | Contrib | Product | Decision | Why                                                                                                 | Deps shed                                            |
| -------------------- | :-----: | :-----: | :------: | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| `skills`             |   ON    |   ON    | **KEEP** | python/js `SKILL.md` execution is a stated opencompany use case                                     | none (surface/prompt/startup only)                   |
| `flows`              |   ON    |   ON    | **KEEP** | saved-workflow (`flows_create`+`flows_run`) runs are a stated use case                              | — (adds `tinyflows`, `jaq-*`, `rhai`; see cost note) |
| `voice`              |   OFF   |   ON    | **DROP** | STT/TTS/dictation — a headless host does no audio I/O                                       | Loaded TinyVoice module                                    |
| `web3`               |   OFF   |   ON    | **DROP** | crypto wallet / swap / x402 machine payments — not an opencompany path                              | `bitcoin`, `curve25519-dalek`                        |
| `media`              |   ON    |   ON    | **DROP** | `media_generate_*` image/video tools — surface-only                                                 | none (backend-proxied)                               |
| `meet`               |   OFF   |   ON    | **DROP** | Google-Meet join/live-STT/TTS bot — no headless use                                                 | none                                                 |
| `mcp`                |   ON    |   ON    | **DROP** | MCP stdio/HTTP server + Smithery registry (~20k LOC, ~19 tools) — a library host is not an MCP host | none (hand-rolled over tokio/reqwest/axum)           |
| `desktop-automation` |    —    |    —    | **DROP** | AX / `computer` tool family drives a **local desktop UI** — meaningless headless                    | `uiautomation`                                       |
| `tui`                |   OFF   |    —    | **DROP** | `openhuman tui`/`chat` terminal UI — no terminal in a library host                                  | `ratatui`, `crossterm`, `unicode-width`              |

The historical recipe also left optional `browser-native`/`fantoccini`,
`whatsapp-web`, `e2e-test-support`, `rss-bench` and `rss-bench-dhat` off.
Check the selected host crate's current feature list before adding optional
domains. `rss-bench` and `rss-bench-dhat` remain development profiling features.

## Measured results

These are historical measurements from the July 2026 feature-shed work,
before the backend extraction in #6362. They describe the earlier core build;
measure the current `openhuman-embed` and `openhuman-tinyhumans` builds separately
when sizing a deployment. The hosted layer adds the SDK transport and hosted
controllers, so these figures serve as a dated reference.

All numbers were gathered on Apple-Silicon macOS, `--release` profile
(`optimized + debuginfo`). "default" = the prior 2026-07-21 session baselines in
[`library-benchmarking.md`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md); "pure slim" =
`--no-default-features --features rss-bench` (drops everything). Both slim numbers
matched the prior document's 68.4 MiB baseline in that measurement session.

### Binary size

| Build               | Features       |    Unstripped |      Stripped |
| ------------------- | -------------- | ------------: | ------------: |
| default             | (all gates)    |    115.9 MiB¹ |             — |
| **library-minimal** | `skills,flows` | **~81.1 MiB** | **~60.4 MiB** |
| pure slim           | (none)         |      68.4 MiB |      51.0 MiB |

¹ from the prior session (unstripped, same profile). library-minimal bins measured
directly: `rss-bench` 81.1 MiB, `library-profile` 83.0 MiB unstripped (the extra
~2 MiB is the harness itself). The domain recipe (`skills,flows`, no `rss-bench`)
matches the `rss-bench` figure — the bench feature adds negligible code.

- **library-minimal vs default: -34.8 MiB (~30% smaller)**, and a correspondingly
  narrower code-paging surface (the dominant cold-turn RSS factor per the prior
  session's executable-paging finding).
- **library-minimal vs pure slim: +12.7 MiB unstripped / +9.4 MiB stripped — all
  of it `flows`.** At the time, `cargo tree` attributed the delta to `rhai 1.25`,
  `rhai_codegen`, `jaq-core/std/json` and `tinyflows`; `skills` shed **zero** deps
  (its value is tool-surface/prompt/startup, not size). `flows` was the most
  expensive retained domain — see follow-up #2.

### Per-scenario RSS (5 fresh-process repeats, median, `OPENHUMAN_PROFILE_FORCE_UTC=1`)

| Scenario                                | minimal settled | minimal retained Δ |      default settled² | default retained² |    Δ settled |
| --------------------------------------- | --------------: | -----------------: | --------------------: | ----------------: | -----------: |
| `agent-turn` (cold, 1 turn)             |        44.0 MiB |           26.6 MiB |              47.6 MiB |          29.5 MiB | **-3.6 MiB** |
| `subagents` (cold, 2 children)          |        44.5 MiB |           27.1 MiB |              48.0 MiB |          29.9 MiB | **-3.5 MiB** |
| `workflow` (`flows_create`+`flows_run`) |        46.2 MiB |           26.0 MiB |              50.9 MiB |          29.9 MiB | **-4.7 MiB** |
| `memory-ingest` (100 msgs)              |        24.7 MiB |            8.8 MiB |              25.8 MiB |           9.3 MiB | **-1.1 MiB** |
| `long-agent` (10 turns)                 |        46.4 MiB |            2.9 MiB | — (25-turn: 65.8 MiB) |                 — |         n/a³ |

² default column from openhuman-benchmarks' `profile/docs/library-benchmarking.md` (2026-07-21). Those medians
may not have used `OPENHUMAN_PROFILE_FORCE_UTC=1`, so treat the Δ as approximate
(±~1 MiB). The direction and magnitude match the prior session's "slim saves
~3.2 MiB settled RSS" finding.

³ `long-agent` was run at 10 turns here vs 25 in the default baseline, so the
absolute settled figures aren't comparable. The low 2.9 MiB retained Δ confirms
per-turn growth plateaus (matches the prior "not linear" observation).

**Takeaway (consistent with the prior session):** compile-time gates shrink the
_binary_ substantially (-30%) but move _settled RSS_ by only ~3-5 MiB per
scenario. Most of the RSS story is initialization + allocator high-water, not
linked code size. The binary/code-paging win is the primary reason to prefer this
recipe; the RSS win is real but secondary.

## What is functionally absent in this build

Summarized from the per-gate comments in `crates/openhuman-core/Cargo.toml`. Dropped domains fail
_closed and cleanly_ — controllers become unknown-method, tools are simply absent
from the tool list (not degraded to runtime errors), CLI subcommands report a
build-fact error:

- **voice:** voice controllers unregistered (unknown-method over RPC, absent
  from `/schema`); `openhuman voice` returns "voice disabled".
- **web3:** wallet / web3 / x402 controllers unregistered; swap/bridge/dapp agent
  tools absent; the x402 402-retry path returns unpaid; tinyplace on-chain
  payments degrade to graceful "wallet disabled" errors (tinyplace comms +
  ed25519 signing are unaffected).
- **media:** `media_generate_*` agent tools absent.
- **meet:** meet controllers unregistered; live Meet bot / STT-LLM-TTS loop absent.
- **mcp:** `mcp_server` / `mcp_registry` (`mcp_clients` namespace) / `mcp_audit`
  controllers unknown-method; ~19 MCP agent tools absent; `openhuman mcp` CLI
  returns a "rebuild with --features mcp" build-fact error. (`McpHttpClient` +
  `sanitize` stay compiled — the gitbooks docs tool and the orchestrator prompt
  sanitizer still work.)
- **desktop-automation:** `accessibility` / `autocomplete`
  / `desktop_companion` domains + the `computer` tool family (`ax_interact`,
  `automate`, mouse/keyboard) absent.
- **tui:** `openhuman tui` / `chat` returns "tui feature disabled at compile time".

Everything the opencompany use cases need remains: the agent harness + turn
runner, subagent delegation (`spawn_parallel_agents`), the Memory v2 host
(engine binding, sources, conversations, context + secret/PII scrubbing), threads, config,
security policy, provider routing/inference, `skills` (SKILL.md discovery/install

- node/python execution + `run_workflow`/`await_workflow`), and `flows` (saved
  graph create/run/schedule + `workflow_builder`/`flow_discovery` agents).

## Test verification

The disabled-build test gotcha (AGENTS.md says to test both enabled and disabled
builds after changing a gate; CI's `cargo check` lanes never compile
`--no-default-features` test code) was checked directly:

```bash
cargo test -p openhuman --lib --no-default-features --features "skills,flows" core::
# result: ok. 660 passed; 0 failed; 1 ignored; 10513 filtered out
```

The both-ways gate tests in `crates/openhuman-core/src/core/all_tests.rs` (which assert dropped domains
become unknown-method) pass under this recipe. No pre-existing failures.

## CI note

Nothing is added to the `default` feature list — this is a **subtractive**
`--no-default-features` recipe, not a new default-ON gate. The **Feature
Forwarding Gate** (`scripts/ci/check-feature-forwarding.mjs`) inspects the
`default` list, its forwarding into the desktop shell's `Cargo.toml`, and the
`openhuman-embed` → `openhuman-tinyhumans` → `openhuman-cli` chain — all of
which are about gates that exist, so the gate **does not apply** here: there is
nothing to forward. This recipe carries no CI risk and needs no
`INTENTIONALLY_NOT_FORWARDED` entry.

## Why no `Cargo.toml` alias

The repo convention (the `[features]` policy comments in `crates/openhuman-core/Cargo.toml`) is deliberate: **no
`full` meta-feature; build slim variants with an explicit feature list.** A
`library-minimal = ["skills","flows"]` alias would be convenient, but it:

- duplicates the `default` list's maintenance burden — a new default-ON gate that
  opencompany _should_ pick up would silently be missing from a frozen alias
  (the exact failure mode the "no meta-feature" rule exists to avoid), and
- hides the subtractive intent behind a name, making the drop set invisible at
  the call site.

**Recommendation: document the explicit list (this file), do not add the alias.**
If maintainers later decide an alias is worth it, the minimal-drift option is to
express it _subtractively_ in tooling rather than as a frozen additive list —
but that is a follow-up decision, not part of this recipe.

## Follow-up shed list (ranked)

Largest remaining always-on costs a headless library host does not need. These
are **not implemented here** — they require new gates/refactors — listed for
prioritization.

1. ~~**`inference` gate → shed `whisper-rs` + `whisper-rs-sys` (+ `cpal`/`coreaudio`).**~~
   **DONE, and better than proposed.** The bundled whisper.cpp STT engine was not
   gated — it was **deleted**. `whisper-rs` / `whisper-rs-sys` (and the
   `[patch.crates-io] whisper-rs-sys` fork entries in both Cargo worlds) are gone
   from every build, not just the slim one, and with them the whisper.cpp + GGML
   C++ static link that previously required a platform-specific build workaround.
   Speech-to-text is a hosted call now, with the engine chosen by
   `voice_server.stt_engine` (see the `inference` gate comment in `crates/openhuman-core/Cargo.toml`). The `inference`
   feature survives with a narrower job: it gates `cpal` alone, which is what a
   headless library host wanted to shed anyway.

2. **Split `rhai` out of the `flows` gate.** `flows` is the most expensive domain
   we _keep_ (+12.7 MiB, dominated by `rhai 1.25` — a full scripting engine).
   `rhai` arrives only via `tinyagents/repl`, which powers the `.ragsh`
   language-workflow tool (`rhai_workflows`). If opencompany needs `tinyflows`
   saved-graph runs but **not** the `.ragsh` rhai tool, splitting `rhai_workflows`
   into its own sub-gate would reclaim most of that 12.7 MiB while keeping the
   flows graph engine. Currently all-or-nothing.

3. **`reqwest` dual TLS backends.** The root `reqwest` enables both `rustls-tls`
   **and** `native-tls` — two full TLS stacks linked simultaneously. A headless
   host on a known target could pick one, shedding the other.

4. **Node/Python runtime bootstrap deps** (`tar`, `xz2`+liblzma, `zip`, `flate2`).
   Only needed if `skills`/`flows` actually execute node/python workloads; kept
   here because `skills` is on. If a deployment runs only pure-LLM skills, these
   archive/decompression deps become sheddable.

## See also

- [`library-benchmarking.md`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md) (openhuman-benchmarks) — the benchmark
  environment, scenario definitions, and default/slim baselines.
- The original profiling session write-up covering deep memory/CPU attribution
  (why RSS is mostly not live heap) was removed from the tree; see git history
  at `0017c58d86~1`.
- `crates/openhuman-core/README.md` "Feature flags" and the per-gate comments in `crates/openhuman-core/Cargo.toml` — the per-gate behavior and dependency notes.
