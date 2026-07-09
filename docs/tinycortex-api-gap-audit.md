# TinyCortex API Gap Audit (Phase 0.2)

**Purpose.** Map every host call site into the engine-mapping modules
(`memory_store`, `memory_tree`, `memory_queue`, and the long tail) onto a TinyCortex
public API. Where no crate API covers a host need, record a **gap** — each gap becomes a
`tinycortex` issue/PR (engine-side) or an explicit host-adapter decision, and gates the
workstream that touches it.

Anchors: host `7850cf363` · tinycortex `d1a8c7be` (v0.1.1). Crate API surface inventoried
from `vendor/tinycortex/src/memory/**` (`pub` traits/fns/types, verbatim signatures).

## Call-site volume (host → engine, external references only)

External refs = uses of `memory_<mod>::…` from host code **outside** that module.

| Engine module | External refs | Maps to crate | Coverage |
| --- | --- | --- | --- |
| `memory_store` | 454 | `store/`, `chunks/` | High — see per-API below |
| `memory_tree` | 169 | `tree/`, `retrieval/`, `score/` | High (gaps G3, G5) |
| `memory_queue` | 42 | `queue/` | High (drift D2) |
| `memory_sources` | 35 | `sources/` | Full (local readers only) |
| `memory_conversations` | 24 | `conversations/` | Full (drift D1) |
| `memory_tools` | 12 | `tool_memory/` | Full |
| `memory_diff` | 11 | `diff/` | Full |
| `memory_archivist` | 9 | `archivist/` | Full |
| `memory_goals` | 6 | `goals/` | Full |
| `memory_search` | 1 | `retrieval/`, `score/` | tools stay host |
| `memory_entities` | 0 | `entities/` | Full |
| `memory_graph` | 0 | `graph/` | Derive-on-read (gap G2) |

Highest-traffic `memory_store` sub-APIs (external): `chunks::store` (152), `chunks::types`
(93), `trees::types` (54), `create_memory` (31), `trees::store` (27), `profile` (26),
`types` (19), `content` (19), `content::compose` (14), `content::paths` (10),
`content::read` (9), `content::atomic` (8), `content::raw` (7). All of `chunks::*`,
`trees::*`, `content::*`, `profile` map to crate re-exports (`chunks::…`, `tree::…`,
`store::content::…`, `config::WeightProfile`). `create_memory` is host glue (see below).

## Seam-trait roster (the W1 adapter targets — all present in crate)

The crate exposes every seam the plan expects, plus more. Host implements these:

| Trait | Path | Host adapter (W1) |
| --- | --- | --- |
| `EmbeddingBackend` | `store/vectors/embedding.rs:32` | `tinycortex/embeddings.rs` over `openhuman::embeddings` |
| `Embedder` (retrieval/summarise) | `score/embed.rs:28` | same seam file |
| `ChatProvider` | `score/extract/llm.rs:70` | `tinycortex/chat.rs` over `memory::chat` / `inference` |
| `Summariser` (bucket-seal, structured) | `tree/summarise.rs:70` | `tinycortex/chat.rs` |
| `Summariser` (time-tree, string) | `tree/runtime/engine.rs:29` | **divergent — see G6** |
| `EntityExtractor` (regex + LLM) | `score/extract/composite.rs:16` | `tinycortex/chat.rs` (LLM variant) |
| `QueueDelegates` | `queue/handlers.rs:96` | `tinycortex/queue_driver.rs` |
| `TreeJobSink` | `ingest/types.rs:15` | `tinycortex/sinks.rs` |
| `TreeLeafSink` | `archivist/sink.rs:44` | `tinycortex/sinks.rs` |
| `SnapshotItemSource` | `diff/source.rs:28` | `tinycortex/sinks.rs` |
| `EntityOccurrenceIndex` | `graph/types.rs:44` | `tinycortex/sinks.rs` |
| `SelfIdentity` | `store/entity_index/store.rs:25` | host identity registry |
| `GoalsGenerator` | `goals/reflect.rs:57` | `tinycortex/chat.rs` |
| `SourceReader` | `sources/readers/mod.rs:1732` | local readers only; live sync stays host |
| `ConversationEventBus` / `ChannelEventHandler` | `conversations/bus.rs:106/95` | `tinycortex/bus.rs` (translate to `DomainEvent`) |

