use anyhow::Context;
use once_cell::sync::OnceCell;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{ffi, Connection, OpenFlags, OptionalExtension};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// r2d2 pool of read-only SQLite connections. Used for file-backed indexes
/// in production so WAL actually buys us concurrent readers; in-memory test
/// indexes keep the single-connection layout since a shared-cache URI adds
/// ceremony for no throughput win at test-fixture scale.
pub(crate) type ReaderPool = r2d2::Pool<SqliteConnectionManager>;

/// Personal index — SQLite database with FTS5 + sqlite-vec virtual tables loaded.
///
/// Internally split into a dedicated writer connection (serialised behind
/// `tokio::sync::Mutex`) and an optional pool of read-only connections. Writes
/// go through the writer (single-writer SQLite model); searches use the pool
/// when present so concurrent RPC callers don't block on each other or on a
/// long-running ingest. In-memory handles leave the pool `None` — both
/// readers and writers share the single connection, matching the pre-pool
/// behaviour tests depend on.
pub struct PersonalIndex {
    /// Writer connection — always present. Also used as the sole reader when
    /// `reader_pool` is `None` (in-memory handles).
    pub writer: Arc<Mutex<Connection>>,
    /// Optional read-only connection pool. `Some` for file-backed indexes;
    /// `None` for in-memory ones so the writer's schema is visible to readers
    /// without a shared-cache URI.
    pub(crate) reader_pool: Option<Arc<ReaderPool>>,
}

static VEC_REGISTERED: OnceCell<()> = OnceCell::new();

/// Register `sqlite3_vec_init` as a SQLite auto-extension exactly once per process.
/// Every connection opened after this point loads the vec0 module automatically.
pub(crate) fn ensure_vec_extension_registered() {
    VEC_REGISTERED.get_or_init(|| unsafe {
        let init: unsafe extern "C" fn() = sqlite_vec::sqlite3_vec_init;
        let entry: unsafe extern "C" fn(
            *mut ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int = std::mem::transmute(init as *const ());
        let rc = ffi::sqlite3_auto_extension(Some(entry));
        if rc != ffi::SQLITE_OK {
            panic!("sqlite3_auto_extension(sqlite_vec) failed: rc={rc}");
        }
    });
}

/// Default pool size — small enough to stay within SQLite's default
/// connection fan-out without heroics, large enough that a handful of
/// concurrent RPC searches don't queue up behind each other while an
/// ingest holds the writer. Override is deliberately not exposed: the
/// numbers here are only loosely tuned and are best treated as an
/// implementation detail, not a knob.
const READER_POOL_SIZE: u32 = 4;

impl PersonalIndex {
    /// Open (or create) the personal index at `path`. Loads sqlite-vec, runs
    /// migrations on the writer, then spins up a read-only r2d2 pool against
    /// the same file so searches can run concurrently with an in-flight
    /// write.
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        ensure_vec_extension_registered();
        let path_buf = path.to_path_buf();
        let (writer, pool) =
            tokio::task::spawn_blocking(move || -> anyhow::Result<(Connection, ReaderPool)> {
                // Writer: exclusive RW connection, WAL journal so readers
                // don't block on it, migrations run here.
                let writer = Connection::open(&path_buf).context("open sqlite writer")?;
                writer.pragma_update(None, "journal_mode", "WAL")?;
                writer.pragma_update(None, "foreign_keys", "ON")?;
                super::migrations::run(&writer).context("run life_capture migrations")?;

                // Reader pool: read-only connections on the same file. Each
                // pooled connection gets `query_only = 1` as a belt-and-
                // suspenders guard so a mis-routed write here fails loudly
                // instead of racing the writer. Foreign-key enforcement is
                // writer-only in SQLite, so readers skip it.
                //
                // `SQLITE_OPEN_READ_ONLY` alone would also work, but keeping
                // the connection RW + `query_only` lets sqlite-vec register
                // on the connection at open time (the extension's own init
                // writes to the sqlite_sequence table on first use).
                let manager = SqliteConnectionManager::file(&path_buf).with_init(|c| {
                    c.pragma_update(None, "query_only", "ON")?;
                    Ok(())
                });
                let pool = r2d2::Pool::builder()
                    .max_size(READER_POOL_SIZE)
                    .build(manager)
                    .context("build reader pool")?;
                Ok((writer, pool))
            })
            .await
            .context("open task panicked")??;
        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
            reader_pool: Some(Arc::new(pool)),
        })
    }

    /// Open an in-memory index (for tests). Single connection shared between
    /// reader and writer — keeps the fixture path free of shared-cache URI
    /// ceremony that buys no real concurrency at test-fixture scale.
    pub async fn open_in_memory() -> anyhow::Result<Self> {
        ensure_vec_extension_registered();
        let conn = tokio::task::spawn_blocking(|| -> anyhow::Result<Connection> {
            let conn = Connection::open_with_flags(
                ":memory:",
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_CREATE
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .context("open in-memory sqlite")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            super::migrations::run(&conn).context("run life_capture migrations")?;
            Ok(conn)
        })
        .await
        .context("open task panicked")??;
        Ok(Self {
            writer: Arc::new(Mutex::new(conn)),
            reader_pool: None,
        })
    }
}

