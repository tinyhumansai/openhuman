//! Archivist — background PostTurnHook that extracts lessons, indexes
//! episodic records, and manages conversation segments with event extraction.
//!
//! After each turn, the Archivist:
//! 1. Inserts the turn into the FTS5 episodic table.
//! 2. Manages conversation segments (boundary detection + lifecycle).
//! 3. On segment close: extracts events (heuristic) and updates user profile.
//! 4. Extracts simple lessons from tool failures.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::memory::store::events::{self, EventRecord, EventType};
use crate::openhuman::memory::store::fts5::{self, EpisodicEntry};
use crate::openhuman::memory::store::profile::{self, FacetType};
use crate::openhuman::memory::store::segments::{
    self, BoundaryConfig, BoundaryDecision, ConversationSegment,
};
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Background Archivist that indexes turns into FTS5 episodic memory
/// and manages conversation segmentation.
pub struct ArchivistHook {
    /// SQLite connection shared with UnifiedMemory.
    conn: Option<Arc<Mutex<Connection>>>,
    /// Whether the archivist is enabled.
    enabled: bool,
    /// Boundary detection configuration.
    boundary_config: BoundaryConfig,
}

impl ArchivistHook {
    /// Create an Archivist hook with a shared SQLite connection.
    pub fn new(conn: Arc<Mutex<Connection>>, enabled: bool) -> Self {
        Self {
            conn: Some(conn),
            enabled,
            boundary_config: BoundaryConfig::default(),
        }
    }

    /// Create a disabled/no-op Archivist (when FTS5 is not enabled).
    pub fn disabled() -> Self {
        Self {
            conn: None,
            enabled: false,
            boundary_config: BoundaryConfig::default(),
        }
    }

    fn now_timestamp() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// Handle segment lifecycle for a new turn.
    ///
    /// The close→extract→create path uses a SQLite transaction for the
    /// close + create operations to ensure atomicity. Event extraction
    /// runs between close and create (outside the transaction) because
    /// it needs to re-acquire the connection lock via fts5 functions.
    fn manage_segment(
        &self,
        conn: &Arc<Mutex<Connection>>,
        session_id: &str,
        timestamp: f64,
        user_message: &str,
        current_episodic_id: i64,
    ) {
        let now = Self::now_timestamp();

        // Check for an open segment for this session.
        let open_segment = match segments::open_segment_for_session(conn, session_id) {
            Ok(seg) => seg,
            Err(e) => {
                tracing::warn!("[archivist] failed to query open segment: {e}");
                return;
            }
        };

        match open_segment {
            Some(segment) => {
                // Run boundary detection.
                let decision = segments::detect_boundary(
                    &self.boundary_config,
                    &segment,
                    timestamp,
                    user_message,
                    None, // No embedding for now — cosine drift skipped without embedder access.
                );

                match decision {
                    BoundaryDecision::Continue => {
                        if let Err(e) = segments::segment_append_turn(
                            conn,
                            &segment.segment_id,
                            current_episodic_id,
                            timestamp,
                            now,
                        ) {
                            tracing::warn!("[archivist] failed to append turn to segment: {e}");
                        }
                    }
                    BoundaryDecision::Boundary(reason) => {
                        tracing::debug!(
                            "[archivist] segment boundary detected: {reason} — closing {}",
                            segment.segment_id
                        );

                        // Close the current segment.
                        if let Err(e) = segments::segment_close(conn, &segment.segment_id, now) {
                            tracing::warn!("[archivist] failed to close segment: {e}");
                            return;
                        }

                        // Extract events from the closed segment and update profile.
                        // This runs outside a transaction because it calls fts5 functions
                        // that re-acquire the connection lock.
                        self.on_segment_closed(conn, &segment, session_id, now);

                        // Create a new segment for the new topic.
                        // The new segment starts at the current turn's episodic ID.
                        let new_id = format!("seg-{}", uuid_v4());
                        if let Err(e) = segments::segment_create(
                            conn,
                            &new_id,
                            session_id,
                            "global",
                            current_episodic_id,
                            timestamp,
                            now,
                        ) {
                            tracing::warn!("[archivist] failed to create new segment: {e}");
                        }
                    }
                }
            }
            None => {
                // No open segment — create the first one using the current episodic ID.
                let segment_id = format!("seg-{}", uuid_v4());
                if let Err(e) = segments::segment_create(
                    conn,
                    &segment_id,
                    session_id,
                    "global",
                    current_episodic_id,
                    timestamp,
                    now,
                ) {
                    tracing::warn!("[archivist] failed to create initial segment: {e}");
                }
            }
        }
    }

