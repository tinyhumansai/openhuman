//! [`ContextManager`] — the single per-session handle agents use to
//! manage their prompt and their in-flight conversation context.
//!
//! # What this owns
//!
//! 1. **System prompt assembly** — a default [`SystemPromptBuilder`]
//!    configured once at session start (usually
//!    `SystemPromptBuilder::with_defaults()`). Callers that need a
//!    different builder shape — sub-agent archetype sections, channel
//!    capabilities sections — pass their own via
//!    [`ContextManager::build_system_prompt_with`].
//!
//! 2. **Mechanical context reduction** — a [`ContextPipeline`] with its
//!    guard, microcompact stage, and session-memory tracker.
//!
//! 3. **LLM summarization dispatch** — an `Arc<dyn Summarizer>` that
//!    gets called when the pipeline reports
//!    [`PipelineOutcome::AutocompactionRequested`]. The manager records
//!    the summarizer outcome on the guard's circuit breaker so
//!    repeated failures don't loop forever.
//!
//! # What it doesn't own
//!
//! The session-memory extraction *task itself* still lives in the
//! agent harness (`turn.rs` spawns the archivist sub-agent). The
//! manager only owns the *state* that decides whether the trigger
//! should fire; it exposes that via
//! [`ContextManager::should_extract_session_memory`] so `turn.rs` can
//! gate its existing `spawn_subagent` call.

use std::sync::Arc;

use super::pipeline::{
    ContextPipeline, ContextPipelineConfig, PipelineOutcome, SessionMemoryHandle,
};
use super::prompt::{PromptContext, RenderedPrompt, SystemPromptBuilder};
use super::session_memory::SessionMemoryConfig;
use super::summarizer::{Summarizer, SummaryStats};
use crate::openhuman::config::ContextConfig;
use crate::openhuman::providers::{ConversationMessage, UsageInfo};
use anyhow::Result;

/// Outcome of a reduction pass driven by [`ContextManager::reduce_before_call`].
///
/// This is a slightly wider shape than [`PipelineOutcome`] because the
/// manager surfaces the result of the summarizer LLM call as a
/// first-class variant — the pipeline alone can only return
/// `AutocompactionRequested`.
#[derive(Debug, Clone)]
pub enum ReductionOutcome {
    /// No stage fired — budget is healthy and history was untouched.
    NoOp,
    /// The pipeline's microcompact stage cleared one or more older
    /// tool-result envelopes. The history has been mutated in place.
    Microcompacted {
        envelopes_cleared: usize,
        entries_cleared: usize,
        bytes_freed: usize,
    },
    /// The pipeline asked for summarization and the summarizer
    /// successfully rewrote the head of the history. Contains the
    /// summarizer's own stats for logging / RPC surfacing.
    Summarized(SummaryStats),
    /// The summarizer was asked to run but failed — the guard's
    /// compaction circuit breaker has been nudged. If this happens
    /// three times in a row the breaker trips and subsequent calls
    /// return [`ReductionOutcome::Exhausted`].
    SummarizationFailed { utilisation_pct: u8, reason: String },
    /// The circuit breaker is tripped and the context is still above
    /// the hard limit — the agent turn should abort.
    Exhausted { utilisation_pct: u8, reason: String },
    /// Autocompaction was requested but disabled by config. The
    /// caller is expected to surface this via the guard directly.
    NotAttempted { utilisation_pct: u8 },
}

/// Read-only snapshot of per-session context state. Returned by
/// [`ContextManager::stats`] for observability and the optional
/// `context.get_stats` RPC.
#[derive(Debug, Clone, Default)]
pub struct ContextStats {
    pub utilisation_pct: Option<u8>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub context_window: u64,
    pub compaction_disabled: bool,
    pub consecutive_compaction_failures: u8,
    pub session_memory_total_tokens: u64,
    pub session_memory_current_turn: u64,
    pub session_memory_total_tool_calls: u64,
}

