//! Event extraction and storage — atomic facts, decisions, commitments, and
//! preferences extracted from closed conversation segments.
//!
//! Two-tier extraction:
//! - Tier A (heuristic/regex): always runs, free — pattern matching for
//!   decisions, commitments, preferences, and facts.
//! - Tier B (local LLM): runs on segment close if local AI is enabled.

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// SQL to create the event tables. Called during UnifiedMemory init.
pub const EVENTS_INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS event_log (
    event_id TEXT PRIMARY KEY,
    segment_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    namespace TEXT NOT NULL DEFAULT 'global',
    event_type TEXT NOT NULL,
    content TEXT NOT NULL,
    subject TEXT,
    timestamp_ref TEXT,
    confidence REAL NOT NULL,
    embedding BLOB,
    source_turn_ids TEXT,
    created_at REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_events_segment
    ON event_log(segment_id);

CREATE INDEX IF NOT EXISTS idx_events_namespace
    ON event_log(namespace, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_events_type
    ON event_log(event_type, namespace);

CREATE VIRTUAL TABLE IF NOT EXISTS event_fts USING fts5(
    content,
    subject,
    event_type,
    content=event_log,
    content_rowid=rowid,
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS event_ai AFTER INSERT ON event_log BEGIN
    INSERT INTO event_fts(rowid, content, subject, event_type)
    VALUES (new.rowid, new.content, new.subject, new.event_type);
END;

CREATE TRIGGER IF NOT EXISTS event_ad AFTER DELETE ON event_log BEGIN
    INSERT INTO event_fts(event_fts, rowid, content, subject, event_type)
    VALUES ('delete', old.rowid, old.content, old.subject, old.event_type);
END;

CREATE TRIGGER IF NOT EXISTS event_au AFTER UPDATE ON event_log BEGIN
    INSERT INTO event_fts(event_fts, rowid, content, subject, event_type)
    VALUES ('delete', old.rowid, old.content, old.subject, old.event_type);
    INSERT INTO event_fts(rowid, content, subject, event_type)
    VALUES (new.rowid, new.content, new.subject, new.event_type);
END;

-- Per-(event, embedding model) vectors (#1574). The legacy event_log.embedding
-- column stays available during the dual-write migration; this table records
-- vector-space provenance for safe provider/model switches.
CREATE TABLE IF NOT EXISTS event_embeddings (
    event_id        TEXT NOT NULL REFERENCES event_log(event_id) ON DELETE CASCADE,
    model_signature TEXT NOT NULL,
    vector          BLOB NOT NULL,
    dim             INTEGER NOT NULL,
    created_at      REAL NOT NULL,
    PRIMARY KEY (event_id, model_signature)
);

CREATE INDEX IF NOT EXISTS idx_event_embeddings_model
    ON event_embeddings(model_signature);
"#;

/// Event types extracted from conversations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Fact,
    Decision,
    Commitment,
    Preference,
    Question,
    Foresight,
}

impl EventType {
    /// Stable lowercase identifier persisted in the `event_log` table.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Decision => "decision",
            Self::Commitment => "commitment",
            Self::Preference => "preference",
            Self::Question => "question",
            Self::Foresight => "foresight",
        }
    }

    /// Parse a stored string back to an `EventType`; unknown values fall back
    /// to `Fact`.
    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "decision" => Self::Decision,
            "commitment" => Self::Commitment,
            "preference" => Self::Preference,
            "question" => Self::Question,
            "foresight" => Self::Foresight,
            _ => Self::Fact,
        }
    }
}

/// An extracted event record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub event_id: String,
    pub segment_id: String,
    pub session_id: String,
    pub namespace: String,
    pub event_type: EventType,
    pub content: String,
    pub subject: Option<String>,
    pub timestamp_ref: Option<String>,
    pub confidence: f64,
    pub embedding: Option<Vec<f32>>,
    pub source_turn_ids: Option<String>,
    pub created_at: f64,
}

