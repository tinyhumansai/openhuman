//! Phase 1.5 — segment-recap-backed compaction summarizer.
//!
//! [`SegmentRecapSummarizer`] wraps an inner [`Summarizer`] (normally a
//! [`super::ProviderSummarizer`]) and intercepts the autocompaction call to
//! try the rolling segment recap from the [`ArchivistHook`] first.
//!
//! ## Strategy
//!
//! When `AutocompactionRequested` fires, the manager calls
//! [`SegmentRecapSummarizer::summarize`]. That method:
//!
//! 1. Calls [`ArchivistHook::rolling_segment_recap`] for the current session.
//! 2. If a non-empty recap is returned, it replaces the evicted head of the
//!    history with a single `[segment-recap]` system message containing that
//!    text. The head/tail split follows the same "never break a tool-call
//!    pair" rule as the inner summarizer.
//! 3. If the recap is `None`/empty (archivist absent, no open segment, LLM
//!    fail, flag off) — soft-fallback: delegate to the inner summarizer
//!    unchanged. The prompt is never left over-budget; the existing
//!    compaction safety net always fires.
//!
//! ## Invariants (never violated by this code)
//!
//! - The `ArchivistHook` is only *read* through `rolling_segment_recap` — no
//!   segment is closed, no `segment_set_summary` is written, no embedding is
//!   produced. Those side-effects are finalize-only (Phase 1 owns them).
//! - Events/profile/tree derive from RAW episodic rows — this path never
//!   touches those subsystems.
//! - On any error in the recap path, the inner summarizer runs. The
//!   circuit-breaker logic in [`super::ContextManager`] is fed the result of
//!   whichever path actually ran; the breaker sees a success if either path
//!   succeeds.
//! - The history is either fully rewritten (success) or left completely
//!   untouched (the inner summarizer also fails, in which case its `Err` is
//!   returned — the breaker will nudge and eventually trip, preventing
//!   infinite loops).

use super::summarizer::{Summarizer, SummaryStats};
use crate::openhuman::agent::harness::archivist::ArchivistHook;
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// How many messages at the tail of the history are preserved verbatim when
/// the recap path fires (same default as [`super::ProviderSummarizer`] so the
/// two paths preserve the same recent context window).
const DEFAULT_KEEP_RECENT: usize = super::summarizer::DEFAULT_KEEP_RECENT;

/// Compaction summarizer that prefers the rolling segment recap from the
/// archivist over a standalone LLM summarization call.
///
/// Constructed by [`super::SessionBuilder`] when
/// `config.learning.unified_compaction_enabled` is `true` and an
/// `ArchivistHook` is wired in; otherwise the plain `ProviderSummarizer`
/// is used and this type is never instantiated.
pub struct SegmentRecapSummarizer {
    /// The archivist that owns episodic state and can produce rolling recaps.
    archivist: Arc<ArchivistHook>,
    /// Session ID needed to look up the open segment.
    session_id: String,
    /// Inner summarizer — the safety-net fallback (normally a
    /// [`super::ProviderSummarizer`] wrapping the same provider the agent uses
    /// for its normal turns).
    inner: Arc<dyn Summarizer>,
    /// How many tail messages to preserve verbatim when the recap path fires.
    keep_recent: usize,
}

impl SegmentRecapSummarizer {
    /// Construct a new [`SegmentRecapSummarizer`].
    ///
    /// * `archivist`  — shared archivist handle (same Arc the agent holds).
    /// * `session_id` — the session whose open segment will be recapped.
    /// * `inner`      — fallback summarizer; used when the recap is unavailable.
    pub fn new(
        archivist: Arc<ArchivistHook>,
        session_id: String,
        inner: Arc<dyn Summarizer>,
    ) -> Self {
        Self {
            archivist,
            session_id,
            inner,
            keep_recent: DEFAULT_KEEP_RECENT,
        }
    }

    /// Override how many tail messages are preserved verbatim. Useful in
    /// tests that want to exercise the recap path with short histories.
    #[cfg(test)]
    pub fn with_keep_recent(mut self, n: usize) -> Self {
        self.keep_recent = n;
        self
    }
}

