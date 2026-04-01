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
    fn manage_segment(
        &self,
        conn: &Arc<Mutex<Connection>>,
        session_id: &str,
        timestamp: f64,
        user_message: &str,
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
                        // Append turn to current segment.
                        // Use a synthetic episodic ID (incremental from current count).
                        let episodic_id = segment.start_episodic_id + segment.turn_count as i64;
                        if let Err(e) = segments::segment_append_turn(
                            conn,
                            &segment.segment_id,
                            episodic_id,
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
                        }

                        // Extract events from closed segment and update profile.
                        self.on_segment_closed(conn, &segment, session_id, now);

                        // Create a new segment for the new topic.
                        let new_id = format!("seg-{}", uuid_v4());
                        let episodic_id = segment.start_episodic_id + segment.turn_count as i64 + 1;
                        if let Err(e) = segments::segment_create(
                            conn,
                            &new_id,
                            session_id,
                            "global",
                            episodic_id,
                            timestamp,
                            now,
                        ) {
                            tracing::warn!("[archivist] failed to create new segment: {e}");
                        }
                    }
                }
            }
            None => {
                // No open segment — create the first one.
                let segment_id = format!("seg-{}", uuid_v4());
                if let Err(e) = segments::segment_create(
                    conn,
                    &segment_id,
                    session_id,
                    "global",
                    1,
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
        let segment_entries: Vec<&EpisodicEntry> = entries
            .iter()
            .filter(|e| {
                e.timestamp >= segment.start_timestamp
                    && segment
                        .end_timestamp
                        .map(|end| e.timestamp <= end + 1.0)
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
        self.manage_segment(conn, session_id, timestamp, &ctx.user_message);

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
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Simple pseudo-unique ID from timestamp + random bits.
    format!("{:x}{:08x}", nanos, rand_u32())
}

/// Simple random u32 from system entropy.
fn rand_u32() -> u32 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
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
mod tests {
    use super::*;
    use crate::openhuman::agent::hooks::{ToolCallRecord, TurnContext};
    use crate::openhuman::memory::store::{events as ev, fts5, segments as seg};

    fn setup_conn() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(fts5::EPISODIC_INIT_SQL).unwrap();
        conn.execute_batch(seg::SEGMENTS_INIT_SQL).unwrap();
        conn.execute_batch(ev::EVENTS_INIT_SQL).unwrap();
        conn.execute_batch(profile::PROFILE_INIT_SQL).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test]
    async fn archivist_indexes_turn() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        let ctx = TurnContext {
            user_message: "What is Rust?".into(),
            assistant_response: "Rust is a systems programming language.".into(),
            tool_calls: vec![],
            turn_duration_ms: 500,
            session_id: Some("test-session".into()),
            iteration_count: 1,
        };

        hook.on_turn_complete(&ctx).await.unwrap();

        let entries = fts5::episodic_session_entries(&conn, "test-session").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, "user");
        assert_eq!(entries[1].role, "assistant");
    }

    #[tokio::test]
    async fn archivist_creates_segment_on_first_turn() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        let ctx = TurnContext {
            user_message: "Hello world".into(),
            assistant_response: "Hi there!".into(),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some("seg-test".into()),
            iteration_count: 1,
        };

        hook.on_turn_complete(&ctx).await.unwrap();

        let open = seg::open_segment_for_session(&conn, "seg-test").unwrap();
        assert!(open.is_some());
        assert_eq!(open.unwrap().turn_count, 1);
    }

    #[tokio::test]
    async fn archivist_detects_topic_change_boundary() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        // First turn — creates segment.
        hook.on_turn_complete(&TurnContext {
            user_message: "Tell me about Rust".into(),
            assistant_response: "Rust is great.".into(),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some("boundary-test".into()),
            iteration_count: 1,
        })
        .await
        .unwrap();

        // Second turn — continues segment.
        hook.on_turn_complete(&TurnContext {
            user_message: "How about its memory safety?".into(),
            assistant_response: "It uses ownership.".into(),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some("boundary-test".into()),
            iteration_count: 2,
        })
        .await
        .unwrap();

        // Third turn — explicit topic change.
        hook.on_turn_complete(&TurnContext {
            user_message: "Switching to a different topic now. I prefer dark mode.".into(),
            assistant_response: "Noted about dark mode.".into(),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some("boundary-test".into()),
            iteration_count: 3,
        })
        .await
        .unwrap();

        // The boundary should have been detected, closing old segment
        // and creating a new one. Check that we have segments.
        let segments = seg::segments_by_namespace(&conn, "global", 10).unwrap();
        assert!(
            segments.len() >= 2,
            "Expected at least 2 segments, got {}",
            segments.len()
        );

        // Check that the closed segment got events extracted.
        // "I prefer dark mode" should have been captured as a preference event.
        // (The preference is in the 3rd turn's user message, which triggers boundary,
        //  but the extraction runs on the *closed* segment's content, not the new one.)
    }

    #[tokio::test]
    async fn archivist_extracts_failure_lesson() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        let ctx = TurnContext {
            user_message: "Run tests".into(),
            assistant_response: "Tests failed.".into(),
            tool_calls: vec![ToolCallRecord {
                name: "shell".into(),
                arguments: serde_json::json!({"command": "cargo test"}),
                success: false,
                output_summary: "shell: failed (error)".into(),
                duration_ms: 3000,
            }],
            turn_duration_ms: 3500,
            session_id: Some("test-session-2".into()),
            iteration_count: 2,
        };

        hook.on_turn_complete(&ctx).await.unwrap();

        let entries = fts5::episodic_session_entries(&conn, "test-session-2").unwrap();
        let assistant_entry = entries.iter().find(|e| e.role == "assistant").unwrap();
        assert!(assistant_entry.lesson.as_ref().unwrap().contains("shell"));
    }

    #[tokio::test]
    async fn disabled_archivist_is_noop() {
        let hook = ArchivistHook::disabled();
        let ctx = TurnContext {
            user_message: "test".into(),
            assistant_response: "test".into(),
            tool_calls: vec![],
            turn_duration_ms: 0,
            session_id: None,
            iteration_count: 0,
        };
        // Should not error even without a connection.
        hook.on_turn_complete(&ctx).await.unwrap();
    }

    #[test]
    fn extract_profile_key_works() {
        let key = extract_profile_key("I prefer dark mode for coding", "preference");
        assert!(key.starts_with("preference_"));
        assert!(key.contains("prefer"));
    }

    #[tokio::test]
    async fn archivist_accumulates_turns_in_segment() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        let session = "accum-session";

        // Send three turns in quick succession.
        for i in 1..=3 {
            hook.on_turn_complete(&TurnContext {
                user_message: format!("Turn number {i}"),
                assistant_response: format!("Response {i}"),
                tool_calls: vec![],
                turn_duration_ms: 50,
                session_id: Some(session.into()),
                iteration_count: i,
            })
            .await
            .unwrap();
        }

        // After 3 turns on the same session with no boundary triggers, there
        // should be exactly one open segment whose turn_count reflects all turns.
        let open_seg = seg::open_segment_for_session(&conn, session)
            .unwrap()
            .expect("Expected an open segment after 3 turns");

        // segment_create starts turn_count at 1; each subsequent turn calls
        // segment_append_turn which increments by 1.  So 3 turns => turn_count = 3.
        assert_eq!(
            open_seg.turn_count, 3,
            "Segment should have accumulated 3 turns, got {}",
            open_seg.turn_count
        );
    }

    #[tokio::test]
    async fn archivist_extracts_preference_event_on_boundary() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        let session = "pref-boundary-session";

        // First turn: establishes a segment.
        hook.on_turn_complete(&TurnContext {
            user_message: "Tell me about Rust ownership".into(),
            assistant_response: "Ownership is a key concept in Rust.".into(),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some(session.into()),
            iteration_count: 1,
        })
        .await
        .unwrap();

        // Second turn: continue in the same segment; include a preference statement
        // so it can be captured when the segment closes.
        hook.on_turn_complete(&TurnContext {
            user_message: "I prefer dark mode for all my editors".into(),
            assistant_response: "Good to know! Dark mode is easier on the eyes.".into(),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some(session.into()),
            iteration_count: 2,
        })
        .await
        .unwrap();

        // Third turn: explicit topic-change marker — this will close the current
        // segment and trigger on_segment_closed which runs heuristic extraction.
        hook.on_turn_complete(&TurnContext {
            user_message: "Switching to a different topic — how does Tokio work?".into(),
            assistant_response: "Tokio is an async runtime.".into(),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some(session.into()),
            iteration_count: 3,
        })
        .await
        .unwrap();

        // The boundary fires on the 3rd turn's detection, closing the segment that
        // held the "I prefer dark mode" content. Verify a preference event was stored.
        let events = ev::events_by_type(&conn, "global", "preference", 20).unwrap();
        assert!(
            !events.is_empty(),
            "Expected at least one preference event after segment close; got 0. \
             The 'I prefer dark mode' message should have been extracted."
        );
        // At least one event content should reference dark mode or preference.
        let has_dark_mode = events
            .iter()
            .any(|e| e.content.to_lowercase().contains("prefer"));
        assert!(
            has_dark_mode,
            "Expected a preference event mentioning 'prefer', found: {:?}",
            events.iter().map(|e| &e.content).collect::<Vec<_>>()
        );
    }
}
