//! LLM-backed conversation summarization.
//!
//! The context [`super::ContextPipeline`] is deliberately pure — when
//! it decides the agent history is over budget and can't be rescued by
//! cheap stages (microcompact, tool-result budget), it returns
//! [`super::PipelineOutcome::AutocompactionRequested`] and trusts the
//! caller to dispatch an LLM summarization.
//!
//! This module owns that dispatch. [`Summarizer`] is the async trait
//! [`super::ContextManager`] calls on behalf of agents; the default
//! implementation [`ProviderSummarizer`] wraps an `Arc<dyn Provider>`
//! and executes a single chat completion against the same provider the
//! agent uses for its normal turns. Tests pass a mock implementation
//! so `ContextManager::reduce_before_call` can be exercised without
//! touching the network.
//!
//! ## Reduction strategy
//!
//! The summarizer keeps the `keep_recent` most-recent messages
//! untouched (so the model still has fresh context for its next turn),
//! replays the older head of the conversation as a plain-text
//! transcript, asks the LLM to compress it into a dense note, and
//! replaces the head with a single `system` [`ConversationMessage`]
//! holding that note. The API invariant
//! (`AssistantToolCalls` ↔ `ToolResults`) is preserved because we
//! never split a pair across the head/tail boundary — if the
//! boundary lands mid-pair we push it forward until it sits between
//! complete turns.

use super::microcompact::MicrocompactStats;
use crate::openhuman::providers::{ChatMessage, ConversationMessage, Provider};
use anyhow::Result;
use async_trait::async_trait;
use std::fmt::Write as _;
use std::sync::Arc;

/// Default number of most-recent messages preserved verbatim by the
/// summarizer. Anything older gets collapsed into the summary note.
pub const DEFAULT_KEEP_RECENT: usize = 10;

/// Default temperature for summarization calls. Low-ish so the same
/// history produces stable summaries across retries.
pub const DEFAULT_SUMMARIZER_TEMPERATURE: f64 = 0.2;

/// The system prompt pinned to every summarization call. Intentionally
/// short so it burns as few tokens as possible on a call whose whole
/// purpose is to *free* tokens.
pub const SUMMARIZER_SYSTEM_PROMPT: &str =
    "You are a conversation summarizer. Your job is to take \
a chronological history of a conversation between a user and an AI assistant (including any tool \
calls and their results) and produce a compact, information-dense summary that preserves: \
(1) the user's goals and constraints, (2) decisions made so far, (3) important facts discovered \
via tool calls, (4) open questions or pending work. Do NOT preserve verbatim quotes, greetings, \
small talk, or redundant acknowledgements. Return ONLY the summary text — no preamble, no \
closing remarks.";

/// Outcome of a single summarization pass.
///
/// Returned by [`Summarizer::summarize`] so callers — chiefly
/// [`super::ContextManager`] — can log, telemeter, and feed the result
/// back into the compaction circuit breaker on the [`super::ContextGuard`].
#[derive(Debug, Clone, Default)]
pub struct SummaryStats {
    /// How many entries were removed from the head of the history and
    /// replaced with the summary message.
    pub messages_removed: usize,
    /// Character-heuristic estimate of freed tokens (input transcript
    /// bytes minus summary bytes, divided by 4). Rough but stable and
    /// free.
    pub approx_tokens_freed: u64,
    /// Total character length of the summary message that replaced the
    /// head. Useful for detecting degenerate "summarizer kept every
    /// word" responses.
    pub summary_chars: usize,
}

impl SummaryStats {
    /// Helper to turn a [`MicrocompactStats`] into a [`SummaryStats`]
    /// shaped value when reporting the union through
    /// [`super::ReductionOutcome`]. Currently unused but included so
    /// the types compose cleanly if a caller ever wants a uniform
    /// stats payload.
    #[doc(hidden)]
    pub fn from_microcompact(stats: &MicrocompactStats) -> Self {
        Self {
            messages_removed: stats.entries_cleared,
            approx_tokens_freed: (stats.bytes_freed as u64).div_ceil(4),
            summary_chars: 0,
        }
    }
}

