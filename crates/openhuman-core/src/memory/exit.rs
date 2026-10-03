//! What memory does on the way out of the process.
//!
//! Conversation turns are buffered per thread until `batch_turns` or
//! `idle_secs` is reached ([`super::conversations`]), so a quit would drop
//! whatever is still buffered. This stores it, bounded: a quit that hangs on an
//! unreachable engine is worse than losing the last few turns of a thread.

use std::time::Duration;

use crate::config::Config;

/// The whole of memory's exit work must fit in this. The Tauri shell sizes its
/// drain wait from it, plus a short margin for the drain itself.
pub const EXIT_BUDGET: Duration = Duration::from_secs(2);

/// Stores every buffered conversation turn within [`EXIT_BUDGET`].
///
/// Returns how many thread batches were flushed, or `None` when the budget ran
/// out first (the remaining turns are lost; nothing else is affected).
pub async fn run(config: &Config) -> Option<usize> {
    run_within(config, EXIT_BUDGET).await
}

/// [`run`] with an explicit budget.
pub async fn run_within(config: &Config, budget: Duration) -> Option<usize> {
    match tokio::time::timeout(budget, super::conversations::flush_all(config)).await {
        Ok(flushed) => {
            log::debug!("[memory:exit] flushed {flushed} buffered conversation batch(es)");
            Some(flushed)
        }
        Err(_) => {
            log::warn!("[memory:exit] conversation flush exceeded {budget:?}; dropping the rest");
            None
        }
    }
}

#[cfg(test)]
#[path = "exit_tests.rs"]
mod tests;