/// Per-session context manager. Constructed once by the agent harness
/// at session start; lives for the whole lifetime of the `Agent`.
pub struct ContextManager {
    pipeline: ContextPipeline,
    summarizer: Arc<dyn Summarizer>,
    /// Model used for the summarization LLM call. Defaults to the
    /// session's main model; can be overridden via
    /// [`ContextConfig::summarizer_model`] when the user wants a
    /// cheaper model for compaction.
    summarizer_model: String,
    /// The default system-prompt builder used by
    /// [`ContextManager::build_system_prompt`]. Held by value so the
    /// agent's construction-time builder configuration survives the
    /// move into the manager.
    default_prompt_builder: SystemPromptBuilder,
    /// Whether the entire module is enabled. When `false`,
    /// [`ContextManager::reduce_before_call`] always returns `NoOp`.
    /// Useful for tests and debugging; see
    /// [`ContextConfig::enabled`].
    enabled: bool,
    /// Per-tool-result byte cap applied inline at tool-execution time.
    /// Stored on the manager (rather than on the agent directly) so
    /// every caller that touches "what's in the model's context window"
    /// reads the same source of truth.
    tool_result_budget_bytes: usize,
}

impl ContextManager {
    /// Construct a manager for a session.
    ///
    /// * `config` — the loaded [`ContextConfig`] section.
    /// * `summarizer` — typically a [`super::ProviderSummarizer`]
    ///   wrapping the session's provider, but tests pass a mock.
    /// * `main_model` — the agent's main model; used as the
    ///   summarizer model unless `config.summarizer_model` overrides.
    /// * `default_prompt_builder` — the builder [`build_system_prompt`]
    ///   calls. For most agents this is `SystemPromptBuilder::with_defaults()`.
    pub fn new(
        config: &ContextConfig,
        summarizer: Arc<dyn Summarizer>,
        main_model: String,
        default_prompt_builder: SystemPromptBuilder,
    ) -> Self {
        // Map ContextConfig into the mechanical pipeline's own config
        // struct. Session-memory thresholds flow through unchanged.
        let pipeline_config = ContextPipelineConfig {
            microcompact_keep_recent: config.microcompact_keep_recent,
            microcompact_enabled: config.microcompact_enabled,
            autocompact_enabled: config.autocompact_enabled,
            session_memory: SessionMemoryConfig {
                min_token_growth: config.session_memory.min_token_growth,
                min_tool_calls: config.session_memory.min_tool_calls,
                min_turns_between: config.session_memory.min_turns_between,
            },
        };

        let summarizer_model = config.summarizer_model.clone().unwrap_or(main_model);

        Self {
            pipeline: ContextPipeline::new(pipeline_config),
            summarizer,
            summarizer_model,
            default_prompt_builder,
            enabled: config.enabled,
            tool_result_budget_bytes: config.tool_result_budget_bytes,
        }
    }

    /// Byte budget for an individual tool result before the context
    /// pipeline's inline truncation stage fires. Agents read this when
    /// a tool returns to apply the cap before the result enters
    /// history.
    pub fn tool_result_budget_bytes(&self) -> usize {
        self.tool_result_budget_bytes
    }

    // ─── Budget tracking ──────────────────────────────────────────

    /// Feed the latest provider [`UsageInfo`] into the guard + the
    /// session-memory state.
    pub fn record_usage(&mut self, usage: &UsageInfo) {
        self.pipeline.record_usage(usage);
    }

    /// Bump the session-memory turn counter (called once per user turn).
    pub fn tick_turn(&mut self) {
        self.pipeline.tick_turn();
    }

    /// Accumulate a turn's tool-call count into the session-memory state.
    pub fn record_tool_calls(&mut self, n: usize) {
        self.pipeline.record_tool_calls(n);
    }

