//! An agent's removal state: whether it still accepts turns, how many are in
//! flight, and the teardown of the per-agent state the core keeps for it.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{watch, Notify};

use crate::CoreError;

/// Turn admission and in-flight accounting for one agent.
pub(crate) struct Lifecycle {
    removed: watch::Sender<bool>,
    approvals: Arc<ApprovalState>,
    in_flight: AtomicUsize,
    idle: Notify,
    torn_down: AtomicBool,
}

/// Serializes live approval decisions with the instance's removal claim.
#[derive(Debug, Default)]
pub(crate) struct ApprovalState {
    claimed: AtomicBool,
    decisions: Mutex<()>,
    scope: Arc<openhuman_core::security::approval::ApprovalScope>,
}

impl ApprovalState {
    pub(crate) fn with_live<T>(&self, decide: impl FnOnce() -> T) -> Option<T> {
        let _decision = self
            .decisions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.scope.is_closed() || self.claimed.load(Ordering::SeqCst) {
            None
        } else {
            Some(decide())
        }
    }

    fn claim_removal(&self, reason: &str) -> bool {
        // Publish closing before either mutex can block. Admission and all
        // approval surfaces share that irreversible, nonblocking observation.
        // Accepted core work finishes before the denial snapshot; accepted
        // facade work finishes before this caller can claim that snapshot.
        self.scope.close(reason);
        let _decision = self
            .decisions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !self.claimed.swap(true, Ordering::SeqCst)
    }
}

impl Lifecycle {
    pub(crate) fn new() -> Self {
        Self {
            removed: watch::Sender::new(false),
            approvals: Arc::new(ApprovalState::default()),
            in_flight: AtomicUsize::new(0),
            idle: Notify::new(),
            torn_down: AtomicBool::new(false),
        }
    }

    /// Stops admitting turns and ends the ones in flight; returns whether this
    /// caller claimed removal before another caller.
    pub(crate) fn mark_removed(&self, reason: &str) -> bool {
        self.mark_removed_with(reason, || {})
    }

    /// Claim removal and settle owned approvals before waking in-flight turns.
    pub(crate) fn mark_removed_with(&self, reason: &str, before_notify: impl FnOnce()) -> bool {
        if !self.approvals.claim_removal(reason) {
            return false;
        }
        before_notify();
        // A cancelled turn can retain the watch read while its future drops.
        // Only the first claimant writes; repeated teardown never waits on it.
        self.removed.send_replace(true);
        true
    }

    pub(crate) fn approval_state(&self) -> Arc<ApprovalState> {
        self.approvals.clone()
    }

    /// The same instance barrier captured by every request at the shared gate.
    pub(crate) fn approval_scope(&self) -> Arc<openhuman_core::security::approval::ApprovalScope> {
        self.approvals.scope.clone()
    }

    /// Bind handles to this agent instance, even after its public id is reused.
    pub(crate) fn removed(&self) -> watch::Receiver<bool> {
        self.removed.subscribe()
    }

    /// Runs `turn` unless the agent was removed, ending it early with
    /// [`CoreError::AgentRemoved`] if the agent is removed meanwhile.
    pub(crate) async fn admit<T>(
        &self,
        agent_id: &str,
        method: &'static str,
        turn: impl std::future::Future<Output = Result<T, CoreError>>,
    ) -> Result<T, CoreError> {
        let removed = || CoreError::AgentRemoved {
            method,
            agent_id: agent_id.to_string(),
        };
        let mut watcher = self.removed.subscribe();
        // Claiming removal closes admission immediately; waking existing turns
        // waits until their approvals have been settled with the removal reason.
        if self.approvals.scope.is_closed() || *watcher.borrow_and_update() {
            log::debug!("[embed][agent] turn refused: agent removed id={agent_id}");
            return Err(removed());
        }
        let _in_flight = InFlight::enter(self);
        tokio::select! {
            biased;
            _ = watcher.wait_for(|removed| *removed) => {
                log::debug!("[embed][agent] in-flight turn ended: agent removed id={agent_id}");
                Err(removed())
            }
            outcome = turn => outcome,
        }
    }

    /// Waits until no turn is in flight, for at most `limit`. Returns whether
    /// the agent went idle in time.
    pub(crate) async fn wait_idle(&self, limit: Duration) -> bool {
        let wait = async {
            loop {
                let notified = self.idle.notified();
                if self.in_flight.load(Ordering::SeqCst) == 0 {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(limit, wait).await.is_ok()
    }

    /// Claims the one-time teardown. `false` when it already ran.
    pub(crate) fn begin_teardown(&self) -> bool {
        !self.torn_down.swap(true, Ordering::SeqCst)
    }
}

struct InFlight<'a>(&'a Lifecycle);

impl<'a> InFlight<'a> {
    fn enter(lifecycle: &'a Lifecycle) -> Self {
        lifecycle.in_flight.fetch_add(1, Ordering::SeqCst);
        Self(lifecycle)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if self.0.in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
