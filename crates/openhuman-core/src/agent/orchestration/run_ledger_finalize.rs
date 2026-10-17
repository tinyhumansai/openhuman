//! Global-bus finalizer that settles subagent rows in the run ledger.
//!
//! # Why this exists
//!
//! Every spawn path (`spawn_subagent`, `spawn_async_subagent`,
//! `spawn_parallel_agents`, `continue_subagent`, `dispatch`) creates an
//! `agent_runs` row at `status = "running"` and, on completion or user-input
//! pause, fires **both**:
//!   * `BUS.publish(DomainEvent::SubagentCompleted/Failed/AwaitingUser)` —
//!     the global bus,
//!   * `progress_sink.send(AgentProgress::SubagentCompleted/Failed/AwaitingUser)`
//!     — the *spawning turn's* progress channel.
//!
//! The run-ledger's terminal transition used to live **only** in the per-turn
//! [`progress_bridge`](crate::web_chat::progress_bridge),
//! which consumes that progress channel. That works for synchronous subagents
//! (they finish inside the turn, sink still alive) but **leaks** for detached
//! `spawn_async_subagent` runs: they outlive the parent turn, so when they
//! finish the progress sink is already dropped and `let _ = tx.send(...)` fails
//! silently. The ledger row stays `running` forever, and every thread reopen
//! re-renders it as a perpetual agent timeline row.
//!
//! This subscriber closes that gap by settling the ledger from the **global
//! bus**, which always fires from the detached task regardless of the parent
//! turn's lifecycle. It is idempotent with the progress-bridge path: a run that
//! was already settled there is simply re-stamped to the same lifecycle status.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinyagents_session::run_ledger::AgentRunStatus;
use tinybus::EventHandler;

const LOG_PREFIX: &str = "[run_ledger][finalize]";

/// Subscribes to subagent lifecycle [`DomainEvent`]s and transitions the
/// matching `agent_runs` row to the projected lifecycle status. Holds its own [`Config`]
/// clone so it can reach the session DB from the global-bus context (handlers
/// receive no config).
struct RunLedgerFinalizeSubscriber {
    config: Config,
}

#[async_trait]
impl EventHandler<DomainEvent> for RunLedgerFinalizeSubscriber {
    fn name(&self) -> &str {
        "agent_orchestration::run_ledger_finalize"
    }

    async fn handle(&self, event: &DomainEvent) {
        let (task_id, status, error, completed_at) = match event {
            DomainEvent::SubagentCompleted { task_id, .. } => (
                task_id.clone(),
                AgentRunStatus::Completed,
                None,
                Some(chrono::Utc::now()),
            ),
            DomainEvent::SubagentFailed { task_id, error, .. } => (
                task_id.clone(),
                AgentRunStatus::Failed,
                Some(error.clone()),
                Some(chrono::Utc::now()),
            ),
            DomainEvent::SubagentAwaitingUser { task_id, .. } => {
                (task_id.clone(), AgentRunStatus::AwaitingUser, None, None)
            }
            _ => return,
        };

        let result = crate::agent::session_store::with_ledger_async(
            &self.config.workspace_dir,
            |ledger| async move {
                ledger
                    .transition_agent_run_status(&task_id, status, error.as_deref(), completed_at)
                    .await
                    .map(|run| (task_id, run))
            },
        )
        .await;

        match result {
            Ok((task_id, Some(_))) => {
                log::debug!(
                    "{LOG_PREFIX} settled run id={task_id} status={}",
                    status.as_str()
                );
            }
            Ok((task_id, None)) => {
                // No row matched — the spawn upsert never reached the ledger, or
                // the run was deleted. Nothing to settle.
                log::debug!("{LOG_PREFIX} no run row to settle id={task_id}");
            }
            Err(err) => {
                log::warn!("{LOG_PREFIX} failed to settle run: {err}");
            }
        }
    }
}

/// Register the run-ledger finalizer on the global event bus. Leaks the
/// subscription handle so it lives for the whole process (its `Drop` would
/// cancel the subscriber). Called once from `register_domain_subscribers`.
pub(crate) fn register_run_ledger_finalize_subscriber(config: &Config) {
    if let Some(handle) = BUS.subscribe(Arc::new(RunLedgerFinalizeSubscriber {
        config: config.clone(),
    })) {
        std::mem::forget(handle);
        log::info!("{LOG_PREFIX} run-ledger finalize subscriber registered");
    } else {
        log::warn!(
            "{LOG_PREFIX} failed to register run-ledger finalize subscriber — bus not initialized"
        );
    }
}

#[cfg(test)]
#[path = "run_ledger_finalize_tests.rs"]
mod tests;