/// Insert an event record.
pub fn event_insert(conn: &Arc<Mutex<Connection>>, event: &EventRecord) -> anyhow::Result<()> {
    let embedding_bytes: Option<Vec<u8>> = event.embedding.as_ref().map(|v| vec_to_bytes(v));
    let conn = conn.lock();
    conn.execute(
        "INSERT OR REPLACE INTO event_log
         (event_id, segment_id, session_id, namespace, event_type, content,
          subject, timestamp_ref, confidence, embedding, source_turn_ids, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            event.event_id,
            event.segment_id,
            event.session_id,
            event.namespace,
            event.event_type.as_str(),
            event.content,
            event.subject,
            event.timestamp_ref,
            event.confidence,
            embedding_bytes,
            event.source_turn_ids,
            event.created_at,
        ],
    )?;
    tracing::debug!(
        "[events] inserted event {} type={} for segment={}",
        event.event_id,
        event.event_type.as_str(),
        event.segment_id
    );
    Ok(())
}

/// Store an event embedding for a specific provider/model/dimension signature.
///
/// This writes only the per-model table introduced for #1574. The legacy
/// `event_log.embedding` column remains available for dual-read fallback.
pub fn event_embedding_upsert(
    conn: &Arc<Mutex<Connection>>,
    event_id: &str,
    model_signature: &str,
    embedding: &[f32],
    created_at: f64,
) -> anyhow::Result<()> {
    let bytes = vec_to_bytes(embedding);
    let dim = i64::try_from(embedding.len())?;
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO event_embeddings (event_id, model_signature, vector, dim, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(event_id, model_signature) DO UPDATE SET
            vector = excluded.vector,
            dim = excluded.dim,
            created_at = excluded.created_at",
        params![event_id, model_signature, bytes, dim, created_at],
    )?;
    Ok(())
}

/// Fetch an event embedding for exactly one provider/model/dimension signature.
pub fn event_embedding_get(
    conn: &Arc<Mutex<Connection>>,
    event_id: &str,
    model_signature: &str,
) -> anyhow::Result<Option<Vec<f32>>> {
    let conn = conn.lock();
    let row: Option<(Vec<u8>, i64)> = conn
        .query_row(
            "SELECT vector, dim
               FROM event_embeddings
              WHERE event_id = ?1 AND model_signature = ?2",
            params![event_id, model_signature],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        None => Ok(None),
        Some((bytes, dim)) => decode_embedding_row(&bytes, dim),
    }
}

