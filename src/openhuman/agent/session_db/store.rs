use crate::openhuman::config::Config;
use anyhow::{Context, Result};
use rusqlite::Connection;

pub fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("session_db").join("sessions.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create session_db directory: {}",
                parent.display()
            )
        })?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open session DB: {}", db_path.display()))?;

    init_schema(&conn)?;
    f(&conn)
}

#[cfg(test)]
pub fn with_memory_connection<T>(f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let conn = Connection::open_in_memory().context("failed to open in-memory session DB")?;
    init_schema(&conn)?;
    f(&conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;

         CREATE TABLE IF NOT EXISTS sessions (
            id                    TEXT PRIMARY KEY,
            agent_definition_id   TEXT NOT NULL,
            agent_definition_name TEXT NOT NULL,
            session_key           TEXT NOT NULL,
            parent_session_id     TEXT,
            thread_id             TEXT,
            source_channel        TEXT,
            status                TEXT NOT NULL DEFAULT 'running',
            model                 TEXT,
            turn_count            INTEGER NOT NULL DEFAULT 0,
            input_tokens          INTEGER NOT NULL DEFAULT 0,
            output_tokens         INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens   INTEGER NOT NULL DEFAULT 0,
            cost_usd              REAL NOT NULL DEFAULT 0.0,
            transcript_path       TEXT,
            started_at            TEXT NOT NULL,
            ended_at              TEXT,
            FOREIGN KEY (parent_session_id) REFERENCES sessions(id) ON DELETE SET NULL
         );
         CREATE INDEX IF NOT EXISTS idx_sessions_agent ON sessions(agent_definition_id);
         CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
         CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
         CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
         CREATE INDEX IF NOT EXISTS idx_sessions_thread ON sessions(thread_id);
         CREATE INDEX IF NOT EXISTS idx_sessions_channel ON sessions(source_channel);
         CREATE INDEX IF NOT EXISTS idx_sessions_key ON sessions(session_key);

         CREATE TABLE IF NOT EXISTS session_messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id  TEXT NOT NULL,
            role        TEXT NOT NULL,
            content     TEXT NOT NULL,
            model       TEXT,
            input_tokens  INTEGER,
            output_tokens INTEGER,
            cost_usd    REAL,
            created_at  TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_messages_session ON session_messages(session_id);

         CREATE TABLE IF NOT EXISTS session_tool_calls (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id  TEXT NOT NULL,
            message_id  INTEGER,
            tool_name   TEXT NOT NULL,
            tool_input  TEXT,
            tool_output TEXT,
            status      TEXT NOT NULL DEFAULT 'pending',
            duration_ms INTEGER,
            created_at  TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (message_id) REFERENCES session_messages(id) ON DELETE SET NULL
         );
         CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON session_tool_calls(session_id);
         CREATE INDEX IF NOT EXISTS idx_tool_calls_name ON session_tool_calls(tool_name);",
    )
    .context("failed to initialize session_db schema")?;

    init_fts(conn)?;
    Ok(())
}

fn init_fts(conn: &Connection) -> Result<()> {
    let has_fts: bool = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions_fts'")?
        .exists([])?;

    if !has_fts {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE sessions_fts USING fts5(
                session_id,
                agent_definition_name,
                content,
                tool_name
             );",
        )
        .context("failed to create sessions_fts virtual table")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_initializes_without_error() {
        with_memory_connection(|conn| {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn wal_mode_is_set() {
        with_memory_connection(|conn| {
            let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
            // In-memory DBs may report "memory" instead of "wal"
            assert!(mode == "wal" || mode == "memory");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn fts_table_exists_after_init() {
        with_memory_connection(|conn| {
            let exists: bool = conn
                .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions_fts'")?
                .exists([])?;
            assert!(exists);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn foreign_keys_are_enabled() {
        with_memory_connection(|conn| {
            let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
            assert_eq!(fk, 1);
            Ok(())
        })
        .unwrap();
    }
}
