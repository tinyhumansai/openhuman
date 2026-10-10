---
description: >-
  TokenJuice compacts verbose tool output before it enters the model's
  context, and keeps the full original recoverable.
icon: file-zipper
---

# Token compression

LLM tokens cost money, and verbose tool output wastes most of them. A `git status` in a busy repo, a `cargo build` log, a 600-message email thread and a `docker ps -a` against a real cluster can each fill a context window for almost no information.

OpenHuman ships with TokenJuice, a compression router built into the agent's tool-execution path. Before a tool result reaches a model, TokenJuice classifies it, sends it to a specialized compressor, optionally stores the full original in a recoverable cache, and records how many tokens and dollars it saved. The compression engine is the TinyJuice library, vendored in OpenHuman. The command and log rules come from [vincentkoc/tokenjuice](https://github.com/vincentkoc/tokenjuice).

## The pipeline

Every tool result takes the same path through the TinyJuice router (`vendor/tinyjuice/src/compress.rs`):

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

1. **Size gate.** If the router is off or the input is below `min_bytes_to_compress` (default 2048 bytes), it passes through untouched. Tiny outputs are not worth compressing.
2. **Content detection** (`detect/kind.rs`). The result is classified into one of seven kinds. The order of precedence is an explicit hint, then a MIME or extension tag, then a per-tool prior (for example `grep` means Search, `git_operations` means Diff, `run_tests` means Log), then cheap structural heuristics (JSON, Diff, HTML, Search, Code, Log, PlainText). There is no regex on the hot path.
3. **Compressor selection.** Each kind goes to its own compressor, honoring per-kind toggles (`search_enabled`, `code_enabled`, `html_enabled`).
4. **Compression.** The compressor runs. If it declines or its output is no smaller than the input, TokenJuice falls back to the generic compressor or passes the original through. It never makes things bigger.
5. **CCR offload.** When a compression is lossy and the original is large enough (`ccr_min_tokens`, default about 500 tokens), the full original goes into the Compress-Cache-Retrieve (CCR) store, so nothing is lost for good.
6. **Recovery marker.** A footer with the marker `⟦tj:<hash>⟧` is appended. It tells the agent the view is partial and how to fetch the rest.
7. **Savings accounting.** Tokens saved and estimated cost saved are recorded by model and by compressor.

## The compressors

Each content kind has a purpose-built compressor (`vendor/tinyjuice/src/compressors/`):

| Compressor | Kind | What it does |
| --- | --- | --- |
| SmartCrusher | JSON | Re-renders arrays of objects as a compact table. Past about 40 rows it keeps the head, the tail, error rows and numeric outliers. |
| Code | Code | Keeps signatures and imports and collapses deep function bodies to `{ … N lines … }` (tree-sitter when available, a brace-depth heuristic otherwise). Preserves `TODO`, `FIXME`, `error`, `panic` and `unsafe` markers. |
| Log | Log | For command output, uses the JSON rule engine (below). For other logs, keeps errors, warnings, stack traces and summaries and drops the noise. |
| Search | Search | Groups grep and ripgrep `path:line:body` hits by file, ranks by query-term density, keeps the top matches per file and tallies `[+N more]`. |
| Diff | Diff | Keeps changed lines and hunk headers and collapses long unchanged runs to an anchor. Lockfile hunks shrink to a one-line `+A/-B` summary. |
| Html | HTML | Strips markup to readable text with sensible block-boundary newlines and entity decoding (allocation-light, no DOM). |
| TextCrusher | PlainText | Deterministic salience compression that preserves sentence and paragraph boundaries. |
| Generic | fallback | Head and tail summary for command output that no specific rule matched. It declines on structured results so they are preserved. |

Multi-byte text (CJK, emoji, combining marks) is handled grapheme by grapheme and never split mid-character.

## Handle preview and the juice tools (default)

Compaction is on by default. For a result of at least `ccr_min_tokens`, the router does not compress to one blob. It stores the original in the CCR cache and shows the model a small preview instead:

- a one-line stats description of the shape (estimated tokens, bytes, lines, JSON keys or a Markdown outline, never values),
- the first 500 characters,
- an extractive outline, and
- a footer naming the handle (the CCR token).

The model then queries the stored original with three read-only tools, none of which calls a model:

| Tool | What it does |
| --- | --- |
| `juice_find` | `text`, `grep`, `regex`, `rank` (BM25), `sed`, `awk` and `jq` over the output, with a Python-style `scope` slice. |
| `juice_extract` | Links or headings from HTML or Markdown output. |
| `juice_summarize` | Size, outline or JSON shape, then head and tail, or the parts most relevant to a `hint`. |

Answers are size-capped, and an unknown or evicted handle is an error. `juice_retrieve` still returns the whole original. A handle preview is built without a model call, so it also replaces the LLM summary for results big enough to get one. A slow summarizer cannot stall the turn. The three tools are registered (about 1.5 KB of schema) only while this mode is on.

