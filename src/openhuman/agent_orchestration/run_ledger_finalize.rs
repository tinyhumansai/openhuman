//! Global-bus finalizer that settles subagent rows in the run ledger.
//!
//! # Why this exists
//!
//! Every spawn path (`spawn_subagent`, `spawn_async_subagent`,
//! `spawn_parallel_agents`, `continue_subagent`, `dispatch`) creates an
//! `agent_runs` row at `status = "running"` and, on completion, fires **both**:
//!   * `publish_global(DomainEvent::SubagentCompleted/Failed)` — the global bus,
//!   * `progress_sink.send(AgentProgress::SubagentCompleted/Failed)` — the
//!     *spawning turn's* progress channel.
//!
//! The run-ledger's terminal transition used to live **only** in the per-turn
//! [`progress_bridge`](crate::openhuman::channels::providers::web::progress_bridge),
//! which consumes that progress channel. That works for synchronous subagents
//! (they finish inside the turn, sink still alive) but **leaks** for detached
//! `spawn_async_subagent` runs: they outlive the parent turn, so when they
//! finish the progress sink is already dropped and `let _ = tx.send(...)` fails
//! silently. The ledger row stays `running` forever, and every thread reopen
//! re-renders it as a perpetual "Tinyplace Agent" timeline row.
//!
//! This subscriber closes that gap by finalizing the ledger from the **global
//! bus**, which always fires from the detached task regardless of the parent
//! turn's lifecycle. It is idempotent with the progress-bridge path: a run that
//! was already settled there is simply re-stamped to the same terminal status.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler};
use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::{transition_agent_run_status, AgentRunStatus};

const LOG_PREFIX: &str = "[run_ledger][finalize]";

/// Subscribes to terminal subagent [`DomainEvent`]s and transitions the
/// matching `agent_runs` row to a terminal status. Holds its own [`Config`]
/// clone so it can reach the session DB from the global-bus context (handlers
/// receive no config).
struct RunLedgerFinalizeSubscriber {
    config: Config,
}

#[async_trait]
impl EventHandler for RunLedgerFinalizeSubscriber {
    fn name(&self) -> &str {
        "agent_orchestration::run_ledger_finalize"
    }

    async fn handle(&self, event: &DomainEvent) {
        let (task_id, status, error) = match event {
            DomainEvent::SubagentCompleted { task_id, .. } => {
                (task_id.clone(), AgentRunStatus::Completed, None)
            }
            DomainEvent::SubagentFailed { task_id, error, .. } => {
                (task_id.clone(), AgentRunStatus::Failed, Some(error.clone()))
            }
            _ => return,
        };

        // Single fast UPDATE, but keep it off the async runtime to honour the
        // EventHandler "must not block" contract.
        let config = self.config.clone();
        let completed_at = chrono::Utc::now();
        let result = tokio::task::spawn_blocking(move || {
            transition_agent_run_status(
                &config,
                &task_id,
                status,
                error.as_deref(),
                Some(completed_at),
            )
            .map(|run| (task_id, run))
        })
        .await;

        match result {
            Ok(Ok((task_id, Some(_)))) => {
                log::debug!(
                    "{LOG_PREFIX} settled run id={task_id} status={}",
                    status.as_str()
                );
            }
            Ok(Ok((task_id, None))) => {
                // No row matched — the spawn upsert never reached the ledger, or
                // the run was deleted. Nothing to settle.
                log::debug!("{LOG_PREFIX} no run row to settle id={task_id}");
            }
            Ok(Err(err)) => {
                log::warn!("{LOG_PREFIX} failed to settle run: {err}");
            }
            Err(join_err) => {
                log::warn!("{LOG_PREFIX} settle task panicked: {join_err}");
            }
        }
    }
}

/// Register the run-ledger finalizer on the global event bus. Leaks the
/// subscription handle so it lives for the whole process (its `Drop` would
/// cancel the subscriber). Called once from `register_domain_subscribers`.
pub fn register_run_ledger_finalize_subscriber(config: &Config) {
    if let Some(handle) = subscribe_global(Arc::new(RunLedgerFinalizeSubscriber {
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
