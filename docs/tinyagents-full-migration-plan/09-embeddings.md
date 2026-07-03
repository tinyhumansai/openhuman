# 09 — Embeddings & retrieval

Adapt OpenHuman embeddings/memory retrieval to crate interfaces; OpenHuman
stores stay authoritative.

Target SDK surface: `EmbeddingModel` trait, `VectorStore`, concrete `Retriever`,
`ScoredDoc`, `cosine_similarity`, `MemoryScope`, `AgentEvent::
{MemoryLoaded, MemorySaved}`.

## Steps

1. Adapter: implement crate `EmbeddingModel` over
   `embeddings/provider_trait.rs::EmbeddingProvider` (voyage/openai/cohere/
   ollama/cloud/noop impls unchanged underneath). Keep OpenHuman
   `factory.rs`/rate-limit/retry as construction policy.
2. Retriever facade: adapt the memory search surface the harness uses
   (`harness/memory_context.rs` recall injection,
   `agent_memory::memory_loader`) to TinyAgents `EmbeddingModel`/`VectorStore`,
   using the concrete crate `Retriever` where applicable and returning
   `ScoredDoc`s; the harness loads retrieval context through the facade so
   retrieval becomes swappable and event-visible (`MemoryLoaded`).
3. Memory source identity: preserve `metadata.path_scope` dedupe semantics
   (CLAUDE.md rule) in the facade mapping.
4. Usage/cost: embedding calls emit crate `Usage` + catalog pricing (06.4).
5. Done: the vestigial `agent/memory_loader.rs` facade and unwired
   `agent/tree_loader.rs` eager digest prefetch module were deleted; callers
   use `agent_memory::memory_loader` directly.
6. Done: `embeddings::mod` no longer re-exports `memory_store::vectors`, and
   `memory_search::vector::store` was deleted. Vector storage now has one
   canonical owner (`memory_store::vectors`); `memory_search::vector` only owns
   MMR/diversity selection.

## Deletions

- Done: `agent/memory_loader.rs` (tracked in `99-deletion-ledger.md`).
- Done: `agent/tree_loader.rs` (tracked in `99-deletion-ledger.md`).
- Done: `memory_search::vector::store` compatibility shim; direct callers import
  `memory_store::vectors::cosine_similarity`.

## Acceptance

- Recall-injection parity on a scripted turn (same citations via
  `collect_recall_citations`); embedding usage appears in cost records.

Keep (product): memory stores, retrieval ranking policy,
`memory_context_safety.rs` gating, archivist (orthogonal durable memory —
assessed KEEP 2026-06-30d; optional later: re-hang as `after_agent`
middleware without changing behavior).
