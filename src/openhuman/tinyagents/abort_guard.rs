//! RAII guard that aborts a spawned task when it is dropped (issue #4460).
//!
//! `ProviderModel::stream` runs the provider call in a detached `tokio::spawn`
//! producer. Without a lifetime tie, a hard turn cancellation (`AbortHandle`)
//! drops the consumer stream but leaves the producer running to completion — the
//! provider call still finishes and is still billed. Holding the producer's
//! [`JoinHandle`] inside this guard, and moving the guard into the consumer
//! stream's state, ties the two lifetimes: dropping the stream (turn future
//! aborted/dropped) drops the guard, which aborts the in-flight provider call.

use tokio::task::JoinHandle;

/// Aborts the wrapped task on drop unless it has already finished.
pub(super) struct AbortOnDrop {
    handle: JoinHandle<()>,
    /// Grep-friendly label for the abort debug log (e.g. the model name).
    label: String,
}

impl AbortOnDrop {
    pub(super) fn new(handle: JoinHandle<()>, label: impl Into<String>) -> Self {
        Self {
            handle,
            label: label.into(),
        }
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if self.handle.is_finished() {
            // Producer already emitted its terminal item — nothing to abort.
            return;
        }
        tracing::debug!(
            label = %self.label,
            "[tinyagents] aborting in-flight provider stream producer on drop (turn cancelled) — #4460"
        );
        self.handle.abort();
    }
}