/// Trait for anything that can summarize an agent conversation history
/// in place.
///
/// Implementations must not partially mutate `history` on failure —
/// either the full rewrite succeeds and the function returns `Ok`, or
/// `history` is untouched and the error bubbles up. This contract
/// lets [`super::ContextManager`] treat failures as "nothing happened"
/// when it records the result on its compaction circuit breaker.
#[async_trait]
pub trait Summarizer: Send + Sync {
    async fn summarize(
        &self,
        history: &mut Vec<ConversationMessage>,
        model: &str,
    ) -> Result<SummaryStats>;
}

/// Default summarizer that wraps an `Arc<dyn Provider>`.
///
/// Instantiated once per [`super::ContextManager`] — usually by the
/// agent harness at session start — so every summarization inside a
/// session hits the same provider/model. A cheaper `summarizer_model`
/// can be threaded through the caller's
/// [`crate::openhuman::config::ContextConfig`] if summarization on
/// the main model gets expensive; [`super::ContextManager::new`] is
/// responsible for choosing which model string to pass in.
pub struct ProviderSummarizer {
    provider: Arc<dyn Provider>,
    keep_recent: usize,
    temperature: f64,
}

impl ProviderSummarizer {
    /// Construct a summarizer around `provider` with default tunables.
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        Self {
            provider,
            keep_recent: DEFAULT_KEEP_RECENT,
            temperature: DEFAULT_SUMMARIZER_TEMPERATURE,
        }
    }

    /// Override how many messages are preserved verbatim at the tail.
    pub fn with_keep_recent(mut self, n: usize) -> Self {
        self.keep_recent = n;
        self
    }

    /// Override the temperature used for the summarization chat call.
    pub fn with_temperature(mut self, t: f64) -> Self {
        self.temperature = t;
        self
    }
}

#[async_trait]
impl Summarizer for ProviderSummarizer {
    async fn summarize(
        &self,
        history: &mut Vec<ConversationMessage>,
        model: &str,
    ) -> Result<SummaryStats> {
        let total = history.len();
        if total <= self.keep_recent {
            tracing::debug!(
                total,
                keep_recent = self.keep_recent,
                "[context::summarizer] nothing to summarize — history below keep_recent"
            );
            return Ok(SummaryStats::default());
        }

        // Head = everything before the preserved tail. Snap the split
        // forward so we never break an AssistantToolCalls ↔ ToolResults
        // pair. If an `AssistantToolCalls` sits at the proposed split
        // point, walk forward until we're past its matching
        // `ToolResults` envelope (or until the tail would collapse to
        // zero, in which case there's nothing to summarize).
        let head_len = snap_split_forward(history, total - self.keep_recent);
        if head_len == 0 {
            return Ok(SummaryStats::default());
        }

        // Build the plain-text transcript the summarizer reads.
        let transcript = render_transcript(&history[..head_len]);
        let approx_input_bytes = transcript.len();

        // Summarization chat call — one turn, no tools, fixed system.
        let messages = vec![
            ChatMessage::system(SUMMARIZER_SYSTEM_PROMPT),
            ChatMessage::user(format!(
                "Summarize this conversation history for continuation. Focus on goals, \
                 decisions, facts, and pending work.\n\n--- BEGIN HISTORY ---\n{transcript}\n\
                 --- END HISTORY ---"
            )),
        ];

        tracing::info!(
            model,
            head_messages = head_len,
            tail_preserved = total - head_len,
            approx_input_bytes,
            "[context::summarizer] dispatching autocompaction summary"
        );

        let response = self
            .provider
            .chat_with_history(&messages, model, self.temperature)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "[context::summarizer] provider call failed");
                e
            })?;

        let summary = response.trim();
        if summary.is_empty() {
            anyhow::bail!("summarizer returned empty response");
        }

        let summary_body =
            format!("[auto-compacted] Summary of {head_len} earlier messages:\n\n{summary}");
        let summary_chars = summary_body.len();
        let approx_tokens_freed = (approx_input_bytes as u64)
            .saturating_sub(summary_chars as u64)
            .div_ceil(4);

        // Replace the head in place. Drain the tail, clear the vec,
        // push the summary, and put the tail back. No partial mutation
        // on error paths — everything above returned early.
        let tail: Vec<ConversationMessage> = history.drain(head_len..).collect();
        history.clear();
        history.push(ConversationMessage::Chat(ChatMessage::system(summary_body)));
        history.extend(tail);

        tracing::info!(
            messages_removed = head_len,
            approx_tokens_freed,
            summary_chars,
            "[context::summarizer] autocompaction complete"
        );

        Ok(SummaryStats {
            messages_removed: head_len,
            approx_tokens_freed,
            summary_chars,
        })
    }
}

