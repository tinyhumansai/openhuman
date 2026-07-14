//! LLM-backed conversation summarization for the tinyagents harness path
//! (issue #4249).
//!
//! The harness historically only **front-trimmed** a long transcript
//! ([`tinyagents::harness::middleware::MessageTrimMiddleware`] with
//! [`TrimStrategy::MaxTokens`][tinyagents::harness::summarization::TrimStrategy]),
//! dropping the oldest turns wholesale once the thread neared the model's
//! context window. That is lossy: the dropped turns vanish.
//!
//! This module supplies the missing **summarization step** the per-folder graphs
//! install: an LLM-backed [`Summarizer`] that condenses the older slice of the
//! transcript into a single system message, driven by the crate's
//! [`ContextCompressionMiddleware`][tinyagents::harness::middleware::ContextCompressionMiddleware]
//! and a context-window-aware [`SummarizationPolicy`]. The policy only fires once
//! the running token estimate crosses [`SUMMARIZE_THRESHOLD_FRACTION`] of the
//! **current model's** context window — so the trigger is keyed to "whatever
//! model we are using", mirroring the historical OpenHuman compaction threshold
//! (0.90).
//!
//! Layering: a graph installs the compression middleware **before** the
//! deterministic trim, so summarization is preferred and trimming remains only a
//! last-resort hard cap when even the summary + recent window overflow.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use tinyagents::error::{Result as TaResult, TinyAgentsError};
use tinyagents::harness::message::Message as TaMessage;
use tinyagents::harness::summarization::{
    estimate_tokens, CompressionProvenance, SummarizationPolicy, Summarizer, SummaryRecord,
};

use tinyagents::harness::message::Message as HarnessMessage;
use tinyagents::harness::model::{ChatModel, ModelRequest};

/// Fraction of the model's context window at which summarization fires.
///
/// Mirrors the old OpenHuman context guard soft threshold (0.90) so the
/// tinyagents path compacts at the same point the legacy `ContextManager` did.
const SUMMARIZE_THRESHOLD_FRACTION: f64 = 0.90;

/// Number of most-recent non-system messages kept verbatim after a compaction.
/// The older head is folded into the summary; this tail stays untouched so the
/// model retains the live working context.
const SUMMARIZE_KEEP_LAST: usize = 8;

/// An LLM-backed [`Summarizer`] that condenses a slice of harness [`TaMessage`]s
/// into a single system summary via a crate [`ChatModel`] call.
///
/// Wraps the **same** model the turn is already running on (its id/temperature
/// baked in), so the summary is produced by the active model (a cheaper
/// summarizer model can be threaded later if compaction on the main model proves
/// expensive — the legacy `ContextConfig::summarizer_model` hook).
pub(super) struct ProviderModelSummarizer {
    model: Arc<dyn ChatModel<()>>,
    /// Model id, kept for logging/provenance only (the id rides the wrapped
    /// [`ChatModel`]).
    model_id: String,
}

impl ProviderModelSummarizer {
    /// Build a summarizer over `model` (its id/temperature pinned).
    pub(super) fn new(model: Arc<dyn ChatModel<()>>, model_id: impl Into<String>) -> Self {
        Self {
            model,
            model_id: model_id.into(),
        }
    }
}

/// Role label for a harness message, used to render the plain-text transcript
/// the summarizer reads.
fn role_label(msg: &TaMessage) -> &'static str {
    match msg {
        TaMessage::System(_) => "system",
        TaMessage::User(_) => "user",
        TaMessage::Assistant(_) => "assistant",
        TaMessage::Tool(_) => "tool",
    }
}

