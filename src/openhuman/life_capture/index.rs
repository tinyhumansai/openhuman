use anyhow::Context;
use once_cell::sync::OnceCell;
use rusqlite::{ffi, Connection};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Personal index — SQLite database with FTS5 + sqlite-vec virtual tables loaded.
///
/// Wraps a single rusqlite `Connection` behind an async mutex so it can be shared
/// across tasks. Reads and writes serialise; this is intentional — SQLite handles
/// concurrent readers via WAL but a single writer is the simpler model and matches
/// our access pattern (one ingest worker + a few reader call sites).
pub struct PersonalIndex {
    pub conn: Arc<Mutex<Connection>>,
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

impl PersonalIndex {
    /// Open (or create) the personal index at `path`. Loads sqlite-vec, runs migrations.
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        ensure_vec_extension_registered();
        let path = path.to_path_buf();
        let conn = tokio::task::spawn_blocking(move || -> anyhow::Result<Connection> {
            let conn = Connection::open(&path).context("open sqlite db")?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            super::migrations::run(&conn).context("run life_capture migrations")?;
            Ok(conn)
        })
        .await
        .context("open task panicked")??;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Open an in-memory index (for tests). Same setup as `open`.
    pub async fn open_in_memory() -> anyhow::Result<Self> {
        ensure_vec_extension_registered();
        let conn = tokio::task::spawn_blocking(|| -> anyhow::Result<Connection> {
            let conn = Connection::open_in_memory().context("open in-memory sqlite")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            super::migrations::run(&conn).context("run life_capture migrations")?;
            Ok(conn)
        })
        .await
        .context("open task panicked")??;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
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
        Self { conn: Arc::clone(&idx.conn) }
    }

