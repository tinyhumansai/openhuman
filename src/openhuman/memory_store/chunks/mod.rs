//! Chunks — the unit of memory_store persistence.
//!
//! One module for the full chunk lifecycle:
//!
//! - [`types`]    — `Chunk`, `Metadata`, `SourceKind`, `RawRef`,
//!                  `ListChunksQuery`. The persisted shape.
//! - [`store`]    — SQLite persistence (`chunks` table + connection cache).
//! - [`produce`]  — source-kind-dispatch chunker (chat / email / document).
//!                  Used by the memory ingest pipeline; produces stable
//!                  per-source sequence numbers and bounded segments.
//! - [`semantic`] — heading- and paragraph-aware chunker used by the
//!                  unified memory writer to split large documents into
//!                  LLM-context-sized pieces while preserving heading
//!                  context.
//!
//! `produce::chunk_markdown` (the default) and `semantic::chunk_markdown`
//! both yield string-shaped chunks; the store side decides what to do with
//! them.

pub mod produce;
pub mod semantic;
pub mod store;
pub mod types;

pub use produce::{chunk_markdown, ChunkerInput, ChunkerOptions};
pub use semantic::chunk_markdown as chunk_semantic;
pub use store::*;
pub use types::*;
