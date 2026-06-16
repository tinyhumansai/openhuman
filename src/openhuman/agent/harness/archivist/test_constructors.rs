//! Test-only constructors for `ArchivistHook` that inject stub providers
//! directly, bypassing `with_config`'s provider-build logic.

use super::types::ArchivistHook;
use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::ChatProvider;
use crate::openhuman::memory_store::segments::BoundaryConfig;
use crate::openhuman::memory_tree::score::embed::Embedder;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

#[cfg(test)]
impl ArchivistHook {
    /// Test-only constructor that injects a stub `ChatProvider` and `Embedder`
    /// directly, bypassing `with_config`'s provider-build logic. Used by
    /// Phase 1 tests to verify LLM recap and embedding paths without hitting
    /// a real LLM or Ollama daemon. Exposed as `pub(crate)` so Phase 3
    /// STM recall integration tests can drive the full archivist path.
    pub(crate) fn new_with_stubs(
        conn: Arc<Mutex<Connection>>,
        chat_provider: Arc<dyn ChatProvider>,
        embedder: Arc<dyn Embedder>,
    ) -> Self {
        Self {
            conn: Some(conn),
            enabled: true,
            boundary_config: BoundaryConfig::default(),
            config: Some(Config::default()),
            chat_provider: Some(chat_provider),
            embedder: Some(embedder),
        }
    }

    /// Test-only constructor that injects stub providers AND a `Config`, so the
    /// Phase 2 segment-tree ingest path (gated by
    /// `config.learning.chat_to_tree_enabled`) can be exercised hermetically.
    ///
    /// `config.learning.chat_to_tree_enabled` must be set to `true` by the caller
    /// for the tree ingest to fire; the hook does NOT force it on.
    pub(crate) fn new_with_stubs_and_config(
        conn: Arc<Mutex<Connection>>,
        chat_provider: Arc<dyn ChatProvider>,
        embedder: Arc<dyn Embedder>,
        config: Config,
    ) -> Self {
        Self {
            conn: Some(conn),
            enabled: true,
            boundary_config: BoundaryConfig::default(),
            config: Some(config),
            chat_provider: Some(chat_provider),
            embedder: Some(embedder),
        }
    }
}
