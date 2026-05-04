//! RPC operations for the memory system.
//!
//! This module implements the handlers for memory-related RPC requests, including
//! document management, semantic queries, key-value storage, and knowledge graph
//! operations. It manages the active memory client and provides utility functions
//! for formatting and filtering memory results.
//!
//! Internally the implementation is split across submodules by RPC family:
//!
//! - [`envelope`] — `ApiEnvelope`/`ApiError` wrapping helpers shared by every
//!   envelope-style handler.
//! - [`helpers`] — formatting, default constants, path validators, and the
//!   active memory-client lookup.
//! - [`documents`] — document/namespace direct API and the envelope-style
//!   façade (`memory_init`, `memory_list_documents`, `memory_query_namespace`,
//!   recall_*).
//! - [`kv_graph`] — key-value and knowledge-graph handlers.
//! - [`sync`] — `memory_sync_*` and `memory_ingestion_status`.
//! - [`learn`] — `memory_learn_all`.
//! - [`files`] — `ai_*_memory_file` handlers (use `tokio::fs`).

pub mod documents;
pub mod envelope;
pub mod files;
pub mod helpers;
pub mod kv_graph;
pub mod learn;
pub mod sync;

// ---------------------------------------------------------------------------
// Re-exports preserving the previous flat `memory::ops::*` surface.
// ---------------------------------------------------------------------------

pub use documents::{
    clear_namespace, context_query, context_recall, doc_delete, doc_ingest, doc_list, doc_put,
    memory_delete_document, memory_init, memory_list_documents, memory_list_namespaces,
    memory_query_namespace, memory_recall_context, memory_recall_memories, namespace_list,
    ClearNamespaceParams, ClearNamespaceResult, DeleteDocParams, IngestDocParams,
    NamespaceOnlyParams, PutDocParams, PutDocResult, QueryNamespaceParams, RecallNamespaceParams,
};
pub use files::{ai_list_memory_files, ai_read_memory_file, ai_write_memory_file};
pub use kv_graph::{
    graph_query, graph_upsert, kv_delete, kv_get, kv_list_namespace, kv_set, GraphQueryParams,
    GraphUpsertParams, KvGetDeleteParams, KvSetParams,
};
pub use learn::{memory_learn_all, LearnAllParams, LearnAllResult, NamespaceLearnResult};
pub use sync::{
    memory_ingestion_status, memory_sync_all, memory_sync_channel, IngestionStatusResult,
    SyncAllResult, SyncChannelParams, SyncChannelResult,
};

// ---------------------------------------------------------------------------
// Test-only re-exports — keep the existing `ops_tests.rs` happy without
// changing the test file. The tests reference private helpers via `super::*`.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) use envelope::{error_envelope, memory_counts, memory_request_id};
#[cfg(test)]
pub(crate) use helpers::{
    build_retrieval_context, chunk_metadata, default_category, default_priority,
    default_source_type, extract_entity_type, filter_hits_by_document_ids,
    format_llm_context_message, maybe_retrieval_context, memory_kind_label, relation_identity,
    relation_metadata, timestamp_to_rfc3339, validate_memory_relative_path,
};

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
