//! When a scheduled delivery drain may run: after its delay, once its thread
//! is idle, and only while this node still holds the owning profile's lease.
//!
//! A drain is scheduled (debounced, a retry backoff, or a recovered record
//! after a profile open) and runs later. In SaaS the profile's lease can be
//! lost in between, and another node may already have opened the profile and
//! started delivering the same records. A drain therefore carries the
//! [`LeaseFence`] of the profile that scheduled it ([`DrainFence`]) and checks
//! it after its delay, on every busy poll, and before the delivery turn; a
//! latched fence also cuts the delay short. A fenced drain is dropped, not
//! retried: its records stay pending in the profile's completion log, which
//! the new holder recovers when it opens the profile.
//!
//! Outside SaaS, and for work that serves no hosted profile, a drain has no
//! fence and always runs.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::storage::fence::LeaseFence;

use super::busy_guard::is_busy;

/// Poll interval and ceiling while a recovered drain waits for a busy thread.
pub(super) const RECOVERY_BUSY_POLL: Duration = Duration::from_secs(5);
pub(super) const RECOVERY_BUSY_POLLS: u32 = 360;

/// The lease fence a drain runs under: the owning profile's, or none (the
/// desktop, embedded agents).
#[derive(Clone, Debug, Default)]
pub(crate) struct DrainFence(Option<Arc<LeaseFence>>);

impl DrainFence {
    /// No fence: the drain always runs.
    pub(crate) fn none() -> Self {
        Self(None)
    }

    /// The fence of a profile's grant.
    pub(crate) fn of(fence: Arc<LeaseFence>) -> Self {
        Self(Some(fence))
    }

    /// The fence of the profile the calling task serves, when this node hosts
    /// it. `None` outside SaaS or outside a profile.
    pub(crate) fn current() -> Self {
        Self(crate::profiles::host::current().map(|profile| Arc::clone(profile.lease_fence())))
    }

    /// Whether this node may still act for the profile: the fence is not
    /// latched and the grant has not run out by this node's clock.
    pub(crate) fn admits(&self) -> bool {
        self.0.as_ref().is_none_or(|fence| {
            fence
                .check_local(crate::storage::fence::system_now_ms())
                .is_ok()
        })
    }

    /// Sleep for `delay`, cut short when the fence latches. `true` when the
    /// drain may go on.
    pub(crate) async fn sleep(&self, delay: Duration) -> bool {
        match &self.0 {
            None => {
                tokio::time::sleep(delay).await;
                true
            }
            Some(fence) => {
                tokio::select! {
                    () = tokio::time::sleep(delay) => self.admits(),
                    () = fence.fenced() => false,
                }
            }
        }
    }
}

/// How a drain ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum DrainOutcome {
    /// `deliver` ran.
    Ran,
    /// The profile's lease was lost; nothing ran.
    Fenced,
    /// The thread stayed busy past the poll ceiling; nothing ran.
    StillBusy,
}

/// Wait `delay`, then (with `wait_idle`) for `thread_id` to go idle, then run
/// `deliver`, unless `fence` stops admitting the profile at any point.
///
/// `wait_idle` serves a recovered record: it has no owner noted (that table is
/// process-local), so the busy turn's completion event would not wake it.
pub(super) async fn run_drain<F, Fut>(
    thread_id: &str,
    delay: Duration,
    wait_idle: bool,
    fence: &DrainFence,
    poll: Duration,
    deliver: F,
) -> DrainOutcome
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    if !fence.sleep(delay).await {
        log::info!(
            "[background_delivery] profile lease lost; dropping the drain thread_id={thread_id}"
        );
        return DrainOutcome::Fenced;
    }
    if wait_idle {
        let mut polls = 0;
        while is_busy(thread_id) && polls < RECOVERY_BUSY_POLLS {
            polls += 1;
            if !fence.sleep(poll).await {
                log::info!(
                    "[background_delivery] profile lease lost while waiting for an idle \
                     thread; dropping the drain thread_id={thread_id}"
                );
                return DrainOutcome::Fenced;
            }
        }
        if is_busy(thread_id) {
            return DrainOutcome::StillBusy;
        }
    }
    if !fence.admits() {
        log::info!(
            "[background_delivery] profile lease lost; dropping the drain thread_id={thread_id}"
        );
        return DrainOutcome::Fenced;
    }
    deliver().await;
    DrainOutcome::Ran
}

#[cfg(test)]
#[path = "delivery_drain_tests.rs"]
mod tests;
