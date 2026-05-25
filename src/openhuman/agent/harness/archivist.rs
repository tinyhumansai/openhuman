//! Archivist — background PostTurnHook that extracts lessons, indexes
//! episodic records, and manages conversation segments with event extraction.
//!
//! After each turn, the Archivist:
//! 1. Inserts the turn into the FTS5 episodic table.
//! 2. Manages conversation segments (boundary detection + lifecycle).
//! 3. On segment close: produces an LLM recap (soft-fallback to heuristic),
//!    embeds the recap, extracts events, and updates user profile.
//! 4. Extracts simple lessons from tool failures.
//! 5. (Phase 2 / #566) At segment close/flush, ingests the segment's raw prose
//!    turns (user + assistant; tool-call JSON stripped) into the memory tree as
//!    `source_id = "conversations:agent"` when
//!    `config.learning.chat_to_tree_enabled` is true. The leaf is RAW PROSE —
//!    the LLM recap is NEVER fed into the tree (evidence-vs-interpretation
//!    policy). Each leaf carries episodic provenance stamped in `source_ref`.
//! 6. `flush_open_segment` force-closes the trailing open segment at session
//!    end so the last segment always gets a recap + embedding + tree ingest.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::ChatProvider;
use crate::openhuman::memory::ingest_pipeline;
use crate::openhuman::memory_store::events::{self, EventRecord, EventType};
use crate::openhuman::memory_store::fts5::{self, EpisodicEntry};
use crate::openhuman::memory_store::profile::{self, FacetType};
use crate::openhuman::memory_store::segments::{
    self, BoundaryConfig, BoundaryDecision, ConversationSegment,
};
use crate::openhuman::memory_store::trees::types::TreeKind;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::memory_tree::score::embed::{build_embedder_from_config, Embedder};
use crate::openhuman::memory_tree::summarise::{summarise, SummaryContext, SummaryInput};
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Background Archivist that indexes turns into FTS5 episodic memory
/// and manages conversation segmentation.
///
/// Produces an LLM recap + embedding for each closed segment and flushes
/// the trailing open segment at session end.
pub struct ArchivistHook {
    /// SQLite connection shared with UnifiedMemory.
    conn: Option<Arc<Mutex<Connection>>>,
    /// Whether the archivist is enabled.
    enabled: bool,
    /// Boundary detection configuration.
    boundary_config: BoundaryConfig,
    /// Optional runtime config — used to gate the tree-ingest path and to
    /// build the LLM chat provider + embedder.
    ///
    /// When `None`, the tree-ingest path is skipped. Set via
    /// [`ArchivistHook::with_config`] on the production path.
    config: Option<Config>,
    /// Optional LLM provider for segment recap. When `None`, the
    /// fallback heuristic summary is used instead.
    chat_provider: Option<Arc<dyn ChatProvider>>,
    /// Optional embedder for segment recap vectors. When `None`, embedding
    /// is skipped (segment is still summarised).
    embedder: Option<Arc<dyn Embedder>>,
}

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

    fn now_timestamp() -> f64 {
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
    fn manage_segment_sync(
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
    async fn on_segment_closed(
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
        // Embed the recap only when the segment is being finalized (closed).
        // Never embed per-turn or on an open segment — this is the single
        // write point for segment_embeddings rows.
        if let Some(ref embedder) = self.embedder {
            let model_signature = embedder.name().to_string();
            tracing::debug!(
                "[archivist] embedding recap segment={} model={}",
                segment.segment_id,
                model_signature
            );
            match embedder.embed(&summary).await {
                Ok(vec) => {
                    match segments::segment_embedding_upsert(
                        conn,
                        &segment.segment_id,
                        &model_signature,
                        &vec,
                        now,
                    ) {
                        Ok(()) => {
                            tracing::debug!(
                                "[archivist] embedding stored segment={} model={} dim={}",
                                segment.segment_id,
                                model_signature,
                                vec.len()
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[archivist] failed to persist segment embedding (non-fatal) segment={}: {e}",
                                segment.segment_id
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[archivist] embed call failed (non-fatal) segment={} model={}: {e}",
                        segment.segment_id,
                        model_signature
                    );
                }
            }
        } else {
            tracing::debug!(
                "[archivist] no embedder — skipping segment embedding segment={}",
                segment.segment_id
            );
        }

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

        tracing::debug!("[archivist] episodic rows written: session={session_id}");

        // Dual-write into memory_archivist::store (md-backed) so we can
        // validate the FTS5 → md migration before flipping the read side.
        // Best-effort: a write failure here must not break the turn. The
        // user turn's assigned seq is captured into `current_seq` so the
        // segment ops can store it alongside the FTS5 episodic id.
        let mut current_seq: Option<u32> = None;
        if let Some(cfg) = self.config.as_ref() {
            let ts_ms = (timestamp * 1000.0) as i64;
            let user_turn = crate::openhuman::memory_archivist::ArchivedTurn {
                session_id: session_id.to_string(),
                seq: 0, // assigned by record_turn
                timestamp_ms: ts_ms,
                role: "user".to_string(),
                content: ctx.user_message.clone(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            };
            match crate::openhuman::memory_archivist::store::record_turn(cfg, user_turn) {
                Ok(stored) => current_seq = Some(stored.seq),
                Err(e) => {
                    tracing::warn!("[archivist] memory_archivist user dual-write failed: {e}");
                }
            }
            // Assistant turn carries the tool_calls_json + lesson the FTS5
            // insert just wrote. Re-derive locally so we don't depend on
            // FTS5 having returned.
            let assistant_lesson = extract_lesson_from_tools(&ctx.tool_calls);
            let assistant_tool_calls = if ctx.tool_calls.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&ctx.tool_calls).unwrap_or_default())
            };
            let assistant_turn = crate::openhuman::memory_archivist::ArchivedTurn {
                session_id: session_id.to_string(),
                seq: 0,
                timestamp_ms: ts_ms + 1,
                role: "assistant".to_string(),
                content: ctx.assistant_response.clone(),
                lesson: assistant_lesson,
                tool_calls_json: assistant_tool_calls,
                cost_microdollars: 0,
            };
            if let Err(e) =
                crate::openhuman::memory_archivist::store::record_turn(cfg, assistant_turn)
            {
                tracing::warn!("[archivist] memory_archivist assistant dual-write failed: {e}");
            }
        }

        // Manage conversation segmentation (sync boundary detection + SQLite
        // operations). Returns the just-closed segment when a boundary fired.
        let closed_segment = self.manage_segment_sync(
            conn,
            session_id,
            timestamp,
            &ctx.user_message,
            current_episodic_id,
            current_seq,
        );

        // Run async recap + embed + segment-tree ingest on the closed segment
        // (if any). Per-turn tree ingest is intentionally absent — Phase 2
        // moves the tree write to segment granularity inside on_segment_closed.
        if let Some(ref segment) = closed_segment {
            let now = Self::now_timestamp();
            self.on_segment_closed(conn, segment, session_id, now).await;
        }

        tracing::debug!("[archivist] turn indexed successfully: session={session_id}");
        Ok(())
    }
}

impl ArchivistHook {
    /// Read every entry recorded for `session_id`, preferring the
    /// md-backed `memory_archivist::store` when `self.config` is set and
    /// falling back to the legacy FTS5 episodic table otherwise.
    ///
    /// Returns `EpisodicEntry` so the existing call sites (segment
    /// gathering, recap rendering, tree push) keep their shape unchanged
    /// during the FTS5 retirement migration.
    fn read_session_entries(
        &self,
        conn: &Arc<Mutex<Connection>>,
        session_id: &str,
    ) -> Vec<EpisodicEntry> {
        if let Some(cfg) = self.config.as_ref() {
            match crate::openhuman::memory_archivist::store::session_entries(cfg, session_id) {
                Ok(turns) => {
                    return turns
                        .into_iter()
                        .map(|t| EpisodicEntry {
                            id: None,
                            session_id: t.session_id,
                            // ArchivedTurn stores epoch-ms; EpisodicEntry
                            // takes epoch-seconds as f64.
                            timestamp: (t.timestamp_ms as f64) / 1000.0,
                            role: t.role,
                            content: t.content,
                            lesson: t.lesson,
                            tool_calls_json: t.tool_calls_json,
                            cost_microdollars: t.cost_microdollars,
                        })
                        .collect();
                }
                Err(e) => {
                    tracing::warn!(
                        "[archivist] memory_archivist read failed (falling back to FTS5): {e}"
                    );
                }
            }
        }
        fts5::episodic_session_entries(conn, session_id).unwrap_or_default()
    }

    /// Shared summarize helper — the **single LLM summarizer** used by both
    /// the finalize path (`on_segment_closed`) and the rolling-recap path
    /// (`rolling_segment_recap`).
    ///
    /// Builds a prose corpus from `entries`, calls the `LlmSummariser` when a
    /// `chat_provider` is configured, and falls back to the heuristic
    /// `segments::fallback_summary` on any failure or when no provider is
    /// wired in. Always returns a non-empty string.
    ///
    /// Invariants:
    /// - NEVER mutates DB state (no `segment_set_summary`, no embedding).
    /// - NEVER closes a segment.
    /// - Safe to call on both open and closed segments.
    /// Summarize a set of episodic entries into a recap string.
    ///
    /// Returns `(text, produced_by_llm)`. `produced_by_llm == false` means the
    /// LLM was unavailable / failed / returned empty and `text` is the shallow
    /// heuristic `fallback_summary` bookend stub. That stub is an acceptable
    /// durable last-resort on the *finalize* path, but callers driving the
    /// **live prompt** (rolling recap → compaction) must treat
    /// `produced_by_llm == false` as "no real recap" and fall back to their
    /// own strategy — the stub must never become live compaction text.
    async fn summarize_entries(
        &self,
        entries: &[&EpisodicEntry],
        segment_id: &str,
        turn_count: i32,
    ) -> (String, bool) {
        if entries.is_empty() {
            tracing::debug!(
                "[archivist] summarize_entries: no entries for segment={segment_id} — \
                 returning empty fallback"
            );
            return (segments::fallback_summary("", "", turn_count), false);
        }

        // Build a full prose corpus from ALL entries (user + assistant prose;
        // tool-call JSON is already excluded because the archivist stores
        // stripped prose in the `content` column).
        let corpus_inputs: Vec<SummaryInput> = entries
            .iter()
            .filter(|e| !e.content.trim().is_empty())
            .map(|e| {
                use crate::openhuman::memory_store::chunks::types::approx_token_count;
                let content = e.content.clone();
                let token_count = approx_token_count(&content);
                let ts = chrono::DateTime::from_timestamp(e.timestamp as i64, 0)
                    .unwrap_or_else(chrono::Utc::now);
                SummaryInput {
                    id: format!("{}-{}", e.role, e.timestamp as u64),
                    content,
                    token_count,
                    entities: Vec::new(),
                    topics: Vec::new(),
                    time_range_start: ts,
                    time_range_end: ts,
                    score: 0.5,
                }
            })
            .collect();

        let summary_ctx = SummaryContext {
            tree_id: segment_id,
            tree_kind: TreeKind::Source,
            target_level: 0,
            token_budget: 2_000,
        };

        let first = entries.first().map(|e| e.content.as_str()).unwrap_or("");
        let last = entries.last().map(|e| e.content.as_str()).unwrap_or(first);

        if self.chat_provider.is_some() {
            if let Some(ref config) = self.config {
                tracing::debug!(
                    "[archivist] summarize_entries: LLM recap segment={segment_id} entries={}",
                    entries.len()
                );
                #[cfg(test)]
                let summary_result = if let Some(provider) = self.chat_provider.as_ref() {
                    crate::openhuman::memory::chat::test_override::with_provider(
                        Arc::clone(provider),
                        summarise(config, &corpus_inputs, &summary_ctx),
                    )
                    .await
                } else {
                    summarise(config, &corpus_inputs, &summary_ctx).await
                };
                #[cfg(not(test))]
                let summary_result = summarise(config, &corpus_inputs, &summary_ctx).await;

                match summary_result {
                    Ok(output) if !output.content.is_empty() => {
                        tracing::debug!(
                            "[archivist] summarize_entries: LLM recap ok segment={segment_id} \
                             chars={}",
                            output.content.len()
                        );
                        return (output.content, true);
                    }
                    Ok(_) => {
                        tracing::debug!(
                            "[archivist] summarize_entries: LLM returned empty — \
                             heuristic fallback segment={segment_id}"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[archivist] summarize_entries: LLM recap failed (non-fatal) \
                             segment={segment_id}: {e} — heuristic fallback"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    "[archivist] summarize_entries: no config — \
                     heuristic fallback segment={segment_id}"
                );
            }
        } else {
            tracing::debug!(
                "[archivist] summarize_entries: no chat provider — \
                 heuristic fallback segment={segment_id}"
            );
        }
        (segments::fallback_summary(first, last, turn_count), false)
    }

    /// Produce a rolling recap of the **currently-open** segment for
    /// `session_id` WITHOUT closing it, writing `segment_set_summary`, or
    /// embedding.
    ///
    /// This is the Phase 1.5 "one summarizer" entry point. Both
    /// `on_segment_closed` (finalize) and this function delegate to the same
    /// [`Self::summarize_entries`] helper so the same LLM path is used in both
    /// cases. The distinction is purely in what happens *after* the summary
    /// string is produced:
    ///
    /// - **Finalize** (`on_segment_closed`): persists the summary via
    ///   `segment_set_summary`, embeds it, extracts events, pipes tree ingest.
    /// - **Rolling** (this function): returns the summary string and does
    ///   nothing else — segment stays open, DB is untouched.
    ///
    /// Returns `None` when:
    /// - The archivist is disabled or has no connection.
    /// - There is no open segment for `session_id`.
    /// - The open segment has no episodic entries.
    /// - No real LLM recap was produced (LLM unavailable / failed / empty, so
    ///   only the heuristic bookend stub is available). The shallow stub is
    ///   deliberately NOT used as live compaction text.
    ///
    /// Callers must treat `None` as "recap unavailable" and fall back to
    /// their own compaction strategy (e.g. `ProviderSummarizer`).
    pub async fn rolling_segment_recap(&self, session_id: &str) -> Option<String> {
        if !self.enabled {
            tracing::debug!(
                "[archivist] rolling_segment_recap: archivist disabled \
                 session={session_id} — returning None"
            );
            return None;
        }
        let conn = self.conn.as_ref()?;

        // Find the currently-open segment for this session.
        let open_segment = match segments::open_segment_for_session(conn, session_id) {
            Ok(Some(seg)) => seg,
            Ok(None) => {
                tracing::debug!(
                    "[archivist] rolling_segment_recap: no open segment for \
                     session={session_id} — returning None"
                );
                return None;
            }
            Err(e) => {
                tracing::warn!(
                    "[archivist] rolling_segment_recap: failed to query open segment \
                     session={session_id}: {e} — returning None"
                );
                return None;
            }
        };

        // Gather the episodic entries for this session so far.
        let all_entries = self.read_session_entries(conn, session_id);

        // Keep only entries within the open segment's time window (start →
        // now, inclusive). An open segment has `end_timestamp = None`.
        let segment_entries: Vec<&EpisodicEntry> = all_entries
            .iter()
            .filter(|e| e.timestamp >= open_segment.start_timestamp)
            .collect();

        if segment_entries.is_empty() {
            tracing::debug!(
                "[archivist] rolling_segment_recap: no entries in open segment={} \
                 session={session_id} — returning None",
                open_segment.segment_id
            );
            return None;
        }

        tracing::debug!(
            "[archivist] rolling_segment_recap: summarizing open segment={} \
             entries={} session={session_id}",
            open_segment.segment_id,
            segment_entries.len()
        );

        let (recap, from_llm) = self
            .summarize_entries(
                &segment_entries,
                &open_segment.segment_id,
                open_segment.turn_count,
            )
            .await;

        if !from_llm {
            tracing::debug!(
                "[archivist] rolling_segment_recap: only heuristic bookend stub \
                 available (no real LLM recap) session={session_id} segment={} — \
                 returning None so compaction falls back to ProviderSummarizer",
                open_segment.segment_id
            );
            return None;
        }

        if recap.is_empty() {
            tracing::debug!(
                "[archivist] rolling_segment_recap: summarize_entries returned empty \
                 session={session_id} segment={} — returning None",
                open_segment.segment_id
            );
            return None;
        }

        tracing::debug!(
            "[archivist] rolling_segment_recap: produced LLM recap chars={} \
             session={session_id} segment={}",
            recap.len(),
            open_segment.segment_id
        );
        Some(recap)
    }

    /// Pipe a closed segment's raw prose turns into the memory tree as
    /// `source_id="conversations:agent"`.
    ///
    /// **Design contract (Phase 2):**
    /// - ONE ingest per segment (not per turn) — the batch boundary is the
    ///   segment, so all turns land as a single ChatBatch.
    /// - RAW PROSE only — the LLM recap (summary) is explicitly NOT ingested.
    ///   The tree must build its own summaries from evidence (raw turns);
    ///   feeding a summary-of-a-summary violates the evidence-vs-interpretation
    ///   policy.
    /// - `source_id = "conversations:agent"` is a CONSTANT — a single shared
    ///   tree source for all agent chat sessions (never per-session or per-segment).
    /// - Tool-call JSON is stripped from assistant entries so structured
    ///   payloads do not reach the tree (memory ingestion policy).
    /// - Provenance is stamped on each `ChatMessage.source_ref` as
    ///   `agent://session/{session_id}/segment/{segment_id}#ep{start}-{end}`
    ///   so tree leaves can be traced back to episodic rows for drill-down and
    ///   deduplication.
    ///
    /// Failures are logged and swallowed; the episodic write is the source of
    /// truth.
    async fn pipe_segment_to_tree(
        &self,
        config: &Config,
        segment: &crate::openhuman::memory_store::segments::ConversationSegment,
        session_id: &str,
        entries: &[&fts5::EpisodicEntry],
    ) {
        use chrono::{TimeZone, Utc};

        // Collect the episodic id span for provenance stamping.
        // start_episodic_id comes from the segment record (set at creation);
        // end_episodic_id is the latest turn id (may be None if only one turn).
        let start_ep = segment.start_episodic_id;
        let end_ep = segment.end_episodic_id.unwrap_or(start_ep);
        let segment_id = &segment.segment_id;

        // The provenance URI embeds session + segment + episodic id span so
        // tree leaves can be traced back to episodic_log rows without a
        // full-text scan.
        let provenance =
            format!("agent://session/{session_id}/segment/{segment_id}#ep{start_ep}-{end_ep}");

        // Build one ChatMessage per episodic entry (user + assistant; skip
        // empties). Tool-call JSON is stripped from assistant content so only
        // prose flows into the tree.
        let messages: Vec<ChatMessage> = entries
            .iter()
            .filter_map(|e| {
                let raw_text = if e.role == "assistant" {
                    strip_tool_calls_from_response(&e.content)
                } else {
                    e.content.clone()
                };
                let text = raw_text.trim().to_string();
                if text.is_empty() {
                    return None;
                }

                // Convert the f64 Unix timestamp to DateTime<Utc>.
                let secs = e.timestamp as i64;
                let nanos = ((e.timestamp.fract()) * 1e9) as u32;
                let ts = Utc
                    .timestamp_opt(secs, nanos.min(999_999_999))
                    .single()
                    .unwrap_or_else(Utc::now);

                Some(ChatMessage {
                    author: e.role.clone(),
                    timestamp: ts,
                    text,
                    source_ref: Some(provenance.clone()),
                })
            })
            .collect();

        if messages.is_empty() {
            tracing::debug!(
                "[archivist] pipe_segment_to_tree: no prose messages in segment={segment_id} — skipping"
            );
            return;
        }

        let batch = ChatBatch {
            platform: "agent".into(),
            // channel_label carries session_id for human-readable context.
            channel_label: session_id.to_string(),
            messages,
        };

        // `source_id` is intentionally a CONSTANT — all agent sessions share
        // one tree source so cross-session summarisation sees the full history.
        let source_id = "conversations:agent";
        // `owner` scopes the memory to the session; `tags` enable filtering.
        let owner = session_id;
        let tags = vec!["agent_chat".to_string()];

        tracing::debug!(
            "[archivist] tree ingest start: source_id={source_id} session={session_id} \
             segment={segment_id} ep_span={start_ep}-{end_ep} provenance={provenance}"
        );

        match ingest_pipeline::ingest_chat(config, source_id, owner, tags, batch).await {
            Ok(result) => {
                tracing::debug!(
                    "[archivist] tree ingest ok: source_id={source_id} \
                     session={session_id} segment={segment_id} \
                     chunks_written={} provenance={provenance}",
                    result.chunks_written
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[archivist] tree ingest failed (non-fatal): source_id={source_id} \
                     session={session_id} segment={segment_id} error={e}"
                );
            }
        }
    }
}

/// Strip tool-call JSON blocks from an assistant response, leaving only the
/// prose text.
///
/// The archivist stores the full response (including `tool_calls_json`) in
/// the episodic log for diagnostic purposes. However, per the memory
/// ingestion policy, structured tool-call payloads must not reach the memory
/// tree — only the assistant's natural-language prose is ingested.
///
/// This function applies a lightweight heuristic: it removes any contiguous
/// spans of text that look like `<tool_call>…</tool_call>` XML/JSON blocks or
/// raw JSON objects that begin with `{"tool_calls":`. The output may be empty
/// if the entire response was tool-call markup — callers should handle that
/// case (empty text → no-op ingest).
fn strip_tool_calls_from_response(response: &str) -> String {
    // Fast path: if the response contains no obvious tool-call markers, return
    // it unchanged to avoid unnecessary allocation.
    if !response.contains("<tool_call>")
        && !response.contains("{\"tool_calls\"")
        && !response.contains("\"tool_use\"")
    {
        return response.to_string();
    }

    // Remove XML-style tool-call blocks.
    let mut cleaned = response.to_string();

    // Strip <tool_call>…</tool_call> spans (may span multiple lines).
    while let Some(start) = cleaned.find("<tool_call>") {
        if let Some(end) = cleaned[start..].find("</tool_call>") {
            cleaned.drain(start..start + end + "</tool_call>".len());
        } else {
            // Unclosed tag — remove from the tag to end of string.
            cleaned.truncate(start);
            break;
        }
    }

    // Drop JSON / tool-use payload lines the XML strip above cannot catch
    // (evidence-vs-interpretation policy: tool-call payloads must never reach
    // tree ingest).
    cleaned = cleaned
        .lines()
        .filter(|line| {
            let l = line.trim();
            !(l.contains("\"tool_use\"")
                || l.starts_with("{\"tool_calls\"")
                || l.starts_with("\"tool_calls\""))
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Trim and collapse runs of blank lines left by block removal.
    let trimmed = cleaned
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");

    // Collapse more than two consecutive newlines to two.
    let mut result = String::with_capacity(trimmed.len());
    let mut blank_run = 0usize;
    for line in trimmed.lines() {
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 2 {
                result.push('\n');
            }
        } else {
            blank_run = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
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
impl ArchivistHook {
    /// Test-only constructor that injects a stub `ChatProvider` and `Embedder`
    /// directly, bypassing `with_config`'s provider-build logic. Used by
    /// Phase 1 tests to verify LLM recap and embedding paths without hitting
    /// a real LLM or Ollama daemon. Exposed as `pub(crate)` so Phase 3
    /// STM recall integration tests can drive the full archivist path.
    pub(crate) fn new_with_stubs(
        conn: Arc<Mutex<Connection>>,
        chat_provider: Arc<dyn ChatProvider>,
        embedder: Arc<dyn Embedder>,
    ) -> Self {
        Self {
            conn: Some(conn),
            enabled: true,
            boundary_config: BoundaryConfig::default(),
            config: Some(Config::default()),
            chat_provider: Some(chat_provider),
            embedder: Some(embedder),
        }
    }

    /// Test-only constructor that injects stub providers AND a `Config`, so the
    /// Phase 2 segment-tree ingest path (gated by
    /// `config.learning.chat_to_tree_enabled`) can be exercised hermetically.
    ///
    /// `config.learning.chat_to_tree_enabled` must be set to `true` by the caller
    /// for the tree ingest to fire; the hook does NOT force it on.
    pub(crate) fn new_with_stubs_and_config(
        conn: Arc<Mutex<Connection>>,
        chat_provider: Arc<dyn ChatProvider>,
        embedder: Arc<dyn Embedder>,
        config: Config,
    ) -> Self {
        Self {
            conn: Some(conn),
            enabled: true,
            boundary_config: BoundaryConfig::default(),
            config: Some(config),
            chat_provider: Some(chat_provider),
            embedder: Some(embedder),
        }
    }
}

#[cfg(test)]
#[path = "archivist_tests.rs"]
mod tests;
