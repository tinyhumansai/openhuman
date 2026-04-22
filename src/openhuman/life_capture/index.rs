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
fn ensure_vec_extension_registered() {
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