/// Writer for the personal index. Upserts items keyed by (source, external_id)
/// so re-ingesting the same source row updates in place; vectors are written
/// separately via `upsert_vector` since embeddings come from a remote call.
pub struct IndexWriter {
    conn: Arc<Mutex<Connection>>,
}

impl IndexWriter {
    pub fn new(idx: &PersonalIndex) -> Self {
        Self {
            conn: Arc::clone(&idx.writer),
        }
    }

    /// Upsert items by (source, external_id). FTS rows are kept in sync via
    /// the SQL triggers defined in 0001_init.sql.
    ///
    /// Mutates `items[i].id` to the canonical (existing) UUID when a row with
    /// the same (source, external_id) already exists — so callers can write the
    /// vector under the correct id without orphaning the previous one. Cascades
    /// `item_vectors` deletes for any caller-supplied id that loses the race.
    pub async fn upsert(
        &self,
        items: &mut [crate::openhuman::life_capture::types::Item],
    ) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        // Clone across the blocking boundary, write canonical ids back below.
        let mut owned = items.to_vec();
        let conn = Arc::clone(&self.conn);
        let result: anyhow::Result<Vec<crate::openhuman::life_capture::types::Item>> =
            tokio::task::spawn_blocking(move || {
                let mut guard = conn.blocking_lock();
                let tx = guard.transaction().context("begin upsert tx")?;
                for item in owned.iter_mut() {
                    let source = serde_json::to_value(item.source)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default();
                    let author_json = item
                        .author
                        .as_ref()
                        .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "null".into()));
                    let metadata_json = serde_json::to_string(&item.metadata)
                        .unwrap_or_else(|_| "{}".into());
                    let new_id = item.id.to_string();

                    // Look up the existing canonical id, if any.
                    let existing_id: Option<String> = tx
                        .query_row(
                            "SELECT id FROM items WHERE source = ?1 AND external_id = ?2",
                            rusqlite::params![source, item.external_id],
                            |row| row.get(0),
                        )
                        .optional()
                        .context("lookup existing item by (source, external_id)")?;

                    match existing_id {
                        Some(canonical) => {
                            // Existing row wins. If the caller supplied a
                            // different id, drop any vector they may have
                            // already written under that wrong id.
                            if canonical != new_id {
                                tx.execute(
                                    "DELETE FROM item_vectors WHERE item_id = ?1",
                                    rusqlite::params![new_id],
                                )
                                .context("orphan-clean stale vector by caller id")?;
                            }
                            tx.execute(
                                "UPDATE items \
                                 SET ts = ?1, author_json = ?2, subject = ?3, \
                                     text = ?4, metadata_json = ?5 \
                                 WHERE id = ?6",
                                rusqlite::params![
                                    item.ts.timestamp(),
                                    author_json,
                                    item.subject.as_deref(),
                                    item.text,
                                    metadata_json,
                                    canonical,
                                ],
                            )
                            .context("update existing item row")?;
                            // Mutate the caller's item so subsequent
                            // upsert_vector writes under the canonical id.
                            item.id = uuid::Uuid::parse_str(&canonical)
                                .context("existing items.id is not a valid uuid")?;
                        }
                        None => {
                            tx.execute(
                                "INSERT INTO items(id, source, external_id, ts, author_json, subject, text, metadata_json) \
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                                rusqlite::params![
                                    new_id,
                                    source,
                                    item.external_id,
                                    item.ts.timestamp(),
                                    author_json,
                                    item.subject.as_deref(),
                                    item.text,
                                    metadata_json,
                                ],
                            )
                            .context("insert new item row")?;
                        }
                    }
                }
                tx.commit().context("commit upsert tx")?;
                Ok(owned)
            })
            .await
            .context("upsert task panicked")?;

        let updated = result?;
        // Move canonical ids back into the caller's slice.
        for (slot, item) in items.iter_mut().zip(updated.into_iter()) {
            *slot = item;
        }
        Ok(())
    }

    /// Replace the vector for an item atomically: DELETE + INSERT inside a
    /// single transaction so a failed INSERT doesn't permanently remove the
    /// item's vector. (vec0 doesn't support ON CONFLICT on its primary key.)
    pub async fn upsert_vector(&self, item_id: &uuid::Uuid, vector: &[f32]) -> anyhow::Result<()> {
        let id = item_id.to_string();
        let v_json = serde_json::to_string(vector).context("serialize vector")?;
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut guard = conn.blocking_lock();
            let tx = guard.transaction().context("begin upsert_vector tx")?;
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(1) FROM items WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .context("check item exists")?;
            if exists == 0 {
                anyhow::bail!("cannot upsert vector for unknown item {id}");
            }
            tx.execute(
                "DELETE FROM item_vectors WHERE item_id = ?1",
                rusqlite::params![id],
            )
            .context("delete prior vector")?;
            tx.execute(
                "INSERT INTO item_vectors(item_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![id, v_json],
            )
            .context("insert vector")?;
            tx.commit().context("commit upsert_vector tx")?;
            Ok(())
        })
        .await
        .context("upsert_vector task panicked")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn vec_extension_loads_and_reports_version() {
        let idx = PersonalIndex::open_in_memory().await.expect("open");
        let conn = idx.writer.lock().await;
        let version: String = conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .expect("vec_version");
        assert!(
            version.starts_with('v'),
            "unexpected vec_version: {version}"
        );
    }

    #[tokio::test]
    async fn vec0_table_accepts_insert_and_returns_match() {
        let idx = PersonalIndex::open_in_memory().await.expect("open");
        let conn = idx.writer.lock().await;

        let v: Vec<f32> = (0..1536).map(|i| (i as f32) * 0.001).collect();
        let v_json = serde_json::to_string(&v).unwrap();

        conn.execute(
            "INSERT INTO item_vectors(item_id, embedding) VALUES (?1, ?2)",
            rusqlite::params!["00000000-0000-0000-0000-000000000001", v_json],
        )
        .expect("insert vec");

        let id: String = conn
            .query_row(
                "SELECT item_id FROM item_vectors \
                 WHERE embedding MATCH ?1 \
                 ORDER BY distance LIMIT 1",
                rusqlite::params![v_json],
                |row| row.get(0),
            )
            .expect("vec MATCH query");
        assert_eq!(id, "00000000-0000-0000-0000-000000000001");
    }
}

