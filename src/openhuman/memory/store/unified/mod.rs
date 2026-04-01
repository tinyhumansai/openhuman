//! SQLite-backed unified namespace memory store.

use parking_lot::Mutex;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::memory::embeddings::EmbeddingProvider;

pub struct UnifiedMemory {
    pub(crate) workspace_dir: PathBuf,
    pub(crate) db_path: PathBuf,
    pub(crate) vectors_dir: PathBuf,
    pub(crate) conn: Arc<Mutex<Connection>>,
    pub(crate) embedder: Arc<dyn EmbeddingProvider>,
}

mod documents;
pub mod fts5;
mod graph;
mod helpers;
mod init;
mod kv;
mod query;
