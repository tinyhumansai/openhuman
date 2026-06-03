//! Core data types for the OpenHuman memory system.
//!
//! This crate contains the pure data structures used throughout the memory
//! pipeline: chunks, trees, summaries, entities, scores. They have no logic
//! dependencies — only `serde`, `chrono`, and `sha2`.

pub mod chunk;
pub mod tree;

// Re-exports for convenience
pub use chunk::{
    approx_token_count, chunk_id, Chunk, DataSource, Metadata, SourceKind, SourceRef,
};
pub use tree::{
    Buffer, EntityIndexStats, HotnessCounters, SummaryNode, Tree, TreeKind, TreeStatus,
    DEFAULT_FLUSH_AGE_SECS, INPUT_TOKEN_BUDGET, OUTPUT_TOKEN_BUDGET, SUMMARY_FANOUT,
    TOPIC_ARCHIVE_THRESHOLD, TOPIC_CREATION_THRESHOLD, TOPIC_RECHECK_EVERY,
};