/// Reader for the personal index. All queries enforce the v1 ACL token
/// (`user:local`) via `EXISTS (... json_each(access_control_list) = 'user:local')`
/// so the same query shape works for the multi-token team v2 ACL without rewrites.
///
/// Reads route through the [`ReaderPool`] when available (file-backed
/// `PersonalIndex`), falling back to the writer connection otherwise
/// (in-memory handles). The fallback keeps the existing `open_in_memory`
/// contract intact — readers and writers share one connection for tests.
pub struct IndexReader {
    writer: Arc<Mutex<Connection>>,
    pool: Option<Arc<ReaderPool>>,
}

/// Internal row shape for both keyword and vector search. We hand-build it
/// inside the `query_row` closure because rusqlite has no derive equivalent.
struct ItemRow {
    id: String,
    source: String,
    external_id: String,
    ts: i64,
    author_json: Option<String>,
    subject: Option<String>,
    text: String,
    metadata_json: String,
    score: f64,
    snip: String,
}

impl ItemRow {
    fn into_hit(self) -> crate::openhuman::life_capture::types::Hit {
        use crate::openhuman::life_capture::types::{Hit, Item, Source};
        let author = self.author_json.and_then(|s| serde_json::from_str(&s).ok());
        let metadata = serde_json::from_str(&self.metadata_json).unwrap_or(serde_json::json!({}));
        let source: Source = serde_json::from_value(serde_json::Value::String(self.source.clone()))
            .unwrap_or(Source::Gmail);
        Hit {
            score: self.score as f32,
            snippet: self.snip,
            item: Item {
                id: uuid::Uuid::parse_str(&self.id).unwrap_or_else(|_| uuid::Uuid::nil()),
                source,
                external_id: self.external_id,
                ts: chrono::DateTime::from_timestamp(self.ts, 0).unwrap_or_default(),
                author,
                subject: self.subject,
                text: self.text,
                metadata,
            },
        }
    }
}