Config seam: `MemoryConfig{workspace, embedding: EmbeddingConfig{dim,model,strict},
tree: TreeConfig{input_token_budget,output_token_budget,summary_fanout,flush_age_secs},
retrieval: RetrievalConfig{default_profile}, sync_budget: SyncBudgetConfig}` +
`WeightProfile{graph,vector,keyword,freshness}` — all `pub` with the exact fields the W1
`config.rs` adapter maps `Config` onto.

---

## Gaps (each → tinycortex issue/PR or explicit host-adapter decision)

### G1 — Raw SQLite connection escape hatch (`sqlite_conn()`) — **HARD, high blast radius**

- **Host:** two trait methods return a live handle:
  `memory/traits.rs:277` and `memory_store/memory_trait.rs:458`
  `fn sqlite_conn(&self) -> Option<Arc<Mutex<Connection>>>`. ~312 references to
  `sqlite_conn`/`raw_conn`/`with_connection` across the host.
- **Crate:** deliberately omits it (`traits.rs:3-6`). Offers instead a **scoped** accessor
  `chunks::with_connection(config, |conn| …)` (`chunks/mod.rs:217`) over the cached handle;
  `conn_cache()` is private.
- **Decision (W2):** do **not** upstream a handle-getter (it breaks the crate's connection-cache
  invariants). Instead: (a) migrate host internal raw-SQL sites to `chunks::with_connection`
  during W3; (b) keep `sqlite_conn()` as a **host-side extension trait** over the crate store for
  the transition, shrinking to zero as sites migrate. Track residual call sites in the deletion
  ledger. **Gates W2/W3.**

### G2 — Graph relation-edge persistence (E2GraphRAG accumulation) — **HARD**

- **Host:** `memory_store/unified/graph.rs`, `unified/query.rs` persist and query co-occurrence
  / LLM-triple relations; `NamespaceMemoryHit.supporting_relations: Vec<GraphRelationRecord>`
  is populated at retrieval.
