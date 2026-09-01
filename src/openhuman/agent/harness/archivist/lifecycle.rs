//! Constructor methods, segment lifecycle management, and flush logic for
//! `ArchivistHook`.

use super::boundary::{BoundaryConfig, BoundaryDecision};
use super::events_heuristic::{extract_events_heuristic, ExtractedEventKind};
use super::helpers::{extract_profile_key, uuid_v4};
use super::types::ArchivistHook;
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::{
    ConversationSegment, EpisodicEvent, EpisodicTurn, FacetType, MemoryEpisodic, MemoryProvider,
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The inference role the segment recap runs as.
///
/// Named here rather than inlined because it is the one string that has to
/// match what the memory summariser asks for — `tinymemory_core::chat::
/// build_chat_runtime` routes `"summarization"`, and the probe in
/// [`ArchivistHook::with_config`] is only meaningful if it asks the same
/// question of the same role.
const RECAP_INFERENCE_ROLE: &str = "summarization";

impl ArchivistHook {
    /// Create an Archivist hook over the workspace's bound memory driver.
    ///
    /// LLM recap and embedding are disabled by default; call
    /// [`Self::with_config`] on the production path to wire them in.
    pub fn new(provider: Arc<dyn MemoryProvider>, enabled: bool) -> Self {
        Self {
            provider: Some(provider),
            enabled,
            boundary_config: BoundaryConfig::default(),
            config: None,
            summariser_available: false,
            #[cfg(test)]
            chat_provider: None,
        }
    }

    /// Attach runtime config so the archivist can gate the tree-ingest path
    /// and record whether an LLM summariser and an embedder are available.
    ///
    /// When `config.learning.chat_to_tree_enabled` is `true`, each closed
    /// segment's raw prose turns are ingested into the memory tree as
    /// `source_id="conversations:agent"` (one batch per segment, not per turn).
    /// The summariser probe is soft-fallback: if construction fails, the
    /// archivist falls back to the heuristic summary rather than failing the
    /// turn. Embedding is also non-fatal and goes through the provider's
    /// `as_scoring()` family.
    ///
    /// # Why the summariser is probed and not held
    ///
    /// This used to call `tinymemory_core::chat::build_chat_provider(&config)`
    /// and store the `Arc<dyn ChatProvider>` it returned. Nothing ever called
    /// that handle: the summariser the archivist drives is
    /// `tinymemory_core::tree::summarise::summarise`, which builds
    /// its own provider from the same `Config` on every call. So the stored
    /// value was only ever read as `is_some()`, and what it actually asserted
    /// was "a chat model for the summarise role can be constructed".
    ///
    /// That question is the host's to answer, and this asks it directly.
    /// `build_chat_provider` wraps `tinymemory_core::chat_host::
    /// create_chat_model_with_model_id`, which is a process-global seam whose
    /// only implementation is `OpenHumanChatHost` in `memory/host_impls.rs`,
    /// and that forwards verbatim to the call below — same role, same config,
    /// same temperature. The predicate is therefore unchanged; what changed is
    /// that the archivist no longer names the memory engine to evaluate it
    /// (#5560), and no longer builds a model it will not use.
    ///
    /// One deliberate difference, stated rather than hidden: under `cfg(test)`
    /// the engine's builder short-circuits on its chat task-local, so a stubbed
    /// test would have reported "available" without a real provider. No test
    /// takes this path — every archivist test constructs the hook through
    /// `new`, `disabled` or `new_with_stubs*` — and the tests that do need a
    /// deterministic LLM install the task-local around the call itself, which
    /// is where it has to be anyway for `summarise` to see it.
    pub fn with_config(mut self, config: Config) -> Self {
        // Probe the summariser: can this host build a chat model for the recap
        // role right now? The model is dropped immediately — see above.
        let probe = crate::openhuman::inference::provider::create_chat_model_with_model_id(
            RECAP_INFERENCE_ROLE,
            &config,
            config.default_temperature,
        );
        let summariser_available = match probe {
            Ok((_model, model_id)) => {
                tracing::debug!(
                    "[archivist] segment recap summariser ready role={RECAP_INFERENCE_ROLE} \
                     model={model_id}"
                );
                true
            }
            Err(e) => {
                tracing::warn!(
                    "[archivist] no chat model for role={RECAP_INFERENCE_ROLE} \
                     (segment recap falls back to the heuristic summary): {e}"
                );
                false
            }
        };

        self.summariser_available = summariser_available;
        self.config = Some(config);
        self
    }

    /// Create a disabled/no-op Archivist.
    pub fn disabled() -> Self {
        Self {
            provider: None,
            enabled: false,
            boundary_config: BoundaryConfig::default(),
            config: None,
            summariser_available: false,
            #[cfg(test)]
            chat_provider: None,
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
        let Some(episodic) = self.episodic() else {
            return;
        };
        let now = Self::now_timestamp();
        tracing::debug!("[archivist] flush_open_segment: checking session={session_id}");
        let open_segment = match episodic.open_segment(session_id).await {
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
        if let Err(e) = episodic.close_segment(&segment.segment_id, now).await {
            tracing::warn!("[archivist] flush: failed to close segment: {e}");
            return;
        }
        self.on_segment_closed(&segment, session_id, now).await;
    }

    /// The bound driver's episodic family, when both are present.
    ///
    /// `None` when the archivist is disabled, has no provider, or the driver
    /// does not serve `Episodic` — every caller treats that as "nothing to
    /// write", matching the old `conn: None` behaviour.
    pub(super) fn episodic(&self) -> Option<&dyn MemoryEpisodic> {
        self.provider.as_deref().and_then(|p| p.as_episodic())
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
    pub(super) async fn manage_segment(
        &self,
        session_id: &str,
        timestamp: f64,
        user_message: &str,
        current_episodic_id: i64,
        current_seq: Option<u32>,
    ) -> Option<ConversationSegment> {
        let episodic = self.episodic()?;
        let now = Self::now_timestamp();

        // Check for an open segment for this session.
        let open_segment = match episodic.open_segment(session_id).await {
            Ok(seg) => seg,
            Err(e) => {
                tracing::warn!("[archivist] failed to query open segment: {e}");
                return None;
            }
        };

        match open_segment {
            Some(segment) => {
                // Run boundary detection.
                // Boundary detection is host policy and lives in
                // `archivist::boundary`; the engine only persists what it
                // decides. `SegmentBoundaryState` names the four fields the
                // decision actually reads.
                let decision = super::boundary::detect_boundary(
                    &self.boundary_config,
                    &super::boundary::SegmentBoundaryState {
                        turn_count: segment.turn_count,
                        start_timestamp: segment.start_timestamp,
                        end_timestamp: segment.end_timestamp,
                        embedding: segment.embedding.clone(),
                    },
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
                        if let Err(e) = episodic
                            .append_turn(
                                &segment.segment_id,
                                current_episodic_id,
                                current_seq,
                                timestamp,
                                now,
                            )
                            .await
                        {
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
                        if let Err(e) = episodic.close_segment(&segment.segment_id, now).await {
                            tracing::warn!("[archivist] failed to close segment: {e}");
                            return None;
                        }

                        // Create a new segment for the new topic.
                        // The new segment starts at the current turn's episodic ID.
                        let new_id = format!("seg-{}", uuid_v4());
                        if let Err(e) = episodic
                            .create_segment(
                                &new_id,
                                session_id,
                                "global",
                                current_episodic_id,
                                current_seq,
                                timestamp,
                                now,
                            )
                            .await
                        {
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
                if let Err(e) = episodic
                    .create_segment(
                        &segment_id,
                        session_id,
                        "global",
                        current_episodic_id,
                        current_seq,
                        timestamp,
                        now,
                    )
                    .await
                {
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
        segment: &ConversationSegment,
        session_id: &str,
        now: f64,
    ) {
        // Gather the conversation text for this segment. Prefer the
        // md-backed memory_archivist read when config is available; fall
        // back to the driver's episodic family otherwise.
        let entries = self.read_session_entries(session_id).await;

        // Filter entries by their stable per-session sequence or episodic row
        // id. The md store rounds timestamps to milliseconds, which can move a
        // fast turn just before its segment's higher-precision start time.
        let segment_entries: Vec<&EpisodicTurn> = entries
            .iter()
            .filter(|record| record.is_in_segment(segment))
            .map(|record| &record.turn)
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
        let set_summary = match self.episodic() {
            Some(episodic) => {
                episodic
                    .set_segment_summary(&segment.segment_id, &summary, now)
                    .await
            }
            None => return,
        };
        if let Err(e) = set_summary {
            tracing::warn!("[archivist] failed to set segment summary: {e}");
        } else {
            tracing::debug!(
                "[archivist] recap persisted segment={} summary_chars={}",
                segment.segment_id,
                summary.len()
            );
        }

        // ── Finalize-time embedding ───────────────────────────────────────
        self.embed_segment_recap(&segment.segment_id, &summary, now)
            .await;

        // ── Heuristic event extraction ────────────────────────────────────
        if !segment_text.is_empty() {
            let extracted = extract_events_heuristic(&segment_text);
            tracing::debug!(
                "[archivist] extracted {} events from segment {}",
                extracted.len(),
                segment.segment_id
            );

            for (event_kind, content) in &extracted {
                let event_id = format!("evt-{}", uuid_v4());
                let event = EpisodicEvent {
                    event_id,
                    segment_id: segment.segment_id.clone(),
                    session_id: session_id.to_string(),
                    namespace: segment.namespace.clone(),
                    kind: event_kind.contract(),
                    content: content.clone(),
                    subject: None,
                    timestamp_ref: None,
                    confidence: 0.6,
                    embedding: None,
                    source_turn_ids: None,
                    created_at: now,
                };
                if let Some(episodic) = self.episodic() {
                    if let Err(e) = episodic.insert_event(&event).await {
                        tracing::warn!("[archivist] failed to insert event: {e}");
                    }
                }

                // Update user profile from preference and fact events.
                // Preference and fact events double as profile observations.
                // `upsert_provider_facet` is the confidence-aware door for a
                // provider-sourced claim; merging is the driver's, so a
                // lower-confidence re-observation cannot overwrite a stronger
                // one.
                let profile_write = match event_kind {
                    ExtractedEventKind::Preference => Some((
                        extract_profile_key(content, "preference"),
                        FacetType::Preference,
                    )),
                    ExtractedEventKind::Fact => {
                        Some((extract_profile_key(content, "fact"), FacetType::Context))
                    }
                    _ => None,
                };
                if let Some((key, facet_type)) = profile_write {
                    let facet_id = format!("prf-{}", uuid_v4());
                    let upsert =
                        self.provider
                            .as_deref()
                            .and_then(|p| p.as_profile())
                            .map(|profile| {
                                profile.upsert_provider_facet(
                                    &facet_id,
                                    facet_type,
                                    &key,
                                    content,
                                    0.6,
                                    Some(&segment.segment_id),
                                    now,
                                )
                            });
                    if let Some(fut) = upsert {
                        if let Err(e) = fut.await {
                            tracing::warn!("[archivist] failed to upsert profile facet: {e}");
                        }
                    }
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
                crate::openhuman::memory::goals::spawn_enrich_goals(
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
    pub(super) async fn embed_segment_recap(&self, segment_id: &str, summary: &str, now: f64) {
        if summary.trim().is_empty() {
            tracing::warn!(
                "[archivist] skipping embedding: recap is empty/whitespace segment={segment_id}"
            );
            return;
        }
        let Some(ref provider) = self.provider else {
            tracing::debug!(
                "[archivist] no provider — skipping segment embedding segment={segment_id}"
            );
            return;
        };
        let Some(scoring) = provider.as_scoring() else {
            #[cfg(feature = "modules")]
            {
                use tinymemory_api::capabilities::Capability;
                if crate::openhuman::modules::memory::ARTIFACT_CAPABILITIES
                    .contains(&Capability::Scoring)
                {
                    tracing::warn!(
                        "[archivist] driver does not expose scoring but the pinned artifact is \
                         expected to serve it — check module version; \
                         skipping segment embedding segment={segment_id}"
                    );
                    return;
                }
            }
            tracing::debug!(
                "[archivist] driver does not support scoring — skipping segment embedding \
                 segment={segment_id}"
            );
            return;
        };
        let model_signature = match scoring.embedder_slug().await {
            Ok(slug) => slug,
            Err(e) => {
                tracing::warn!(
                    "[archivist] embedder_slug failed (non-fatal) segment={segment_id}: {e}"
                );
                return;
            }
        };
        tracing::debug!("[archivist] embedding recap segment={segment_id} model={model_signature}");
        match scoring.embed_text(summary).await {
            Ok(vec) => {
                let Some(episodic) = self.episodic() else {
                    return;
                };
                match episodic
                    .upsert_segment_embedding(segment_id, &model_signature, &vec, now)
                    .await
                {
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