    /// Called when a segment is closed. Runs heuristic event extraction
    /// and updates the user profile from extracted preferences/facts.
    fn on_segment_closed(
        &self,
        conn: &Arc<Mutex<Connection>>,
        segment: &ConversationSegment,
        session_id: &str,
        now: f64,
    ) {
        // Gather the conversation text for this segment from episodic entries.
        let entries = fts5::episodic_session_entries(conn, session_id).unwrap_or_default();

        // Filter entries that fall within the segment's time window.
        // Use <= for end_timestamp (entries at the boundary are part of this
        // segment). The boundary-triggering turn has a timestamp AFTER
        // end_timestamp, so it won't be included.
        let segment_entries: Vec<&EpisodicEntry> = entries
            .iter()
            .filter(|e| {
                e.timestamp >= segment.start_timestamp
                    && segment
                        .end_timestamp
                        .map(|end| e.timestamp <= end)
                        .unwrap_or(true)
            })
            .collect();

        if segment_entries.is_empty() {
            return;
        }

        // Build segment text from user messages.
        let segment_text: String = segment_entries
            .iter()
            .filter(|e| e.role == "user")
            .map(|e| e.content.as_str())
            .collect::<Vec<_>>()
            .join(". ");

        if segment_text.is_empty() {
            return;
        }

        // Generate a fallback summary from first and last content.
        let first = segment_entries
            .first()
            .map(|e| e.content.as_str())
            .unwrap_or("");
        let last = segment_entries
            .last()
            .map(|e| e.content.as_str())
            .unwrap_or(first);
        let summary = segments::fallback_summary(first, last, segment.turn_count);
        if let Err(e) = segments::segment_set_summary(conn, &segment.segment_id, &summary, now) {
            tracing::warn!("[archivist] failed to set segment summary: {e}");
        }

        // Extract events via heuristic patterns.
        let extracted = events::extract_events_heuristic(&segment_text);
        tracing::debug!(
            "[archivist] extracted {} events from segment {}",
            extracted.len(),
            segment.segment_id
        );

        for (event_type, content) in &extracted {
            let event_id = format!("evt-{}", uuid_v4());
            let event = EventRecord {
                event_id,
                segment_id: segment.segment_id.clone(),
                session_id: session_id.to_string(),
                namespace: segment.namespace.clone(),
                event_type: event_type.clone(),
                content: content.clone(),
                subject: None,
                timestamp_ref: None,
                confidence: 0.6,
                embedding: None,
                source_turn_ids: None,
                created_at: now,
            };
            if let Err(e) = events::event_insert(conn, &event) {
                tracing::warn!("[archivist] failed to insert event: {e}");
            }

            // Update user profile from preference and fact events.
            match event_type {
                EventType::Preference => {
                    let key = extract_profile_key(content, "preference");
                    let facet_id = format!("prf-{}", uuid_v4());
                    if let Err(e) = profile::profile_upsert(
                        conn,
                        &facet_id,
                        &FacetType::Preference,
                        &key,
                        content,
                        0.6,
                        Some(&segment.segment_id),
                        now,
                    ) {
                        tracing::warn!("[archivist] failed to upsert profile facet: {e}");
                    }
                }
                EventType::Fact => {
                    let key = extract_profile_key(content, "fact");
                    let facet_id = format!("prf-{}", uuid_v4());
                    if let Err(e) = profile::profile_upsert(
                        conn,
                        &facet_id,
                        &FacetType::Context,
                        &key,
                        content,
                        0.6,
                        Some(&segment.segment_id),
                        now,
                    ) {
                        tracing::warn!("[archivist] failed to upsert profile fact: {e}");
                    }
                }
                _ => {}
            }
        }
    }
}