- **Crate:** persists the **entity occurrence index** at persist time
  (`score::persist_score`/`persist_score_tx`, `score/mod.rs:847-906`) but **does not write
  co-occurrence graph edges** — docstring `score/mod.rs:843-846`: *"Co-occurrence graph edges
  (OpenHuman's E2GraphRAG accumulation) are not written here."* The `graph` module **derives**
  edges on read from the occurrence index (`graph/query.rs`, "owns no state, performs no
  writes"). `GraphRelationRecord` exists as a type but nothing in the crate writes it.
- **Decision (W6/W7):** verify whether derive-on-read reproduces the host's persisted-relation
  retrieval (`supporting_relations`) with parity. If yes → host relation store stays host
  (namespace doc/graph store is explicitly host per plan §1); if the crate must own accumulation
  → upstream a relation-persist path. **Gates W6/W7.** Parity-critical for retrieval output.

### G3 — Seal-time embedding — **MEDIUM (parity timing)**

- **Host:** computes summary embeddings around seal (`memory_tree/tree/bucket_seal.rs`,
  `memory_tree/score/embed/*`, `set_summary_embedding`).
- **Crate:** `bucket_seal.rs:21` / `tree/mod.rs:22` list "seal-time embedding" as **not ported**;
  `seal_one_level` computes labels but never embeds. Summary embedding runs later via the queue's
  `reembed_batch` delegate. The `mem_tree_summary_embeddings` sidecar + setters exist.
- **Decision (W5):** confirm retrieval parity on freshly-sealed summaries — if the host embeds at
  seal but the crate defers to the queue, a summary is momentarily unembedded until the worker
  drains. Either drive an embed job at seal from the host queue driver, or upstream seal-time
  embedding. **Gates W5.**

### G4 — Post-seal notification sink (for host `wiki_git` mirror) — **SOFT**

- **Host:** `6395f642e` wired a seal-time hook in `bucket_seal.rs` to commit summary markdown to
  the git-backed `wiki_git` mirror (host-retained, per the drift ledger).
- **Crate:** no observer/sink trait for seal events — but `append_leaf` / `cascade_all_from` /
  `TreeFactory::seal_now` **return `Vec<String>` of sealed summary ids**. The host can drive
  `wiki_git` from the returned ids without a callback.
- **Decision (W3/W5):** no crate change required; the host queue driver calls `wiki_git` after each
  seal using returned ids. Record as a seam-wiring task, not an upstream gap. Revisit only if a
  push-notification is later wanted.

### G5 — Tree health / doctor (storage-degraded state machine) — **SOFT (host-retained)**

- **Host:** `memory_tree/health/*` (`mark_storage_degraded`/`clear_storage_degraded`, degraded
  flag), consumed by `memory/tools/doctor.rs` and fed by the D2 host-FS classifier.
- **Crate:** defers tree health/doctor entirely (`tree/mod.rs:20`); only `Memory::health_check()
  -> bool`. Queue uses its own `JobFailure` (no storage-degraded machine).
- **Decision (W5):** **host-retained** (like `wiki_git`). `memory_tree::health` stays host; the D2
  predicate (queue classifier) upstreams, its Sentry-once + degraded-flag consumers stay host.

### G6 — Two divergent `Summariser` traits — **MEDIUM (ambiguity)**

- Crate has **two** traits named `Summariser`: `tree/summarise.rs:70` (structured batch fold —
  `summarise(&[SummaryInput], &SummaryContext) -> SummaryOutput`) used by bucket-seal, and
  `tree/runtime/engine.rs:29` (string in/out — `summarise(Option<&str>, &str) -> String`) used by
  the markdown time-tree runtime. They are distinct contracts with the same name.
- **Decision (W1/W5):** the host seam must implement **both**. Flag for possible upstream
  unification (rename one, e.g. `TimeTreeSummariser`) to reduce confusion — non-blocking. Note
  which tree path OpenHuman actually drives (bucket-seal SQLite trees vs markdown time-tree) so we
  don't wire a dead seam.

### Non-gaps (expected host glue — no crate change)

| Host API | Refs | Disposition |
| --- | --- | --- |
| `memory_store::create_memory` / `create_memory_with_local_ai` (`factories.rs:297,322`) | 31 | Host glue building `UnifiedMemory`. Crate has no `UnifiedMemory`/`MemoryClient` (by design). **W3 re-implements these factories over `tinycortex::store` + `chunks`, keeping the host-facing `MemoryClient` API stable.** Not a crate gap. |
| `memory_store::profile` (`unified::profile`) | 26 | Maps to `config::WeightProfile` (fields `graph/vector/keyword/freshness`, `by_name`). |
| `memory_store::fts` / keyword search | 7 | Crate provides `retrieval::keyword_relevance` + `hybrid_score` (not SQLite FTS5). **Verify keyword-recall parity in W5** (behavioral, not a missing API). |
| Namespace document/graph store | — | Explicitly host-retained per plan §1 until/unless upstreamed (ties to G2). |

---

## Gap → workstream gate summary

| Gap | Severity | Gates | Resolution path |
| --- | --- | --- | --- |
| G1 escape hatch | High | W2/W3 | Migrate to `with_connection`; host extension trait for transition |
| G2 graph relations | High | W6/W7 | Parity check derive-on-read; upstream persist iff needed |
| G3 seal-time embedding | Medium | W5 | Drive embed at seal from host queue driver, or upstream |
| G4 seal notification | Soft | W3/W5 | Drive `wiki_git` from returned sealed ids (no crate change) |
| G5 tree health/doctor | Soft | W5 | Host-retained |
| G6 two `Summariser` traits | Medium | W1/W5 | Implement both; optional upstream rename |

**No hard gap blocks W1 (seam scaffolding) or W4 (queue, only drift D2).** W3 is gated by G1;
W5 by G3/G6; W6/W7 by G2. These are recorded here and mirrored in the spec's deletion ledger.
