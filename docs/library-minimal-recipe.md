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

- `GGML_NATIVE=OFF` was the Apple-Silicon dev workaround for whisper-rs/llama.
  With whisper deleted it only matters for llama; on the x86-64 Linux target it
  is unnecessary anyway (the historical always-on whisper build used the
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

The single `default` list this session was written against no longer exists.
There are two sets now (AGENTS.md, "Compile-time domain gates"): **Contrib** is
`[features] default`, what a bare `cargo check` compiles; **Product** is
`scripts/ci/product-features.txt`, what the desktop app ships. Both columns
below are current. `desktop-automation` has since been removed from the tree
altogether, hence the dashes; `tui` is in neither set.

Note how much of this recipe the contributor set already gives you for free —
`voice`, `web3`, `meet` and `tui` are default-OFF today. The Decision column
still records what a **library host** wants, which is the thing this document
is actually for.

| Gate | Contrib | Product | Decision | Why | Deps shed |
| --- | :---: | :---: | :---: | --- | --- |
| `skills` | ON | ON | **KEEP** | python/js `SKILL.md` execution is a stated opencompany use case | none (surface/prompt/startup only) |
| `flows` | ON | ON | **KEEP** | saved-workflow (`flows_create`+`flows_run`) runs are a stated use case | — (adds `tinyflows`, `jaq-*`, `rhai`; see cost note) |
| `voice` | OFF | ON | **DROP** | STT/TTS/dictation/podcast — a headless host does no audio I/O | `hound`, `lettre` |
| `web3` | OFF | ON | **DROP** | crypto wallet / swap / x402 machine payments — not an opencompany path | `bitcoin`, `curve25519-dalek` |
| `media` | ON | ON | **DROP** | `media_generate_*` image/video tools — surface-only | none (backend-proxied) |
| `meet` | OFF | ON | **DROP** | Google-Meet join/live-STT/TTS bot — no headless use | none |
| `mcp` | ON | ON | **DROP** | MCP stdio/HTTP server + Smithery registry (~20k LOC, ~19 tools) — a library host is not an MCP host | none (hand-rolled over tokio/reqwest/axum) |
| `desktop-automation` | — | — | **DROP** | AX / `computer` tool family drives a **local desktop UI** — meaningless headless | `uiautomation` |
| `tui` | OFF | — | **DROP** | `openhuman tui`/`chat` terminal UI — no terminal in a library host | `ratatui`, `crossterm`, `unicode-width` |

**Non-default optional features** (`sandbox-landlock`, `sandbox-bubblewrap`,
`peripheral-rpi`, `browser-native`/`fantoccini`, `landlock`, `whatsapp-web`,
`e2e-test-support`, `rss-bench`, `rss-bench-dhat`) are all default-OFF, so a
`--no-default-features` build never links them unless explicitly added. None are
needed for opencompany; `rss-bench`/`rss-bench-dhat` are dev/benchmark-only.

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
  tools absent; the x402 402-retry path returns unpaid.
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

1. ~~**`inference` gate → shed `whisper-rs` + `whisper-rs-sys` (+ `cpal`/`coreaudio`).**~~
   **DONE, and better than proposed.** The bundled whisper.cpp STT engine was not
   gated — it was **deleted**. `whisper-rs` / `whisper-rs-sys` (and the
   `[patch.crates-io] whisper-rs-sys` fork entries in both Cargo worlds) are gone
   from every build, not just the slim one, and with them the whisper.cpp + GGML
   C++ static link that was the reason for the `GGML_NATIVE=OFF` build dance.
   Speech-to-text is a hosted call now, with the engine chosen by
   `voice_server.stt_engine` (see the AGENTS.md scope note). The `inference`
   feature survives with a narrower job: it gates `cpal` alone, which is what a
   headless library host wanted shed anyway.

2. **Split `rhai` out of the `flows` gate.** `flows` is the most expensive domain
   we *keep* (+12.7 MiB, dominated by `rhai 1.25` — a full scripting engine).
   `rhai` arrives only via `tinyagents/repl`, which powers the `.ragsh`
   language-workflow tool (`rhai_workflows`). If opencompany needs `tinyflows`
   saved-graph runs but **not** the `.ragsh` rhai tool, splitting `rhai_workflows`
   into its own sub-gate would reclaim most of that 12.7 MiB while keeping the
   flows graph engine. Currently all-or-nothing.

3. ~~**`git2` (vendored libgit2).**~~ — **no longer applicable.** This entry
   proposed gating the git-backed `memory_diff` change ledger. That went further:
   the `memory-git` gate, the `memory::diff` RPC surface and the `memory_diff`
   agent tool were deleted outright, so `git2` — with `libgit2-sys` and
   `libz-sys` — is absent from every profile rather than merely gateable.
   `cargo tree -i git2` finds no package. tinycortex still owns the only libgit2
   code in the stack and keeps its `git-diff` / `wiki-git` features; nothing in
   this repository enables them.

4. **`reqwest` dual TLS backends.** The root `reqwest` enables both `rustls-tls`
   **and** `native-tls` — two full TLS stacks linked simultaneously. A headless
   host on a known target could pick one, shedding the other.

5. ~~**Node/Python runtime bootstrap deps**~~ — **no longer applicable.**
   Downloading and unpacking language toolchains moved into the `tinyruntime`
   module, so `xz2` and its liblzma build left this manifest entirely. `tar`,
   `zip`, and `flate2` remain, but for the Piper voice installer and the document
   tools rather than for any runtime bootstrap; they are sheddable with those
   features, not with `skills`.

## See also

- [`docs/library-benchmarking.md`](library-benchmarking.md) — the benchmark
  environment, scenario definitions, and default/slim baselines.
- [`docs/resource-profiling-session-2026-07-21.md`](resource-profiling-session-2026-07-21.md)
  — deep memory/CPU attribution (why RSS is mostly not live heap).
- AGENTS.md "Compile-time domain gates" — the per-gate behavior and dependency notes.