/// Sanitize arbitrary user input for FTS5 MATCH. Each whitespace-separated
/// token is wrapped as a quoted string (with embedded `"` doubled per FTS5
/// syntax) so operator tokens like `AND`/`OR`/`NEAR`, column filters, and
/// stray quotes from user input are treated as literals. Tokens are joined
/// with spaces, which FTS5 interprets as implicit AND — preserving the
/// pre-sanitization behavior of "all terms must be present".
fn fts5_quote(input: &str) -> String {
    let mut out = String::new();
    for tok in input.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('"');
        for ch in tok.chars() {
            if ch == '"' {
                out.push('"');
                out.push('"');
            } else {
                out.push(ch);
            }
        }
        out.push('"');
    }
    out
}

/// Post-filter hits against a Query's `sources`/`since`/`until`. Applied on
/// the fused output so both legs share a single consistent filter.
fn apply_query_filters(
    hits: Vec<crate::openhuman::life_capture::types::Hit>,
    q: &crate::openhuman::life_capture::types::Query,
) -> Vec<crate::openhuman::life_capture::types::Hit> {
    hits.into_iter()
        .filter(|h| {
            if !q.sources.is_empty() && !q.sources.contains(&h.item.source) {
                return false;
            }
            if let Some(since) = q.since {
                if h.item.ts < since {
                    return false;
                }
            }
            if let Some(until) = q.until {
                if h.item.ts > until {
                    return false;
                }
            }
            true
        })
        .collect()
}

impl IndexReader {
    pub fn new(idx: &PersonalIndex) -> Self {
        Self {
            writer: Arc::clone(&idx.writer),
            pool: idx.reader_pool.as_ref().map(Arc::clone),
        }
    }