    /// Whether the caller should spawn a background session-memory
    /// extraction this turn. Delegates to the underlying pipeline
    /// state; the manager does not spawn the extraction itself.
    pub fn should_extract_session_memory(&self) -> bool {
        self.pipeline.should_extract_session_memory()
    }

    /// Mark a session-memory extraction as started (so repeated
    /// calls to [`should_extract_session_memory`] return `false` until
    /// the extraction completes).
    pub fn mark_session_memory_started(&mut self) {
        if let Ok(mut sm) = self.pipeline.session_memory.lock() {
            sm.mark_extraction_started();
        }
    }

    /// Mark a session-memory extraction as complete — resets deltas.
    pub fn mark_session_memory_complete(&mut self) {
        if let Ok(mut sm) = self.pipeline.session_memory.lock() {
            sm.mark_extraction_complete();
        }
    }

    /// Mark a session-memory extraction as failed — keeps deltas
    /// intact so the next turn retries.
    pub fn mark_session_memory_failed(&mut self) {
        if let Ok(mut sm) = self.pipeline.session_memory.lock() {
            sm.mark_extraction_failed();
        }
    }

    /// Clone the shared session-memory handle so a detached background
    /// task (see `turn.rs::spawn_session_memory_extraction`) can mark
    /// the extraction complete or failed once it finishes. The
    /// foreground path is expected to call
    /// [`Self::mark_session_memory_started`] *before* spawning so
    /// overlapping turns don't fire duplicate extractions while this
    /// one is in flight.
    pub fn session_memory_handle(&self) -> SessionMemoryHandle {
        self.pipeline.session_memory_handle()
    }

    // ─── Prompt building ───────────────────────────────────────────

    /// Assemble the opening system prompt for a session using the
    /// manager's default [`SystemPromptBuilder`].
    pub fn build_system_prompt(&self, ctx: &PromptContext<'_>) -> Result<String> {
        self.default_prompt_builder.build(ctx)
    }

    /// Assemble the opening system prompt for a session using the
    /// manager's default builder and preserve cache-boundary metadata
    /// for provider request prefix caching.
    pub fn build_system_prompt_with_cache_metadata(
        &self,
        ctx: &PromptContext<'_>,
    ) -> Result<RenderedPrompt> {
        self.default_prompt_builder.build_with_cache_metadata(ctx)
    }

    /// Assemble the system prompt via a caller-supplied builder.
    ///
    /// Sub-agents pass `SystemPromptBuilder::for_subagent(...)` and
    /// channels pass `with_defaults()` chained with a
    /// `ChannelCapabilitiesSection`. Either way the builder itself
    /// lives in [`super::prompt`] — no caller needs to know how
    /// sections are composed internally.
    pub fn build_system_prompt_with(
        &self,
        builder: &SystemPromptBuilder,
        ctx: &PromptContext<'_>,
    ) -> Result<String> {
        builder.build(ctx)
    }

    // ─── Reduction ─────────────────────────────────────────────────

