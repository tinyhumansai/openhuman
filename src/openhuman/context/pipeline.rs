//! The layered context pipeline orchestrator.
//!
//! Ordered reduction chain applied before each provider hit:
//!
//! 1. **Tool-result budget** — applied inline in `Agent::execute_tool_call`
//!    (not here). Oversized tool results are truncated before they enter
//!    history, so they never show up as a pipeline stage.
//! 2. **Snip compact** — hard cap on message count. Implemented by the
//!    pre-existing `Agent::trim_history`; the pipeline leaves it to the
//!    caller because trimming is a terminal fallback.
//! 3. **Microcompact** — this module. Runs when `ContextGuard` reports
//!    `CompactionNeeded` (soft threshold). Replaces the payload of older
//!    `ToolResults` envelopes with a placeholder, preserving the
//!    `AssistantToolCalls ⇔ ToolResults` API invariant.
//! 4. **Autocompact** — prose summarisation of older messages.
//!    OpenHuman's existing `auto_compact_history` lives in
//!    `agent/loop_/history.rs` and operates on `ChatMessage` (not
//!    `ConversationMessage`), so we don't call it here — the pipeline
//!    instead signals a `PipelineOutcome::AutocompactionRequested` to
//!    the caller and trusts the caller to dispatch its own summariser
//!    when ready. Keeping the pipeline pure (no LLM calls) means the
//!    integration tests can exercise every stage without a provider.
//! 5. **Session memory** — handled separately by
//!    [`crate::openhuman::context::session_memory`].
//!
//! # Cache contract
//!
//! Stages 1–2 are byte-neutral with respect to previously-sent history
//! (stage 1 applies to a fresh tool result before insertion; stage 2 is
//! a terminal trim). Stages 3–4 deliberately mutate previously-sent
//! history and therefore break the KV-cache prefix; they run **only
//! when the context guard says we'd otherwise bust the window**. Each
//! firing resets the stable prefix to the new, smaller history so
//! subsequent turns hit the cache again.

use super::guard::{ContextCheckResult, ContextGuard};
use super::microcompact::{microcompact, MicrocompactStats, DEFAULT_KEEP_RECENT_TOOL_RESULTS};
use super::session_memory::{SessionMemoryConfig, SessionMemoryState};
use crate::openhuman::providers::{ConversationMessage, UsageInfo};
use std::sync::{Arc, Mutex};

/// Shared handle to a [`SessionMemoryState`] so both the synchronous
/// pipeline path and a detached background archivist task can inspect
/// and mutate the same extraction bookkeeping without fighting over
/// `&mut self`. The pipeline clones this `Arc` into every task it
/// spawns — the `Mutex` lock is only held for microsecond-scale state
/// flips, so contention is negligible in practice.
pub type SessionMemoryHandle = Arc<Mutex<SessionMemoryState>>;

/// Pipeline configuration. Defaults are tuned for an `agentic-v1`
/// 128k-context run.
#[derive(Debug, Clone, Copy)]
pub struct ContextPipelineConfig {
    /// Number of recent `ToolResults` envelopes microcompact leaves
    /// untouched. See [`DEFAULT_KEEP_RECENT_TOOL_RESULTS`].
    pub microcompact_keep_recent: usize,
    /// Whether to surface the microcompact pass in the pipeline
    /// outcome. When `false` the pipeline skips stage 3 entirely —
    /// useful for tests that want to exercise autocompaction in
    /// isolation.
    pub microcompact_enabled: bool,
    /// Whether the pipeline should report an autocompaction request
    /// when the guard says we're at the hard threshold. When `false`
    /// the pipeline silently tolerates an exhausted context (the caller
    /// is expected to surface the error via the guard directly).
    pub autocompact_enabled: bool,
    /// Session-memory extraction tunables.
    pub session_memory: SessionMemoryConfig,
}

impl Default for ContextPipelineConfig {
    fn default() -> Self {
        Self {
            microcompact_keep_recent: DEFAULT_KEEP_RECENT_TOOL_RESULTS,
            microcompact_enabled: true,
            autocompact_enabled: true,
            session_memory: SessionMemoryConfig::default(),
        }
    }
}