#[async_trait]
impl Summarizer for SegmentRecapSummarizer {
    async fn summarize(
        &self,
        history: &mut Vec<ConversationMessage>,
        model: &str,
    ) -> Result<SummaryStats> {
        // ── 1. Try the rolling segment recap ─────────────────────────────
        let recap_opt = self.archivist.rolling_segment_recap(&self.session_id).await;

        match recap_opt {
            Some(recap) if !recap.is_empty() => {
                tracing::info!(
                    session_id = %self.session_id,
                    recap_chars = recap.len(),
                    history_len = history.len(),
                    keep_recent = self.keep_recent,
                    "[context::segment_recap] using rolling segment recap as compaction text"
                );

                let total = history.len();
                if total <= self.keep_recent {
                    tracing::debug!(
                        session_id = %self.session_id,
                        "[context::segment_recap] history below keep_recent — \
                         nothing to compact (NoOp)"
                    );
                    return Ok(SummaryStats::default());
                }

                // Use the same "never break a tool-call pair" split rule as
                // ProviderSummarizer so the API invariant
                // (AssistantToolCalls ↔ ToolResults) is preserved.
                // Delegates to the canonical implementation in `summarizer`
                // so the two compaction paths share a single definition.
                let proposed_head = total - self.keep_recent;
                let head_len = super::summarizer::snap_split_forward(history, proposed_head);
                if head_len == 0 {
                    tracing::debug!(
                        session_id = %self.session_id,
                        "[context::segment_recap] split snapped to 0 — \
                         falling back to inner summarizer"
                    );
                    return self.inner.summarize(history, model).await;
                }

                // Estimate bytes freed (same formula as ProviderSummarizer).
                let approx_input_bytes: usize = history[..head_len]
                    .iter()
                    .map(|m| conversation_message_approx_bytes(m))
                    .sum();

                let summary_body = format!(
                    "[segment-recap] Summary of {head_len} earlier messages \
                     (archivist rolling recap):\n\n{recap}"
                );
                let summary_chars = summary_body.len();
                let approx_tokens_freed = (approx_input_bytes as u64)
                    .saturating_sub(summary_chars as u64)
                    .div_ceil(4);

                // Atomically rewrite the head in place — no partial mutation on
                // failure because all failure paths returned early above.
                let tail: Vec<ConversationMessage> = history.drain(head_len..).collect();
                history.clear();
                history.push(ConversationMessage::Chat(ChatMessage::system(summary_body)));
                history.extend(tail);

                tracing::info!(
                    session_id = %self.session_id,
                    messages_removed = head_len,
                    approx_tokens_freed,
                    summary_chars,
                    "[context::segment_recap] compaction via segment recap complete"
                );

                Ok(SummaryStats {
                    messages_removed: head_len,
                    approx_tokens_freed,
                    summary_chars,
                })
            }

            // ── 2. Soft-fallback ─────────────────────────────────────────
            //
            // The rolling recap was unavailable (None, empty, LLM fail, no
            // open segment, archivist disabled). Delegate to the inner
            // summarizer so the prompt is NEVER left over-budget. This is the
            // "always bounded" guarantee.
            recap_result => {
                let reason = match &recap_result {
                    None => "rolling_segment_recap returned None",
                    Some(s) if s.is_empty() => "rolling_segment_recap returned empty string",
                    _ => "unreachable",
                };
                tracing::info!(
                    session_id = %self.session_id,
                    reason,
                    "[context::segment_recap] recap unavailable — \
                     falling back to inner summarizer"
                );
                self.inner.summarize(history, model).await
            }
        }
    }
}

/// Very rough byte count for a [`ConversationMessage`] — used only for the
/// "approx_tokens_freed" stat. Accuracy doesn't matter much (it's the same
/// rough accounting ProviderSummarizer uses).
fn conversation_message_approx_bytes(msg: &ConversationMessage) -> usize {
    match msg {
        ConversationMessage::Chat(m) => m.content.len(),
        ConversationMessage::AssistantToolCalls {
            text, tool_calls, ..
        } => {
            text.as_deref().map_or(0, str::len)
                + tool_calls
                    .iter()
                    .map(|tc| tc.arguments.len())
                    .sum::<usize>()
        }
        ConversationMessage::ToolResults(results) => results.iter().map(|r| r.content.len()).sum(),
    }
}

#[cfg(test)]
#[path = "segment_recap_summarizer_tests.rs"]
mod tests;
