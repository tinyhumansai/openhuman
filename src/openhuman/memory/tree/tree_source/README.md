# Tree source

Phase 3a (#709) — per-source summary trees with bucket-seal mechanics. One tree per ingest source (Slack channel, Gmail account, document corpus, ...). Time-aligned L0 buffers accumulate canonical chunks; once a buffer crosses its gate it seals into an L1 summary, and the cascade may continue upward (L1 → L2 → ...). Storage primitives are reused by the topic and global trees in Phases 3b / 3c.

## Public surface

- `pub fn get_or_create_source_tree` — `registry.rs` — idempotent tree lookup keyed by `(kind=source, scope)`.
- `pub fn append_leaf` / `pub fn append_leaf_deferred` / `pub struct LeafRef` / `pub enum LabelStrategy` — `bucket_seal.rs` — push a chunk into its tree and cascade-seal on budget.
- `pub fn flush_stale_buffers` / `pub fn flush_stale_buffers_default` / `pub fn force_flush_tree` — `flush.rs` — time-based seal of buffers that never cross the token gate.
- `pub fn build_summariser` / `pub trait Summariser` / `pub struct SummaryInput` / `pub struct SummaryContext` / `pub struct SummaryOutput` — `summariser/mod.rs` — folds N inputs into one summary.
- `pub struct InertSummariser` — `summariser/inert.rs` — deterministic dependency-free fallback.
- `pub struct LlmSummariser` / `pub struct LlmSummariserConfig` — `summariser/llm.rs` — Ollama-backed implementation with soft-fallback to inert.
- `pub struct Tree` / `pub struct SummaryNode` / `pub struct Buffer` / `pub enum TreeKind` / `pub enum TreeStatus` / `pub const TOKEN_BUDGET` / `pub const SUMMARY_FANOUT` — `types.rs`.
- `pub fn get_summary_embedding` / `pub fn set_summary_embedding` / `pub fn insert_tree` / `pub fn get_tree_by_scope` / `pub fn get_tree` / `pub fn list_trees_by_kind` / `pub fn get_summary` / `pub fn list_summaries_at_level` / `pub fn count_summaries` / `pub fn get_buffer` / `pub fn list_stale_buffers` — `store.rs`.

## Files

- `mod.rs` — module surface and re-exports.
- `types.rs` — `Tree`, `SummaryNode`, `Buffer`, gating constants.
- `registry.rs` — get-or-create + UNIQUE-race recovery; `new_summary_id` helper.
- `store.rs` — SQLite persistence for `mem_tree_trees` / `mem_tree_summaries` / `mem_tree_buffers`, including embedding blob handling.
- `bucket_seal.rs` — `append_leaf`, level-aware seal gate, single-tx `seal_one_level` with atomic follow-up enqueue.
- `flush.rs` — time-based stale-buffer flush.
- `summariser/` — summariser trait and implementations (see `summariser/README.md`).
- `bucket_seal_tests.rs` / `store_tests.rs` — per-module unit tests, included via `#[path]`.
