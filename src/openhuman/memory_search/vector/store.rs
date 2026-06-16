//! Re-export of VectorStore from memory_store::vectors::store.
//!
//! The implementation stays in memory_store::vectors::store.rs; this module
//! provides the canonical import path under memory_search.

pub use crate::openhuman::memory_store::vectors::store::*;
