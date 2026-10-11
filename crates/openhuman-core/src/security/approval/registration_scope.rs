//! Serializes an agent instance's approval registration and decisions with removal.

use std::sync::{Mutex, OnceLock};

/// An instance-owned barrier; reusing an agent ID requires a fresh scope.
#[derive(Debug, Default)]
pub struct ApprovalScope {
    closed: Mutex<()>,
    reason: OnceLock<String>,
}

impl ApprovalScope {
    /// Stop registration and decisions; wait for accepted work to finish.
    /// The first teardown reason remains authoritative.
    pub fn close(&self, reason: &str) {
        // Publish closure before waiting for accepted work. Turn admission and
        // callers waiting to acquire the barrier must already see removal.
        self.reason.get_or_init(|| reason.to_owned());
        let _closed = self
            .closed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    /// Whether closing has begun, without waiting for accepted work to finish.
    pub fn is_closed(&self) -> bool {
        self.reason.get().is_some()
    }

    /// Run synchronous registration or a decision while holding the removal barrier.
    pub(crate) fn with_open<T>(&self, register: impl FnOnce() -> T) -> Result<T, String> {
        let _closed = self
            .closed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reason) = self.reason.get() {
            return Err(reason.clone());
        }
        // Keep the barrier through persistence and approval event/waiter publication.
        // No asynchronous work or user callbacks run in this critical section.
        Ok(register())
    }
}

#[cfg(test)]
#[path = "registration_scope_tests.rs"]
mod tests;
