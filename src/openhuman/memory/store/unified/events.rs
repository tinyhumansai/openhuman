//! Event extraction and storage — atomic facts, decisions, commitments, and
//! preferences extracted from closed conversation segments.
//!
//! Two-tier extraction:
//! - Tier A (heuristic/regex): always runs, free — pattern matching for
//!   decisions, commitments, preferences, and facts.
//! - Tier B (local LLM): runs on segment close if local AI is enabled.

use parking_lot::Mutex;
use rusqlite::{params, Connection};
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

    pub fn from_str(s: &str) -> Self {
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

/// Search events via FTS5.
pub fn event_search_fts(
    conn: &Arc<Mutex<Connection>>,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<EventRecord>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT el.event_id, el.segment_id, el.session_id, el.namespace,
                el.event_type, el.content, el.subject, el.timestamp_ref,
                el.confidence, el.embedding, el.source_turn_ids, el.created_at
         FROM event_fts AS ef
         JOIN event_log AS el ON ef.rowid = el.rowid
         WHERE event_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![query, limit as i64], |row| row_to_event(row))?
        .collect::<Result<Vec<_>, _>>()?;
    tracing::debug!(
        "[events] FTS search '{}' returned {} results",
        query,
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
        .query_map(params![segment_id], |row| row_to_event(row))?
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
        .split(|c: char| c == '.' || c == '!' || c == '?' || c == '\n')
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
        event_type: EventType::from_str(&event_type_str),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(EVENTS_INIT_SQL).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn insert_and_search_event() {
        let conn = setup_db();
        let event = EventRecord {
            event_id: "evt-1".into(),
            segment_id: "seg-1".into(),
            session_id: "s1".into(),
            namespace: "global".into(),
            event_type: EventType::Decision,
            content: "We decided to use Rust for the backend".into(),
            subject: Some("backend language".into()),
            timestamp_ref: None,
            confidence: 0.8,
            embedding: None,
            source_turn_ids: None,
            created_at: 1000.0,
        };
        event_insert(&conn, &event).unwrap();

        let results = event_search_fts(&conn, "Rust backend", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].event_type, EventType::Decision);
    }

    #[test]
    fn heuristic_extraction_finds_patterns() {
        let text = "I prefer dark mode for coding. We decided to use PostgreSQL. \
                     The deadline is by Friday. I live in Berlin. \
                     This is a regular sentence with no pattern.";
        let events = extract_events_heuristic(text);

        let types: Vec<&EventType> = events.iter().map(|(t, _)| t).collect();
        assert!(types.contains(&&EventType::Preference));
        assert!(types.contains(&&EventType::Decision));
        assert!(types.contains(&&EventType::Commitment));
        assert!(types.contains(&&EventType::Fact));
        // Regular sentence should NOT be extracted.
        assert!(!events.iter().any(|(_, s)| s.contains("regular sentence")));
    }

    #[test]
    fn events_for_segment_returns_ordered() {
        let conn = setup_db();
        for i in 0..3 {
            event_insert(
                &conn,
                &EventRecord {
                    event_id: format!("evt-{i}"),
                    segment_id: "seg-1".into(),
                    session_id: "s1".into(),
                    namespace: "global".into(),
                    event_type: EventType::Fact,
                    content: format!("Fact number {i}"),
                    subject: None,
                    timestamp_ref: None,
                    confidence: 0.7,
                    embedding: None,
                    source_turn_ids: None,
                    created_at: 1000.0 + i as f64,
                },
            )
            .unwrap();
        }

        let events = events_for_segment(&conn, "seg-1").unwrap();
        assert_eq!(events.len(), 3);
        assert!(events[0].created_at < events[2].created_at);
    }

    #[test]
    fn event_insert_idempotent() {
        let conn = setup_db();
        let event = EventRecord {
            event_id: "evt-idem".into(),
            segment_id: "seg-1".into(),
            session_id: "s1".into(),
            namespace: "global".into(),
            event_type: EventType::Fact,
            content: "Rust is a systems language".into(),
            subject: None,
            timestamp_ref: None,
            confidence: 0.9,
            embedding: None,
            source_turn_ids: None,
            created_at: 1000.0,
        };
        // Insert same event_id twice — OR REPLACE semantics; no duplicate row.
        event_insert(&conn, &event).unwrap();
        event_insert(&conn, &event).unwrap();

        let events = events_for_segment(&conn, "seg-1").unwrap();
        assert_eq!(events.len(), 1, "Duplicate insert should not create a second row");
    }

    #[test]
    fn events_by_type_filters_correctly() {
        let conn = setup_db();

        let make_event = |id: &str, event_type: EventType, ns: &str| EventRecord {
            event_id: id.to_string(),
            segment_id: "seg-x".into(),
            session_id: "s1".into(),
            namespace: ns.to_string(),
            event_type,
            content: format!("Content for {id}"),
            subject: None,
            timestamp_ref: None,
            confidence: 0.7,
            embedding: None,
            source_turn_ids: None,
            created_at: 1000.0,
        };

        event_insert(&conn, &make_event("e-dec", EventType::Decision, "ns1")).unwrap();
        event_insert(&conn, &make_event("e-pref", EventType::Preference, "ns1")).unwrap();
        event_insert(&conn, &make_event("e-fact", EventType::Fact, "ns1")).unwrap();

        let decisions = events_by_type(&conn, "ns1", "decision", 10).unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].event_id, "e-dec");
        assert_eq!(decisions[0].event_type, EventType::Decision);

        let prefs = events_by_type(&conn, "ns1", "preference", 10).unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].event_id, "e-pref");

        // Different namespace should return nothing.
        let other = events_by_type(&conn, "ns2", "decision", 10).unwrap();
        assert!(other.is_empty(), "No events expected for unrelated namespace");
    }

    #[test]
    fn heuristic_extracts_multiple_from_same_sentence() {
        // A sentence that simultaneously satisfies a preference pattern AND a fact
        // pattern will only produce one event (dedup guard). Use two separate
        // sentences to confirm both types are emitted.
        let text = "I prefer Python for scripting. I live in Berlin.";
        let events = extract_events_heuristic(text);

        let types: Vec<&EventType> = events.iter().map(|(t, _)| t).collect();
        assert!(
            types.contains(&&EventType::Preference),
            "Expected a Preference event from 'I prefer Python'"
        );
        assert!(
            types.contains(&&EventType::Fact),
            "Expected a Fact event from 'I live in Berlin'"
        );
        assert!(
            events.len() >= 2,
            "Expected at least 2 events, got {}",
            events.len()
        );
    }

    #[test]
    fn heuristic_handles_empty_and_whitespace() {
        assert!(
            extract_events_heuristic("").is_empty(),
            "Empty string should yield no events"
        );
        assert!(
            extract_events_heuristic("   \n\t  ").is_empty(),
            "Whitespace-only string should yield no events"
        );
    }

    #[test]
    fn event_fts_matches_subject_field() {
        let conn = setup_db();
        let event = EventRecord {
            event_id: "evt-subj".into(),
            segment_id: "seg-1".into(),
            session_id: "s1".into(),
            namespace: "global".into(),
            event_type: EventType::Decision,
            content: "We agreed on the final design".into(),
            subject: Some("microservice architecture".into()),
            timestamp_ref: None,
            confidence: 0.85,
            embedding: None,
            source_turn_ids: None,
            created_at: 1000.0,
        };
        event_insert(&conn, &event).unwrap();

        // Search by content (should match).
        let by_content = event_search_fts(&conn, "design", 5).unwrap();
        assert_eq!(by_content.len(), 1, "FTS should match on content field");

        // Search by subject text (should also match via event_fts).
        let by_subject = event_search_fts(&conn, "microservice", 5).unwrap();
        assert_eq!(by_subject.len(), 1, "FTS should match on subject field");
        assert_eq!(by_subject[0].event_id, "evt-subj");
    }
}