    /// Run the reduction chain against `history` before a provider
    /// call. Cheap when the guard is healthy; executes the
    /// summarization LLM call internally when the pipeline asks for
    /// autocompaction.
    ///
    /// This is the single reduction entry point — agents call it once
    /// before every provider hit and map the returned
    /// [`ReductionOutcome`] into their own logging / abort logic.
    pub async fn reduce_before_call(
        &mut self,
        history: &mut Vec<ConversationMessage>,
    ) -> Result<ReductionOutcome> {
        if !self.enabled {
            return Ok(ReductionOutcome::NoOp);
        }

        match self.pipeline.run_before_call(history) {
            PipelineOutcome::NoOp => Ok(ReductionOutcome::NoOp),

            PipelineOutcome::Microcompacted(stats) => Ok(ReductionOutcome::Microcompacted {
                envelopes_cleared: stats.envelopes_cleared,
                entries_cleared: stats.entries_cleared,
                bytes_freed: stats.bytes_freed,
            }),

            PipelineOutcome::ContextExhausted {
                utilisation_pct,
                reason,
            } => Ok(ReductionOutcome::Exhausted {
                utilisation_pct,
                reason,
            }),

            PipelineOutcome::AutocompactionDisabled { utilisation_pct } => {
                Ok(ReductionOutcome::NotAttempted { utilisation_pct })
            }

            PipelineOutcome::AutocompactionRequested { utilisation_pct } => {
                // Dispatch the summarizer. If it succeeds we reset the
                // guard's circuit breaker so a prior string of failures
                // doesn't leave us permanently disabled after a good
                // run. On failure, we nudge the breaker — three
                // consecutive failures trip it and we return
                // `Exhausted` the next time the guard is checked.
                tracing::info!(
                    utilisation_pct,
                    model = %self.summarizer_model,
                    "[context::manager] dispatching autocompaction summarizer"
                );
                match self
                    .summarizer
                    .summarize(history, &self.summarizer_model)
                    .await
                {
                    Ok(stats) => {
                        self.pipeline.guard.record_compaction_success();
                        Ok(ReductionOutcome::Summarized(stats))
                    }
                    Err(e) => {
                        let reason = e.to_string();
                        tracing::warn!(
                            utilisation_pct,
                            error = %reason,
                            "[context::manager] summarizer failed — nudging circuit breaker"
                        );
                        self.pipeline.guard.record_compaction_failure();
                        Ok(ReductionOutcome::SummarizationFailed {
                            utilisation_pct,
                            reason,
                        })
                    }
                }
            }
        }
    }

    // ─── Observability ─────────────────────────────────────────────

