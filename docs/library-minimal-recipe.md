# Library-minimal feature recipe

A **supported, measured** compile-time feature recipe for embedding the OpenHuman
Rust core as a library in "opencompany" — headless, no RPC server, no Tauri
shell, targeting 100-1000 live agents in a 2 GB RAM / 2 vCPU box.

It follows the repo's existing slim convention (`cargo build --no-default-features
--features "<explicit list>"`, see AGENTS.md "Compile-time domain gates") and keeps
only the domains the opencompany use cases actually exercise: **agent turns,
subagent delegation, memory ingest, workflow (flows) runs, and python/js skill
execution.**

## The build command

Opencompany recipe (production embed — no benchmark/harness code):

```bash
GGML_NATIVE=OFF cargo build --release \
  --no-default-features --features "skills,flows"
```

- `GGML_NATIVE=OFF` is the Apple-Silicon dev workaround for whisper-rs/llama; on
  the x86-64 Linux target it is unnecessary (the always-on whisper build uses the
  AVX path). Keep it in the command for macOS developers.
- To build the profiling harness against the same recipe, add the dev-only
  `rss-bench` feature and the two bench bins:

  ```bash
  GGML_NATIVE=OFF cargo build --release \
    --no-default-features --features "rss-bench,skills,flows" \
    --bin library-profile --bin rss-bench
  ```

There is **no** `library-minimal` meta-feature in `Cargo.toml`, on purpose — see
[Why no alias](#why-no-cargotoml-alias) below.

## Keep / drop table

`default = ["tokenjuice-treesitter","voice","web3","media","meet","skills","flows","mcp","desktop-automation","tui"]`

| Gate | Default | Decision | Why | Deps shed |
| --- | :---: | :---: | --- | --- |
| `skills` | ON | **KEEP** | python/js `SKILL.md` execution is a stated opencompany use case | none (surface/prompt/startup only) |
| `flows` | ON | **KEEP** | saved-workflow (`flows_create`+`flows_run`) runs are a stated use case | — (adds `tinyflows`, `jaq-*`, `rhai`; see cost note) |
| `tokenjuice-treesitter` | ON | **DROP** | AST-aware code compression → gracefully falls back to the brace-depth heuristic. Functional, only compression *quality* degrades. | tree-sitter Rust/TS/Python grammars + their C build |
| `voice` | ON | **DROP** | STT/TTS/dictation/podcast — a headless host does no audio I/O | `hound`, `lettre` |
| `web3` | ON | **DROP** | crypto wallet / swap / x402 machine payments — not an opencompany path | `bitcoin`, `curve25519-dalek` |
| `media` | ON | **DROP** | `media_generate_*` image/video tools — surface-only | none (backend-proxied) |
| `meet` | ON | **DROP** | Google-Meet join/live-STT/TTS bot — no headless use | none |
| `mcp` | ON | **DROP** | MCP stdio/HTTP server + Smithery registry (~20k LOC, ~19 tools) — a library host is not an MCP host | none (hand-rolled over tokio/reqwest/axum) |
| `desktop-automation` | ON | **DROP** | AX / `computer` tool family drives a **local desktop UI** — meaningless headless | `uiautomation` |
| `tui` | ON | **DROP** | `openhuman tui`/`chat` terminal UI — no terminal in a library host | `ratatui`, `crossterm`, `unicode-width` |

**Non-default optional features** (`sandbox-landlock`, `sandbox-bubblewrap`,
`peripheral-rpi`, `browser-native`/`fantoccini`, `landlock`, `whatsapp-web`,
`e2e-test-support`, `rss-bench`, `rss-bench-dhat`) are all default-OFF, so a
`--no-default-features` build never links them unless explicitly added. None are
needed for opencompany; `rss-bench`/`rss-bench-dhat` are dev/benchmark-only.

### On `tokenjuice-treesitter`

This is the one judgment call. Dropping it removes the largest *native C build*
in the domain gates (three tree-sitter grammars) and shrinks the binary, at the
cost of coarser code-context compression (brace-depth heuristic instead of AST).
For a memory/binary-minimal library host it is dropped here. **If token budget
per agent turn matters more than binary size, add `tokenjuice-treesitter` back**
— it sheds no runtime behavior beyond compression fidelity.

## Measured results

All numbers gathered on this branch, Apple-Silicon macOS, `--release` profile
(`optimized + debuginfo`). "default" = the prior 2026-07-21 session baselines in
[`docs/library-benchmarking.md`](library-benchmarking.md); "pure slim" =
`--no-default-features --features rss-bench` (drops everything). Both slim numbers
were reproduced on this machine and match the prior doc exactly (68.4 MiB).

### Binary size

| Build | Features | Unstripped | Stripped |
| --- | --- | ---: | ---: |
| default | (all gates) | 115.9 MiB¹ | — |
| **library-minimal** | `skills,flows` | **~81.1 MiB** | **~60.4 MiB** |
| pure slim | (none) | 68.4 MiB | 51.0 MiB |

¹ from the prior session (unstripped, same profile). library-minimal bins measured
directly: `rss-bench` 81.1 MiB, `library-profile` 83.0 MiB unstripped (the extra
~2 MiB is the harness itself). The domain recipe (`skills,flows`, no `rss-bench`)
matches the `rss-bench` figure — the bench feature adds negligible code.

- **library-minimal vs default: -34.8 MiB (~30% smaller)**, and a correspondingly
  narrower code-paging surface (the dominant cold-turn RSS factor per the prior
  session's executable-paging finding).
- **library-minimal vs pure slim: +12.7 MiB unstripped / +9.4 MiB stripped — all
  of it `flows`.** `cargo tree` confirms the delta is `rhai 1.25` + `rhai_codegen`
  + `jaq-core/std/json` + `tinyflows`; `skills` sheds **zero** deps (its value is
  tool-surface/prompt/startup, not size). `flows` is by far the most expensive
  domain we *keep* — see follow-up #2.

### Per-scenario RSS (5 fresh-process repeats, median, `OPENHUMAN_PROFILE_FORCE_UTC=1`)

| Scenario | minimal settled | minimal retained Δ | default settled² | default retained² | Δ settled |
| --- | ---: | ---: | ---: | ---: | ---: |
| `agent-turn` (cold, 1 turn) | 44.0 MiB | 26.6 MiB | 47.6 MiB | 29.5 MiB | **-3.6 MiB** |
| `subagents` (cold, 2 children) | 44.5 MiB | 27.1 MiB | 48.0 MiB | 29.9 MiB | **-3.5 MiB** |
| `workflow` (`flows_create`+`flows_run`) | 46.2 MiB | 26.0 MiB | 50.9 MiB | 29.9 MiB | **-4.7 MiB** |
| `memory-ingest` (100 msgs) | 24.7 MiB | 8.8 MiB | 25.8 MiB | 9.3 MiB | **-1.1 MiB** |
| `long-agent` (10 turns) | 46.4 MiB | 2.9 MiB | — (25-turn: 65.8 MiB) | — | n/a³ |

² default column from `docs/library-benchmarking.md` (2026-07-21). Those medians
may not have used `OPENHUMAN_PROFILE_FORCE_UTC=1`, so treat the Δ as approximate
(±~1 MiB). The direction and magnitude match the prior session's "slim saves
~3.2 MiB settled RSS" finding.

³ `long-agent` was run at 10 turns here vs 25 in the default baseline, so the
absolute settled figures aren't comparable. The low 2.9 MiB retained Δ confirms
per-turn growth plateaus (matches the prior "not linear" observation).

**Takeaway (consistent with the prior session):** compile-time gates shrink the
*binary* substantially (-30%) but move *settled RSS* by only ~3-5 MiB per
scenario. Most of the RSS story is initialization + allocator high-water, not
linked code size. The binary/code-paging win is the primary reason to prefer this
recipe; the RSS win is real but secondary.

## What is functionally absent in this build

Summarized from the per-gate behavior notes in AGENTS.md. Dropped domains fail
*closed and cleanly* — controllers become unknown-method, tools are simply absent
from the tool list (not degraded to runtime errors), CLI subcommands report a
build-fact error:

- **voice/audio:** voice + audio controllers unregistered (unknown-method over
  RPC, absent from `/schema`); `audio_generate_podcast` tools absent; `openhuman
  voice` returns "voice disabled".
- **web3:** wallet / web3 / x402 controllers unregistered; swap/bridge/dapp agent
  tools absent; the x402 402-retry path returns unpaid; tinyplace on-chain
  payments + Polymarket *writes* degrade to graceful "wallet disabled" errors
  (tinyplace comms + ed25519 signing are unaffected).
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
- **tokenjuice-treesitter:** code compression falls back to the brace-depth
  heuristic — degraded fidelity, not absent.

Everything the opencompany use cases need remains: the agent harness + turn
runner, subagent delegation (`spawn_parallel_agents`), the full memory stack
(TinyCortex store/tree/queue/ingest + PII/injection detectors), threads, config,
security policy, provider routing/inference, `skills` (SKILL.md discovery/install
+ node/python execution + `run_workflow`/`await_workflow`), and `flows` (saved
graph create/run/schedule + `workflow_builder`/`flow_discovery` agents).

## Test verification

The disabled-build test gotcha (AGENTS.md: CI's smoke lane runs `cargo check`
only and never compiles `--no-default-features` test code) was checked directly:

```bash
GGML_NATIVE=OFF cargo test --lib --no-default-features --features "skills,flows" core::
# result: ok. 660 passed; 0 failed; 1 ignored; 10513 filtered out
```

The both-ways gate tests in `src/core/all_tests.rs` (which assert dropped domains
become unknown-method) pass under this recipe. No pre-existing failures.

## CI note

Nothing is added to the `default` feature list — this is a **subtractive**
`--no-default-features` recipe, not a new default-ON gate. The **Feature
Forwarding Gate** (`scripts/ci/check-feature-forwarding.mjs`) only inspects the
`default` list and its forwarding into the desktop shell's `Cargo.toml`, so it
**does not apply** here: there is nothing to forward. This recipe carries no CI
risk and needs no `INTENTIONALLY_NOT_FORWARDED` entry.

## Why no `Cargo.toml` alias

The repo convention (AGENTS.md "Slim-profile convention") is deliberate: **no
`full` meta-feature; build slim variants with an explicit feature list.** A
`library-minimal = ["skills","flows"]` alias would be convenient, but it:

- duplicates the `default` list's maintenance burden — a new default-ON gate that
  opencompany *should* pick up would silently be missing from a frozen alias
  (the exact failure mode the "no meta-feature" rule exists to avoid), and
- hides the subtractive intent behind a name, making the drop set invisible at
  the call site.

**Recommendation: document the explicit list (this file), do not add the alias.**
If maintainers later decide an alias is worth it, the minimal-drift option is to
express it *subtractively* in tooling rather than as a frozen additive list —
but that is a follow-up decision, not part of this recipe.

## Follow-up shed list (ranked)

Largest remaining always-on costs a headless library host does not need. These
are **not implemented here** — they require new gates/refactors — listed for
prioritization.

1. **`inference` gate → shed `whisper-rs` + `whisper-rs-sys` (+ `cpal`/`coreaudio`).**
   `whisper-rs-sys` statically links the whisper.cpp + GGML C++ inference library
   — the single largest always-on *native* chunk in the binary and the reason for
   the `GGML_NATIVE=OFF` build dance. `cargo tree` confirms `whisper-rs 0.16` is a
   **direct always-on dependency of `openhuman`** (not gated by `voice`, per the
   AGENTS.md scope note), pulling `whisper-rs-sys 0.15`; `cpal 0.15` + `coreaudio`
   ride alongside for audio capture. A headless library host does no local STT, so
   an `inference` gate would shed all of this — the biggest remaining binary +
   native-build win by far. Bonus: `cpal` is shared only with `accessibility`,
   which `desktop-automation` (already dropped here) owns — so with this recipe,
   `cpal` becomes sheddable the moment the inference gate lands.
   *(No `llama`/`candle`/`tokenizers`/`onnx` crates appear in the recipe's tree, so
   the local-LLM path is either already optional or absent — whisper is the target.)*

2. **Split `rhai` out of the `flows` gate.** `flows` is the most expensive domain
   we *keep* (+12.7 MiB, dominated by `rhai 1.25` — a full scripting engine).
   `rhai` arrives only via `tinyagents/repl`, which powers the `.ragsh`
   language-workflow tool (`rhai_workflows`). If opencompany needs `tinyflows`
   saved-graph runs but **not** the `.ragsh` rhai tool, splitting `rhai_workflows`
   into its own sub-gate would reclaim most of that 12.7 MiB while keeping the
   flows graph engine. Currently all-or-nothing.

3. **`git2` (vendored libgit2).** Always-on native dependency of the `memory_diff`
   change-ledger (git-backed snapshots/checkpoints/diffs). A large vendored C lib.
   If a library host does not need git-backed memory diffs, this is a candidate for
   a future gate.

4. **`reqwest` dual TLS backends.** The root `reqwest` enables both `rustls-tls`
   **and** `native-tls` — two full TLS stacks linked simultaneously. A headless
   host on a known target could pick one, shedding the other.

5. **Node/Python runtime bootstrap deps** (`tar`, `xz2`+liblzma, `zip`, `flate2`).
   Only needed if `skills`/`flows` actually execute node/python workloads; kept
   here because `skills` is on. If a deployment runs only pure-LLM skills, these
   archive/decompression deps become sheddable.

## See also

- [`docs/library-benchmarking.md`](library-benchmarking.md) — the benchmark
  environment, scenario definitions, and default/slim baselines.
- [`docs/resource-profiling-session-2026-07-21.md`](resource-profiling-session-2026-07-21.md)
  — deep memory/CPU attribution (why RSS is mostly not live heap).
- AGENTS.md "Compile-time domain gates" — the per-gate behavior and dependency notes.
