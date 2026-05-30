//! Max-iteration checkpoint seam.
//!
//! When a turn exhausts its iteration budget the three callers diverge:
//!
//! * the channel/CLI loop returns the typed `AgentError::MaxIterationsExceeded`
//!   so `Agent::run_single` can downcast and suppress Sentry noise
//!   ([`ErrorCheckpoint`]);
//! * the subagent and `Agent::turn` instead summarize the run-so-far into a
//!   resumable checkpoint string and return it as the turn's result (the
//!   `SummarizeCheckpoint`, landed with the subagent/Agent migrations).
//!
//! [`CheckpointStrategy::on_max_iter`] receives the accumulated tool digest so a
//! summarizing strategy can produce a root-cause-aware checkpoint.

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::inference::provider::UsageInfo;

/// A checkpoint result. `usage`, when present, is the provider usage from a
/// summarization call the strategy made — the engine folds it into the turn's
/// cost and reports it to the observer so token accounting stays complete.
pub(crate) struct CheckpointOutcome {
    pub text: String,
    pub usage: Option<UsageInfo>,
}

#[async_trait]
pub(crate) trait CheckpointStrategy: Send + Sync {
    /// Produce the turn's result after the iteration cap is hit, or return an
    /// error to surface the cap to the caller. `digest` is the accumulated
    /// `tool → outcome` summary of the run so far.
    async fn on_max_iter(&self, digest: &str, max_iterations: usize) -> Result<CheckpointOutcome>;
}

/// Surface the cap as the typed [`AgentError::MaxIterationsExceeded`], boxed
/// through `anyhow::Error`, so downstream wrappers — notably
/// `Agent::run_single` — can downcast and suppress Sentry emission for this
/// deterministic agent-state outcome (OPENHUMAN-TAURI-99 / -98).
pub(crate) struct ErrorCheckpoint;

#[async_trait]
impl CheckpointStrategy for ErrorCheckpoint {
    async fn on_max_iter(&self, _digest: &str, max_iterations: usize) -> Result<CheckpointOutcome> {
        Err(anyhow::Error::new(
            crate::openhuman::agent::error::AgentError::MaxIterationsExceeded {
                max: max_iterations,
            },
        ))
    }
}
