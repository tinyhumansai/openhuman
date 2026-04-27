//! Conversation segmentation — groups consecutive episodic turns into
//! coherent "segments" using lightweight heuristic boundary detection.
//!
//! Inspired by EverMemOS MemCells: instead of indexing raw turns individually,
//! segments capture a topic-coherent block of conversation that can be
//! summarised, searched, and used for downstream extraction (events, profile).

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// SQL to create the conversation_segments table. Called during UnifiedMemory init.
pub const SEGMENTS_INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS conversation_segments (
    segment_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    namespace TEXT NOT NULL DEFAULT 'global',
    start_episodic_id INTEGER NOT NULL,
    end_episodic_id INTEGER,
    start_timestamp REAL NOT NULL,
    end_timestamp REAL,
    turn_count INTEGER NOT NULL DEFAULT 0,
    summary TEXT,
    embedding BLOB,
    topic_keywords TEXT,
    status TEXT NOT NULL DEFAULT 'open',
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_segments_session
    ON conversation_segments(session_id, start_timestamp);

CREATE INDEX IF NOT EXISTS idx_segments_namespace
    ON conversation_segments(namespace, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_segments_status
    ON conversation_segments(status, session_id);
"#;

/// Segment status lifecycle: open → closed → summarised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentStatus {
    Open,
    Closed,
    Summarised,
}

impl SegmentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Summarised => "summarised",
        }
    }

    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "closed" => Self::Closed,
            "summarised" => Self::Summarised,
            _ => Self::Open,
        }
    }
}

/// A conversation segment record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSegment {
    pub segment_id: String,
    pub session_id: String,
    pub namespace: String,
    pub start_episodic_id: i64,
    pub end_episodic_id: Option<i64>,
    pub start_timestamp: f64,
    pub end_timestamp: Option<f64>,
    pub turn_count: i32,
    pub summary: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub topic_keywords: Option<String>,
    pub status: SegmentStatus,
    pub created_at: f64,
    pub updated_at: f64,
}

/// Boundary detection configuration.
#[derive(Debug, Clone)]
pub struct BoundaryConfig {
    /// Maximum time gap (seconds) between turns before forcing a new segment.
    pub max_time_gap_secs: f64,
    /// Minimum cosine similarity between turn embedding and segment centroid.
    /// Below this threshold, a boundary is detected.
    pub min_cosine_similarity: f32,
    /// Maximum turns per segment before forcing a boundary.
    pub max_turns_per_segment: i32,
}

impl Default for BoundaryConfig {
    fn default() -> Self {
        Self {
            max_time_gap_secs: 600.0, // 10 minutes
            min_cosine_similarity: 0.4,
            max_turns_per_segment: 20,
        }
    }
}

/// Result of boundary detection for a new turn.
#[derive(Debug, Clone)]
pub enum BoundaryDecision {
    /// Continue accumulating into the current segment.
    Continue,
    /// Close the current segment and start a new one.
    Boundary(BoundaryReason),
}

#[derive(Debug, Clone)]
pub enum BoundaryReason {
    TimeGap,
    EmbeddingDrift,
    ExplicitMarker,
    TurnCountExceeded,
}

impl std::fmt::Display for BoundaryReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimeGap => write!(f, "time_gap"),
            Self::EmbeddingDrift => write!(f, "embedding_drift"),
            Self::ExplicitMarker => write!(f, "explicit_marker"),
            Self::TurnCountExceeded => write!(f, "turn_count_exceeded"),
        }
    }
}

/// Regex patterns that signal an explicit topic change.
const TOPIC_CHANGE_MARKERS: &[&str] = &[
    "now let's",
    "now lets",
    "switching to",
    "different topic",
    "moving on to",
    "let's move on",
    "lets move on",
    "can you help me with",
    "new question",
    "unrelated but",
    "changing subject",
    "on another note",
    "anyway,",
    "by the way,",
    "btw,",
];

/// Create a new open segment.
pub fn segment_create(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
    session_id: &str,
    namespace: &str,
    start_episodic_id: i64,
    start_timestamp: f64,
    now: f64,
) -> anyhow::Result<()> {
    let conn = conn.lock();
    conn.execute(
        "INSERT INTO conversation_segments
         (segment_id, session_id, namespace, start_episodic_id, start_timestamp,
          turn_count, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 'open', ?6, ?6)",
        params![
            segment_id,
            session_id,
            namespace,
            start_episodic_id,
            start_timestamp,
            now
        ],
    )?;
    tracing::debug!("[segments] created segment {segment_id} for session={session_id}");
    Ok(())
}