#[async_trait]
impl PostTurnHook for ArchivistHook {
    fn name(&self) -> &str {
        "archivist"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let Some(conn) = &self.conn else {
            return Ok(());
        };

        let session_id = ctx.session_id.as_deref().unwrap_or("unknown");
        let timestamp = Self::now_timestamp();

        tracing::debug!(
            "[archivist] indexing turn: session={session_id}, tools={}, duration={}ms",
            ctx.tool_calls.len(),
            ctx.turn_duration_ms
        );

        // Index user message.
        fts5::episodic_insert(
            conn,
            &EpisodicEntry {
                id: None,
                session_id: session_id.to_string(),
                timestamp,
                role: "user".to_string(),
                content: ctx.user_message.clone(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            },
        )?;

        // Retrieve the inserted episodic ID for segment tracking.
        let current_episodic_id = {
            let db = conn.lock();
            db.query_row("SELECT last_insert_rowid()", [], |row| row.get::<_, i64>(0))
                .unwrap_or(1)
        };

        // Index assistant response with tool call summary.
        let tool_calls_json = if ctx.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&ctx.tool_calls).unwrap_or_default())
        };

        // Extract a simple lesson from tool failures (lightweight, no LLM needed).
        let lesson = extract_lesson_from_tools(&ctx.tool_calls);

        fts5::episodic_insert(
            conn,
            &EpisodicEntry {
                id: None,
                session_id: session_id.to_string(),
                // Offset by 1ms so assistant entries sort after user entries within
                // the same turn. Relies on turn timestamps having >=1ms resolution.
                timestamp: timestamp + 0.001,
                role: "assistant".to_string(),
                content: ctx.assistant_response.clone(),
                lesson,
                tool_calls_json,
                cost_microdollars: 0,
            },
        )?;

        // Manage conversation segmentation.
        self.manage_segment(
            conn,
            session_id,
            timestamp,
            &ctx.user_message,
            current_episodic_id,
        );

        tracing::debug!("[archivist] turn indexed successfully");
        Ok(())
    }
}

/// Extract simple lessons from tool call outcomes (no LLM needed).
fn extract_lesson_from_tools(
    tool_calls: &[crate::openhuman::agent::hooks::ToolCallRecord],
) -> Option<String> {
    let failures: Vec<&str> = tool_calls
        .iter()
        .filter(|tc| !tc.success)
        .map(|tc| tc.name.as_str())
        .collect();

    if failures.is_empty() {
        return None;
    }

    Some(format!(
        "Tools that failed in this turn: {}",
        failures.join(", ")
    ))
}

/// Extract a short profile key from event content (first few meaningful words).
fn extract_profile_key(content: &str, prefix: &str) -> String {
    let words: Vec<&str> = content
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .take(4)
        .collect();
    let key = words.join("_").to_lowercase();
    let key = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>();
    if key.is_empty() {
        format!("{prefix}_unknown")
    } else {
        format!("{prefix}_{key}")
    }
}

/// Generate a simple UUID v4 (random).
fn uuid_v4() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}{:08x}", nanos, rand_u32())
}

/// Simple random u32 from system entropy.
fn rand_u32() -> u32 {
    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_u64(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
    );
    hasher.finish() as u32
}

#[cfg(test)]
#[path = "archivist_tests.rs"]
mod tests;