    /// Read-only snapshot of the current budget state.
    pub fn stats(&self) -> ContextStats {
        let utilisation_pct = self
            .pipeline
            .guard
            .utilization()
            .map(|u| (u * 100.0).round() as u8);
        let sm = self.pipeline.session_memory_snapshot();
        ContextStats {
            utilisation_pct,
            input_tokens: self.pipeline.guard.last_input_tokens(),
            output_tokens: self.pipeline.guard.last_output_tokens(),
            context_window: self.pipeline.guard.context_window(),
            compaction_disabled: self.pipeline.guard.is_compaction_disabled(),
            consecutive_compaction_failures: self.pipeline.guard.consecutive_failures(),
            session_memory_total_tokens: sm.total_tokens,
            session_memory_current_turn: sm.current_turn,
            session_memory_total_tool_calls: sm.total_tool_calls,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::providers::{ChatMessage, ToolCall, ToolResultMessage};
    use async_trait::async_trait;
    use std::sync::Mutex;

    fn user(s: &str) -> ConversationMessage {
        ConversationMessage::Chat(ChatMessage::user(s))
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

    /// Mock summarizer that records how many times it was called and
    /// can be configured to succeed or fail.
    struct MockSummarizer {
        calls: Mutex<usize>,
        should_fail: bool,
    }

    impl MockSummarizer {
        fn ok() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(0),
                should_fail: false,
            })
        }
        fn failing() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(0),
                should_fail: true,
            })
        }
        fn call_count(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl Summarizer for MockSummarizer {
        async fn summarize(
            &self,
            history: &mut Vec<ConversationMessage>,
            _model: &str,
        ) -> Result<SummaryStats> {
            *self.calls.lock().unwrap() += 1;
            if self.should_fail {
                anyhow::bail!("mock failure");
            }
            // Rewrite the history to a single system summary to
            // simulate a successful reduction.
            let removed = history.len();
            history.clear();
            history.push(ConversationMessage::Chat(ChatMessage::system(
                "mock summary",
            )));
            Ok(SummaryStats {
                messages_removed: removed,
                approx_tokens_freed: 1_000,
                summary_chars: 12,
            })
        }
    }

    fn manager_with(summarizer: Arc<dyn Summarizer>) -> ContextManager {
        let config = ContextConfig::default();
        ContextManager::new(
            &config,
            summarizer,
            "test-model".into(),
            SystemPromptBuilder::with_defaults(),
        )
    }

    #[tokio::test]
    async fn reduce_returns_noop_when_guard_is_healthy() {
        let summarizer = MockSummarizer::ok();
        let mut manager = manager_with(summarizer.clone());

        // Low utilisation — guard says ok, pipeline is a no-op.
        manager.record_usage(&UsageInfo {
            input_tokens: 5_000,
            output_tokens: 500,
            context_window: 100_000,
            ..Default::default()
        });

        let mut history = vec![user("hi")];
        let outcome = manager.reduce_before_call(&mut history).await.unwrap();

        assert!(matches!(outcome, ReductionOutcome::NoOp));
        assert_eq!(summarizer.call_count(), 0);
    }

    #[tokio::test]
    async fn reduce_surfaces_microcompact_without_calling_summarizer() {
        let summarizer = MockSummarizer::ok();
        let mut manager = manager_with(summarizer.clone());

        // Push utilisation above the 90% soft threshold.
        manager.record_usage(&UsageInfo {
            input_tokens: 92_000,
            output_tokens: 4_000,
            context_window: 100_000,
            ..Default::default()
        });

        // Build a history with several older tool-result envelopes
        // that microcompact can clear — the default keep_recent is
        // DEFAULT_KEEP_RECENT_TOOL_RESULTS (5), so include at least
        // 7 pairs so the older ones are eligible.
        let mut history = vec![
            call("t1"),
            result("t1", &"x".repeat(5_000)),
            call("t2"),
            result("t2", &"x".repeat(5_000)),
            call("t3"),
            result("t3", "a"),
            call("t4"),
            result("t4", "b"),
            call("t5"),
            result("t5", "c"),
            call("t6"),
            result("t6", "d"),
            call("t7"),
            result("t7", "e"),
        ];
        let outcome = manager.reduce_before_call(&mut history).await.unwrap();

        match outcome {
            ReductionOutcome::Microcompacted {
                envelopes_cleared, ..
            } => {
                assert!(envelopes_cleared > 0);
            }
            other => panic!("expected Microcompacted, got {other:?}"),
        }
        assert_eq!(
            summarizer.call_count(),
            0,
            "microcompact must not invoke summarizer"
        );
    }

    #[tokio::test]
    async fn reduce_dispatches_summarizer_and_records_success() {
        let summarizer = MockSummarizer::ok();
        let mut manager = manager_with(summarizer.clone());

        manager.record_usage(&UsageInfo {
            input_tokens: 92_000,
            output_tokens: 4_000,
            context_window: 100_000,
            ..Default::default()
        });

        // History with no old tool-result envelopes — microcompact
        // has nothing to clear, so the pipeline signals
        // AutocompactionRequested and the manager calls the summarizer.
        let mut history = vec![user("one"), user("two"), user("three")];
        let outcome = manager.reduce_before_call(&mut history).await.unwrap();

        match outcome {
            ReductionOutcome::Summarized(stats) => {
                assert_eq!(stats.messages_removed, 3);
            }
            other => panic!("expected Summarized, got {other:?}"),
        }
        assert_eq!(summarizer.call_count(), 1);
        assert_eq!(
            history.len(),
            1,
            "mock replaced history with a single summary msg"
        );
        // Guard breaker should NOT be tripped on success.
        assert!(!manager.pipeline.guard.is_compaction_disabled());
    }

    #[tokio::test]
    async fn summarizer_failure_trips_breaker_after_three_tries() {
        let summarizer = MockSummarizer::failing();
        let mut manager = manager_with(summarizer);

        manager.record_usage(&UsageInfo {
            input_tokens: 92_000,
            output_tokens: 4_000,
            context_window: 100_000,
            ..Default::default()
        });

        // Try three times — each call sends the pipeline into
        // AutocompactionRequested, the mock summarizer fails, and
        // the breaker nudges forward. The fourth call should report
        // Exhausted because the breaker is tripped.
        for _ in 0..3 {
            let mut history = vec![user("a"), user("b"), user("c")];
            let outcome = manager.reduce_before_call(&mut history).await.unwrap();
            match outcome {
                ReductionOutcome::SummarizationFailed { .. } => {}
                other => panic!("expected SummarizationFailed, got {other:?}"),
            }
        }
        assert!(manager.pipeline.guard.is_compaction_disabled());

        // Nudge the guard above the hard limit so the next pipeline
        // pass returns ContextExhausted.
        manager.record_usage(&UsageInfo {
            input_tokens: 96_000,
            output_tokens: 2_000,
            context_window: 100_000,
            ..Default::default()
        });
        let mut history = vec![user("x")];
        let outcome = manager.reduce_before_call(&mut history).await.unwrap();
        assert!(matches!(outcome, ReductionOutcome::Exhausted { .. }));
    }

    #[tokio::test]
    async fn disabled_autocompact_returns_not_attempted() {
        let summarizer = MockSummarizer::ok();
        let mut config = ContextConfig::default();
        // Keep master switch on but disable just the autocompact stage
        // so the pipeline routes through AutocompactionDisabled instead
        // of NoOp.
        config.autocompact_enabled = false;
        let mut manager = ContextManager::new(
            &config,
            summarizer.clone(),
            "test-model".into(),
            SystemPromptBuilder::with_defaults(),
        );

        manager.record_usage(&UsageInfo {
            input_tokens: 92_000,
            output_tokens: 4_000,
            context_window: 100_000,
            ..Default::default()
        });

        // No old tool-result envelopes — microcompact cannot free
        // anything, so the pipeline lands in the autocompact branch.
        let mut history = vec![user("one"), user("two"), user("three")];
        let outcome = manager.reduce_before_call(&mut history).await.unwrap();

        match outcome {
            ReductionOutcome::NotAttempted { utilisation_pct } => {
                assert!(utilisation_pct >= 90);
            }
            other => panic!("expected NotAttempted, got {other:?}"),
        }
        assert_eq!(
            summarizer.call_count(),
            0,
            "summarizer must not run when autocompact is disabled"
        );
    }

    #[tokio::test]
    async fn disabled_manager_returns_noop() {
        let summarizer = MockSummarizer::ok();
        let mut config = ContextConfig::default();
        config.enabled = false;
        let mut manager = ContextManager::new(
            &config,
            summarizer.clone(),
            "test-model".into(),
            SystemPromptBuilder::with_defaults(),
        );

        // High utilisation would normally trigger something.
        manager.record_usage(&UsageInfo {
            input_tokens: 96_000,
            output_tokens: 2_000,
            context_window: 100_000,
            ..Default::default()
        });

        let mut history = vec![user("a"), user("b"), user("c")];
        let outcome = manager.reduce_before_call(&mut history).await.unwrap();
        assert!(matches!(outcome, ReductionOutcome::NoOp));
        assert_eq!(summarizer.call_count(), 0);
    }

    #[test]
    fn stats_reports_snapshot() {
        let summarizer = MockSummarizer::ok();
        let mut manager = manager_with(summarizer);
        manager.record_usage(&UsageInfo {
            input_tokens: 10_000,
            output_tokens: 2_000,
            context_window: 100_000,
            ..Default::default()
        });
        manager.tick_turn();
        manager.record_tool_calls(3);

        let s = manager.stats();
        assert_eq!(s.input_tokens, 10_000);
        assert_eq!(s.output_tokens, 2_000);
        assert_eq!(s.context_window, 100_000);
        assert_eq!(s.utilisation_pct, Some(12));
        assert_eq!(s.session_memory_total_tokens, 12_000);
        assert_eq!(s.session_memory_current_turn, 1);
        assert_eq!(s.session_memory_total_tool_calls, 3);
    }
}
