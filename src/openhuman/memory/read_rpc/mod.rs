//! Read RPCs that back the new Memory tab UI.
//!
//! Distinct from [`super::rpc`] (write/ingest) and [`super::retrieval::rpc`]
//! (LLM-callable retrieval primitives), this module exposes a small set of
//! "list / inspect / search / recall / score-for / delete" methods designed
//! for a human-facing dashboard — not for an LLM tool loop.
//!
//! All methods are scoped under the existing `memory_tree` JSON-RPC
//! namespace so they share authentication, telemetry, and discovery with
//! the other memory-tree RPCs.

pub mod admin;
pub mod chunks;
pub mod entities;
pub mod graph;
pub mod types;
pub mod vault;

// Re-export everything so consumers and the test file keep working with `use super::*;`
pub use admin::{
    backfill_connector_trees_rpc, delete_source_rpc, flush_now_rpc, flush_source_tree_rpc,
    reset_tree_rpc, wipe_all_rpc,
};
pub use chunks::{
    display_name_for_source, list_chunks_rpc, list_sources_rpc, read_chunk_row, recall_rpc,
    search_rpc,
};
pub use entities::{
    chunk_score_rpc, chunks_for_entity_rpc, delete_chunk_rpc, entity_index_for_rpc,
    top_entities_rpc,
};
pub use graph::{
    graph_export_rpc, sanitize_basename, GraphEdge, GraphExportResponse, GraphMode, GraphNode,
};
pub use types::{
    BackfillConnectorTreesResponse, ChunkFilter, ChunkRow, DeleteChunkResponse,
    DeleteSourceResponse, EntityRef, FlushNowResponse, FlushSourceTreeResponse, ListChunksResponse,
    ObsidianVaultStatusResponse, RecallResponse, ResetTreeResponse, ScoreBreakdown, ScoreSignal,
    Source, VaultHealthCheckResponse, WipeAllResponse,
};
pub use vault::{obsidian_vault_status_rpc, vault_health_check_rpc};

#[allow(dead_code)]
pub(crate) fn parse_source_kind_str(s: &str) -> Option<tinymemory_api::chunks::SourceKind> {
    tinymemory_api::chunks::SourceKind::parse(s).ok()
}

// Re-exports for `read_rpc_tests.rs`, which drives this module with
// `use super::*;`.
//
// `with_connection` is the raw SQLite door into the engine's chunk store, and
// it is deliberately the only one left here: the tests use it to assert on rows
// the RPCs wrote, which is the point of asserting *storage* rather than
// re-reading through the same handler. It survives the #5560 shed because it is
// `#[cfg(test)]` — `cfg(test)` code links the `tinymemory-core`
// **dev-dependency**, which stays after the normal dependency is dropped, so
// this line does not keep the engine crate in the shipped binary. Nothing
// production-side in `read_rpc` names the engine any more; `wipe_all`,
// `clear_composio_sync_state` and `delete_source` left for `purge_all`,
// `kv_list` + `kv_delete` and `forget_matching(Source)`.
#[cfg(test)]
pub(crate) use crate::openhuman::config::Config;
#[cfg(test)]
pub(crate) use tinymemory_api::chunks::SourceKind;

#[cfg(test)]
#[path = "../read_rpc_tests.rs"]
mod tests;
