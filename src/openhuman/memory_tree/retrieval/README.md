# Retrieval

Phase 4 (#710) — search-time pipeline for the hierarchical memory tree. Exposes six LLM-callable primitives that read across the source / topic / global trees built by Phase 3 and surface results in a uniform [`RetrievalHit`] shape. There is no classifier, gate, or composer in this phase — orchestration (which tool to call, how to combine) is left to the calling LLM.

## Public surface

- `pub fn query_source` / `pub struct QuerySourceRequest` — `source.rs`, `rpc.rs` — per-source summary retrieval, optional semantic rerank.
- `pub fn query_global` / `pub struct QueryGlobalRequest` — `global.rs`, `rpc.rs` — cross-source digest for a window in days.
- `pub fn query_topic` / `pub struct QueryTopicRequest` — `topic.rs`, `rpc.rs` — entity-scoped retrieval across every tree.
- `pub fn search_entities` / `pub struct SearchEntitiesRequest` — `search.rs`, `rpc.rs` — fuzzy LIKE lookup over the entity index.
- `pub fn drill_down` / `pub struct DrillDownRequest` — `drill_down.rs`, `rpc.rs` — walk `child_ids` from a summary one (or more) levels down.
- `pub fn fetch_leaves` / `pub struct FetchLeavesRequest` — `fetch.rs`, `rpc.rs` — batch-hydrate raw chunks by id (cap 20).
- `pub struct RetrievalHit` / `pub enum NodeKind` / `pub struct QueryResponse` / `pub struct EntityMatch` — `types.rs` — wire shapes shared by every tool.
- `pub fn all_retrieval_controller_schemas` / `pub fn all_retrieval_registered_controllers` — `schemas.rs` — registry exports wired into `core::all`.

## Files

- `mod.rs` — module surface; declares submodules and the `pub use` re-exports.
- `types.rs` — shared wire types and the `hit_from_summary` / `hit_from_chunk` helpers.
- `source.rs` / `global.rs` / `topic.rs` — query the corresponding tree level.
- `search.rs` — free-text LIKE search over `mem_tree_entity_index`.
- `drill_down.rs` — BFS walk of summary children with optional semantic rerank.
- `fetch.rs` — batch hydration of leaf chunks.
- `rpc.rs` — request / response structs and the JSON-RPC handler bodies.
- `schemas.rs` — `ControllerSchema` definitions and dispatch table for the controller registry.
- `integration_test.rs` — end-to-end test that drives the real ingest pipeline through every retrieval tool.

## Tests

Per-tool unit tests live in `mod tests` inside each file. The `integration_test.rs` module is private to this crate and exercises ingest → seal → retrieve in one workspace.
