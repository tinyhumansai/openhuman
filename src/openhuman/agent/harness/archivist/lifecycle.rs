//! Constructor methods, segment lifecycle management, and flush logic for
//! `ArchivistHook`.

use super::helpers::{extract_profile_key, uuid_v4};
use super::types::ArchivistHook;
use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::ChatProvider;
use crate::openhuman::memory_store::events::{self, EventRecord, EventType};
use crate::openhuman::memory_store::fts5::EpisodicEntry;
use crate::openhuman::memory_store::profile::{self, FacetType};
use crate::openhuman::memory_store::segments::{
    self, BoundaryConfig, BoundaryDecision, ConversationSegment,
};
use crate::openhuman::memory_tree::score::embed::{build_embedder_from_config, Embedder};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

impl ArchivistHook {
    /// Create an Archivist hook with a shared SQLite connection.
    ///
    /// LLM recap and embedding are disabled by default; call
    /// [`Self::with_config`] on the production path to wire them in.
    pub fn new(conn: Arc<Mutex<Connection>>, enabled: bool) -> Self {
        Self {
            conn: Some(conn),
            enabled,
            boundary_config: BoundaryConfig::default(),
            config: None,
            chat_provider: None,
            embedder: None,
        }
    }

    /// Attach runtime config so the archivist can gate the tree-ingest path
    /// and build its LLM chat provider + embedder from config.
    ///
    /// When `config.learning.chat_to_tree_enabled` is `true`, each closed
    /// segment's raw prose turns are ingested into the memory tree as
    /// `source_id="conversations:agent"` (one batch per segment, not per turn).
    /// The chat provider is built via `build_chat_provider(config, Summarise)`;
    /// the embedder via `build_embedder_from_config(config)`. Both are
    /// soft-fallback: if construction fails, the fields stay `None` and the
    /// archivist falls back to heuristic summary / no embedding.
    pub fn with_config(mut self, config: Config) -> Self {
        // Build the LLM chat provider for segment recap.
        let chat_provider: Option<Arc<dyn ChatProvider>> =
            match crate::openhuman::memory::chat::build_chat_provider(&config) {
                Ok(p) => {
                    tracing::debug!("[archivist] segment recap provider={} registered", p.name());
                    Some(p)
                }
                Err(e) => {
                    tracing::warn!(
                        "[archivist] failed to build chat provider for recap (will use fallback): {e}"
                    );
                    None
                }
            };

        // Build the embedder for segment recap vectors.
        let embedder: Option<Arc<dyn Embedder>> = match build_embedder_from_config(&config) {
            Ok(e) => {
                tracing::debug!("[archivist] segment embed provider={} registered", e.name());
                Some(Arc::from(e))
            }
            Err(e) => {
                tracing::warn!(
                        "[archivist] failed to build embedder for segment recap (embedding skipped): {e}"
                    );
                None
            }
        };

        self.chat_provider = chat_provider;
        self.embedder = embedder;
        self.config = Some(config);
        self
    }

    /// Create a disabled/no-op Archivist (when FTS5 is not available).
    pub fn disabled() -> Self {
        Self {
            conn: None,
            enabled: false,
            boundary_config: BoundaryConfig::default(),
            config: None,
            chat_provider: None,
            embedder: None,
        }
    }

    /// Flush the currently-open segment for `session_id`, if any, by
    /// force-closing it and running the same close path (recap + embed +
    /// event extraction). This guarantees the trailing segment of a session
    /// is always finalized even when no boundary-triggering turn arrives.
    ///
    /// Called at session end (see `Agent::spawn_session_memory_extraction`
    /// in `session/turn.rs`). Safe to call multiple times — segment_close
    /// is idempotent (only transitions `open → closed`).
    pub async fn flush_open_segment(&self, session_id: &str) {
        if !self.enabled {
            return;
        }
        let Some(conn) = &self.conn else {
            return;
        };
        let now = Self::now_timestamp();
        tracing::debug!("[archivist] flush_open_segment: checking session={session_id}");
        let open_segment = match segments::open_segment_for_session(conn, session_id) {
            Ok(seg) => seg,
            Err(e) => {
                tracing::warn!("[archivist] flush: failed to query open segment: {e}");
                return;
            }
        };
        let Some(segment) = open_segment else {
            tracing::debug!("[archivist] flush: no open segment for session={session_id}");
            return;
        };
        tracing::debug!(
            "[archivist] flush: force-closing segment={} turn_count={}",
            segment.segment_id,
            segment.turn_count
        );
        if let Err(e) = segments::segment_close(conn, &segment.segment_id, now) {
            tracing::warn!("[archivist] flush: failed to close segment: {e}");
            return;
        }
        self.on_segment_closed(conn, &segment, session_id, now)
            .await;
    }