/// Increment turn count and update the latest episodic ID / timestamp.
pub fn segment_append_turn(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
    episodic_id: i64,
    timestamp: f64,
    now: f64,
) -> anyhow::Result<()> {
    let conn = conn.lock();
    conn.execute(
        "UPDATE conversation_segments
         SET turn_count = turn_count + 1,
             end_episodic_id = ?2,
             end_timestamp = ?3,
             updated_at = ?4
         WHERE segment_id = ?1",
        params![segment_id, episodic_id, timestamp, now],
    )?;
    Ok(())
}

/// Close a segment (transition from open → closed).
pub fn segment_close(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
    now: f64,
) -> anyhow::Result<()> {
    let conn = conn.lock();
    conn.execute(
        "UPDATE conversation_segments
         SET status = 'closed', updated_at = ?2
         WHERE segment_id = ?1 AND status = 'open'",
        params![segment_id, now],
    )?;
    tracing::debug!("[segments] closed segment {segment_id}");
    Ok(())
}

/// Update a segment's summary and mark as summarised.
pub fn segment_set_summary(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
    summary: &str,
    now: f64,
) -> anyhow::Result<()> {
    let conn = conn.lock();
    conn.execute(
        "UPDATE conversation_segments
         SET summary = ?2, status = 'summarised', updated_at = ?3
         WHERE segment_id = ?1",
        params![segment_id, summary, now],
    )?;
    Ok(())
}

/// Store the segment-level embedding.
pub fn segment_set_embedding(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
    embedding: &[f32],
    now: f64,
) -> anyhow::Result<()> {
    let bytes = vec_to_bytes(embedding);
    let conn = conn.lock();
    conn.execute(
        "UPDATE conversation_segments SET embedding = ?2, updated_at = ?3 WHERE segment_id = ?1",
        params![segment_id, bytes, now],
    )?;
    Ok(())
}

/// Store topic keywords for the segment.
pub fn segment_set_keywords(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
    keywords: &str,
    now: f64,
) -> anyhow::Result<()> {
    let conn = conn.lock();
    conn.execute(
        "UPDATE conversation_segments SET topic_keywords = ?2, updated_at = ?3 WHERE segment_id = ?1",
        params![segment_id, keywords, now],
    )?;
    Ok(())
}

/// Get the currently open segment for a session (if any).
pub fn open_segment_for_session(
    conn: &Arc<Mutex<Connection>>,
    session_id: &str,
) -> anyhow::Result<Option<ConversationSegment>> {
    let conn = conn.lock();
    let row = conn
        .query_row(
            "SELECT segment_id, session_id, namespace, start_episodic_id, end_episodic_id,
                    start_timestamp, end_timestamp, turn_count, summary, embedding,
                    topic_keywords, status, created_at, updated_at
             FROM conversation_segments
             WHERE session_id = ?1 AND status = 'open'
             ORDER BY created_at DESC
             LIMIT 1",
            params![session_id],
            row_to_segment,
        )
        .optional()?;
    Ok(row)
}

/// List segments for a namespace (most recent first).
pub fn segments_by_namespace(
    conn: &Arc<Mutex<Connection>>,
    namespace: &str,
    limit: usize,
) -> anyhow::Result<Vec<ConversationSegment>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT segment_id, session_id, namespace, start_episodic_id, end_episodic_id,
                start_timestamp, end_timestamp, turn_count, summary, embedding,
                topic_keywords, status, created_at, updated_at
         FROM conversation_segments
         WHERE namespace = ?1
         ORDER BY updated_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![namespace, limit as i64], row_to_segment)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get a specific segment by ID.
pub fn segment_get(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
) -> anyhow::Result<Option<ConversationSegment>> {
    let conn = conn.lock();
    let row = conn
        .query_row(
            "SELECT segment_id, session_id, namespace, start_episodic_id, end_episodic_id,
                    start_timestamp, end_timestamp, turn_count, summary, embedding,
                    topic_keywords, status, created_at, updated_at
             FROM conversation_segments
             WHERE segment_id = ?1",
            params![segment_id],
            row_to_segment,
        )
        .optional()?;
    Ok(row)
}

