//! Source-specific policy layer for the memory tree.
//!
//! This module owns the parts of the source-tree path that are not generic:
//! - [`file`] — the `_source.md` on-disk mirror (one file per ingest source)
//! - [`registry`] — `get_or_create_source_tree`: wraps the generic
//!   [`crate::openhuman::memory_tree::tree::registry::get_or_create_tree`]
//!   and triggers the `_source.md` write as a source-specific side-effect.
//!
//! Generic tree mechanics (storage, buffer management, bucket-seal,
//! flush, id generation) live in [`crate::openhuman::memory_tree::tree`].

pub mod file;
pub mod registry;

pub use registry::get_or_create_source_tree;