    /// Upsert items by (source, external_id). FTS rows are kept in sync via
    /// the SQL triggers defined in 0001_init.sql.
    pub async fn upsert(&self, items: &[crate::openhuman::life_capture::types::Item]) -> anyhow::Result<()> {
        let items = items.to_vec();
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut guard = conn.blocking_lock();
            let tx = guard.transaction().context("begin upsert tx")?;
            for item in &items {
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

                tx.execute(
                    "INSERT INTO items(id, source, external_id, ts, author_json, subject, text, metadata_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT(source, external_id) DO UPDATE SET \
                       ts            = excluded.ts, \
                       author_json   = excluded.author_json, \
                       subject       = excluded.subject, \
                       text          = excluded.text, \
                       metadata_json = excluded.metadata_json",
                    rusqlite::params![
                        item.id.to_string(),
                        source,
                        item.external_id,
                        item.ts.timestamp(),
                        author_json,
                        item.subject.as_deref(),
                        item.text,
                        metadata_json,
                    ],
                )
                .context("upsert item row")?;
            }
            tx.commit().context("commit upsert tx")?;
            Ok(())
        })
        .await
        .context("upsert task panicked")?
    }

    /// Replace the vector for an item. DELETE + INSERT because vec0 doesn't
    /// support ON CONFLICT for virtual table primary keys.
    pub async fn upsert_vector(&self, item_id: &uuid::Uuid, vector: &[f32]) -> anyhow::Result<()> {
        let id = item_id.to_string();
        let v_json = serde_json::to_string(vector).context("serialize vector")?;
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let guard = conn.blocking_lock();
            guard
                .execute(
                    "DELETE FROM item_vectors WHERE item_id = ?1",
                    rusqlite::params![id],
                )
                .context("delete prior vector")?;
            guard
                .execute(
                    "INSERT INTO item_vectors(item_id, embedding) VALUES (?1, ?2)",
                    rusqlite::params![id, v_json],
                )
                .context("insert vector")?;
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
        let conn = idx.conn.lock().await;
        let version: String = conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .expect("vec_version");
        assert!(version.starts_with('v'), "unexpected vec_version: {version}");
    }

    #[tokio::test]
    async fn vec0_table_accepts_insert_and_returns_match() {
        let idx = PersonalIndex::open_in_memory().await.expect("open");
        let conn = idx.conn.lock().await;

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
pub struct IndexReader {
    conn: Arc<Mutex<Connection>>,
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
        let metadata =
            serde_json::from_str(&self.metadata_json).unwrap_or(serde_json::json!({}));
        let source: Source =
            serde_json::from_value(serde_json::Value::String(self.source.clone()))
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

impl IndexReader {
    pub fn new(idx: &PersonalIndex) -> Self {
        Self { conn: Arc::clone(&idx.conn) }
    }

    /// Keyword search via FTS5 ranked by bm25 (negated so higher = better).
    pub async fn keyword_search(
        &self,
        query: &str,
        k: usize,
    ) -> anyhow::Result<Vec<crate::openhuman::life_capture::types::Hit>> {
        let query = query.to_string();
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<_>> {
            let guard = conn.blocking_lock();
            let mut stmt = guard.prepare(
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
        .context("keyword_search task panicked")?
    }

    /// Hybrid search — pulls oversampled candidates from both the keyword and
    /// vector legs, normalises each leg independently, and re-ranks in app code
    /// with `0.55 * vec_norm + 0.35 * kw_norm + 0.10 * recency` where recency
    /// is `exp(-age_days / 30)` (half-life ~21 days).
    ///
    /// Oversample factor `k * 3` (min 20) is enough to catch documents that
    /// rank well on one signal but not the other. Items missing a vector still
    /// appear (vec_norm = 0) and items missing keyword hits still appear
    /// (kw_norm = 0); only documents with neither signal are dropped.
    pub async fn hybrid_search(
        &self,
        q: &crate::openhuman::life_capture::types::Query,
        query_vector: &[f32],
    ) -> anyhow::Result<Vec<crate::openhuman::life_capture::types::Hit>> {
        use crate::openhuman::life_capture::types::Hit;
        use std::collections::HashMap;

        let oversample = (q.k * 3).max(20);
        let kw = self.keyword_search(&q.text, oversample).await?;
        let vc = self.vector_search(query_vector, oversample).await?;

        let max_kw = kw.iter().map(|h| h.score).fold(f32::MIN, f32::max).max(1e-6);
        let max_vc = vc.iter().map(|h| h.score).fold(f32::MIN, f32::max).max(1e-6);

        let mut by_id: HashMap<uuid::Uuid, (Hit, f32, f32)> = HashMap::new();
        for h in kw {
            let s = h.score / max_kw;
            let id = h.item.id;
            by_id.insert(id, (h, s, 0.0));
        }
        for h in vc {
            let s = h.score / max_vc;
            by_id
                .entry(h.item.id)
                .and_modify(|(_, _, vs)| *vs = s)
                .or_insert_with(|| (h.clone(), 0.0, s));
        }

        let now = chrono::Utc::now().timestamp();
        let mut out: Vec<Hit> = by_id
            .into_values()
            .map(|(mut hit, kw_n, vc_n)| {
                let age_days = ((now - hit.item.ts.timestamp()).max(0) as f32) / 86400.0;
                let recency = (-age_days / 30.0).exp();
                hit.score = 0.55 * vc_n + 0.35 * kw_n + 0.10 * recency;
                hit
            })
            .collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
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
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<_>> {
            let guard = conn.blocking_lock();
            let mut stmt = guard.prepare(
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
        .context("vector_search task panicked")?
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

        writer.upsert(&[sample_item("a", "first")]).await.unwrap();
        writer.upsert(&[sample_item("a", "first updated")]).await.unwrap();
        writer.upsert(&[sample_item("b", "second")]).await.unwrap();

        let conn = idx.conn.lock().await;
        let count: i64 = conn
            .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "dedupe by (source, external_id)");

        let updated: String = conn
            .query_row(
                "SELECT text FROM items WHERE external_id='a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(updated, "first updated", "upsert overwrites text");
    }

    #[tokio::test]
    async fn upsert_vector_replaces_existing_row() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);

        let id = Uuid::new_v4();
        let v1: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        let v2: Vec<f32> = (0..1536).map(|i| (i as f32 * 0.001) + 0.5).collect();

        writer.upsert_vector(&id, &v1).await.unwrap();
        writer.upsert_vector(&id, &v2).await.unwrap();

        let conn = idx.conn.lock().await;
        let count: i64 = conn
            .query_row("SELECT count(*) FROM item_vectors WHERE item_id = ?1", rusqlite::params![id.to_string()], |r| r.get(0))
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
            .upsert(&[
                it("a", "ledger contract", "the ledger contract draft is attached", 100),
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
        writer.upsert(&[a.clone(), b.clone(), c.clone()]).await.unwrap();

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
        w.upsert(&[a.clone(), b.clone()]).await.unwrap();

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