/// Get all closed (unsummarised) segments that need summary generation.
pub fn segments_pending_summary(
    conn: &Arc<Mutex<Connection>>,
    limit: usize,
) -> anyhow::Result<Vec<ConversationSegment>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT segment_id, session_id, namespace, start_episodic_id, end_episodic_id,
                start_timestamp, end_timestamp, turn_count, summary, embedding,
                topic_keywords, status, created_at, updated_at
         FROM conversation_segments
         WHERE status = 'closed'
         ORDER BY created_at ASC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit as i64], row_to_segment)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Detect whether a boundary should be created based on heuristics.
pub fn detect_boundary(
    config: &BoundaryConfig,
    current_segment: &ConversationSegment,
    new_turn_timestamp: f64,
    new_turn_content: &str,
    new_turn_embedding: Option<&[f32]>,
) -> BoundaryDecision {
    // 1. Turn count exceeded.
    if current_segment.turn_count >= config.max_turns_per_segment {
        tracing::debug!(
            "[segments] boundary: turn count {} >= {}",
            current_segment.turn_count,
            config.max_turns_per_segment
        );
        return BoundaryDecision::Boundary(BoundaryReason::TurnCountExceeded);
    }

    // 2. Time gap check.
    let last_timestamp = current_segment
        .end_timestamp
        .unwrap_or(current_segment.start_timestamp);
    let gap = new_turn_timestamp - last_timestamp;
    if gap > config.max_time_gap_secs {
        tracing::debug!(
            "[segments] boundary: time gap {gap:.0}s > {}s",
            config.max_time_gap_secs
        );
        return BoundaryDecision::Boundary(BoundaryReason::TimeGap);
    }

    // 3. Explicit topic-change markers.
    let content_lower = new_turn_content.to_lowercase();
    for marker in TOPIC_CHANGE_MARKERS {
        if content_lower.contains(marker) {
            tracing::debug!("[segments] boundary: explicit marker '{marker}'");
            return BoundaryDecision::Boundary(BoundaryReason::ExplicitMarker);
        }
    }

    // 4. Embedding drift (cosine similarity).
    if let (Some(segment_emb), Some(turn_emb)) =
        (current_segment.embedding.as_ref(), new_turn_embedding)
    {
        if !segment_emb.is_empty() && segment_emb.len() == turn_emb.len() {
            let similarity = cosine_similarity_f32(segment_emb, turn_emb);
            if similarity < config.min_cosine_similarity {
                tracing::debug!(
                    "[segments] boundary: embedding drift (sim={similarity:.3} < {})",
                    config.min_cosine_similarity
                );
                return BoundaryDecision::Boundary(BoundaryReason::EmbeddingDrift);
            }
        }
    }

    BoundaryDecision::Continue
}

/// Compute mean embedding from an existing centroid and a new vector.
/// Returns a new centroid that is the incremental mean.
pub fn incremental_mean_embedding(
    current_centroid: &[f32],
    new_embedding: &[f32],
    count: usize,
) -> Vec<f32> {
    if current_centroid.is_empty() || current_centroid.len() != new_embedding.len() {
        return new_embedding.to_vec();
    }
    current_centroid
        .iter()
        .zip(new_embedding.iter())
        .map(|(c, n)| c + (n - c) / (count as f32 + 1.0))
        .collect()
}

/// Build a fallback summary from first and last turn content.
pub fn fallback_summary(first_content: &str, last_content: &str, turn_count: i32) -> String {
    let first_truncated = truncate_utf8_safe(first_content, 200);
    let last_truncated = truncate_utf8_safe(last_content, 200);
    format!(
        "Conversation segment ({turn_count} turns). Started with: {first_truncated} | Ended with: {last_truncated}"
    )
}

/// Truncate a string at a safe UTF-8 char boundary.
fn truncate_utf8_safe(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => format!("{}...", &s[..byte_idx]),
        None => s.to_string(),
    }
}

// ── helpers ──

fn row_to_segment(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationSegment> {
    let embedding_blob: Option<Vec<u8>> = row.get(9)?;
    let status_str: String = row.get(11)?;
    Ok(ConversationSegment {
        segment_id: row.get(0)?,
        session_id: row.get(1)?,
        namespace: row.get(2)?,
        start_episodic_id: row.get(3)?,
        end_episodic_id: row.get(4)?,
        start_timestamp: row.get(5)?,
        end_timestamp: row.get(6)?,
        turn_count: row.get(7)?,
        summary: row.get(8)?,
        embedding: embedding_blob.as_deref().map(bytes_to_vec),
        topic_keywords: row.get(10)?,
        status: SegmentStatus::parse_or_default(&status_str),
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn cosine_similarity_f32(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
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
#[path = "segments_tests.rs"]
mod tests;
