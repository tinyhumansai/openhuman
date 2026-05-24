# memory_tree

Generic tree mechanics on top of `memory_store::trees`. Kind-agnostic: a
`Source`, `Global`, or `Topic` tree all flow through the same code here.
Kind-specific policy (when to spawn a topic tree, what scope a global tree
covers, how digests are written) lives in `memory::tree_global` and
`memory::tree_topic`; this module is unaware of it.

```text
memory (orchestrator) ──┐
                        │ writes leaves via TreeWriteRequest
                        ▼
memory_tree            (this module — generic mechanics)
   ├── tree/           append + cascade seal + flush
   ├── summarise.rs    L_n -> L_{n+1} text via the chat model
   ├── sources/        per-source tree registry + .md mirror
   ├── tools/          agent-facing read tools (walk, drill, fetch)
   └── io.rs           canonical Tree{Write,Read}{Request,Outcome,Result}
                        │
                        ▼
memory_store::trees    (persistence: one Tree table, one schema)
```

## Layout

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Re-exports `io::*` and the controller-schema registries hosted in `memory`. Re-exports `memory::tree_global` + `memory::tree_topic` under the legacy `memory_tree::tree_{global,topic}` paths. |
| [`io.rs`](io.rs) | Canonical contract types: `TreeWriteRequest`/`TreeWriteOutcome`, `TreeReadRequest`/`TreeReadHit`/`TreeReadResult`, `TreeLeafPayload`, `TreeLabelStrategy`. Pure types, no IO. |
| [`tree/`](tree/) | `bucket_seal` (append leaf + cascade seal), `flush` (time-based partial seal), `registry` (kind-parameterized `get_or_create_tree` with UNIQUE-race recovery), `mod.rs` (re-exports + `memory_store::trees` shims for legacy paths). |
| [`summarise.rs`](summarise.rs) | One function: produce the next-level summary text for a bucket. Wraps the chat model with a fixed prompt and token budget. |
| [`sources/`](sources/) | `registry::get_or_create_source_tree` wrapper that adds the `_source.md` on-disk mirror to the generic registry. `file.rs` writes the mirror. |
| [`tools/`](tools/) | Agent-facing tools. Read: `walk` (agentic), `drill_down`, `fetch_leaves`, `query_{source,global,topic}`, `search_entities`. Write: `ingest_document` (orchestrator-facing). |

## Layer rules

- **No tree-kind branching here.** `bucket_seal`, `flush`, `registry`,
  `summarise` all take `TreeKind` as a parameter or treat it as opaque.
- **No persistence here.** Reads and writes go through
  `memory_store::trees::{store, registry, hotness}`.
- **No policy here.** Curator gates (hotness thresholds), digest cadence,
  global scope sentinels — all live in `memory::tree_{global,topic}`.
