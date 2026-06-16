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
    delete_source_rpc, flush_now_rpc, flush_source_tree_rpc, reset_tree_rpc, wipe_all_rpc,
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
    ChunkFilter, ChunkRow, DeleteChunkResponse, DeleteSourceResponse, EntityRef, FlushNowResponse,
    FlushSourceTreeResponse, ListChunksResponse, ObsidianVaultStatusResponse, RecallResponse,
    ResetTreeResponse, ScoreBreakdown, ScoreSignal, Source, VaultHealthCheckResponse,
    WipeAllResponse,
};
pub use vault::{obsidian_vault_status_rpc, vault_health_check_rpc};

#[allow(dead_code)]
pub(crate) fn parse_source_kind_str(
    s: &str,
) -> Option<crate::openhuman::memory_store::chunks::types::SourceKind> {
    crate::openhuman::memory_store::chunks::types::SourceKind::parse(s).ok()
}

#[cfg(test)]
pub(crate) use crate::openhuman::config::Config;
#[cfg(test)]
pub(crate) use crate::openhuman::memory_store::chunks::store::with_connection;
#[cfg(test)]
pub(crate) use crate::openhuman::memory_store::chunks::types::SourceKind;
#[cfg(test)]
pub(crate) use admin::clear_composio_sync_state;

#[cfg(test)]
#[path = "../read_rpc_tests.rs"]
mod tests;
