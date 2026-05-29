---
description: >-
  The generic summary-tree engine under the Memory Tree feature - bucket-seal
  cascades, scoring, embedding, entity extraction, retrieval, summarisation.
  Kind-agnostic mechanics that Source / Global / Topic trees all share.
icon: diagram-project
---

# Memory Tree (`src/openhuman/memory_tree/`)

`src/openhuman/memory_tree/` is the **generic tree engine** sitting under the user-facing [Memory Tree feature](../../features/obsidian-wiki/memory-tree.md). It owns the kind-agnostic mechanics — appending leaves, cascading bucket seals, summarising one level to the next, scoring and embedding, retrieving for agents — that every concrete tree flavour (`Source`, `Global`, `Topic`) shares. It is deliberately **unaware** of which flavour a tree belongs to.

Kind-specific policy — when to spawn a topic tree, what scope a global tree covers, how digests are written — lives in `src/openhuman/memory/tree_global` and `src/openhuman/memory/tree_topic`. Persistence (the single `Tree` table and its schema) lives one layer down in `memory_store::trees`. This module sits between them.

```text
memory (orchestrator) ──┐
                        │ writes leaves via TreeWriteRequest
                        ▼
memory_tree            (this module — generic mechanics)
   ├── tree/           append + cascade seal + flush
   ├── summarise.rs    L_n -> L_{n+1} text via the chat model
   ├── retrieval/      agent-facing read tools (walk, drill, fetch)
   ├── score/          scoring, embedding, entity extraction
   ├── tools.rs        re-exports from memory::query
   └── io.rs           canonical Tree{Write,Read}{Request,Outcome,Result}
                        │
                        ▼
memory_store::trees    (persistence: one Tree table, one schema)
```

## Layout

| Path                                                                                             | Role                                                                                                                                                                                                                                 |
| ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [`mod.rs`](https://github.com/tinyhumansai/openhuman/blob/main/src/openhuman/memory_tree/mod.rs) | Re-exports `io::*` and the controller-schema registries hosted in `memory`. Also re-exports `memory::tree_global` + `memory::tree_topic` under the legacy `memory_tree::tree_{global,topic}` paths for backward compatibility.       |
| `io.rs`                                                                                          | Canonical contract types: `TreeWriteRequest` / `TreeWriteOutcome`, `TreeReadRequest` / `TreeReadHit` / `TreeReadResult`, `TreeLeafPayload`, `TreeLabelStrategy`. Pure types, no IO.                                                  |
| `tree/`                                                                                          | `bucket_seal` (append leaf + cascade seal), `flush` (time-based partial seal), `registry` (kind-parameterized `get_or_create_tree` with UNIQUE-race recovery), `mod.rs` (re-exports + `memory_store::trees` shims for legacy paths). |
| `summarise.rs`                                                                                   | One function: produce the next-level summary text for a bucket. Wraps the chat model with a fixed prompt and token budget.                                                                                                           |
| `retrieval/`                                                                                     | Agent-facing tools. Read: `walk` (agentic), `drill_down`, `fetch_leaves`, `query_{source,global,topic}`, `search_entities`. Write: `ingest_document` (orchestrator-facing).                                                          |
| `score/`                                                                                         | Scoring signals, embedding (cloud / Ollama / inert), entity extraction (regex / LLM), canonical resolver, entity index store.                                                                                                        |
| `tools.rs`                                                                                       | Re-exports from `memory::query` for backward compatibility.                                                                                                                                                                          |
| `tree_runtime/`                                                                                  | Tree-summarizer controller registry — exposed through `all_tree_summarizer_controller_schemas` / `all_tree_summarizer_registered_controllers` re-exports in `mod.rs`.                                                                |

## Layer rules

These are load-bearing invariants — break one and the engine stops being kind-agnostic:

- **No tree-kind branching here.** `bucket_seal`, `flush`, `registry`, and `summarise` all take `TreeKind` as a parameter or treat it as opaque. Conditionals on "is this a Source tree?" belong in the orchestrator (`src/openhuman/memory/`), not here.
- **No persistence here.** Reads and writes go through `memory_store::trees::{store, registry, hotness}`. This module does not open SQLite handles directly.
- **No policy here.** Curator gates (hotness thresholds), digest cadence, global scope sentinels all live in `memory::tree_{global,topic}`. This module reacts to policy decisions, it does not make them.

## How a write flows in

1. The orchestrator (`memory::*`) constructs a `TreeWriteRequest` with a `TreeKind` and a `TreeLeafPayload`.
2. `tree::bucket_seal` appends the leaf to the open bucket at L0. If the bucket fills, it seals — `summarise.rs` produces the L1 summary, which becomes a leaf in the L1 bucket, and the cascade continues upward until a non-full bucket is hit.
3. `score/` runs in the background: embeddings (cloud / Ollama / inert backend), entity extraction (regex first, LLM optional), hotness signals. None of this blocks the write path.
4. The outcome (`TreeWriteOutcome`) is returned synchronously to the orchestrator; scoring catches up asynchronously.

`tree::flush` exists for the time-bounded case — if a bucket hasn't filled within its TTL, it gets sealed partially so the next level always has something fresh to summarise.

## How a read flows out

Agents reach this module through the tools in `retrieval/`:

- `walk` — agentic exploration; the agent picks summary nodes to drill into.
- `drill_down` — deterministic traversal from a known starting summary.
- `fetch_leaves` — pull raw leaves for a sealed bucket.
- `query_{source,global,topic}` — kind-scoped retrieval; the orchestrator's tree-kind policy decides which one the agent sees.
- `search_entities` — entity-index lookup backed by `score/`.

All retrieval handlers consult `memory_store::trees::hotness` so warm content surfaces first.

## Controller registry

`memory_tree::mod.rs` re-exports two controller registries that get wired into the global registry in `src/core/all.rs`:

- `all_memory_tree_controller_schemas` / `all_memory_tree_registered_controllers` — sourced from `memory::schema` (the orchestrator hosts them; this module just surfaces them under the `memory_tree` path).
- `all_retrieval_controller_schemas` / `all_retrieval_registered_controllers` — the agent-facing read tools listed above.
- `all_tree_summarizer_controller_schemas` / `all_tree_summarizer_registered_controllers` — from `tree_runtime`, for summariser admin / inspection.

## Related

- [`memory_tree/README.md`](https://github.com/tinyhumansai/openhuman/blob/main/src/openhuman/memory_tree/README.md) — authoritative internal-audience overview this page mirrors.
- [Memory Tree feature](../../features/obsidian-wiki/memory-tree.md) — what end users see.
- [Architecture overview](../architecture.md) — where this fits in the wider system.