    /// Run a synchronous read closure on a connection from the pool (when
    /// available) or the writer lock (when not). Handles the spawn_blocking
    /// boundary so callers just write straight rusqlite.
    async fn with_read_conn<F, T>(&self, label: &'static str, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        match &self.pool {
            Some(pool) => {
                let pool = Arc::clone(pool);
                tokio::task::spawn_blocking(move || -> anyhow::Result<T> {
                    let conn = pool
                        .get()
                        .with_context(|| format!("acquire pooled reader connection for {label}"))?;
                    f(&conn)
                })
                .await
                .with_context(|| format!("{label} task panicked"))?
            }
            None => {
                let writer = Arc::clone(&self.writer);
                tokio::task::spawn_blocking(move || -> anyhow::Result<T> {
                    let guard = writer.blocking_lock();
                    f(&guard)
                })
                .await
                .with_context(|| format!("{label} task panicked"))?
            }
        }
    }

    /// Keyword search via FTS5 ranked by bm25 (negated so higher = better).
    ///
    /// The query string is wrapped as an FTS5 quoted phrase so special tokens
    /// like `AND`/`OR`/`NEAR`, quotes, and column specifiers from user input
    /// are treated as literals instead of FTS5 operators.
    pub async fn keyword_search(
        &self,
        query: &str,
        k: usize,
    ) -> anyhow::Result<Vec<crate::openhuman::life_capture::types::Hit>> {
        let query = fts5_quote(query);
        self.with_read_conn("keyword_search", move |conn| {
            let mut stmt = conn.prepare(
                "SELECT i.id, i.source, i.external_id, i.ts, i.author_json, i.subject, i.text, i.metadata_json, \
                        -bm25(items_fts) AS score, \
                        snippet(items_fts, 1, '«', '»', '…', 12) AS snip \
                 FROM items_fts JOIN items i ON i.rowid = items_fts.rowid \
                 WHERE items_fts MATCH ?1 \
                   AND EXISTS (SELECT 1 FROM json_each(i.access_control_list) WHERE value = 'user:local') \
                 ORDER BY score DESC \
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![query, k as i64], |row| {
                    Ok(ItemRow {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        external_id: row.get(2)?,
                        ts: row.get(3)?,
                        author_json: row.get(4)?,
                        subject: row.get(5)?,
                        text: row.get(6)?,
                        metadata_json: row.get(7)?,
                        score: row.get(8)?,
                        snip: row.get(9)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().map(ItemRow::into_hit).collect())
        })
        .await
    }

    /// Hybrid search via Reciprocal Rank Fusion. Pulls oversampled candidates
    /// from keyword + vector legs, scores each doc by `w_v/(k+vec_rank) +
    /// w_k/(k+kw_rank)` (RRF, k=60), plus a recency bump scaled to the same
    /// magnitude so freshness breaks ties without dominating.
    ///
    /// Why RRF over max-normalisation: BM25 and vector distance live on
    /// incomparable scales, a single outlier in either leg skews a max-norm,
    /// and rank-based fusion degrades gracefully when one leg is empty.
    pub async fn hybrid_search(
        &self,
        q: &crate::openhuman::life_capture::types::Query,
        query_vector: &[f32],
    ) -> anyhow::Result<Vec<crate::openhuman::life_capture::types::Hit>> {
        use crate::openhuman::life_capture::types::Hit;
        use std::collections::HashMap;

        const RRF_K: f32 = 60.0;
        const W_VEC: f32 = 0.55;
        const W_KW: f32 = 0.35;
        const W_RECENCY: f32 = 0.10;

        let oversample = (q.k * 3).max(20);
        let kw = self.keyword_search(&q.text, oversample).await?;
        let vc = self.vector_search(query_vector, oversample).await?;

        // (hit, kw_rrf, vc_rrf)
        let mut by_id: HashMap<uuid::Uuid, (Hit, f32, f32)> = HashMap::new();
        for (rank0, h) in kw.into_iter().enumerate() {
            let rrf = 1.0 / (RRF_K + (rank0 + 1) as f32);
            let id = h.item.id;
            by_id.insert(id, (h, rrf, 0.0));
        }
        for (rank0, h) in vc.into_iter().enumerate() {
            let rrf = 1.0 / (RRF_K + (rank0 + 1) as f32);
            by_id
                .entry(h.item.id)
                .and_modify(|(_, _, vs)| *vs = rrf)
                .or_insert_with(|| (h.clone(), 0.0, rrf));
        }

        // Put recency on the same order of magnitude as a top-ranked RRF term.
        let recency_scale = 1.0 / (RRF_K + 1.0);
        let now = chrono::Utc::now().timestamp();
        let mut out: Vec<Hit> = by_id
            .into_values()
            .map(|(mut hit, kw_rrf, vc_rrf)| {
                let age_days = ((now - hit.item.ts.timestamp()).max(0) as f32) / 86400.0;
                let recency = (-age_days / 30.0).exp();
                hit.score = W_VEC * vc_rrf + W_KW * kw_rrf + W_RECENCY * recency * recency_scale;
                hit
            })
            .collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut out = apply_query_filters(out, q);
        out.truncate(q.k);
        Ok(out)
    }

    /// Vector search via sqlite-vec MATCH. Score is `1 / (1 + distance)` so
    /// callers can blend it with the keyword score on the same monotonic scale.
    pub async fn vector_search(
        &self,
        vector: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<crate::openhuman::life_capture::types::Hit>> {
        let v_json = serde_json::to_string(vector).context("serialize vector")?;
        self.with_read_conn("vector_search", move |conn| {
            let mut stmt = conn.prepare(
                "SELECT i.id, i.source, i.external_id, i.ts, i.author_json, i.subject, i.text, i.metadata_json, \
                        (1.0 / (1.0 + v.distance)) AS score, \
                        substr(i.text, 1, 200) AS snip \
                 FROM item_vectors v JOIN items i ON i.id = v.item_id \
                 WHERE v.embedding MATCH ?1 AND k = ?2 \
                   AND EXISTS (SELECT 1 FROM json_each(i.access_control_list) WHERE value = 'user:local') \
                 ORDER BY v.distance ASC \
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![v_json, k as i64], |row| {
                    Ok(ItemRow {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        external_id: row.get(2)?,
                        ts: row.get(3)?,
                        author_json: row.get(4)?,
                        subject: row.get(5)?,
                        text: row.get(6)?,
                        metadata_json: row.get(7)?,
                        score: row.get(8)?,
                        snip: row.get(9)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().map(ItemRow::into_hit).collect())
        })
        .await
    }
}

#[cfg(test)]
mod writer_tests {
    use super::*;
    use crate::openhuman::life_capture::types::{Item, Source};
    use chrono::Utc;
    use uuid::Uuid;

    fn sample_item(ext: &str, text: &str) -> Item {
        Item {
            id: Uuid::new_v4(),
            source: Source::Gmail,
            external_id: ext.into(),
            ts: Utc::now(),
            author: None,
            subject: Some("subj".into()),
            text: text.into(),
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn upsert_inserts_new_and_dedupes_existing() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);

        writer
            .upsert(&mut [sample_item("a", "first")])
            .await
            .unwrap();
        writer
            .upsert(&mut [sample_item("a", "first updated")])
            .await
            .unwrap();
        writer
            .upsert(&mut [sample_item("b", "second")])
            .await
            .unwrap();

        let conn = idx.writer.lock().await;
        let count: i64 = conn
            .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "dedupe by (source, external_id)");

        let updated: String = conn
            .query_row("SELECT text FROM items WHERE external_id='a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(updated, "first updated", "upsert overwrites text");
    }

    #[tokio::test]
    async fn reingest_with_fresh_uuid_keeps_vector_findable() {
        // Regression: previously upsert kept the existing items.id but the
        // caller's fresh UUID was used for upsert_vector — leaving an
        // orphaned vector under an id no row joined to.
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);

        let mut first = [sample_item("dup", "first")];
        writer.upsert(&mut first).await.unwrap();
        let canonical_id = first[0].id;
        let v: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        writer.upsert_vector(&canonical_id, &v).await.unwrap();

        // Re-ingest with a fresh Uuid for the same (source, external_id).
        let mut second = [sample_item("dup", "updated text")];
        let fresh_uuid = second[0].id;
        assert_ne!(
            fresh_uuid, canonical_id,
            "test setup needs a different uuid"
        );
        writer.upsert(&mut second).await.unwrap();

        // upsert must rewrite the caller's id to the canonical one so the
        // next vector write lands under the right key.
        assert_eq!(
            second[0].id, canonical_id,
            "upsert should rewrite caller id to existing canonical id"
        );

        // And it must not have left the old vector orphaned: vector_search
        // for v finds the (single, updated) row.
        let reader = IndexReader::new(&idx);
        let hits = reader.vector_search(&v, 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.text, "updated text");
        assert_eq!(hits[0].item.id, canonical_id);
    }

    #[tokio::test]
    async fn upsert_vector_replaces_existing_row() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);

        let mut items = [sample_item("vec-replace", "text")];
        writer.upsert(&mut items).await.unwrap();
        let id = items[0].id;
        let v1: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        let v2: Vec<f32> = (0..1536).map(|i| (i as f32 * 0.001) + 0.5).collect();

        writer.upsert_vector(&id, &v1).await.unwrap();
        writer.upsert_vector(&id, &v2).await.unwrap();

        let conn = idx.writer.lock().await;
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM item_vectors WHERE item_id = ?1",
                rusqlite::params![id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "vector replaced, not duplicated");
    }
}

#[cfg(test)]
mod reader_keyword_tests {
    use super::*;
    use crate::openhuman::life_capture::types::{Item, Source};
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn it(ext: &str, subj: &str, text: &str, ts_secs: i64) -> Item {
        Item {
            id: Uuid::new_v4(),
            source: Source::Gmail,
            external_id: ext.into(),
            ts: Utc.timestamp_opt(ts_secs, 0).single().unwrap(),
            author: None,
            subject: Some(subj.into()),
            text: text.into(),
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn keyword_search_ranks_by_relevance() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);
        writer
            .upsert(&mut [
                it(
                    "a",
                    "ledger contract",
                    "the ledger contract draft is attached",
                    100,
                ),
                it("b", "lunch", "let's grab lunch", 200),
                it("c", "ledger", "ledger ledger ledger", 300),
            ])
            .await
            .unwrap();

        let reader = IndexReader::new(&idx);
        let hits = reader.keyword_search("ledger contract", 10).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(hits[0].item.external_id, "a", "best match should be 'a'");
    }
}

#[cfg(test)]
mod reader_vector_tests {
    use super::*;
    use crate::openhuman::life_capture::types::{Item, Source};
    use chrono::Utc;
    use uuid::Uuid;

    fn near(target: &[f32], jitter: f32) -> Vec<f32> {
        target.iter().map(|x| x + jitter).collect()
    }

    #[tokio::test]
    async fn vector_search_returns_nearest_first() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);

        let mk = |ext: &str| Item {
            id: Uuid::new_v4(),
            source: Source::Gmail,
            external_id: ext.into(),
            ts: Utc::now(),
            author: None,
            subject: None,
            text: format!("body of {ext}"),
            metadata: serde_json::json!({}),
        };

        let a = mk("a");
        let b = mk("b");
        let c = mk("c");
        writer
            .upsert(&mut [a.clone(), b.clone(), c.clone()])
            .await
            .unwrap();

        let mut va = vec![0.0_f32; 1536];
        va[0] = 1.0;
        let mut vb = vec![0.0_f32; 1536];
        vb[1] = 1.0;
        let mut vc = vec![0.0_f32; 1536];
        vc[2] = 1.0;

        writer.upsert_vector(&a.id, &va).await.unwrap();
        writer.upsert_vector(&b.id, &vb).await.unwrap();
        writer.upsert_vector(&c.id, &vc).await.unwrap();

        let reader = IndexReader::new(&idx);
        let hits = reader.vector_search(&near(&va, 0.01), 2).await.unwrap();
        assert_eq!(hits.len(), 2, "k=2");
        assert_eq!(hits[0].item.external_id, "a", "nearest first");
    }
}

