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

use crate::openhuman::memory::safety;

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
    if safety::has_likely_secret(&entry.session_id) || safety::has_likely_secret(&entry.role) {
        tracing::warn!(
            "[memory:safety] episodic insert rejected secret-like session/role session_chars={} role_chars={}",
            entry.session_id.chars().count(),
            entry.role.chars().count()
        );
        anyhow::bail!("episodic session_id/role cannot contain secrets");
    }

    let content = safety::sanitize_text(&entry.content);
    let lesson = entry
        .lesson
        .as_ref()
        .map(|value| safety::sanitize_text(value));
    let tool_calls_json = entry.tool_calls_json.as_ref().map(|value| {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) {
            let sanitized = safety::sanitize_json(&parsed);
            safety::Sanitized {
                value: sanitized.value.to_string(),
                report: sanitized.report,
            }
        } else {
            safety::sanitize_text(value)
        }
    });

    let report = content
        .report
        .merge(
            lesson
                .as_ref()
                .map(|value| value.report)
                .unwrap_or_default(),
        )
        .merge(
            tool_calls_json
                .as_ref()
                .map(|value| value.report)
                .unwrap_or_default(),
        );
    if report.changed() {
        tracing::warn!(
            "[memory:safety] episodic insert sanitized session_chars={} role_chars={} text_redactions={} key_redactions={} blocked_secret_hits={} depth_redactions={}",
            entry.session_id.chars().count(),
            entry.role.chars().count(),
            report.text_redactions,
            report.key_redactions,
            report.blocked_secret_hits,
            report.depth_redactions
        );
    }

    let conn = conn.lock();
    conn.execute(
        "INSERT INTO episodic_log (session_id, timestamp, role, content, lesson, tool_calls_json, cost_microdollars)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            &entry.session_id,
            entry.timestamp,
            &entry.role,
            content.value,
            lesson.map(|value| value.value),
            tool_calls_json.map(|value| value.value),
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

    tracing::debug!("[fts5] search '{}' returned {} results", query, rows.len());
    Ok(rows)
}

/// FTS5 search across **all** sessions, optionally excluding one session
/// from the result set. Used by [`crate::openhuman::memory`] to surface
/// cross-chat conversational context for the same user/workspace (issue
/// #1505) without leaking the current chat's own history into the
/// "other chats" block.
///
/// `exclude_session` should be the active session_id when the caller is
/// also pulling same-session entries via [`episodic_session_entries`] —
/// passing `None` returns hits from every session indexed in this DB.
///
/// Workspace/user scope is enforced at the connection level: the SQLite
/// database lives at `<workspace>/memory/...` so one DB == one workspace.
/// This helper cannot cross that boundary.
pub fn episodic_cross_session_search(
    conn: &Arc<Mutex<Connection>>,
    query: &str,
    limit: usize,
    exclude_session: Option<&str>,
) -> anyhow::Result<Vec<EpisodicEntry>> {
    let conn = conn.lock();
    let trimmed = query.trim();
    if trimmed.is_empty() {
        tracing::debug!("[fts5] cross-session search skipped — empty query");
        return Ok(Vec::new());
    }

    // FTS5 MATCH expressions are picky about syntax — bare phrases with
    // punctuation can fail to parse. Wrap the query in double quotes so
    // it's treated as a phrase (FTS5 will still tokenize it). This mirrors
    // how the unified store sanitises queries before MATCH.
    let phrase_query = sanitize_fts_query(trimmed);
    if phrase_query.is_empty() {
        tracing::debug!("[fts5] cross-session search skipped — sanitised query is empty");
        return Ok(Vec::new());
    }

    let mut stmt = match exclude_session {
        Some(_) => conn.prepare(
            "SELECT el.id, el.session_id, el.timestamp, el.role, el.content, el.lesson,
                    el.tool_calls_json, el.cost_microdollars
             FROM episodic_fts AS ef
             JOIN episodic_log AS el ON ef.rowid = el.id
             WHERE episodic_fts MATCH ?1 AND el.session_id != ?2
             ORDER BY rank
             LIMIT ?3",
        )?,
        None => conn.prepare(
            "SELECT el.id, el.session_id, el.timestamp, el.role, el.content, el.lesson,
                    el.tool_calls_json, el.cost_microdollars
             FROM episodic_fts AS ef
             JOIN episodic_log AS el ON ef.rowid = el.id
             WHERE episodic_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?,
    };

    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<EpisodicEntry> {
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
    };

    let rows: Vec<EpisodicEntry> = match exclude_session {
        Some(sid) => stmt
            .query_map(rusqlite::params![phrase_query, sid, limit as i64], map_row)?
            .collect::<Result<Vec<_>, _>>()?,
        None => stmt
            .query_map(rusqlite::params![phrase_query, limit as i64], map_row)?
            .collect::<Result<Vec<_>, _>>()?,
    };

    // Never log the raw query string — may contain secrets / PII. Emit a
    // stable non-reversible hash + length instead so cross-session
    // diagnostics stay grep-friendly without leaking user content.
    let query_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        trimmed.hash(&mut hasher);
        hasher.finish()
    };
    tracing::debug!(
        "[fts5] cross-session search query_hash={:016x} query_len={} (exclude={:?}) returned {} results",
        query_hash,
        trimmed.chars().count(),
        exclude_session,
        rows.len()
    );
    Ok(rows)
}

