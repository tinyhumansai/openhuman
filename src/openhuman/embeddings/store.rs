//! Local vector store backed by SQLite.
//!
//! Provides a self-contained vector database for storing, searching, and
//! managing text embeddings. Uses SQLite for persistence and brute-force
//! cosine similarity for retrieval (fast enough for on-device workloads up
//! to ~100K vectors).
//!
//! # Usage
//!
//! ```ignore
//! let embedder = Arc::new(OllamaEmbedding::default());
//! let store = VectorStore::open(db_path, embedder)?;
//!
//! store.insert("doc-1", "notes", "The quick brown fox", json!({})).await?;
//! let results = store.search("notes", "fast animal", 5).await?;
//! ```

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

use super::EmbeddingProvider;

/// SQL to create the vector store schema.
const INIT_SQL: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;

    CREATE TABLE IF NOT EXISTS vectors (
        id         TEXT    NOT NULL,
        namespace  TEXT    NOT NULL,
        text       TEXT    NOT NULL,
        embedding  BLOB    NOT NULL,
        metadata   TEXT    NOT NULL DEFAULT '{}',
        created_at REAL    NOT NULL,
        updated_at REAL    NOT NULL,
        PRIMARY KEY (namespace, id)
    );
    CREATE INDEX IF NOT EXISTS idx_vectors_ns ON vectors(namespace);

    CREATE TABLE IF NOT EXISTS store_meta (
        key        TEXT    PRIMARY KEY,
        value      TEXT    NOT NULL,
        updated_at REAL    NOT NULL
    );
";

/// A single search result from the vector store.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The stored document ID.
    pub id: String,
    /// The namespace.
    pub namespace: String,
    /// The original text.
    pub text: String,
    /// Cosine similarity score (0.0 – 1.0).
    pub score: f64,
    /// Arbitrary JSON metadata attached at insert time.
    pub metadata: serde_json::Value,
}

/// SQLite-backed local vector store.
///
/// Thread-safe: the inner connection is behind a `parking_lot::Mutex` and
/// the struct is `Send + Sync`. Embedding calls are async and run through
/// the configured [`EmbeddingProvider`].
pub struct VectorStore {
    conn: Arc<Mutex<Connection>>,
    embedder: Arc<dyn EmbeddingProvider>,
}

impl VectorStore {
    /// Opens (or creates) a vector store at the given SQLite database path.
    ///
    /// On first open the embedding provider name, model-name-hint, and
    /// dimensions are persisted to a `store_meta` table. On subsequent opens
    /// the stored dimensions are compared against the runtime embedder and an
    /// error is returned if they mismatch (prevents silent cosine-similarity
    /// corruption from mixed-dimension vectors).
    pub fn open(db_path: &Path, embedder: Arc<dyn EmbeddingProvider>) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch(INIT_SQL)?;

        Self::check_or_store_meta(&conn, &*embedder)?;

