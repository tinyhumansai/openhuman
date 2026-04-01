//! FTS5 episodic memory — full-text search over past sessions.
//!
//! Adds an FTS5 virtual table backed by an `episodic_log` table for storing
//! turn-level records with optional extracted lessons. The Archivist uses
//! this for post-session knowledge extraction and the `search_memory` tool
//! uses it for episodic recall.

use parking_lot::Mutex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A single episodic record (one turn or event).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicEntry {
    pub id: Option<i64>,
    pub session_id: String,
    pub timestamp: f64,
    pub role: String,
    pub content: String,
    pub lesson: Option<String>,
    pub tool_calls_json: Option<String>,
    pub cost_microdollars: u64,
}

/// SQL to create the episodic tables. Called during `UnifiedMemory` init.
pub const EPISODIC_INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS episodic_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    timestamp REAL NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    lesson TEXT,
    tool_calls_json TEXT,
    cost_microdollars INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_episodic_session
    ON episodic_log(session_id, timestamp);

CREATE VIRTUAL TABLE IF NOT EXISTS episodic_fts USING fts5(
    session_id,
    role,
    content,
    lesson,
    content=episodic_log,
    content_rowid=id,
    tokenize='porter unicode61'
);

-- Triggers to keep FTS5 in sync with the backing table.
CREATE TRIGGER IF NOT EXISTS episodic_ai AFTER INSERT ON episodic_log BEGIN
    INSERT INTO episodic_fts(rowid, session_id, role, content, lesson)
    VALUES (new.id, new.session_id, new.role, new.content, new.lesson);
END;

CREATE TRIGGER IF NOT EXISTS episodic_ad AFTER DELETE ON episodic_log BEGIN
    INSERT INTO episodic_fts(episodic_fts, rowid, session_id, role, content, lesson)
    VALUES ('delete', old.id, old.session_id, old.role, old.content, old.lesson);
END;

CREATE TRIGGER IF NOT EXISTS episodic_au AFTER UPDATE ON episodic_log BEGIN
    INSERT INTO episodic_fts(episodic_fts, rowid, session_id, role, content, lesson)
    VALUES ('delete', old.id, old.session_id, old.role, old.content, old.lesson);
    INSERT INTO episodic_fts(rowid, session_id, role, content, lesson)
    VALUES (new.id, new.session_id, new.role, new.content, new.lesson);
END;
"#;

/// Insert an episodic entry.
pub fn episodic_insert(conn: &Arc<Mutex<Connection>>, entry: &EpisodicEntry) -> anyhow::Result<()> {
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO episodic_log (session_id, timestamp, role, content, lesson, tool_calls_json, cost_microdollars)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            entry.session_id,
            entry.timestamp,
            entry.role,
            entry.content,
            entry.lesson,
            entry.tool_calls_json,
            entry.cost_microdollars as i64,
        ],
    )?;
    tracing::debug!(
        "[fts5] inserted episodic entry: session={}, role={}",
        entry.session_id,
        entry.role
    );
    Ok(())
}

/// Full-text search over episodic entries.
pub fn episodic_search(
    conn: &Arc<Mutex<Connection>>,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<EpisodicEntry>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT el.id, el.session_id, el.timestamp, el.role, el.content, el.lesson,
                el.tool_calls_json, el.cost_microdollars
         FROM episodic_fts AS ef
         JOIN episodic_log AS el ON ef.rowid = el.id
         WHERE episodic_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let rows = stmt
        .query_map(rusqlite::params![query, limit as i64], |row| {
            Ok(EpisodicEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                timestamp: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                lesson: row.get(5)?,
                tool_calls_json: row.get(6)?,
                cost_microdollars: row.get::<_, i64>(7)? as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    tracing::debug!(
        "[fts5] search '{}' returned {} results",
        query,
        rows.len()
    );
    Ok(rows)
}

/// Get all entries for a session (for post-session summary).
pub fn episodic_session_entries(
    conn: &Arc<Mutex<Connection>>,
    session_id: &str,
) -> anyhow::Result<Vec<EpisodicEntry>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT id, session_id, timestamp, role, content, lesson, tool_calls_json, cost_microdollars
         FROM episodic_log
         WHERE session_id = ?1
         ORDER BY timestamp ASC",
    )?;

    let rows = stmt
        .query_map(rusqlite::params![session_id], |row| {
            Ok(EpisodicEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                timestamp: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                lesson: row.get(5)?,
                tool_calls_json: row.get(6)?,
                cost_microdollars: row.get::<_, i64>(7)? as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(EPISODIC_INIT_SQL).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn insert_and_search() {
        let conn = setup_db();
        let entry = EpisodicEntry {
            id: None,
            session_id: "s1".into(),
            timestamp: 1000.0,
            role: "user".into(),
            content: "How do I deploy to production?".into(),
            lesson: Some("User frequently asks about deployment".into()),
            tool_calls_json: None,
            cost_microdollars: 100,
        };
        episodic_insert(&conn, &entry).unwrap();

        let results = episodic_search(&conn, "deploy production", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "s1");
        assert!(results[0].content.contains("deploy"));
    }

    #[test]
    fn session_entries() {
        let conn = setup_db();
        for i in 0..3 {
            episodic_insert(
                &conn,
                &EpisodicEntry {
                    id: None,
                    session_id: "s2".into(),
                    timestamp: 1000.0 + i as f64,
                    role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                    content: format!("Turn {i} content"),
                    lesson: None,
                    tool_calls_json: None,
                    cost_microdollars: 0,
                },
            )
            .unwrap();
        }

        let entries = episodic_session_entries(&conn, "s2").unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[0].timestamp < entries[2].timestamp);
    }

    #[test]
    fn empty_search_returns_empty() {
        let conn = setup_db();
        let results = episodic_search(&conn, "nonexistent query", 10).unwrap();
        assert!(results.is_empty());
    }
}
