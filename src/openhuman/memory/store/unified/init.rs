use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::openhuman::memory::embeddings::EmbeddingProvider;
use crate::openhuman::memory::store::types::GLOBAL_NAMESPACE;

use super::UnifiedMemory;

impl UnifiedMemory {
    pub fn new(
        workspace_dir: &Path,
        embedder: Arc<dyn EmbeddingProvider>,
        _open_timeout_secs: Option<u64>,
    ) -> anyhow::Result<Self> {
        let memory_dir = workspace_dir.join("memory");
        let namespaces_dir = memory_dir.join("namespaces");
        let vectors_dir = memory_dir.join("vectors");
        std::fs::create_dir_all(&namespaces_dir)?;
        std::fs::create_dir_all(&vectors_dir)?;

        let db_path = memory_dir.join("memory.db");
        let conn = Connection::open(&db_path)?;
        // Active storage layout for the core memory domain:
        // - memory_docs: namespace-scoped source documents and markdown metadata.
        // - vector_chunks: chunked document text plus optional local embedding bytes.
        // - graph_namespace: namespace graph edges used for relation-first retrieval.
        // - graph_global: cross-namespace graph edges used as fallback/shared memory.
        // - kv_namespace: namespace-scoped durable preferences, decisions, and state.
        // - kv_global: global durable key-value memories outside a namespace scope.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;

             CREATE TABLE IF NOT EXISTS memory_docs (
               document_id TEXT PRIMARY KEY,
               namespace TEXT NOT NULL,
               key TEXT NOT NULL,
               title TEXT NOT NULL,
               content TEXT NOT NULL,
               source_type TEXT NOT NULL,
               priority TEXT NOT NULL,
               tags_json TEXT NOT NULL,
               metadata_json TEXT NOT NULL,
               category TEXT NOT NULL,
               session_id TEXT,
               created_at REAL NOT NULL,
               updated_at REAL NOT NULL,
               markdown_rel_path TEXT NOT NULL,
               UNIQUE(namespace, key)
             );
             CREATE INDEX IF NOT EXISTS idx_memory_docs_ns_updated ON memory_docs(namespace, updated_at DESC);

             CREATE TABLE IF NOT EXISTS kv_global (
               key TEXT PRIMARY KEY,
               value_json TEXT NOT NULL,
               updated_at REAL NOT NULL
             );

             CREATE TABLE IF NOT EXISTS kv_namespace (
               namespace TEXT NOT NULL,
               key TEXT NOT NULL,
               value_json TEXT NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(namespace, key)
             );
             CREATE INDEX IF NOT EXISTS idx_kv_namespace_ns ON kv_namespace(namespace);

             CREATE TABLE IF NOT EXISTS graph_global (
               subject TEXT NOT NULL,
               predicate TEXT NOT NULL,
               object TEXT NOT NULL,
               attrs_json TEXT NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(subject, predicate, object)
             );
             CREATE INDEX IF NOT EXISTS idx_graph_global_subject ON graph_global(subject, predicate);

             CREATE TABLE IF NOT EXISTS graph_namespace (
               namespace TEXT NOT NULL,
               subject TEXT NOT NULL,
               predicate TEXT NOT NULL,
               object TEXT NOT NULL,
               attrs_json TEXT NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(namespace, subject, predicate, object)
             );
             CREATE INDEX IF NOT EXISTS idx_graph_namespace_ns ON graph_namespace(namespace);
             CREATE INDEX IF NOT EXISTS idx_graph_namespace_subject ON graph_namespace(namespace, subject, predicate);

             CREATE TABLE IF NOT EXISTS vector_chunks (
               namespace TEXT NOT NULL,
               document_id TEXT NOT NULL,
               chunk_id TEXT NOT NULL,
               text TEXT NOT NULL,
               embedding BLOB,
               metadata_json TEXT NOT NULL,
               created_at REAL NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(namespace, chunk_id)
             );
             CREATE INDEX IF NOT EXISTS idx_vector_chunks_ns_doc ON vector_chunks(namespace, document_id);",
        )?;

