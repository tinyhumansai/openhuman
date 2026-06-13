//! Persistent, content-addressed store for codegraph.
//!
//! Two tables (SQLite, WAL):
//!
//! - `blob(sha, model, tokens, emb, dim)` PK `(sha, model)` — the shared
//!   content cache: one row per unique file content per embedding model.
//!   `tokens` is the space-joined BM25 token stream; `emb` is the L2-normalised
//!   structural-aug vector stored as little-endian `f32` bytes. Shared across
//!   every repo and branch, so renames / unchanged files are free.
//!
//! - `manifest(repo_id, git_ref, path, sha)` PK `(repo_id, git_ref, path)` —
//!   one row per file per branch/commit. A branch's index is its rows here,
//!   joined to `blob` at query time. A file deleted on a branch drops from
//!   *that ref's* rows; its blob lingers until no manifest references it
//!   ([`CodegraphStore::gc`]).
//!
//! This is the storage layer only — tree-sitter extraction, FTS5 ranking, and
//! the embeddings call live in `index`/`search`.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS blob (
  sha    TEXT NOT NULL,
  model  TEXT NOT NULL,
  tokens TEXT NOT NULL,
  emb    BLOB NOT NULL,
  dim    INTEGER NOT NULL,
  PRIMARY KEY (sha, model)
);
CREATE TABLE IF NOT EXISTS manifest (
  repo_id TEXT NOT NULL,
  git_ref TEXT NOT NULL,
  path    TEXT NOT NULL,
  sha     TEXT NOT NULL,
  PRIMARY KEY (repo_id, git_ref, path)
);
CREATE INDEX IF NOT EXISTS manifest_repo_ref ON manifest(repo_id, git_ref);
";

/// One hydrated file in a `(repo, ref)` working set: its path plus the cached
/// BM25 tokens and dense vector. Returned by [`CodegraphStore::hydrate`].
#[derive(Debug, Clone)]
pub struct BlobEntry {
    pub path: String,
    pub tokens: Vec<String>,
    pub emb: Vec<f32>,
}

/// Content-addressed blob cache + per-`(repo, ref)` manifests, backed by SQLite.
pub struct CodegraphStore {
    conn: Connection,
}

