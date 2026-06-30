//! Turn-state observer seam.
//!
//! The engine drives the loop over a `Vec<ChatMessage>` working buffer, but the
//! three callers want to *do* different things around each step:
//!
//! * the channel loop wants nothing extra ([`NullObserver`]);
//! * the subagent wants per-iteration transcript persistence, usage
//!   accumulation, and worker-thread mirroring (assistant intents, per-call
//!   results, batched text-mode results, final response);
//! * `Agent::turn` wants its `ContextManager` reduction before each dispatch,
//!   transcript persistence, and per-turn usage/cost snapshots.
//!
//! [`TurnObserver`] is the seam: every method defaults to a no-op, so an impl
//! only overrides the hooks its caller needs. The engine still owns the
//! universal concerns (stop hooks, context guard, token-budget trim, the
//! circuit breaker) inline — the observer is for caller-specific side effects.

use anyhow::Result;
use async_trait::async_trait;

use super::tool_source::ToolSource;
use crate::openhuman::agent::harness::parse::ParsedToolCall;
use crate::openhuman::inference::provider::{ChatMessage, ToolCall, UsageInfo};

#[async_trait]
pub(crate) trait TurnObserver: Send {
    /// Called before each provider dispatch, after the engine's own context
    /// guard + token-budget trim. `Agent::turn` runs its `ContextManager`
    /// reduction chain here. Default: no-op.
    async fn before_dispatch(
        &mut self,
        _history: &mut Vec<ChatMessage>,
        _tools: &mut dyn ToolSource,
        _iteration: usize,
    ) -> Result<()> {
        Ok(())
    }

    /// Called once per provider response that carried a usage block, so the
    /// caller can accumulate its own token tally / transcript usage snapshot.
    fn record_usage(&mut self, _provider: &str, _model: &str, _usage: &UsageInfo) {}

    /// Called after the assistant message for this iteration is committed to
    /// the engine's working buffer. `response_text` is the raw provider text
    /// (pre native serialization); `reasoning_content` is the thinking-model
    /// content to round-trip; `native_tool_calls` are the provider's structured
    /// calls (empty in text/prompt mode); `parsed_calls` are the engine-parsed
    /// calls (empty when `is_final`). `Agent::turn` uses these to rebuild its
    /// typed `ConversationMessage` history; the subagent mirrors to its worker
    /// thread.
    #[allow(clippy::too_many_arguments)]
    async fn on_assistant(
        &mut self,
        _display_text: &str,
        _response_text: &str,
        _reasoning_content: Option<&str>,
        _native_tool_calls: &[ToolCall],
        _parsed_calls: &[ParsedToolCall],
        _iteration: usize,
        _is_final: bool,
    ) {
    }

    /// Called after one tool's result is known, in native-tool mode (one
    /// `role:tool` message per call). Subagent mirrors per-call results to its
    /// worker thread; `Agent::turn` buffers them to rebuild typed history.
    fn on_tool_result(
        &mut self,
        _call_id: &str,
        _tool_name: &str,
        _result_text: &str,
        _success: bool,
        _iteration: usize,
    ) {
    }

    /// Called after a batched `[Tool results]` user message is committed
    /// (text/prompt mode, where there are no per-call `role:tool` messages).
    fn on_results_batch(&mut self, _content: &str, _iteration: usize) {}

    /// Called after the iteration's history is finalized (the transcript
    /// persistence point) — both after the final response and after each tool
    /// round's results are appended.
    fn after_iteration(&mut self, _history: &[ChatMessage], _iteration: usize) {}

    /// Whether an empty final response (no text, no tool calls) is acceptable.
    /// The channel/subagent loops return it as `Ok("")`; `Agent::turn` treats
    /// it as a degenerate/poisoned completion and surfaces an error instead of
    /// a silent blank reply (bug-report-2026-05-26 A1). Default: allowed.
    fn allow_empty_final(&self) -> bool {
        true
    }
}

/// No-op observer for the channel/CLI/triage loop, which keeps no extra state.
pub(crate) struct NullObserver;

impl TurnObserver for NullObserver {}