        // Create FTS5 episodic tables (episodic_log, episodic_fts, and their
        // triggers) so the Archivist can call episodic_insert immediately after
        // the store is initialised.
        conn.execute_batch(super::fts5::EPISODIC_INIT_SQL)?;

        // Conversation segmentation tables.
        conn.execute_batch(super::segments::SEGMENTS_INIT_SQL)?;

        // Event extraction tables.
        conn.execute_batch(super::events::EVENTS_INIT_SQL)?;

        // User profile accumulation table.
        conn.execute_batch(super::profile::PROFILE_INIT_SQL)?;

        // Idempotent legacy-namespace migration.
        //
        // Older writes via MemoryStoreTool packed the intended namespace into
        // the key as `"{namespace}/{actual_key}"` and stored the row under the
        // GLOBAL_NAMESPACE. Split those rows now so the new trait surface can
        // rely on the `namespace` column.
        //
        // The anti-join guard prevents duplicate-split collisions if a
        // post-split row already exists (UNIQUE(namespace, key) would otherwise
        // fail). Safe to run on every boot.
        let migrated = conn.execute(
            "UPDATE memory_docs
             SET namespace = substr(key, 1, instr(key, '/') - 1),
                 key = substr(key, instr(key, '/') + 1)
             WHERE namespace = ?1
               AND instr(key, '/') > 0
               AND NOT EXISTS (
                 SELECT 1 FROM memory_docs m2
                 WHERE m2.namespace = substr(memory_docs.key, 1, instr(memory_docs.key, '/') - 1)
                   AND m2.key = substr(memory_docs.key, instr(memory_docs.key, '/') + 1)
               )",
            rusqlite::params![GLOBAL_NAMESPACE],
        )?;
        if migrated > 0 {
            log::info!(
                "[memory] migrated {migrated} legacy `ns/key` rows out of the `{GLOBAL_NAMESPACE}` namespace"
            );
        }

        // Companion migration: `vector_chunks` rows keyed by `document_id` still
        // point at `GLOBAL_NAMESPACE` after the `memory_docs` split above, so
        // namespace-scoped recall would miss them. Re-home each chunk to its
        // document's new namespace. Idempotent: after both migrations run, no
        // chunk under GLOBAL_NAMESPACE maps to a document in another namespace.
        let chunks_migrated = conn.execute(
            "UPDATE vector_chunks
             SET namespace = (
                 SELECT namespace FROM memory_docs
                 WHERE memory_docs.document_id = vector_chunks.document_id
             )
             WHERE namespace = ?1
               AND document_id IN (
                 SELECT document_id FROM memory_docs WHERE namespace != ?1
               )",
            rusqlite::params![GLOBAL_NAMESPACE],
        )?;
        if chunks_migrated > 0 {
            log::info!(
                "[memory] migrated {chunks_migrated} vector_chunks rows out of the `{GLOBAL_NAMESPACE}` namespace"
            );
        }

        Ok(Self {
            workspace_dir: workspace_dir.to_path_buf(),
            db_path,
            vectors_dir,
            conn: Arc::new(Mutex::new(conn)),
            embedder,
        })
    }

    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn vectors_dir(&self) -> &Path {
        &self.vectors_dir
    }

    pub(crate) fn now_ts() -> f64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    pub(crate) fn sanitize_namespace(namespace: &str) -> String {
        let trimmed = namespace.trim();
        if trimmed.is_empty() {
            return GLOBAL_NAMESPACE.to_string();
        }
        trimmed
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' {
                    ch
                } else {
                    '_'
                }
            })
            .collect()
    }

    pub(crate) fn namespace_dir(&self, namespace: &str) -> PathBuf {
        self.workspace_dir
            .join("memory")
            .join("namespaces")
            .join(Self::sanitize_namespace(namespace))
    }
}
