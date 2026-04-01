//! Graceful interrupt fence — handles SIGINT / Ctrl+C and `/stop` commands.
//!
//! The interrupt fence is checked at key points in the orchestrator loop:
//! - Before each DAG level execution
//! - Before each tool execution in the tool loop
//! - Inside sub-agent spawn points
//!
//! On interrupt, running sub-agents are cancelled, memory is flushed,
//! and the Archivist fires with partial context.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Thread-safe interrupt flag that can be checked throughout the agent harness.
#[derive(Clone)]
pub struct InterruptFence {
    flag: Arc<AtomicBool>,
}

impl InterruptFence {
    /// Create a new interrupt fence (not triggered).
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Check whether an interrupt has been requested.
    pub fn is_interrupted(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Trigger the interrupt (called from signal handler or `/stop` command).
    pub fn trigger(&self) {
        self.flag.store(true, Ordering::Relaxed);
        tracing::info!("[interrupt] interrupt fence triggered");
    }

    /// Reset the fence (e.g. at the start of a new session).
    pub fn reset(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    /// Get a raw `Arc<AtomicBool>` handle for passing to signal handlers.
    pub fn flag_handle(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }

    /// Install a `tokio::signal::ctrl_c()` handler that triggers this fence.
    ///
    /// This spawns a background task that waits for Ctrl+C and sets the flag.
    /// The task runs until the process exits.
    pub fn install_signal_handler(&self) {
        let flag = self.flag.clone();
        tokio::spawn(async move {
            loop {
                match tokio::signal::ctrl_c().await {
                    Ok(()) => {
                        if flag.load(Ordering::Relaxed) {
                            // Second Ctrl+C — hard exit.
                            tracing::warn!("[interrupt] second Ctrl+C received, forcing exit");
                            std::process::exit(130);
                        }
                        flag.store(true, Ordering::Relaxed);
                        tracing::info!(
                            "[interrupt] Ctrl+C received — gracefully stopping. Press again to force exit."
                        );
                    }
                    Err(e) => {
                        tracing::error!("[interrupt] failed to listen for Ctrl+C: {e}");
                        break;
                    }
                }
            }
        });
    }
}

impl Default for InterruptFence {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when an operation is cancelled due to an interrupt.
#[derive(Debug, thiserror::Error)]
#[error("operation interrupted by user")]
pub struct InterruptedError;

/// Helper: check the fence and return `Err(InterruptedError)` if triggered.
pub fn check_interrupt(fence: &InterruptFence) -> Result<(), InterruptedError> {
    if fence.is_interrupted() {
        Err(InterruptedError)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_starts_clear() {
        let fence = InterruptFence::new();
        assert!(!fence.is_interrupted());
    }

    #[test]
    fn trigger_sets_flag() {
        let fence = InterruptFence::new();
        fence.trigger();
        assert!(fence.is_interrupted());
    }

    #[test]
    fn reset_clears_flag() {
        let fence = InterruptFence::new();
        fence.trigger();
        fence.reset();
        assert!(!fence.is_interrupted());
    }

    #[test]
    fn check_interrupt_returns_err_when_triggered() {
        let fence = InterruptFence::new();
        assert!(check_interrupt(&fence).is_ok());
        fence.trigger();
        assert!(check_interrupt(&fence).is_err());
    }

    #[test]
    fn clone_shares_flag() {
        let fence = InterruptFence::new();
        let clone = fence.clone();
        fence.trigger();
        assert!(clone.is_interrupted());
    }
}