/// Best-effort FTS5 query sanitiser: strip characters that break the
/// MATCH grammar (quotes, parens, asterisks) and wrap each surviving
/// whitespace-delimited token in double quotes so the FTS engine treats
/// it as a literal phrase. Returns an empty string when nothing usable
/// survives — the caller short-circuits to "no hits".
fn sanitize_fts_query(query: &str) -> String {
    let cleaned: String = query
        .chars()
        .map(|c| match c {
            '"' | '(' | ')' | '*' | ':' => ' ',
            other => other,
        })
        .collect();
    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .filter(|tok| !tok.is_empty())
        .take(8)
        .map(|tok| format!("\"{tok}\""))
        .collect();
    tokens.join(" ")
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

    #[test]
    fn insert_redacts_secret_like_content() {
        let conn = setup_db();
        episodic_insert(
            &conn,
            &EpisodicEntry {
                id: None,
                session_id: "s1".into(),
                timestamp: 1000.0,
                role: "user".into(),
                content: "Bearer abcdefghijklmnop".into(),
                lesson: Some("token=abc123".into()),
                tool_calls_json: Some("{\"api_key\":\"sk-1234567890123456789012345\"}".into()),
                cost_microdollars: 0,
            },
        )
        .unwrap();

        let rows = episodic_session_entries(&conn, "s1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "Bearer [REDACTED]");
        assert_eq!(rows[0].lesson.as_deref(), Some("[REDACTED]"));
        assert_eq!(
            rows[0].tool_calls_json.as_deref(),
            Some("{\"api_key\":\"[REDACTED_SECRET]\"}")
        );
    }

    #[test]
    fn insert_rejects_secret_like_session_id() {
        let conn = setup_db();
        let err = episodic_insert(
            &conn,
            &EpisodicEntry {
                id: None,
                session_id: "Bearer abcdefghijklmnop".into(),
                timestamp: 1000.0,
                role: "user".into(),
                content: "hello".into(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            },
        )
        .expect_err("secret-like session_id should be rejected");
        assert!(err.to_string().contains("cannot contain secrets"));
    }

    // ── Cross-session search (#1505) ─────────────────────────────────────

    fn insert_turn(conn: &Arc<Mutex<Connection>>, session_id: &str, ts: f64, content: &str) {
        episodic_insert(
            conn,
            &EpisodicEntry {
                id: None,
                session_id: session_id.into(),
                timestamp: ts,
                role: "user".into(),
                content: content.into(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            },
        )
        .unwrap();
    }

    #[test]
    fn cross_session_search_surfaces_other_sessions_excluding_current() {
        let conn = setup_db();
        // Chat A — user shared the durable fact
        insert_turn(
            &conn,
            "session-a",
            1000.0,
            "I prefer Postgres for new services",
        );
        // Chat B — current chat, where the question is being asked
        insert_turn(
            &conn,
            "session-b",
            2000.0,
            "What database should I use today?",
        );
        // Chat C — yet another chat with a related fact
        insert_turn(&conn, "session-c", 1500.0, "Postgres timezone is UTC");

        // Asking from chat B: should see session-a + session-c (not session-b)
        let hits = episodic_cross_session_search(&conn, "Postgres", 10, Some("session-b")).unwrap();
        assert!(
            !hits.is_empty(),
            "cross-session search must surface hits from other sessions"
        );
        for hit in &hits {
            assert_ne!(
                hit.session_id, "session-b",
                "current session must be excluded from cross-session sweep, got {}",
                hit.session_id
            );
        }
        let session_ids: std::collections::HashSet<&str> =
            hits.iter().map(|h| h.session_id.as_str()).collect();
        assert!(session_ids.contains("session-a"));
        assert!(session_ids.contains("session-c"));
    }

    #[test]
    fn cross_session_search_returns_empty_for_unknown_query() {
        let conn = setup_db();
        insert_turn(&conn, "session-a", 1000.0, "I prefer Postgres");
        let hits = episodic_cross_session_search(&conn, "kubernetes", 10, None).unwrap();
        assert!(
            hits.is_empty(),
            "no FTS match should produce zero hits, not all rows"
        );
    }

    #[test]
    fn cross_session_search_handles_empty_query() {
        let conn = setup_db();
        insert_turn(&conn, "session-a", 1000.0, "anything");
        let hits = episodic_cross_session_search(&conn, "   ", 10, None).unwrap();
        assert!(hits.is_empty(), "empty query short-circuits to zero hits");
    }

    #[test]
    fn cross_session_search_sanitises_punctuation_safely() {
        let conn = setup_db();
        insert_turn(&conn, "session-a", 1000.0, "Postgres deployment notes");
        // Query with FTS5-hostile punctuation — should not panic. Tokens
        // shared with the indexed row should still match (FTS5 phrase
        // ANDs every quoted token, so we use words that all appear in
        // the row to avoid AND-mismatch false negatives).
        let hits =
            episodic_cross_session_search(&conn, "\"Postgres\" (deployment)?", 10, None).unwrap();
        assert!(
            !hits.is_empty(),
            "punctuated query whose surviving tokens match the indexed row must still surface it"
        );
        assert!(hits[0].content.contains("Postgres"));
    }

    #[test]
    fn cross_session_search_does_not_panic_on_pure_punctuation() {
        let conn = setup_db();
        insert_turn(&conn, "session-a", 1000.0, "Postgres deployment notes");
        // All-punctuation query should normalise to empty and produce
        // zero hits without panicking.
        let hits = episodic_cross_session_search(&conn, "()*\":", 10, None).unwrap();
        assert!(
            hits.is_empty(),
            "punctuation-only query must produce zero hits"
        );
    }

    #[test]
    fn cross_session_search_no_exclusion_includes_all_matches() {
        let conn = setup_db();
        insert_turn(&conn, "session-a", 1000.0, "Postgres preference");
        insert_turn(&conn, "session-b", 2000.0, "Postgres setup");

        let hits = episodic_cross_session_search(&conn, "Postgres", 10, None).unwrap();
        let session_ids: std::collections::HashSet<&str> =
            hits.iter().map(|h| h.session_id.as_str()).collect();
        assert!(session_ids.contains("session-a"));
        assert!(session_ids.contains("session-b"));
    }
}
