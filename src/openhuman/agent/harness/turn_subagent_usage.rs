//! Task-local collector for **sub-agent token/cost spend within the current
//! turn**.
//!
//! When a parent agent delegates to a sub-agent mid-turn, the sub-agent's
//! provider calls are accounted by its own [`SubagentObserver`] and never reach
//! the parent's `AgentObserver` cumulative totals (the two run on separate
//! observers). Without a bridge, sub-agent spend is invisible to the
//! session-level token/cost meters surfaced in the UI footer, and to the global
//! cost tracker.
//!
//! This module installs an [`Arc<Mutex<Vec<SubagentUsageEntry>>>`] as a
//! task-local around the parent's turn future (the
//! `run_turn_via_tinyagents_shared` drive). Synchronous delegations
//! (`spawn_subagent`) run inline on the same tokio task, so the sub-agent
//! runner can [`record_subagent_usage`] its totals into the active collector.
//! After the turn returns, the parent [`drain`]s the collector to:
//!
//! 1. fold child tokens + USD into the turn's cumulative meters, and
//! 2. attribute per-child spend for the `chat_done` breakdown (hover detail).
//!
//! Background / async sub-agents (`spawn_async_subagent`) run on detached tasks
//! that do **not** inherit this task-local; their spend completes after the
//! parent turn's `chat_done` and is captured by the global cost tracker
//! instead. [`current_collector`] returns `None` outside any scope (CLI /
//! direct invocation / tests), so recording is strictly additive.

use std::sync::{Arc, Mutex};

use super::subagent_runner::SubagentUsage;

/// One sub-agent's spend, tagged with its identity for the per-child breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentUsageEntry {
    pub task_id: String,
    pub agent_id: String,
    pub usage: SubagentUsage,
}

/// Holistic token/cost accounting for a single completed turn, including any
/// sub-agent spend rolled in. Captured on the session at turn end and consumed
/// by the web-channel delivery layer, which forwards it on the `chat_done`
/// event so the UI footer can show session tokens, context-window utilisation,
/// USD cost, and a per-sub-agent hover breakdown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LastTurnUsage {
    /// Input (prompt) tokens for the turn, parent + sub-agents.
    pub input_tokens: u64,
    /// Output (completion) tokens for the turn, parent + sub-agents.
    pub output_tokens: u64,
    /// Cached-input tokens for the turn, parent + sub-agents.
    pub cached_input_tokens: u64,
    /// USD cost for the turn (backend-charged where available, else estimated),
    /// parent + sub-agents.
    pub cost_usd: f64,
    /// The model's context window for this turn (`0` when unknown, e.g. a cloud
    /// model whose window the core couldn't resolve). Lets the UI show real
    /// context utilisation instead of a hard-coded default.
    pub context_window: u64,
    /// Per-sub-agent spend gathered during the turn, for the hover breakdown.
    pub subagents: Vec<SubagentUsageEntry>,
}

/// Shared, mutable list of sub-agent spend gathered during one parent turn.
type TurnSubagentUsage = Arc<Mutex<Vec<SubagentUsageEntry>>>;

tokio::task_local! {
    /// Active per-turn sub-agent usage collector, installed around the parent's
    /// turn future (`run_turn_via_tinyagents_shared`). Absent outside a turn
    /// scope.
    static TURN_SUBAGENT_USAGE: TurnSubagentUsage;
}

/// The collector active for the current turn, or `None` when no scope is in
/// effect (e.g. a sub-agent running on a detached background task, or a direct
/// CLI invocation).
fn current_collector() -> Option<TurnSubagentUsage> {
    TURN_SUBAGENT_USAGE.try_with(|c| c.clone()).ok()
}

/// Record a finished sub-agent's token/cost totals into the active turn
/// collector. No-op when there is no active scope. Called by the sub-agent
/// runner once it has the run's aggregated usage.
pub(crate) fn record_subagent_usage(task_id: &str, agent_id: &str, usage: SubagentUsage) {
    let Some(collector) = current_collector() else {
        tracing::trace!(
            task_id,
            agent_id,
            "[turn_subagent_usage] no active collector — sub-agent spend not rolled into parent turn"
        );
        return;
    };
    let entry = SubagentUsageEntry {
        task_id: task_id.to_string(),
        agent_id: agent_id.to_string(),
        usage,
    };
    // Recover the inner vec on poison (a panic in another sub-agent) rather than
    // dropping this turn's accounting entirely.
    let mut guard = collector.lock().unwrap_or_else(|poisoned| {
        tracing::warn!(
            task_id,
            agent_id,
            "[turn_subagent_usage] collector mutex poisoned — recovering"
        );
        poisoned.into_inner()
    });
    tracing::debug!(
        task_id,
        agent_id,
        input_tokens = usage.input_tokens,
        output_tokens = usage.output_tokens,
        charged_usd = usage.charged_amount_usd,
        "[turn_subagent_usage] recorded sub-agent spend into parent turn"
    );
    guard.push(entry);
}

/// Run `future` with a fresh sub-agent usage collector installed, returning both
/// the future's output and the gathered per-child entries. Intended call site is
/// around the parent agent's turn (`run_turn_via_tinyagents_shared`) invocation.
pub(crate) async fn with_turn_collector<F, R>(future: F) -> (R, Vec<SubagentUsageEntry>)
where
    F: std::future::Future<Output = R>,
{
    let collector: TurnSubagentUsage = Arc::new(Mutex::new(Vec::new()));
    // `scope` takes the inner future **by value**, so a debug build moves the
    // whole nested turn generator through this frame on its way in. On the agent
    // turn path that nesting is hundreds of KiB, and this wrapper was the most
    // expensive of them: measured under gdb, its own frame was 239,184 bytes of
    // a 2,090,360-byte stack, against the 2 MiB thread `libtest` hands a test.
    // Pinning to the heap first means only a pointer moves. Boxing here and at
    // the four sibling `scope` wrappers took the same stack to 731,928 bytes and
    // fixed 131 tests that aborted on a stack overflow; the cost is one
    // allocation per scope, against a turn that is about to call a model.
    let out = TURN_SUBAGENT_USAGE
        .scope(collector.clone(), Box::pin(future))
        .await;
    let entries = collector
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|p| p.into_inner().clone());
    (out, entries)
}

#[cfg(test)]
#[path = "turn_subagent_usage_tests.rs"]
mod tests;