        tracing::debug!(
            target: "embeddings.store",
            "[vector-store] opened at {}, embedder={}, dims={}",
            db_path.display(),
            embedder.name(),
            embedder.dimensions()
        );

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            embedder,
        })
    }

    /// Opens an in-memory vector store (useful for tests).
    pub fn open_in_memory(embedder: Arc<dyn EmbeddingProvider>) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(INIT_SQL)?;
        Self::check_or_store_meta(&conn, &*embedder)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            embedder,
        })
    }

    /// Returns a reference to the embedding provider.
    pub fn embedder(&self) -> &dyn EmbeddingProvider {
        self.embedder.as_ref()
    }

    /// Persist or validate the embedding configuration in `store_meta`.
    fn check_or_store_meta(
        conn: &Connection,
        embedder: &dyn EmbeddingProvider,
    ) -> anyhow::Result<()> {
        let now = now_ts();
        let stored_dims: Option<String> = conn
            .query_row(
                "SELECT value FROM store_meta WHERE key = 'embed_dims'",
                [],
                |row| row.get(0),
            )
            .ok();

        match stored_dims {
            None => {
                // First open — persist metadata.
                let stmts: &[(&str, &str)] = &[
                    ("embed_provider", embedder.name()),
                    ("embed_dims", &embedder.dimensions().to_string()),
                ];
                for (key, value) in stmts {
                    conn.execute(
                        "INSERT OR REPLACE INTO store_meta (key, value, updated_at) VALUES (?1, ?2, ?3)",
                        rusqlite::params![key, value, now],
                    )?;
                }
                tracing::debug!(
                    target: "embeddings.store",
                    "[vector-store] stored meta: provider={}, dims={}",
                    embedder.name(),
                    embedder.dimensions()
                );
            }
            Some(dims_str) => {
                let stored: usize = dims_str.parse().unwrap_or(0);
                let runtime = embedder.dimensions();
                if stored != 0 && runtime != 0 && stored != runtime {
                    anyhow::bail!(
                        "vector store dimension mismatch: database was created with \
                         {stored}-dim embeddings but the current provider ({}) uses \
                         {runtime} dims. Delete the database or reconfigure the provider.",
                        embedder.name()
                    );
                }
            }
        }

        Ok(())
    }

    // ── Write operations ─────────────────────────────────────

    /// Inserts or updates a text entry. The text is embedded automatically.
    ///
    /// If an entry with the same `(namespace, id)` already exists it is replaced.
    pub async fn insert(
        &self,
        id: &str,
        namespace: &str,
        text: &str,
        metadata: serde_json::Value,
    ) -> anyhow::Result<()> {
        tracing::trace!(
            target: "embeddings.store",
            "[vector-store] insert: id={id}, ns={namespace}, text_len={}",
            text.len()
        );
        let embedding = self.embedder.embed_one(text).await?;
        self.insert_with_vector(id, namespace, text, &embedding, metadata)
    }

    /// Inserts with a pre-computed embedding vector (skips the embed call).
    pub fn insert_with_vector(
        &self,
        id: &str,
        namespace: &str,
        text: &str,
        embedding: &[f32],
        metadata: serde_json::Value,
    ) -> anyhow::Result<()> {
        let blob = vec_to_bytes(embedding);
        let meta_str = serde_json::to_string(&metadata)?;
        let now = now_ts();

        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO vectors (id, namespace, text, embedding, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, namespace, text, blob, meta_str, now, now],
        )?;

        tracing::trace!(
            target: "embeddings.store",
            "[vector-store] inserted id={id} ns={namespace} dims={}",
            embedding.len()
        );

        Ok(())
    }

    /// Bulk-insert multiple entries. Each text is embedded automatically.
    pub async fn insert_batch(
        &self,
        namespace: &str,
        entries: &[(&str, &str, serde_json::Value)], // (id, text, metadata)
    ) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        tracing::debug!(
            target: "embeddings.store",
            "[vector-store] insert_batch: ns={namespace}, count={}",
            entries.len()
        );

        let texts: Vec<&str> = entries.iter().map(|(_, text, _)| *text).collect();
        let embeddings = self.embedder.embed(&texts).await?;

        if embeddings.len() != entries.len() {
            anyhow::bail!(
                "embedding count mismatch: got {} embeddings for {} entries",
                embeddings.len(),
                entries.len()
            );
        }

        let now = now_ts();
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        for ((id, text, metadata), embedding) in entries.iter().zip(embeddings.iter()) {
            let blob = vec_to_bytes(embedding);
            let meta_str = serde_json::to_string(metadata)?;
            tx.execute(
                "INSERT OR REPLACE INTO vectors (id, namespace, text, embedding, metadata, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, namespace, text, blob, meta_str, now, now],
            )?;
        }

        tx.commit()?;

        tracing::debug!(
            target: "embeddings.store",
            "[vector-store] batch inserted {} entries in ns={namespace}",
            entries.len()
        );

        Ok(())
    }

    // ── Search ───────────────────────────────────────────────

    /// Searches for the `limit` most similar entries to `query` within a namespace.
    ///
    /// The query is embedded via the configured provider and compared against
    /// all stored vectors using cosine similarity.
    pub async fn search(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        tracing::trace!(
            target: "embeddings.store",
            "[vector-store] search: ns={namespace}, limit={limit}, query_len={}",
            query.len()
        );
        let query_vec = self.embedder.embed_one(query).await?;
        self.search_by_vector(namespace, &query_vec, limit)
    }

    /// Searches using a pre-computed query vector.
    pub fn search_by_vector(
        &self,
        namespace: &str,
        query_vec: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        if limit == 0 {
            tracing::trace!(
                target: "embeddings.store",
                "[vector-store] search_by_vector: limit=0, returning empty"
            );
            return Ok(Vec::new());
        }

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, namespace, text, embedding, metadata FROM vectors WHERE namespace = ?1",
        )?;

        let rows: Vec<(String, String, String, Vec<u8>, String)> = stmt
            .query_map(rusqlite::params![namespace], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut scored: Vec<SearchResult> = rows
            .into_iter()
            .map(|(id, ns, text, blob, meta_str)| {
                let stored_vec = bytes_to_vec(&blob);
                let score = cosine_similarity(query_vec, &stored_vec);
                let metadata = serde_json::from_str(&meta_str).unwrap_or(serde_json::Value::Null);
                SearchResult {
                    id,
                    namespace: ns,
                    text,
                    score,
                    metadata,
                }
            })
            .collect();

        // Sort descending by score.
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);

        tracing::trace!(
            target: "embeddings.store",
            "[vector-store] search_by_vector: ns={namespace}, scanned={}, returned={}",
            scored.len() + scored.capacity() - scored.len(), // approximate total before truncate
            scored.len()
        );

        Ok(scored)
    }

    // ── Delete / management ──────────────────────────────────

    /// Deletes a single entry by ID within a namespace.
    ///
    /// Returns `true` if a row was actually deleted.
    pub fn delete(&self, namespace: &str, id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock();
        let affected = conn.execute(
            "DELETE FROM vectors WHERE namespace = ?1 AND id = ?2",
            rusqlite::params![namespace, id],
        )?;

        tracing::trace!(
            target: "embeddings.store",
            "[vector-store] delete: ns={namespace}, id={id}, affected={affected}"
        );

        Ok(affected > 0)
    }

    /// Deletes all entries in a namespace.
    ///
    /// Returns the number of deleted rows.
    pub fn clear_namespace(&self, namespace: &str) -> anyhow::Result<usize> {
        let conn = self.conn.lock();
        let affected = conn.execute(
            "DELETE FROM vectors WHERE namespace = ?1",
            rusqlite::params![namespace],
        )?;

        tracing::debug!(
            target: "embeddings.store",
            "[vector-store] cleared namespace={namespace}, deleted={affected}"
        );

        Ok(affected)
    }

    /// Returns the number of entries in a namespace (or all if `None`).
    pub fn count(&self, namespace: Option<&str>) -> anyhow::Result<usize> {
        let conn = self.conn.lock();
        let count: usize = match namespace {
            Some(ns) => conn.query_row(
                "SELECT COUNT(*) FROM vectors WHERE namespace = ?1",
                rusqlite::params![ns],
                |row| row.get(0),
            )?,
            None => conn.query_row("SELECT COUNT(*) FROM vectors", [], |row| row.get(0))?,
        };
        Ok(count)
    }

    /// Lists all distinct namespaces.
    pub fn list_namespaces(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT namespace FROM vectors ORDER BY namespace")?;
        let namespaces: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(namespaces)
    }
}

// ── Vector math utilities ────────────────────────────────────

/// Serializes a float vector to little-endian bytes for SQLite BLOB storage.
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// Deserializes little-endian bytes back to a float vector.
pub fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            f32::from_le_bytes(arr)
        })
        .collect()
}

/// Computes cosine similarity between two vectors. Returns 0.0 for
/// mismatched lengths, empty vectors, or zero-magnitude vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= f64::EPSILON {
        return 0.0;
    }
    (dot / denom).clamp(0.0, 1.0)
}

fn now_ts() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