/// Outcome of a single pipeline pass, returned to the caller so it can
/// log/telemeter what happened and decide whether to trigger an
/// autocompaction summariser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineOutcome {
    /// No stage fired — either the guard is happy or the history is
    /// already small enough.
    NoOp,
    /// Microcompact cleared at least one older `ToolResults` envelope.
    Microcompacted(MicrocompactStats),
    /// The guard reports we're above the soft threshold and
    /// microcompact wasn't enough (or was disabled). The caller should
    /// invoke its autocompaction summariser.
    AutocompactionRequested {
        /// The last-known context utilisation as a 0..=100 percentage.
        utilisation_pct: u8,
    },
    /// The guard is above the soft threshold but autocompaction is
    /// disabled by config, so no summariser will run. Surfaced as a
    /// distinct variant so the caller can log/observe the situation
    /// instead of silently falling back to `NoOp`.
    AutocompactionDisabled { utilisation_pct: u8 },
    /// The guard's circuit breaker is tripped and the context is still
    /// above the hard threshold — the caller should abort the turn.
    ContextExhausted { utilisation_pct: u8, reason: String },
}

/// Stateful orchestrator. Owns a [`ContextGuard`] and a
/// [`SessionMemoryState`] so a single instance can live on the `Agent`
/// across turns without threading state through every call site.
///
/// `session_memory` is wrapped in a shared handle so a detached
/// archivist task spawned from `turn.rs` can mark the extraction as
/// complete or failed after the pipeline's synchronous path has
/// already released its borrow on `self`.
#[derive(Debug)]
pub struct ContextPipeline {
    pub config: ContextPipelineConfig,
    pub guard: ContextGuard,
    pub session_memory: SessionMemoryHandle,
}

impl Default for ContextPipeline {
    fn default() -> Self {
        Self::new(ContextPipelineConfig::default())
    }
}

impl ContextPipeline {
    pub fn new(config: ContextPipelineConfig) -> Self {
        Self {
            config,
            guard: ContextGuard::new(),
            session_memory: Arc::new(Mutex::new(SessionMemoryState::default())),
        }
    }