/// Snap the proposed split point forward until it sits on a clean
/// turn boundary (i.e. not mid-way through an
/// `AssistantToolCalls` → `ToolResults` pair). Returns the adjusted
/// head length. Returns 0 when the adjustment would consume the entire
/// history, meaning there is nothing we can safely summarize without
/// breaking the API invariant.
fn snap_split_forward(history: &[ConversationMessage], proposed_head: usize) -> usize {
    let mut head = proposed_head.min(history.len());
    // If the message immediately *before* the split is an
    // AssistantToolCalls and the message *at* the split is its
    // matching ToolResults, advance past the pair so we don't break
    // the API invariant mid-pair. Any other shape (no prev, prev not
    // a tool call, or tool call without a matching result right after)
    // leaves the split where it was.
    if head > 0
        && head < history.len()
        && matches!(
            &history[head - 1],
            ConversationMessage::AssistantToolCalls { .. }
        )
        && matches!(&history[head], ConversationMessage::ToolResults(_))
    {
        head += 1;
    }
    // Don't consume the whole history — there'd be no tail to preserve.
    if head >= history.len() {
        0
    } else {
        head
    }
}

/// Render a slice of `ConversationMessage` as a plain-text transcript
/// for the summarizer prompt. Format is intentionally simple — the
/// summarizer reads it as-is.
fn render_transcript(msgs: &[ConversationMessage]) -> String {
    let mut out = String::new();
    for (i, msg) in msgs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match msg {
            ConversationMessage::Chat(m) => {
                let _ = writeln!(&mut out, "[{i}] {}: {}", m.role, m.content);
            }
            ConversationMessage::AssistantToolCalls { text, tool_calls } => {
                if let Some(t) = text.as_deref() {
                    if !t.is_empty() {
                        let _ = writeln!(&mut out, "[{i}] assistant: {t}");
                    }
                }
                for tc in tool_calls {
                    let _ = writeln!(
                        &mut out,
                        "[{i}] assistant tool_call: {}({})",
                        tc.name, tc.arguments
                    );
                }
            }
            ConversationMessage::ToolResults(results) => {
                for r in results {
                    let _ = writeln!(
                        &mut out,
                        "[{i}] tool_result({}): {}",
                        r.tool_call_id, r.content
                    );
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::providers::{ChatResponse, ToolCall, ToolResultMessage};
    use async_trait::async_trait;
    use std::sync::Mutex;

    fn user(text: &str) -> ConversationMessage {
        ConversationMessage::Chat(ChatMessage::user(text))
    }

    fn assistant(text: &str) -> ConversationMessage {
        ConversationMessage::Chat(ChatMessage::assistant(text))
    }

    fn call(id: &str) -> ConversationMessage {
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![ToolCall {
                id: id.into(),
                name: "t".into(),
                arguments: "{}".into(),
            }],
        }
    }

    fn result(id: &str, body: &str) -> ConversationMessage {
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: id.into(),
            content: body.into(),
        }])
    }

    /// Minimal Provider that returns a pinned reply for every call.
    /// Records how many times `chat_with_history` fired so tests can
    /// assert the summarizer skipped the provider round-trip when it
    /// should have.
    struct StubProvider {
        reply: String,
        calls: Mutex<usize>,
    }

    impl StubProvider {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                calls: Mutex::new(0),
            }
        }
        fn call_count(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl Provider for StubProvider {
        async fn chat_with_system(
            &self,
            _system: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.reply.clone())
        }

        async fn chat_with_history(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.reply.clone())
        }

        async fn chat(
            &self,
            _request: crate::openhuman::providers::ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            *self.calls.lock().unwrap() += 1;
            Ok(ChatResponse {
                text: Some(self.reply.clone()),
                tool_calls: vec![],
                usage: None,
            })
        }
    }

    #[tokio::test]
    async fn noop_when_history_below_keep_recent() {
        let provider = Arc::new(StubProvider::new("IRRELEVANT"));
        let summarizer = ProviderSummarizer::new(provider.clone()).with_keep_recent(10);

        let mut history = vec![user("hi"), assistant("hello")];
        let stats = summarizer
            .summarize(&mut history, "test-model")
            .await
            .unwrap();

        assert_eq!(stats.messages_removed, 0);
        assert_eq!(history.len(), 2);
        assert_eq!(provider.call_count(), 0, "must not call provider on no-op");
    }

    #[tokio::test]
    async fn summarizes_long_history_and_replaces_head() {
        let provider = Arc::new(StubProvider::new("SUMMARY_BODY"));
        let summarizer = ProviderSummarizer::new(provider.clone()).with_keep_recent(2);

        // 6 older messages + 2 tail = 8 total; head should collapse to 1
        // system message, tail of 2 preserved.
        let mut history = vec![
            user("q1"),
            assistant("a1"),
            user("q2"),
            assistant("a2"),
            user("q3"),
            assistant("a3"),
            user("q4-tail"),
            assistant("a4-tail"),
        ];

        let stats = summarizer
            .summarize(&mut history, "test-model")
            .await
            .unwrap();

        assert_eq!(stats.messages_removed, 6);
        assert_eq!(history.len(), 3, "1 summary + 2 tail");
        assert_eq!(provider.call_count(), 1);

        // First message must be a system summary containing the stub reply.
        match &history[0] {
            ConversationMessage::Chat(m) => {
                assert_eq!(m.role, "system");
                assert!(m.content.contains("SUMMARY_BODY"));
                assert!(m.content.contains("[auto-compacted]"));
            }
            other => panic!("expected system summary, got {other:?}"),
        }
        // Tail preserved verbatim.
        match &history[1] {
            ConversationMessage::Chat(m) => assert_eq!(m.content, "q4-tail"),
            _ => panic!(),
        }
        match &history[2] {
            ConversationMessage::Chat(m) => assert_eq!(m.content, "a4-tail"),
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn snaps_split_past_tool_result_pair() {
        // Proposed head = 3 would land between `call("t1")` and its
        // matching `result("t1")` — the snap should push it to 4 so
        // the AssistantToolCalls ↔ ToolResults pair stays together.
        let provider = Arc::new(StubProvider::new("SUMMARY"));
        let summarizer = ProviderSummarizer::new(provider.clone()).with_keep_recent(2);

        let mut history = vec![
            user("q"),
            assistant("ack"),
            call("t1"),
            result("t1", "r1"),
            user("tail-q"),
            assistant("tail-a"),
        ];

        let _ = summarizer
            .summarize(&mut history, "test-model")
            .await
            .unwrap();

        // Expect 1 summary + 2-tail + maybe nothing between. Because
        // the head was snapped to 4, the resulting history is:
        //   [system-summary, user("tail-q"), assistant("tail-a")]
        assert_eq!(history.len(), 3);
        match &history[0] {
            ConversationMessage::Chat(m) => {
                assert_eq!(m.role, "system");
                assert!(m.content.contains("SUMMARY"));
            }
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn empty_summary_errors_and_leaves_history_untouched() {
        let provider = Arc::new(StubProvider::new("   \n\t  "));
        let summarizer = ProviderSummarizer::new(provider).with_keep_recent(1);

        let mut history = vec![user("q1"), assistant("a1"), user("q2-tail")];
        let before = history.clone();

        let err = summarizer
            .summarize(&mut history, "test-model")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("empty"));

        // History must be untouched on error.
        assert_eq!(history.len(), before.len());
    }

    #[test]
    fn transcript_renders_all_message_variants() {
        let msgs = vec![
            user("hello"),
            assistant("hi"),
            ConversationMessage::AssistantToolCalls {
                text: Some("let me check".into()),
                tool_calls: vec![ToolCall {
                    id: "1".into(),
                    name: "shell".into(),
                    arguments: r#"{"cmd":"ls"}"#.into(),
                }],
            },
            result("1", "file.txt"),
        ];
        let rendered = render_transcript(&msgs);
        assert!(rendered.contains("user: hello"));
        assert!(rendered.contains("assistant: hi"));
        assert!(rendered.contains("assistant: let me check"));
        assert!(rendered.contains("assistant tool_call: shell("));
        assert!(rendered.contains("tool_result(1): file.txt"));
    }
}
