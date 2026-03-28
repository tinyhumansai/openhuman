use rusqlite::Connection;
use std::path::PathBuf;

pub(super) fn default_db_path() -> Result<PathBuf, String> {
    let mut base =
        dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    base.push(".openhuman");
    base.push("memory");
    base.push("local_memory.db");
    Ok(base)
}

pub(super) fn with_conn<T, F>(db_path: &PathBuf, f: F) -> Result<T, String>
where
    F: FnOnce(&Connection) -> Result<T, String>,
{
    let conn = Connection::open(db_path)
        .map_err(|e| format!("Open memory db {}: {e}", db_path.display()))?;
    init_schema(&conn)?;
    f(&conn)
}

pub(super) fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;

         CREATE TABLE IF NOT EXISTS memory_documents (
           document_id TEXT PRIMARY KEY,
           namespace   TEXT NOT NULL,
           title       TEXT NOT NULL,
           content     TEXT NOT NULL,
           source_type TEXT NOT NULL,
           metadata    TEXT NOT NULL,
           priority    TEXT NOT NULL,
           created_at  REAL NOT NULL,
           updated_at  REAL NOT NULL
         );

         CREATE INDEX IF NOT EXISTS idx_memory_documents_namespace_updated
           ON memory_documents(namespace, updated_at DESC);

         CREATE TABLE IF NOT EXISTS memory_chunks (
           id          INTEGER PRIMARY KEY AUTOINCREMENT,
           document_id TEXT NOT NULL,
           namespace   TEXT NOT NULL,
           chunk_index INTEGER NOT NULL,
           text        TEXT NOT NULL,
           created_at  REAL NOT NULL,
           updated_at  REAL NOT NULL,
           FOREIGN KEY(document_id) REFERENCES memory_documents(document_id) ON DELETE CASCADE
         );

         CREATE INDEX IF NOT EXISTS idx_memory_chunks_namespace_updated
           ON memory_chunks(namespace, updated_at DESC);

         CREATE INDEX IF NOT EXISTS idx_memory_chunks_doc_chunk
           ON memory_chunks(document_id, chunk_index);",
    )
    .map_err(|e| format!("Init local memory schema: {e}"))
}

pub(super) fn now_ts() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