impl CodegraphStore {
    /// Open (creating if needed) the codegraph DB at `db_path`.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("open codegraph db at {}", db_path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // NORMAL is durable across an app crash under WAL (only a power/OS crash
        // can lose the last commit) and drops the per-commit fsync that
        // otherwise dominates a cold index — and this is a rebuildable cache.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)
            .context("init codegraph schema")?;
        Ok(Self { conn })
    }

    /// True if this content (`sha`) is already cached for `model` — the
    /// incremental check: a cache hit means no re-embed on (re)index.
    pub fn has_blob(&self, sha: &str, model: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM blob WHERE sha=?1 AND model=?2",
            params![sha, model],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Insert a computed blob (idempotent on `(sha, model)`).
    pub fn put_blob(&self, sha: &str, model: &str, tokens: &[String], emb: &[f32]) -> Result<()> {
        let token_str = tokens.join(" ");
        let mut bytes = Vec::with_capacity(emb.len() * 4);
        for f in emb {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO blob(sha, model, tokens, emb, dim) VALUES (?1,?2,?3,?4,?5)",
            params![sha, model, token_str, bytes, emb.len() as i64],
        )?;
        Ok(())
    }

    /// Insert many computed blobs in a single transaction (one fsync for the
    /// batch, not one per blob). Idempotent on `(sha, model)` via `OR IGNORE`,
    /// so duplicate content within the batch keeps its first row. The hot path
    /// for a cold index — prefer this over a `put_blob` loop.
    pub fn put_blobs(
        &mut self,
        model: &str,
        blobs: &[(String, Vec<String>, Vec<f32>)],
    ) -> Result<()> {
        if blobs.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO blob(sha, model, tokens, emb, dim) VALUES (?1,?2,?3,?4,?5)",
            )?;
            for (sha, tokens, emb) in blobs {
                let token_str = tokens.join(" ");
                let mut bytes = Vec::with_capacity(emb.len() * 4);
                for f in emb {
                    bytes.extend_from_slice(&f.to_le_bytes());
                }
                stmt.execute(params![sha, model, token_str, bytes, emb.len() as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Replace a `(repo, ref)` manifest with `files` (`(path, sha)`), handling
    /// deletes/renames: the ref's rows are rewritten to exactly `files`.
    pub fn set_manifest(
        &mut self,
        repo_id: &str,
        git_ref: &str,
        files: &[(String, String)],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM manifest WHERE repo_id=?1 AND git_ref=?2",
            params![repo_id, git_ref],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO manifest(repo_id, git_ref, path, sha) VALUES (?1,?2,?3,?4)",
            )?;
            for (path, sha) in files {
                stmt.execute(params![repo_id, git_ref, path, sha])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Hydrate one `(repo, ref)` working set: manifest joined to the blob cache
    /// for `model`. Files whose blob isn't cached (e.g. skipped/oversized) are
    /// omitted — the caller derives coverage from `returned / manifest_size`.
    pub fn hydrate(&self, repo_id: &str, git_ref: &str, model: &str) -> Result<Vec<BlobEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.path, b.tokens, b.emb FROM manifest m \
             JOIN blob b ON b.sha = m.sha AND b.model = ?1 \
             WHERE m.repo_id = ?2 AND m.git_ref = ?3",
        )?;
        let rows = stmt.query_map(params![model, repo_id, git_ref], |r| {
            let path: String = r.get(0)?;
            let tokens: String = r.get(1)?;
            let bytes: Vec<u8> = r.get(2)?;
            Ok((path, tokens, bytes))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (path, tokens, bytes) = row?;
            let emb = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            out.push(BlobEntry {
                path,
                tokens: tokens.split_whitespace().map(str::to_string).collect(),
                emb,
            });
        }
        Ok(out)
    }

    /// Number of files in a `(repo, ref)` manifest (the coverage denominator).
    pub fn manifest_size(&self, repo_id: &str, git_ref: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM manifest WHERE repo_id=?1 AND git_ref=?2",
            params![repo_id, git_ref],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Distinct refs indexed for a repo.
    pub fn refs(&self, repo_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT git_ref FROM manifest WHERE repo_id=?1")?;
        let rows = stmt.query_map(params![repo_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Drop cached blobs no live manifest references. Returns rows removed.
    pub fn gc(&self) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM blob WHERE sha NOT IN (SELECT DISTINCT sha FROM manifest)",
            [],
        )?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store(dir: &TempDir) -> CodegraphStore {
        CodegraphStore::open(&dir.path().join("codegraph").join("index.db")).unwrap()
    }

    #[test]
    fn blob_roundtrip_and_dedup() {
        let tmp = TempDir::new().unwrap();
        let s = store(&tmp);
        assert!(!s.has_blob("sha1", "m").unwrap());
        s.put_blob("sha1", "m", &["foo".into(), "bar".into()], &[0.5, -0.5])
            .unwrap();
        assert!(s.has_blob("sha1", "m").unwrap());
        // Different model = distinct cache entry.
        assert!(!s.has_blob("sha1", "other").unwrap());
        // Idempotent.
        s.put_blob("sha1", "m", &["foo".into()], &[1.0]).unwrap();
    }

    #[test]
    fn put_blobs_batches_and_dedups() {
        let tmp = TempDir::new().unwrap();
        let mut s = store(&tmp);
        s.put_blobs(
            "m",
            &[
                ("s1".into(), vec!["a".into(), "b".into()], vec![1.0, 0.0]),
                ("s2".into(), vec!["c".into()], vec![0.0, 1.0]),
                ("s1".into(), vec!["dup".into()], vec![9.0]), // OR IGNORE keeps the first
            ],
        )
        .unwrap();
        assert!(s.has_blob("s1", "m").unwrap());
        assert!(s.has_blob("s2", "m").unwrap());
        // Empty batch is a no-op (warm re-index path).
        s.put_blobs("m", &[]).unwrap();
        s.set_manifest(
            "r",
            "main",
            &[("a.rs".into(), "s1".into()), ("b.rs".into(), "s2".into())],
        )
        .unwrap();
        let hits = s.hydrate("r", "main", "m").unwrap();
        assert_eq!(hits.len(), 2);
        let a = hits.iter().find(|h| h.path == "a.rs").unwrap();
        assert_eq!(
            a.tokens,
            vec!["a".to_string(), "b".to_string()],
            "first insert kept, not the dup"
        );
    }

    #[test]
    fn manifest_hydrate_and_coverage() {
        let tmp = TempDir::new().unwrap();
        let mut s = store(&tmp);
        s.put_blob("shaA", "m", &["alpha".into()], &[1.0, 0.0])
            .unwrap();
        // shaB intentionally not cached (simulates skipped/oversized) → omitted from hydrate.
        s.set_manifest(
            "repo",
            "main",
            &[
                ("a.rs".into(), "shaA".into()),
                ("b.rs".into(), "shaB".into()),
            ],
        )
        .unwrap();
        let hits = s.hydrate("repo", "main", "m").unwrap();
        assert_eq!(hits.len(), 1, "only the cached blob hydrates");
        assert_eq!(hits[0].path, "a.rs");
        assert_eq!(hits[0].tokens, vec!["alpha".to_string()]);
        assert_eq!(hits[0].emb, vec![1.0, 0.0]);
        assert_eq!(s.manifest_size("repo", "main").unwrap(), 2);
    }

    #[test]
    fn manifest_is_per_ref_and_rewrites_on_set() {
        let tmp = TempDir::new().unwrap();
        let mut s = store(&tmp);
        s.put_blob("x", "m", &["x".into()], &[0.0]).unwrap();
        s.set_manifest("r", "brA", &[("util.rs".into(), "x".into())])
            .unwrap();
        s.set_manifest("r", "brB", &[("util/mod.rs".into(), "x".into())])
            .unwrap();
        let mut refs = s.refs("r").unwrap();
        refs.sort();
        assert_eq!(refs, vec!["brA".to_string(), "brB".to_string()]);
        // Re-setting a ref rewrites it (delete on brA: file gone from that ref).
        s.set_manifest("r", "brA", &[]).unwrap();
        assert_eq!(s.manifest_size("r", "brA").unwrap(), 0);
        assert_eq!(s.manifest_size("r", "brB").unwrap(), 1);
    }

    #[test]
    fn gc_drops_unreferenced_blobs_and_persists() {
        let path = TempDir::new().unwrap();
        let db = path.path().join("cg.db");
        {
            let mut s = CodegraphStore::open(&db).unwrap();
            s.put_blob("live", "m", &["a".into()], &[1.0]).unwrap();
            s.put_blob("orphan", "m", &["b".into()], &[1.0]).unwrap();
            s.set_manifest("r", "main", &[("a.rs".into(), "live".into())])
                .unwrap();
            assert_eq!(s.gc().unwrap(), 1, "orphan blob removed");
            assert!(s.has_blob("live", "m").unwrap());
            assert!(!s.has_blob("orphan", "m").unwrap());
        }
        // Reopen: state persisted across "restart".
        let s = CodegraphStore::open(&db).unwrap();
        assert!(s.has_blob("live", "m").unwrap());
        assert_eq!(s.hydrate("r", "main", "m").unwrap().len(), 1);
    }
}