#[async_trait]
impl Summarizer for ProviderModelSummarizer {
    async fn summarize(&self, messages: &[TaMessage]) -> TaResult<SummaryRecord> {
        if messages.is_empty() {
            return Err(TinyAgentsError::Validation(
                "cannot summarize an empty message list".into(),
            ));
        }

        let original_token_estimate: u64 =
            messages.iter().map(|m| estimate_tokens(&m.text())).sum();
        // `Message` carries no stable id, so assign synthetic positional ids
        // (matching the crate's `ConcatSummarizer` provenance convention).
        let source_ids: Vec<String> = (0..messages.len()).map(|i| format!("msg-{i}")).collect();

        let transcript = messages
            .iter()
            .map(|m| format!("{}: {}", role_label(m), m.text()))
            .collect::<Vec<_>>()
            .join("\n");

        tracing::info!(
            model = %self.model_id,
            head_messages = messages.len(),
            approx_input_tokens = original_token_estimate,
            "[tinyagents::summarize] dispatching context-window summary"
        );

        let request = ModelRequest::new(vec![
            HarnessMessage::system(SUMMARIZER_SYSTEM_PROMPT),
            HarnessMessage::user(transcript),
        ]);
        let summary = self
            .model
            .invoke(&(), request)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "[tinyagents::summarize] summarizer model call failed");
                TinyAgentsError::Model(format!("summarizer model call failed: {e}"))
            })?
            .text();

        let summary = summary.trim();
        if summary.is_empty() {
            return Err(TinyAgentsError::Model(
                "summarizer returned empty response".into(),
            ));
        }

        let body = format!("=== Conversation Summary (compacted) ===\n{summary}");
        let summary_token_estimate = estimate_tokens(&body);

        tracing::info!(
            model = %self.model_id,
            summary_tokens = summary_token_estimate,
            freed_tokens = original_token_estimate.saturating_sub(summary_token_estimate),
            "[tinyagents::summarize] context-window summary complete"
        );

        Ok(SummaryRecord {
            summary: TaMessage::system(body),
            provenance: CompressionProvenance {
                source_ids,
                original_token_estimate,
                summary_token_estimate,
                reason: format!(
                    "ProviderModelSummarizer via {} (LLM compaction at {:.0}% of context window)",
                    self.model_id,
                    SUMMARIZE_THRESHOLD_FRACTION * 100.0
                ),
            },
        })
    }
}

/// Token budget for the deterministic-trim fallback summary, as a fraction of
/// the policy's summarization trigger budget. The fallback must actually *free*
/// tokens (so the turn shrinks below the window), so it targets a small slice of
/// the trigger point rather than echoing the whole compacted head back.
const FALLBACK_TRIM_TRIGGER_FRACTION: f64 = 0.25;
/// Hard floor / ceiling (tokens) for the deterministic-trim fallback budget, so
/// tiny windows still keep *something* and huge windows don't defeat the point.
const FALLBACK_TRIM_MIN_TOKENS: u64 = 1_024;
const FALLBACK_TRIM_MAX_TOKENS: u64 = 8_192;

/// A single cached summary keyed by the shape of its input slice.
///
/// `key` is a content hash of the exact `to_summarize` slice the crate handed us
/// (message count folded in). Repeat calls within a turn that present the same
/// slice (retries, re-planning, or a stalled tool loop that re-issues an
/// identical model request) reuse the cached [`SummaryRecord`] instead of
/// re-dispatching the summarizer LLM.
struct CachedSummary {
    key: u64,
    record: SummaryRecord,
}

/// Fault-tolerant, per-turn-caching [`Summarizer`] adapter (issue #4461).
///
/// Wraps the real (LLM-backed) [`ProviderModelSummarizer`] the turn hands the
/// crate [`ContextCompressionMiddleware`][tinyagents::harness::middleware::ContextCompressionMiddleware]
/// and hardens two regressions the crate introduced versus the legacy engine:
///
/// 1. **Failure no longer aborts the turn.** The crate's `before_model` does
///    `self.summarizer.summarize(..).await?`, so any provider hiccup maps to
///    [`TinyAgentsError::Model`] and fails the whole run — on exactly the
///    longest, most valuable threads. This adapter instead catches the error,
///    logs a `warn`, trips a **per-turn circuit breaker**, and returns a
///    deterministic (LLM-free) trim of the input. The turn continues, matching
///    the legacy `warn! + circuit-breaker + deterministic-trim` fallback. Once
///    the breaker is tripped, every later compaction in the turn skips the
///    known-bad LLM and trims directly.
///
/// 2. **No re-summarizing identical input.** The crate rebuilds the request from
///    `messages.clone()` each loop iteration and rewrites only that per-call
///    clone, so the working transcript never shrinks. Any call that presents the
///    same `to_summarize` slice (retries, re-planning, an identical re-issued
///    request) would otherwise spend a fresh full-transcript summarizer LLM call.
///    A single-slot content-hash cache makes those repeat calls free until the
///    transcript actually grows past the threshold again.
///
/// Constructed fresh per turn inside [`assemble_turn_harness`][super::assemble_turn_harness],
/// so the breaker flag and cache are naturally per-turn state — no task-locals.
pub(super) struct FaultTolerantCachingSummarizer {
    /// The real LLM-backed summarizer we guard.
    inner: Box<dyn Summarizer>,
    /// Per-turn circuit breaker: set once `inner` fails, thereafter every
    /// compaction trims deterministically without touching the LLM.
    breaker_tripped: AtomicBool,
    /// Single-slot cache of the last produced summary, keyed by input-slice hash.
    cache: Mutex<Option<CachedSummary>>,
    /// Token budget for the deterministic-trim fallback (derived from the
    /// policy's context window at construction).
    fallback_trim_budget: u64,
}

