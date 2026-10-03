//! Host adapter for [`tinyagents_harness::host::LearningSink`] — the seam the
//! generic agent runtime uses to hand a finished turn to the host's post-turn
//! hooks ([`crate::agent::hooks::PostTurnHook`], e.g. the ones an embedder
//! registers through `hooks::register_embedder_post_turn_hook`).
//!
//! The adapter is thin: it translates a [`TurnSummary`] into a
//! [`TurnContext`] and enqueues it through [`crate::agent::hooks::fire_hooks`],
//! which spawns every hook and returns at once.
//!
//! `TurnSummary::tools_invoked` is names-only, while `TurnContext.tool_calls`
//! carries outcomes (`success`, `duration_ms`). There is no honest projection
//! from one to the other, so `tool_calls` is left empty rather than filled
//! with invented outcomes; the names are logged only. `TurnSummary` carries a
//! thread id but no session id, so the thread id is mapped through as the
//! nearest correlation key. Errors are advisory: the turn is already
//! committed, so [`OpenHumanLearningSink::on_turn_complete`] always returns
//! `Ok(())`.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::error::Result;
use tinyagents_harness::host::{LearningSink, TurnSummary};

use crate::agent::hooks::{self, PostTurnHook, TurnContext};

/// Adapts OpenHuman's [`PostTurnHook`] fan-out to the crate's
/// [`LearningSink`] capability.
///
/// Holds the hook list rather than building it, because *which* hooks are
/// installed is a composition decision the session builder already makes (see
/// `agent/session_host/builder/factory.rs`); duplicating that policy here
/// would give the generic runtime a second, silently divergent hook set.
pub struct OpenHumanLearningSink {
    /// Hooks fired, in parallel, for every completed turn.
    hooks: Vec<Arc<dyn PostTurnHook>>,
}

impl OpenHumanLearningSink {
    /// Wraps an already-composed hook list.
    ///
    /// An empty list is legal and turns the sink into a no-op. It is *not* the
    /// way to express "this host has no learning pipeline" — the crate doc asks
    /// hosts to pass `None` for the capability in that case, so absence stays
    /// distinguishable from a sink that ran and did nothing.
    pub fn new(hooks: Vec<Arc<dyn PostTurnHook>>) -> Self {
        Self { hooks }
    }

    /// Appends one more hook.
    pub fn with_hook(mut self, hook: Arc<dyn PostTurnHook>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Number of installed hooks. Exposed for assertions and for a caller
    /// deciding whether to supply the capability at all.
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Projects a crate [`TurnSummary`] onto OpenHuman's [`TurnContext`].
    ///
    /// See the module doc for why `tool_calls` comes out empty. Kept associated
    /// and pure so the projection is testable without a runtime.
    fn turn_context_from(summary: &TurnSummary) -> TurnContext {
        TurnContext {
            user_message: summary.input.clone(),
            assistant_response: summary.output.clone(),
            // Intentionally empty — a names-only summary cannot supply the
            // `success` / `duration_ms` / `output_summary` fields that give a
            // `ToolCallRecord` its meaning. See the module doc.
            tool_calls: Vec::new(),
            // Not carried by `TurnSummary`; reporting-only fields, read by no
            // gate in any installed hook.
            turn_duration_ms: 0,
            iteration_count: 1,
            // `TurnSummary` has no session id; the thread id is the nearest
            // correlation key. Blank ids are normalized to `None` so hooks that
            // key on the session (reflection throttling) fall back to their
            // global bucket rather than keying on "".
            session_id: Some(summary.thread_id.as_str().to_string())
                .filter(|id| !id.trim().is_empty()),
            agent_id: Some(summary.agent_id.clone()).filter(|id| !id.trim().is_empty()),
            // No channel/entrypoint concept exists on the crate side.
            entrypoint: None,
        }
    }
}

#[async_trait]
impl LearningSink for OpenHumanLearningSink {
    /// Enqueues the turn onto OpenHuman's post-turn hook fan-out and returns.
    ///
    /// Always `Ok(())`. `fire_hooks` spawns each hook on the tokio runtime and
    /// logs its failure, so there is nothing fallible left to report — and the
    /// trait doc forbids using an `Err` as a veto in any case, since the turn is
    /// already committed by the time this runs.
    async fn on_turn_complete(&self, summary: &TurnSummary) -> Result<()> {
        if self.hooks.is_empty() {
            return Ok(());
        }

        // Names only. This log line is the sole place the summary's tool list
        // is surfaced, precisely because it must not reach a persistence path
        // as a fabricated outcome.
        log::debug!(
            "[tinyagents][learning] turn complete thread={} agent={} tools=[{}] hooks={}",
            summary.thread_id.as_str(),
            summary.agent_id,
            summary.tools_invoked.join(","),
            self.hooks.len()
        );

        hooks::fire_hooks(&self.hooks, Self::turn_context_from(summary));
        Ok(())
    }
}

#[cfg(test)]
#[path = "learning_sink_tests.rs"]
mod tests;
