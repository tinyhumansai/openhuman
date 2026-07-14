//! Segment summarization logic for `ArchivistHook`.

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

    /// Shared summarize helper used by the finalize path (`on_segment_closed`).
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
    /// durable last-resort on the finalize path.
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
}
