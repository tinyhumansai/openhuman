//! SQLite-backed unified namespace memory store.

use parking_lot::Mutex;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::embeddings::EmbeddingProvider;

/// SQLite-backed unified memory store.
///
/// Owns a single connection (WAL-mode) plus the on-disk markdown sidecar
/// directory and vector storage path. Methods are added across the sibling
/// modules (`documents`, `kv`, `graph`, `query`, …) via `impl` blocks.
pub struct UnifiedMemory {
    pub(crate) workspace_dir: PathBuf,
    pub(crate) db_path: PathBuf,
    pub(crate) vectors_dir: PathBuf,
    pub(crate) conn: Arc<Mutex<Connection>>,
    pub(crate) embedder: Arc<dyn EmbeddingProvider>,
}

mod documents;
pub mod events;
pub mod fts5;
mod graph;
mod helpers;
mod init;
mod kv;
pub mod profile;
mod query;
pub mod segments;
