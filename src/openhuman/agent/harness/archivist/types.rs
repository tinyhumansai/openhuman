//! Core type definition for the Archivist hook.

use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::ChatProvider;
use crate::openhuman::memory_store::segments::BoundaryConfig;
use crate::openhuman::memory_tree::score::embed::Embedder;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

/// Background Archivist that indexes turns into FTS5 episodic memory
/// and manages conversation segmentation.
///
/// Produces an LLM recap + embedding for each closed segment and flushes
/// the trailing open segment at session end.
pub struct ArchivistHook {
    /// SQLite connection shared with UnifiedMemory.
    pub(super) conn: Option<Arc<Mutex<Connection>>>,
    /// Whether the archivist is enabled.
    pub(super) enabled: bool,
    /// Boundary detection configuration.
    pub(super) boundary_config: BoundaryConfig,
    /// Optional runtime config — used to gate the tree-ingest path and to
    /// build the LLM chat provider + embedder.
    ///
    /// When `None`, the tree-ingest path is skipped. Set via
    /// [`ArchivistHook::with_config`] on the production path.
    pub(super) config: Option<Config>,
    /// Optional LLM provider for segment recap. When `None`, the
    /// fallback heuristic summary is used instead.
    pub(super) chat_provider: Option<Arc<dyn ChatProvider>>,
    /// Optional embedder for segment recap vectors. When `None`, embedding
    /// is skipped (segment is still summarised).
    pub(super) embedder: Option<Arc<dyn Embedder>>,
}