    /// Feed the latest provider `UsageInfo` into both the guard and the
    /// session-memory state.
    pub fn record_usage(&mut self, usage: &UsageInfo) {
        self.guard.update_usage(usage);
        let total = usage.input_tokens + usage.output_tokens;
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.record_usage(total);
        }
    }

    /// Bump the session-memory turn counter. Called once per user turn.
    pub fn tick_turn(&mut self) {
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.tick_turn();
        }
    }

    /// Accumulate a turn's tool-call count into the session-memory
    /// state. Called once per user turn after tool dispatch settles.
    pub fn record_tool_calls(&mut self, n: usize) {
        if let Ok(mut sm) = self.session_memory.lock() {
            sm.record_tool_calls(n);
        }
    }

    /// Should the caller spawn a background session-memory extraction
    /// this turn?
    pub fn should_extract_session_memory(&self) -> bool {
        self.session_memory
            .lock()
            .map(|sm| sm.should_extract(&self.config.session_memory))
            .unwrap_or(false)
    }

    /// Read-only snapshot of the session-memory bookkeeping for
    /// observability / [`crate::openhuman::context::ContextStats`].
    pub fn session_memory_snapshot(&self) -> SessionMemoryState {
        self.session_memory
            .lock()
            .map(|sm| sm.clone())
            .unwrap_or_default()
    }

    /// Share a clone of the session-memory handle. The caller takes
    /// ownership of the `Arc` and can move it into a detached
    /// background task to update the extraction state when the task
    /// finishes. See `turn.rs::spawn_session_memory_extraction`.
    pub fn session_memory_handle(&self) -> SessionMemoryHandle {
        Arc::clone(&self.session_memory)
    }

    /// Run the reduction chain against `history` in place. Safe to call
    /// before every provider hit — it's cheap when the guard is happy.
    pub fn run_before_call(&mut self, history: &mut [ConversationMessage]) -> PipelineOutcome {
        match self.guard.check() {
            ContextCheckResult::Ok => PipelineOutcome::NoOp,
            ContextCheckResult::CompactionNeeded => {
                // Stage 3: microcompact the older tool results.
                if self.config.microcompact_enabled {
                    let stats = microcompact(history, self.config.microcompact_keep_recent);
                    if stats.envelopes_cleared > 0 {
                        // A successful reduction should reset the guard's
                        // circuit breaker so a previous string of
                        // autocompaction failures doesn't leave the
                        // breaker tripped after we've just freed tokens.
                        self.guard.record_compaction_success();
                        tracing::info!(
                            envelopes_cleared = stats.envelopes_cleared,
                            entries_cleared = stats.entries_cleared,
                            bytes_freed = stats.bytes_freed,
                            "[context_pipeline] microcompact fired"
                        );
                        return PipelineOutcome::Microcompacted(stats);
                    }
                }

                // Stage 4: if microcompact didn't free anything (no old
                // tool results to clear), signal autocompaction to the
                // caller. The pipeline deliberately does not issue the
                // LLM call itself. When autocompact is disabled we
                // still surface the situation as a distinct variant so
                // the manager can log/observe it rather than silently
                // dropping back to `NoOp`.
                let pct = self
                    .guard
                    .utilization()
                    .map(|u| (u * 100.0).round() as u8)
                    .unwrap_or(0);
                if self.config.autocompact_enabled {
                    tracing::info!(
                        utilisation_pct = pct,
                        "[context_pipeline] autocompaction requested"
                    );
                    return PipelineOutcome::AutocompactionRequested {
                        utilisation_pct: pct,
                    };
                }

                tracing::warn!(
                    utilisation_pct = pct,
                    "[context_pipeline] above soft threshold but autocompact disabled"
                );
                PipelineOutcome::AutocompactionDisabled {
                    utilisation_pct: pct,
                }
            }
            ContextCheckResult::ContextExhausted {
                utilization_pct,
                reason,
            } => PipelineOutcome::ContextExhausted {
                utilisation_pct: utilization_pct,
                reason,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::microcompact::CLEARED_PLACEHOLDER;
    use super::*;
    use crate::openhuman::providers::{
        ChatMessage, ConversationMessage, ToolCall, ToolResultMessage, UsageInfo,
    };

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

    fn user(text: &str) -> ConversationMessage {
        ConversationMessage::Chat(ChatMessage::user(text))
    }

    fn set_high_utilisation(pipeline: &mut ContextPipeline) {
        pipeline.record_usage(&UsageInfo {
            input_tokens: 92_000,
            output_tokens: 4_000,
            context_window: 100_000,
        });
    }

    #[test]
    fn noop_when_guard_is_ok() {
        let mut pipeline = ContextPipeline::default();
        pipeline.record_usage(&UsageInfo {
            input_tokens: 10_000,
            output_tokens: 1_000,
            context_window: 100_000,
        });
        let mut history = vec![
            user("hi"),
            call("t1"),
            result("t1", "x".repeat(2_000).as_str()),
        ];
        let outcome = pipeline.run_before_call(&mut history);
        assert_eq!(outcome, PipelineOutcome::NoOp);
    }

    #[test]
    fn microcompact_fires_at_soft_threshold_when_there_are_old_tool_results() {
        let mut pipeline = ContextPipeline::default();
        let mut history = vec![
            call("t1"),
            result("t1", &"x".repeat(5_000)),
            call("t2"),
            result("t2", &"x".repeat(5_000)),
            call("t3"),
            result("t3", "recent-1"),
            call("t4"),
            result("t4", "recent-2"),
            call("t5"),
            result("t5", "recent-3"),
            call("t6"),
            result("t6", "recent-4"),
            call("t7"),
            result("t7", "recent-5"),
        ];
        set_high_utilisation(&mut pipeline);
        let outcome = pipeline.run_before_call(&mut history);
        match outcome {
            PipelineOutcome::Microcompacted(stats) => {
                assert_eq!(stats.envelopes_cleared, 2);
                assert!(stats.bytes_freed > 9_000);
            }
            other => panic!("expected Microcompacted, got {other:?}"),
        }
        // Older entries are cleared, newer ones are preserved.
        match &history[1] {
            ConversationMessage::ToolResults(r) => {
                assert_eq!(r[0].content, CLEARED_PLACEHOLDER)
            }
            _ => panic!(),
        }
        match &history[13] {
            ConversationMessage::ToolResults(r) => assert_eq!(r[0].content, "recent-5"),
            _ => panic!(),
        }
    }

    #[test]
    fn autocompaction_requested_when_no_old_tool_results_to_clear() {
        let mut pipeline = ContextPipeline::default();
        // Soft threshold crossed but there are zero ToolResults to clear.
        set_high_utilisation(&mut pipeline);
        let mut history = vec![user("one"), user("two"), user("three")];
        let outcome = pipeline.run_before_call(&mut history);
        match outcome {
            PipelineOutcome::AutocompactionRequested { utilisation_pct } => {
                assert!(utilisation_pct >= 90);
            }
            other => panic!("expected AutocompactionRequested, got {other:?}"),
        }
    }

    #[test]
    fn autocompaction_requested_when_only_recent_tool_results_exist() {
        // All tool results fall within `keep_recent`, so microcompact
        // has nothing to clear and the pipeline falls through to
        // autocompaction.
        let mut pipeline = ContextPipeline::default();
        let mut history = vec![call("t1"), result("t1", "a"), call("t2"), result("t2", "b")];
        set_high_utilisation(&mut pipeline);
        let outcome = pipeline.run_before_call(&mut history);
        assert!(matches!(
            outcome,
            PipelineOutcome::AutocompactionRequested { .. }
        ));
    }

    #[test]
    fn microcompact_disabled_skips_to_autocompaction() {
        let mut pipeline = ContextPipeline::new(ContextPipelineConfig {
            microcompact_enabled: false,
            ..ContextPipelineConfig::default()
        });
        let mut history = vec![
            call("t1"),
            result("t1", &"x".repeat(5_000)),
            call("t2"),
            result("t2", "recent"),
        ];
        set_high_utilisation(&mut pipeline);
        let outcome = pipeline.run_before_call(&mut history);
        assert!(matches!(
            outcome,
            PipelineOutcome::AutocompactionRequested { .. }
        ));
        // History must be untouched when microcompact is disabled.
        if let ConversationMessage::ToolResults(r) = &history[1] {
            assert_eq!(r[0].content.len(), 5_000);
        } else {
            panic!();
        }
    }

    #[test]
    fn exhausted_context_propagates_to_caller() {
        let mut pipeline = ContextPipeline::default();
        pipeline.record_usage(&UsageInfo {
            input_tokens: 96_000,
            output_tokens: 2_000,
            context_window: 100_000,
        });
        // Trip the circuit breaker.
        pipeline.guard.record_compaction_failure();
        pipeline.guard.record_compaction_failure();
        pipeline.guard.record_compaction_failure();

        let mut history = vec![user("hi")];
        let outcome = pipeline.run_before_call(&mut history);
        assert!(matches!(outcome, PipelineOutcome::ContextExhausted { .. }));
    }

    #[test]
    fn record_usage_feeds_session_memory() {
        let mut pipeline = ContextPipeline::default();
        pipeline.record_usage(&UsageInfo {
            input_tokens: 10_000,
            output_tokens: 2_000,
            context_window: 100_000,
        });
        assert_eq!(pipeline.session_memory.total_tokens, 12_000);
    }

    #[test]
    fn tick_turn_and_record_tool_calls_affect_session_memory() {
        let mut pipeline = ContextPipeline::default();
        pipeline.tick_turn();
        pipeline.record_tool_calls(5);
        assert_eq!(pipeline.session_memory.current_turn, 1);
        assert_eq!(pipeline.session_memory.total_tool_calls, 5);
    }
}
