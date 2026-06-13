# Memory tree — score (Phase 2 / #708)

Per-chunk admission, enrichment, and entity indexing for the bucket-seal-ready memory tree. Sits between leaf chunking and L0 buffer append: every chunk passes through `score_chunk` which decides whether to keep it, runs entity extraction, and persists score rationale + an inverted entity index used by retrieval.

## Public surface

- `pub fn score_chunk` / `pub fn score_chunks` / `pub fn score_chunks_fast` — `mod.rs` — scoring pipeline entry points (full / batch / cheap-only batch).
- `pub struct ScoreResult` / `pub struct ScoringConfig` — `mod.rs` — outcome and configuration of one scoring pass.
- `pub fn persist_score` / `persist_score_tx` — `mod.rs` — write the score row + entity-index rows for one kept chunk.
- `pub const DEFAULT_DROP_THRESHOLD` / `DEFAULT_DEFINITE_KEEP` / `DEFAULT_DEFINITE_DROP` — `mod.rs` — admission band defaults.

## Subdirectories

- `signals/` — per-signal feature computation (token count, unique words, metadata weight, source weight, interaction tags, entity density, LLM importance) plus the weighted combine that produces the final `[0.0, 1.0]` total.
- `extract/` — entity extraction: `EntityExtractor` trait, `RegexEntityExtractor` for mechanical identifiers (email, URL, handle, hashtag), `LlmEntityExtractor` for semantic NER + importance rating, `CompositeExtractor` for chaining them.
- `embed/` — Phase 4 vector embedder: `Embedder` trait, `OllamaEmbedder` (default), `InertEmbedder` (tests), pack/unpack helpers for the SQLite BLOB storage layout.

## Files

- `mod.rs` — orchestration: `score_chunk` runs extraction → cheap signals → optional borderline LLM call → admission gate → canonicalisation.
- `store.rs` — SQLite CRUD for `mem_tree_score` (per-chunk rationale) and `mem_tree_entity_index` (inverted index `entity_id → node_id`).
- `resolver.rs` — entity canonicalisation: normalises surface forms (lowercase emails, strip leading `@`/`#`) and assigns stable `canonical_id` strings; promotes extracted topics into the canonical entity stream.
- `mod_tests.rs` / `store_tests.rs` — unit tests.