To turn off the whole feature, set `context.compaction_enabled = false` or `OPENHUMAN_COMPACTION=0`. To keep compaction but go back to one-blob compression and `juice_retrieve`, set `tokenjuice.repl_handle_enabled = false` (`OPENHUMAN_TOKENJUICE_REPL_HANDLE_ENABLED=0`). Set `tokenjuice.repl_save_enabled = true` to also write each stored original to `<workspace>/.tokenjuice/repl/<handle>.txt` (mode 0600) so an agent can script over it. That puts raw tool output on disk, and nothing prunes it.

## Nothing is lost: the CCR cache

Lossy compression normally throws data away. TokenJuice instead stores the full original in the CCR store and leaves a breadcrumb (`vendor/tinyjuice/src/cache/`).

- **In-memory tier** (always on): a process-wide store keyed by SHA-256 hash, bounded by entry count (`max_cache_entries`, default 256) and total bytes (`max_cache_bytes`, default 64 MiB), with FIFO eviction.
- **On-disk tier** (optional): `<workspace>/.tokenjuice/ccr/`, enabled with `ccr_disk_enabled`. It survives memory eviction. Set an optional TTL with `ccr_ttl_secs`.
- **The marker:** compacted output ends with a footer like `[compacted tool output: PARTIAL view; full original available via juice_retrieve with token "…"]` carrying the `⟦tj:<hash>⟧` token.
- **Retrieval tool:** the agent calls the read-only `juice_retrieve` tool with that token (optionally a byte or line `range`) to get the full original or a slice. The token is an unguessable SHA-256 digest.

The agent gets the cheap compacted view by default and can zoom in on the full text only when it needs it.

## Savings tracking

An OpenHuman savings callback meters every compression (`crates/openhuman-core/src/inference/tokenjuice/savings.rs`). TokenJuice reports events and token deltas. OpenHuman applies the configured default model's input pricing, aggregates `total`, `by_model` and `by_compressor`, and saves stats to `<workspace>/state/tokenjuice_savings.json`.

Read them over RPC with `openhuman.tokenjuice_savings_stats`; clear them with `openhuman.tokenjuice_savings_reset`.

## The rule overlay (command and log output)

A three-layer JSON rule overlay powers the log and command compressor. Rules merge in order, and later layers override earlier ones:

| Layer | Path | Purpose |
| --- | --- | --- |
| Builtin | shipped with the binary | About 96 vendored rules for git, npm, cargo, docker, kubectl, ls and more |
| User | `~/.config/tokenjuice/rules/` | Personal overrides that apply everywhere |
| Project | `.tokenjuice/rules/` | Repo-specific overrides, checked in and shared with the team |

Each rule names a command or tool pattern and a reduction strategy: skip and keep filters, transforms like strip-ANSI and dedupe, head and tail summaries, named counters and canned messages. Rules are JSON. Add one and it applies with no recompile.

## Configuration, RPC and tools

Everything lives under the `[tokenjuice]` config block (`crates/openhuman-core/src/config/schema/tokenjuice.rs`) and can be changed live.

- Master switches: `context.compaction_enabled` (default `true`; `OPENHUMAN_COMPACTION=0` opts out) and `router_enabled` (default `true`).
- Handle preview: `repl_handle_enabled` (default `true`) and `repl_save_enabled` (default `false`).
- Thresholds: `min_bytes_to_compress` and `ccr_min_tokens`.
- CCR: `ccr_enabled`, `ccr_disk_enabled`, `max_cache_entries`, `max_cache_bytes` and `ccr_ttl_secs`.
- Per kind: `search_enabled`, `code_enabled`, `html_enabled`, plus the `ml_*` keys.
- RPC (`openhuman.tokenjuice_*`): `detect`, `compress` (dry-run the pipeline), `settings_get` and `settings_update` (live partial patch), `cache_stats`, `retrieve`, `savings_stats` and `savings_reset`.
- Agent tools: `juice_retrieve` recovers a whole stored original. `juice_find`, `juice_extract` and `juice_summarize` query one by handle. All are read-only.
- Debugging: start the core with `RUST_LOG=openhuman_core::inference::tokenjuice=debug` to watch detection, matching and how much each result is trimmed.

## Why this matters

An agent's context budget is its limit. One working session can fan out across dozens of tool calls: greps, builds, test runs, `git` output and large [web fetch](native-tools/web-scraper.md) results. TokenJuice compacts each result before it lands in context, so an agent can sweep a noisy repo or a long web page without filling the window. The savings add up across a session and are metered in dollars (see [Billing, cost and usage](billing-and-usage.md)).

TokenJuice runs on the agent's tool results, not on [memory](memory.md) ingestion. Source sync converts documents itself and does not route payloads through TokenJuice.

## See also

- [Native tools](native-tools/README.md): most heavy tool output flows through TokenJuice.
- [Memory](memory.md): ingestion has its own document conversion and scrubbing.
- [Billing, cost and usage](billing-and-usage.md): where token savings show up as real money.