impl FaultTolerantCachingSummarizer {
    /// Wrap `inner` with per-turn fault tolerance + caching, sizing the
    /// deterministic-trim fallback budget from `policy`'s trigger budget.
    pub(super) fn new(inner: Box<dyn Summarizer>, policy: &SummarizationPolicy) -> Self {
        let fallback_trim_budget = ((policy.trigger_budget() as f64
            * FALLBACK_TRIM_TRIGGER_FRACTION) as u64)
            .clamp(FALLBACK_TRIM_MIN_TOKENS, FALLBACK_TRIM_MAX_TOKENS);
        tracing::debug!(
            fallback_trim_budget,
            trigger_budget = policy.trigger_budget(),
            "[tinyagents::summarize] installing fault-tolerant caching summarizer adapter"
        );
        Self {
            inner,
            breaker_tripped: AtomicBool::new(false),
            cache: Mutex::new(None),
            fallback_trim_budget,
        }
    }

    /// Content hash of the exact input slice, folding in the message count so a
    /// count change alone busts the cache (a grown transcript re-summarizes).
    fn slice_key(messages: &[TaMessage]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        messages.len().hash(&mut hasher);
        for msg in messages {
            role_label(msg).hash(&mut hasher);
            msg.text().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Deterministic, LLM-free fallback: front-drop the oldest messages until the
    /// remaining slice fits [`fallback_trim_budget`][Self::fallback_trim_budget]
    /// tokens (the same front-drop semantics as
    /// [`MessageTrimMiddleware`][tinyagents::harness::middleware::MessageTrimMiddleware]
    /// with [`TrimStrategy::MaxTokens`][tinyagents::harness::summarization::TrimStrategy]),
    /// then render the survivors into a single system checkpoint message. Never
    /// fails, spends no tokens, and produces the same [`SummaryRecord`] shape the
    /// LLM path does so provenance still surfaces downstream.
    fn deterministic_trim(&self, messages: &[TaMessage], cause: &str) -> SummaryRecord {
        let original_token_estimate: u64 =
            messages.iter().map(|m| estimate_tokens(&m.text())).sum();
        let source_ids: Vec<String> = (0..messages.len()).map(|i| format!("msg-{i}")).collect();

        // Front-drop oldest messages until the tail fits the budget. Keep at
        // least the single most-recent message so the summary is never empty.
        let mut start = 0usize;
        loop {
            let remaining: u64 = messages[start..]
                .iter()
                .map(|m| estimate_tokens(&m.text()))
                .sum();
            if remaining <= self.fallback_trim_budget || start + 1 >= messages.len() {
                break;
            }
            start += 1;
        }
        let dropped = start;

        let mut body = String::from(
            "=== Conversation Summary (deterministic trim — summarizer unavailable) ===\n",
        );
        if dropped > 0 {
            body.push_str(&format!(
                "[{dropped} older message(s) dropped to fit the context budget]\n",
            ));
        }
        for msg in &messages[start..] {
            body.push_str(&format!("{}: {}\n", role_label(msg), msg.text()));
        }
        let summary_token_estimate = estimate_tokens(&body);

        tracing::warn!(
            cause,
            head_messages = messages.len(),
            dropped,
            from_tokens = original_token_estimate,
            to_tokens = summary_token_estimate,
            "[tinyagents::summarize] deterministic-trim fallback (no LLM); turn continues"
        );

        SummaryRecord {
            summary: TaMessage::system(body),
            provenance: CompressionProvenance {
                source_ids,
                original_token_estimate,
                summary_token_estimate,
                reason: format!(
                    "deterministic-trim fallback (summarizer LLM unavailable: {cause}); \
                     front-dropped {dropped} message(s) to a {}-token budget",
                    self.fallback_trim_budget
                ),
            },
        }
    }
}

#[async_trait]
impl Summarizer for FaultTolerantCachingSummarizer {
    async fn summarize(&self, messages: &[TaMessage]) -> TaResult<SummaryRecord> {
        let key = Self::slice_key(messages);

        // Cache hit: an identical slice was already summarized this turn.
        if let Ok(guard) = self.cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.key == key {
                    tracing::debug!(
                        key,
                        head_messages = messages.len(),
                        "[tinyagents::summarize] reusing cached summary (identical input slice; \
                         no summarizer LLM call)"
                    );
                    return Ok(cached.record.clone());
                }
            }
        }

        // Circuit open from an earlier failure this turn: skip the known-bad LLM
        // and trim deterministically without even attempting a call.
        let record = if self.breaker_tripped.load(Ordering::Relaxed) {
            tracing::debug!(
                key,
                head_messages = messages.len(),
                "[tinyagents::summarize] circuit breaker open; trimming deterministically \
                 (skipping summarizer LLM)"
            );
            self.deterministic_trim(
                messages,
                "circuit breaker open (earlier summarizer failure)",
            )
        } else {
            match self.inner.summarize(messages).await {
                Ok(record) => record,
                Err(err) => {
                    // Trip the per-turn breaker and fall back — never propagate,
                    // so compaction failure can no longer abort the turn.
                    self.breaker_tripped.store(true, Ordering::Relaxed);
                    tracing::warn!(
                        error = %err,
                        key,
                        head_messages = messages.len(),
                        "[tinyagents::summarize] summarizer failed; tripping per-turn circuit \
                         breaker and falling back to deterministic trim"
                    );
                    self.deterministic_trim(messages, &err.to_string())
                }
            }
        };

        // Cache the result (LLM or fallback) so a repeat identical slice is free.
        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some(CachedSummary {
                key,
                record: record.clone(),
            });
        }
        Ok(record)
    }
}

