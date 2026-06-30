---
description: >-
  TokenJuice - a multi-stage compression router that compacts verbose tool
  output before it ever enters LLM context.
icon: file-zipper
---

# Smart Token Compression

LLM tokens are expensive, and verbose tool output is where most of them go to die. A `git status` in a busy repo, a `cargo build` log, a 600-message email thread, a `docker ps -a` against a real cluster — each can balloon a context window for almost no information gain.

OpenHuman ships with **TokenJuice**, a compression router wired directly into the agent's tool-execution path. Before any tool result reaches a model, TokenJuice classifies it, routes it to a specialized compressor, optionally offloads the full original to a recoverable cache, and records how many tokens (and dollars) it saved.

It began as a port of [vincentkoc/tokenjuice](https://github.com/vincentkoc/tokenjuice) — that JSON rule overlay is still in here as the log/command compressor — but it has since grown into a multi-stage, content-aware pipeline.

***

## The pipeline, step by step

Every blob that flows through `compact_tool_output(...)` takes the same path (`src/openhuman/tokenjuice/compress.rs`):

```text
raw tool result
        │
        ▼
1. Size gate          router enabled? input ≥ min_bytes_to_compress (2 KB)?
        │  yes
        ▼
2. Detect kind        Json · Diff · Html · Search · Code · Log · PlainText
        │
        ▼
3. Select compressor  one specialized compressor per kind (+ per-kind toggles)
        │
        ▼
4. Compress           run it; if it declines or grows the output, fall back / pass through
        │
        ▼
5. CCR eligibility    lossy AND ≥ ccr_min_tokens (≈500)? → offload original to cache
        │
        ▼
6. Append marker      ⟦tj:<hash>⟧ footer so the agent can retrieve the full original
        │
        ▼
7. Record savings     tokens + cost saved, by model and by compressor
        │
        ▼
   compact text → LLM context
```

1. **Size gate.** If the router is disabled or the input is below `min_bytes_to_compress` (default **2048 bytes**), it passes through untouched — tiny outputs aren't worth compressing.
2. **Content detection** (`detect/kind.rs`). The blob is classified into one of seven `ContentKind`s. Precedence: an explicit hint → MIME/extension tag → a per-tool prior (e.g. `grep` → Search, `git_operations` → Diff, `run_tests` → Log) → cheap structural heuristics (JSON → Diff → HTML → Search → Code → Log → PlainText). No regex on the hot path.
3. **Compressor selection.** Each kind routes to a dedicated compressor, honoring per-kind toggles (`search_enabled`, `code_enabled`, `html_enabled`, `ml_compression_enabled`).
4. **Compression.** The compressor runs. If it declines or its output is no smaller than the input, TokenJuice falls back to the generic compressor or passes the original through — it never makes things bigger.
5. **CCR offload.** For **lossy** compressions where the original is large enough (`ccr_min_tokens`, default ~500 tokens), the full original is stowed in the **Compress-Cache-Retrieve** store so nothing is permanently lost.
6. **Recovery marker.** A footer carrying the canonical marker `⟦tj:<hash>⟧` is appended, telling the agent it's looking at a partial view and how to fetch the rest.
7. **Savings accounting.** Tokens saved and estimated cost saved are recorded, attributed by model and by compressor.

***

## The compressors

Each content kind has a purpose-built compressor (`src/openhuman/tokenjuice/compressors/`):

| Compressor       | Kind        | What it does                                                                                              |
| ---------------- | ----------- | -------------------------------------------------------------------------------------------------------- |
| **SmartCrusher** | JSON        | Re-renders arrays of objects as a compact table; past ~40 rows keeps head + tail + error rows + numeric outliers. |
| **Code**         | Code        | Keeps signatures and imports, collapses deep function bodies to `{ … N lines … }` (tree-sitter when available, brace-depth heuristic otherwise). Preserves `TODO`/`FIXME`/`error`/`panic`/`unsafe` markers. |
| **Log**          | Log         | For **command output**, delegates to the JSON rule engine (below). For other logs, keeps errors / warnings / stack traces / summaries and drops the noise. |
| **Search**       | Search      | Groups grep/ripgrep `path:line:body` hits by file, ranks by query-term density, keeps the top matches per file, and tallies `[+N more]`. |
| **Diff**         | Diff        | Keeps changed lines and hunk headers, collapses long unchanged runs to an anchor; lockfile hunks shrink to a one-line `+A/-B` summary. |
| **Html**         | HTML        | Strips markup to readable text with sensible block-boundary newlines and entity decoding — allocation-light, no DOM. |
| **MlText**       | PlainText   | Opt-in ML salience compression (see below).                                                              |
| **Generic**      | fallback    | Head/tail summariser for command output that no specific rule matched; declines on structured blobs so they're preserved. |

Multi-byte text (CJK, emoji, combining marks) is handled grapheme-by-grapheme throughout — never split mid-character.

***

## ML compression (opt-in)

Beyond the deterministic compressors, TokenJuice can route plain text through a **ModernBERT** token-salience model that scores and drops low-information spans (`src/openhuman/tokenjuice/ml/`).

* **Off by default.** Enable with `ml_compression_enabled = true` in `[tokenjuice]`.
* **Runs locally** as the `kompress` backend of the shared Python runtime sidecar — no data leaves your machine.
* **Tunable:** `ml_model_id` (default `answerdotai/ModernBERT-base`), `ml_target_ratio` (default `0.5`), `ml_max_input_chars` (default `200000`), `ml_device` (`cpu`/`auto`), `ml_sidecar_idle_timeout_secs`.
* **Graceful:** if the sidecar is unavailable or an input exceeds the char cap, it degrades to the native compressors without ever failing the agent loop.

***

## Nothing is lost: CCR cache & retrieval

Lossy compression would normally mean throwing data away. TokenJuice instead **offloads** the full original into the **Compress-Cache-Retrieve (CCR)** store and leaves a breadcrumb (`src/openhuman/tokenjuice/cache/`).

* **In-memory tier** (always on): a process-global store keyed by SHA-256 hash, bounded by entry count (`max_cache_entries`, default 256) and total bytes (`max_cache_bytes`, default 64 MiB), FIFO eviction.
* **On-disk tier** (optional): `<workspace>/.tokenjuice/ccr/`, enabled with `ccr_disk_enabled`, survives memory eviction. Optional TTL via `ccr_ttl_secs`.
* **The marker:** compacted output ends with a footer like `[compacted tool output — PARTIAL view; full original available via tokenjuice_retrieve with token "…"]` carrying the `⟦tj:<hash>⟧` token.
* **Retrieval tool:** the agent calls the read-only **`tokenjuice_retrieve`** tool with that token (optionally a byte/line `range`) to pull back the full original or a slice. The token is an unguessable SHA-256 digest.

So the agent gets the cheap compacted view by default, and can transparently "zoom in" on the full text only when it actually needs it.

***

## Savings tracking

Every compression is metered (`src/openhuman/tokenjuice/savings.rs`). TokenJuice tracks events, original vs. compacted tokens, tokens saved, and **estimated cost saved in USD** (using per-model input pricing) — aggregated `total`, `by_model`, and `by_compressor`. Stats persist to `<workspace>/state/tokenjuice_savings.json` and survive restarts.

Read them over RPC with `openhuman.tokenjuice_savings_stats`; clear them with `openhuman.tokenjuice_savings_reset`.

***

## The rule overlay (command & log output)

The original three-layer JSON rule overlay still powers the Log/command compressor. Rules merge in order, later layers overriding earlier ones:

| Layer        | Path                          | Purpose                                                        |
| ------------ | ----------------------------- | ------------------------------------------------------------- |
| **Builtin**  | shipped with the binary       | ~96 vendored rules for git, npm, cargo, docker, kubectl, ls…  |
| **User**     | `~/.config/tokenjuice/rules/` | personal overrides, apply everywhere                          |
| **Project**  | `.tokenjuice/rules/`          | repo-specific overrides, checked in and shared with the team  |

Each rule names a command/tool pattern and a reduction strategy (skip/keep filters, transforms like strip-ANSI and dedupe, head/tail summarize, named counters, canned messages). Rules are JSON — add one and it applies with no recompile.

***

## Configuration, RPC & tools

Everything lives under the `[tokenjuice]` config block (`src/openhuman/config/schema/tokenjuice.rs`) and can be changed live.

* **Master switch:** `router_enabled` (default `true`).
* **Thresholds:** `min_bytes_to_compress`, `ccr_min_tokens`.
* **CCR:** `ccr_enabled`, `ccr_disk_enabled`, `max_cache_entries`, `max_cache_bytes`, `ccr_ttl_secs`.
* **Per-kind:** `search_enabled`, `code_enabled`, `html_enabled`, plus the `ml_*` keys.
* **RPC** (`openhuman.tokenjuice_*`): `detect`, `compress` (dry-run the pipeline), `settings_get` / `settings_update` (live partial patch), `cache_stats`, `retrieve`, `savings_stats`, `savings_reset`.
* **Agent tool:** `tokenjuice_retrieve` (read-only) recovers offloaded originals.
* **Debugging:** start the core with `RUST_LOG=openhuman_core::openhuman::tokenjuice=debug` to watch detection, matching, and how much each blob is trimmed.

***

## Why this matters

Agents live or die by their context budget. A single working session can fan out across dozens of tool calls — greps, builds, test runs, `git` output, and large [web-fetch / scrape](native-tools/web-scraper.md) results the agent pulls down. TokenJuice sits on that tool-execution path and compacts each result before it lands in context, so an agent can sweep a noisy repo or a long web page without each step ballooning the window. The savings compound across a session and are metered in real dollars (see [Billing, Cost & Usage](billing-and-usage.md)).

> **Scope note.** TokenJuice runs on the agent's **tool results**, not on the background [auto-fetch](obsidian-wiki/auto-fetch.md) ingestion pipeline. The 20-minute sync that builds the [Memory Tree](obsidian-wiki/memory-tree.md) has its own canonicalization and chunking and does not route payloads through TokenJuice today.

***

## See also

* [Available Tools](native-tools/README.md) — most heavy tool output flows through TokenJuice.
* [Memory Tree](obsidian-wiki/memory-tree.md) — the downstream consumer of compressed output.
* [Billing, Cost & Usage](billing-and-usage.md) — where token savings show up as real money.
