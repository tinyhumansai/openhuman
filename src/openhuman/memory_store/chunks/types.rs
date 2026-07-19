//! Core types for the memory tree ingestion layer (Phase 1 / issue #707).
//!
//! This module defines the canonical [`Chunk`] representation produced by the
//! ingestion pipeline along with its provenance [`Metadata`] and back-pointer
//! [`SourceRef`]. These types feed into later phases (#708 scoring, #709
//! summary trees, #710 retrieval) but are self-contained at Phase 1.
//!
//! All chunk IDs are deterministic: `sha256(source_kind | "\0" | source_id |
//! "\0" | seq | "\0" | content)` truncated to 32 hex chars so re-ingest of the
//! same source material yields stable IDs and idempotent upserts.
//!
//! **W3 type cutover:** these types + chunk-id/token helpers are now
//! **re-exported from the `tinycortex` crate** (ported from this exact module —
//! identical fields, derives, serde wire form, and `chunk_id` derivation, all
//! pinned by `tinycortex::memory::chunks::types_tests`). Re-exporting keeps one source of truth and lets
//! the chunk store operations delegate to the crate without host↔crate type
//! conversions. `DataSource` moved with the ingest cutover and is re-exported
//! here alongside the chunk types. `StagedChunk` remains host-owned in
//! `memory_store::content`.

pub use tinycortex::memory::chunks::{
    approx_token_count, chunk_id, conservative_token_estimate, truncate_to_conservative_tokens,
    Chunk, DataSource, Metadata, SourceKind, SourceRef,
};