/// Search events via FTS5, scoped to a namespace.
pub fn event_search_fts(
    conn: &Arc<Mutex<Connection>>,
    namespace: &str,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<EventRecord>> {
    let conn = conn.lock();
    let trimmed = query.trim();
    if trimmed.is_empty() {
        tracing::debug!("[events] FTS search skipped — empty query");
        return Ok(Vec::new());
    }
    let phrase_query = super::fts5::sanitize_fts_query(trimmed);
    if phrase_query.is_empty() {
        tracing::debug!("[events] FTS search skipped — sanitised query is empty");
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT el.event_id, el.segment_id, el.session_id, el.namespace,
                el.event_type, el.content, el.subject, el.timestamp_ref,
                el.confidence, el.embedding, el.source_turn_ids, el.created_at
         FROM event_fts AS ef
         JOIN event_log AS el ON ef.rowid = el.rowid
         WHERE event_fts MATCH ?1 AND el.namespace = ?2
         ORDER BY rank
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![phrase_query, namespace, limit as i64], |row| {
            row_to_event(row)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    tracing::debug!(
        "[events] FTS search ns={} returned {} results",
        namespace,
        rows.len()
    );
    Ok(rows)
}

/// Get all events for a segment.
pub fn events_for_segment(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
) -> anyhow::Result<Vec<EventRecord>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT event_id, segment_id, session_id, namespace,
                event_type, content, subject, timestamp_ref,
                confidence, embedding, source_turn_ids, created_at
         FROM event_log
         WHERE segment_id = ?1
         ORDER BY created_at ASC",
    )?;
    let rows = stmt
        .query_map(params![segment_id], row_to_event)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get events by type within a namespace.
pub fn events_by_type(
    conn: &Arc<Mutex<Connection>>,
    namespace: &str,
    event_type: &str,
    limit: usize,
) -> anyhow::Result<Vec<EventRecord>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT event_id, segment_id, session_id, namespace,
                event_type, content, subject, timestamp_ref,
                confidence, embedding, source_turn_ids, created_at
         FROM event_log
         WHERE namespace = ?1 AND event_type = ?2
         ORDER BY created_at DESC
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![namespace, event_type, limit as i64], |row| {
            row_to_event(row)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ── Heuristic extraction patterns ──

/// Patterns that indicate a decision.
const DECISION_PATTERNS: &[&str] = &[
    "let's go with",
    "lets go with",
    "i've decided",
    "ive decided",
    "i decided",
    "we decided",
    "we agreed",
    "the decision is",
    "going with",
    "we'll use",
    "well use",
    "i'll use",
    "chosen to",
    "i choose",
    "we choose",
];

/// Patterns that indicate a commitment or deadline.
const COMMITMENT_PATTERNS: &[&str] = &[
    "by friday",
    "by monday",
    "by tuesday",
    "by wednesday",
    "by thursday",
    "by saturday",
    "by sunday",
    "by tomorrow",
    "by next week",
    "by end of",
    "deadline is",
    "due date",
    "i will",
    "i'll do",
    "ill do",
    "i promise",
    "i commit",
    "we need to finish",
    "scheduled for",
    "plan to",
    "planning to",
];

/// Patterns that indicate a preference.
const PREFERENCE_PATTERNS: &[&str] = &[
    "i prefer",
    "i like",
    "i love",
    "i hate",
    "i dislike",
    "i always",
    "i never",
    "my favorite",
    "my favourite",
    "i usually",
    "i tend to",
    "i'm used to",
    "im used to",
];

/// Patterns that indicate a personal fact.
const FACT_PATTERNS: &[&str] = &[
    "i'm based in",
    "im based in",
    "i live in",
    "i work at",
    "i work for",
    "my name is",
    "i'm a",
    "im a",
    "i am a",
    "my role is",
    "i've been",
    "ive been",
    "i have been",
    "i'm from",
    "im from",
    "my timezone",
    "my time zone",
];

/// Extract events from text using heuristic pattern matching.
/// Returns a list of (event_type, matched_sentence) pairs.
pub fn extract_events_heuristic(text: &str) -> Vec<(EventType, String)> {
    let mut events = Vec::new();

    // Split into sentences (rough heuristic).
    let sentences: Vec<&str> = text
        .split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| s.len() > 5)
        .collect();

    for sentence in sentences {
        let lower = sentence.to_lowercase();

        // Check each pattern category.
        for pattern in DECISION_PATTERNS {
            if lower.contains(pattern) {
                events.push((EventType::Decision, sentence.to_string()));
                break;
            }
        }
        for pattern in COMMITMENT_PATTERNS {
            if lower.contains(pattern) {
                // Avoid duplicate if already matched as decision.
                if !events.iter().any(|(_, s)| s == sentence) {
                    events.push((EventType::Commitment, sentence.to_string()));
                }
                break;
            }
        }
        for pattern in PREFERENCE_PATTERNS {
            if lower.contains(pattern) {
                if !events.iter().any(|(_, s)| s == sentence) {
                    events.push((EventType::Preference, sentence.to_string()));
                }
                break;
            }
        }
        for pattern in FACT_PATTERNS {
            if lower.contains(pattern) {
                if !events.iter().any(|(_, s)| s == sentence) {
                    events.push((EventType::Fact, sentence.to_string()));
                }
                break;
            }
        }
    }

    events
}

// ── helpers ──

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRecord> {
    let embedding_blob: Option<Vec<u8>> = row.get(9)?;
    let event_type_str: String = row.get(4)?;
    Ok(EventRecord {
        event_id: row.get(0)?,
        segment_id: row.get(1)?,
        session_id: row.get(2)?,
        namespace: row.get(3)?,
        event_type: EventType::parse_or_default(&event_type_str),
        content: row.get(5)?,
        subject: row.get(6)?,
        timestamp_ref: row.get(7)?,
        confidence: row.get(8)?,
        embedding: embedding_blob.as_deref().map(bytes_to_vec),
        source_turn_ids: row.get(10)?,
        created_at: row.get(11)?,
    })
}

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn decode_embedding_row(bytes: &[u8], dim: i64) -> anyhow::Result<Option<Vec<f32>>> {
    if dim < 0 {
        anyhow::bail!("event embedding has negative dimension {dim}");
    }
    if !bytes.len().is_multiple_of(4) {
        anyhow::bail!(
            "event embedding blob length {} not a multiple of 4",
            bytes.len()
        );
    }
    let vector = bytes_to_vec(bytes);
    if vector.len() != dim as usize {
        anyhow::bail!(
            "event embedding dimension mismatch: dim column says {dim}, blob contains {} floats",
            vector.len()
        );
    }
    Ok(Some(vector))
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