/// Build the context-window-aware [`SummarizationPolicy`] for a model whose
/// input window is `context_window` tokens.
///
/// The policy triggers compaction once the estimated transcript tokens reach
/// `context_window * `[`SUMMARIZE_THRESHOLD_FRACTION`] and keeps the most recent
/// [`SUMMARIZE_KEEP_LAST`] non-system messages (plus all system messages)
/// verbatim. Pair it with [`ProviderModelSummarizer`] via
/// [`ContextCompressionMiddleware::with_summarizer`][tinyagents::harness::middleware::ContextCompressionMiddleware::with_summarizer].
pub(super) fn summarization_policy(context_window: u64) -> SummarizationPolicy {
    let mut policy = SummarizationPolicy::default()
        .with_context_window(context_window)
        .with_threshold_fraction(SUMMARIZE_THRESHOLD_FRACTION);
    policy.keep_last = SUMMARIZE_KEEP_LAST;
    policy
}

/// System prompt for the context-window summarizer. Relocated here from the
/// former `context::summarizer` (issue #4249) — the tinyagents summarization
/// step is now its only consumer.
const SUMMARIZER_SYSTEM_PROMPT: &str = "You are a summarization agent creating a context \
checkpoint for an AI assistant whose conversation has grown too long to fit its context window. \
You are given the earlier portion of a chronological conversation (user, assistant, and tool \
messages). Compress it into a dense, structured handoff note that the assistant will read as \
BACKGROUND REFERENCE — not as new instructions.\n\
\n\
Rules:\n\
- Write ONLY the structured summary below. No greeting, no preamble, no closing remarks.\n\
- This is reference material describing turns that ALREADY happened. Do NOT answer any question \
or perform any task mentioned in it. The assistant acts only on the live messages that appear \
AFTER this summary; if a later message contradicts or changes topic, the later message wins.\n\
- Redact secrets: replace any API keys, tokens, passwords, or credentials with [REDACTED] (note \
that a credential was present).\n\
- Be specific and information-dense: prefer concrete facts (paths, names, values, decisions) over \
narration. Drop greetings, small talk, and redundant acknowledgements.\n\
\n\
Produce exactly these sections (write \"None\" when a section is empty):\n\
\n\
## Goal\n\
What the user is ultimately trying to accomplish.\n\
\n\
## Completed Actions\n\
Numbered list of what has already been done, with key results/outputs.\n\
\n\
## Active State\n\
The current state of the work right now: files touched, systems configured, what is true.\n\
\n\
## Key Decisions\n\
Decisions made and the reasoning, so they are not relitigated.\n\
\n\
## Resolved Questions\n\
Questions already answered — include the answer so it is not repeated.\n\
\n\
## Pending / Open (reference only)\n\
Requests or work outstanding in the compacted turns. These are STALE — do NOT act on them unless \
the latest live message explicitly asks.\n\
\n\
## Relevant Files\n\
Files read, created, or modified, with a one-line note on each.\n\
\n\
## Critical Context\n\
Anything else essential to continue correctly (constraints, environment facts, gotchas).";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_is_context_window_aware_at_the_configured_threshold() {
        let policy = summarization_policy(200_000);
        assert_eq!(policy.context_window, Some(200_000));
        assert_eq!(policy.threshold_fraction, SUMMARIZE_THRESHOLD_FRACTION);
        assert_eq!(policy.keep_last, SUMMARIZE_KEEP_LAST);
    }

    #[test]
    fn threshold_fraction_leaves_headroom_below_the_window() {
        // The policy must trigger *before* the window is full, so the summary
        // call itself has room — 90% by default.
        assert!(SUMMARIZE_THRESHOLD_FRACTION > 0.0 && SUMMARIZE_THRESHOLD_FRACTION < 1.0);
        let policy = summarization_policy(100_000);
        // 90% of 100k = 90k tokens is the effective trigger point.
        let effective = (policy.context_window.unwrap() as f64 * policy.threshold_fraction) as u64;
        assert_eq!(effective, 90_000);
    }
}