#[cfg(test)]
mod reader_hybrid_tests {
    use super::*;
    use crate::openhuman::life_capture::types::{Item, Query, Source};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    #[tokio::test]
    async fn hybrid_combines_signals_and_breaks_keyword_only_ties_with_vector() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let w = IndexWriter::new(&idx);

        let now = Utc::now();
        let mk = |ext: &str, subj: &str, text: &str, days_ago: i64| Item {
            id: Uuid::new_v4(),
            source: Source::Gmail,
            external_id: ext.into(),
            ts: now - Duration::days(days_ago),
            author: None,
            subject: Some(subj.into()),
            text: text.into(),
            metadata: serde_json::json!({}),
        };
        let a = mk("a", "ledger", "ledger contract draft", 1);
        let b = mk("b", "ledger", "ledger contract draft", 30);
        w.upsert(&mut [a.clone(), b.clone()]).await.unwrap();

        // Same vector for both — recency should break the tie in favor of `a`.
        let v: Vec<f32> = (0..1536).map(|i| (i as f32) / 1536.0).collect();
        w.upsert_vector(&a.id, &v).await.unwrap();
        w.upsert_vector(&b.id, &v).await.unwrap();

        let reader = IndexReader::new(&idx);
        let q = Query::simple("ledger contract", 5);
        let hits = reader.hybrid_search(&q, &v).await.unwrap();
        assert_eq!(hits[0].item.external_id, "a", "recency breaks tie");
    }
}
