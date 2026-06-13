//! Summarization and rolling recap logic for `ArchivistHook`.

use super::types::ArchivistHook;
use crate::openhuman::memory_store::fts5::{self, EpisodicEntry};
use crate::openhuman::memory_store::segments;
use crate::openhuman::memory_store::trees::types::TreeKind;
use crate::openhuman::memory_tree::summarise::{summarise, SummaryContext, SummaryInput};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

impl ArchivistHook {
    /// Read every entry recorded for `session_id`, preferring the
    /// md-backed `memory_archivist::store` when `self.config` is set and
    /// falling back to the legacy FTS5 episodic table otherwise.
    ///
    /// Returns `EpisodicEntry` so the existing call sites (segment
    /// gathering, recap rendering, tree push) keep their shape unchanged
    /// during the FTS5 retirement migration.
    pub(super) fn read_session_entries(
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
    pub(super) async fn summarize_entries(
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
        let open_segment = match crate::openhuman::memory_store::segments::open_segment_for_session(
            conn, session_id,
        ) {
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
}