    pub(super) fn now_timestamp() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// Handle segment lifecycle for a new turn.
    ///
    /// Returns the closed segment (if any) so the caller can run
    /// `on_segment_closed` asynchronously after this function returns.
    /// Event extraction and recap run outside this function because they
    /// are async and may re-acquire the connection lock.
    pub(super) fn manage_segment_sync(
        &self,
        conn: &Arc<Mutex<Connection>>,
        session_id: &str,
        timestamp: f64,
        user_message: &str,
        current_episodic_id: i64,
        current_seq: Option<u32>,
    ) -> Option<ConversationSegment> {
        let now = Self::now_timestamp();

        // Check for an open segment for this session.
        let open_segment = match segments::open_segment_for_session(conn, session_id) {
            Ok(seg) => seg,
            Err(e) => {
                tracing::warn!("[archivist] failed to query open segment: {e}");
                return None;
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
                        tracing::debug!(
                            "[archivist] segment={} continues (turn_count={})",
                            segment.segment_id,
                            segment.turn_count
                        );
                        if let Err(e) = segments::segment_append_turn(
                            conn,
                            &segment.segment_id,
                            current_episodic_id,
                            current_seq,
                            timestamp,
                            now,
                        ) {
                            tracing::warn!("[archivist] failed to append turn to segment: {e}");
                        }
                        None
                    }
                    BoundaryDecision::Boundary(reason) => {
                        tracing::debug!(
                            "[archivist] segment boundary detected: {reason} — closing {}",
                            segment.segment_id
                        );

                        // Close the current segment.
                        if let Err(e) = segments::segment_close(conn, &segment.segment_id, now) {
                            tracing::warn!("[archivist] failed to close segment: {e}");
                            return None;
                        }

                        // Create a new segment for the new topic.
                        // The new segment starts at the current turn's episodic ID.
                        let new_id = format!("seg-{}", uuid_v4());
                        if let Err(e) = segments::segment_create(
                            conn,
                            &new_id,
                            session_id,
                            "global",
                            current_episodic_id,
                            current_seq,
                            timestamp,
                            now,
                        ) {
                            tracing::warn!("[archivist] failed to create new segment: {e}");
                        }

                        // Return the closed segment so the caller can run
                        // on_segment_closed asynchronously.
                        Some(segment)
                    }
                }
            }
            None => {
                // No open segment — create the first one using the current episodic ID.
                let segment_id = format!("seg-{}", uuid_v4());
                tracing::debug!(
                    "[archivist] creating first segment={segment_id} for session={session_id}"
                );
                if let Err(e) = segments::segment_create(
                    conn,
                    &segment_id,
                    session_id,
                    "global",
                    current_episodic_id,
                    current_seq,
                    timestamp,
                    now,
                ) {
                    tracing::warn!("[archivist] failed to create initial segment: {e}");
                }
                None
            }
        }
    }

    /// Called when a segment is closed.
    ///
    /// Produces a segment recap (LLM if a chat provider is configured,
    /// otherwise the heuristic fallback), embeds the recap, extracts
    /// heuristic events, and updates the user profile.
    ///
    /// Soft-fallback contract (mirrors `LlmSummariser`): this function
    /// never returns `Err`; all failures are logged and ignored.
    pub(super) async fn on_segment_closed(
        &self,
        conn: &Arc<Mutex<Connection>>,
        segment: &ConversationSegment,
        session_id: &str,
        now: f64,
    ) {
        // Gather the conversation text for this segment. Prefer the
        // md-backed memory_archivist read when config is available; fall
        // back to FTS5 in test paths or when config isn't wired.
        let entries = self.read_session_entries(conn, session_id);

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
            tracing::debug!(
                "[archivist] segment={} has no entries — skipping recap",
                segment.segment_id
            );
            return;
        }

        // Build segment text from user messages (for event extraction).
        let segment_text: String = segment_entries
            .iter()
            .filter(|e| e.role == "user")
            .map(|e| e.content.as_str())
            .collect::<Vec<_>>()
            .join(". ");

        // ── Segment recap (LLM or heuristic fallback) ────────────────────
        let (summary, _from_llm) = self
            .summarize_entries(&segment_entries, &segment.segment_id, segment.turn_count)
            .await;

        // Persist the recap.
        if let Err(e) = segments::segment_set_summary(conn, &segment.segment_id, &summary, now) {
            tracing::warn!("[archivist] failed to set segment summary: {e}");
        } else {
            tracing::debug!(
                "[archivist] recap persisted segment={} summary_chars={}",
                segment.segment_id,
                summary.len()
            );
        }

        // ── Finalize-time embedding ───────────────────────────────────────
        self.embed_segment_recap(conn, &segment.segment_id, &summary, now)
            .await;

        // ── Heuristic event extraction ────────────────────────────────────
        if !segment_text.is_empty() {
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

        // ── Phase 2: tree ingest at segment granularity ───────────────────
        // Gate: only when config is attached and chat_to_tree_enabled is true.
        // Ingest the segment's raw prose turns (NOT the LLM recap) as one
        // ChatBatch into the memory tree under `source_id="conversations:agent"`.
        // Evidence-vs-interpretation: the tree must ingest raw prose and build
        // its own summaries; feeding the recap would make the tree summarise
        // a summary. Non-fatal: failures are logged and swallowed.
        if let Some(ref cfg) = self.config {
            if cfg.learning.chat_to_tree_enabled {
                tracing::debug!(
                    "[archivist] piping segment into tree as conversations:agent \
                     session={session_id} segment={} entries={}",
                    segment.segment_id,
                    segment_entries.len()
                );
                self.pipe_segment_to_tree(cfg, segment, session_id, &segment_entries)
                    .await;
            }
        }

        // ── Long-term goals enrichment (best-effort, background) ───────────
        // When context is summarized we kick the turn-based `goals_agent` so
        // the user's durable goals list stays fresh. Feed it the fresh recap
        // as context. Detached + non-fatal: never blocks segment close.
        if let Some(ref cfg) = self.config {
            if cfg.learning.goals_enrichment_enabled && !summary.trim().is_empty() {
                tracing::debug!(
                    "[memory_goals] segment closed — spawning goals enrichment \
                     session={session_id} segment={}",
                    segment.segment_id
                );
                let context = format!(
                    "Recent conversation recap (segment {}):\n\n{}",
                    segment.segment_id, summary
                );
                crate::openhuman::memory_goals::spawn_enrich_goals(
                    cfg.clone(),
                    cfg.workspace_dir.clone(),
                    context,
                );
            }
        }
    }

    /// Embed `summary` for `segment_id` and write the per-model embedding row.
    ///
    /// Embed the recap only when the segment is being finalized (closed).
    /// Never embed per-turn or on an open segment — this is the single
    /// write point for `segment_embeddings` rows.
    ///
    /// Skip when the recap is empty/whitespace — `summarize_entries` can
    /// return "" (LLM error fallback + the user-turn filter above yielding
    /// zero entries) and an empty embed input is guaranteed to 400 from
    /// the upstream embedding API (#13021). The segment is sealed without
    /// an embedding row; subsequent recap edits can re-embed.
    pub(super) async fn embed_segment_recap(
        &self,
        conn: &Arc<Mutex<Connection>>,
        segment_id: &str,
        summary: &str,
        now: f64,
    ) {
        if summary.trim().is_empty() {
            tracing::warn!(
                "[archivist] skipping embedding: recap is empty/whitespace segment={segment_id}"
            );
            return;
        }
        let Some(ref embedder) = self.embedder else {
            tracing::debug!(
                "[archivist] no embedder — skipping segment embedding segment={segment_id}"
            );
            return;
        };
        let model_signature = embedder.name().to_string();
        tracing::debug!("[archivist] embedding recap segment={segment_id} model={model_signature}");
        match embedder.embed(summary).await {
            Ok(vec) => {
                match segments::segment_embedding_upsert(
                    conn,
                    segment_id,
                    &model_signature,
                    &vec,
                    now,
                ) {
                    Ok(()) => {
                        tracing::debug!(
                            "[archivist] embedding stored segment={segment_id} model={model_signature} dim={}",
                            vec.len()
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[archivist] failed to persist segment embedding (non-fatal) segment={segment_id}: {e}"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "[archivist] embed call failed (non-fatal) segment={segment_id} model={model_signature}: {e}"
                );
            }
        }
    }
}
