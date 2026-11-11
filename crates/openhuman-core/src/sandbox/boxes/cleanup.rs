//! Cleanups of boxes the registry no longer tracks: the pending-cleanup
//! count per owner that makes a new box wait for an old one's close or
//! forget, and the detached box a cleanup task lets go of.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::registry::Owner;
use super::BoxDriver;

/// Cleanups (closes or forgets) still running, per owner. A new box for an
/// owner waits until none is pending, so it never adopts an instance a
/// cleanup is still tearing down.
#[derive(Default)]
pub(super) struct Cleanups {
    pending: Mutex<HashMap<Owner, usize>>,
    finished: tokio::sync::Notify,
}

impl Cleanups {
    /// Count a cleanup of `owner`'s box as pending until the ticket drops.
    pub(super) fn begin(self: &Arc<Self>, owner: &Owner) -> CleanupTicket {
        *self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(owner.clone())
            .or_insert(0) += 1;
        CleanupTicket {
            cleanups: Arc::clone(self),
            owner: owner.clone(),
        }
    }

    /// Wait until no cleanup of `owner`'s box is pending.
    pub(super) async fn settle(&self, owner: &Owner) {
        self.settle_matching(|pending| pending == owner).await;
    }

    /// Wait until no cleanup of an owner `matches` is pending.
    pub(super) async fn settle_matching(&self, matches: impl Fn(&Owner) -> bool) {
        loop {
            // Registered before the check, so a cleanup finishing between the
            // check and the wait is not missed.
            let finished = self.finished.notified();
            let busy = self
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .keys()
                .any(&matches);
            if !busy {
                return;
            }
            finished.await;
        }
    }
}

/// A pending cleanup; dropping it (the cleanup finished, or its task ended)
/// lifts the hold on its owner.
struct CleanupTicket {
    cleanups: Arc<Cleanups>,
    owner: Owner,
}

impl Drop for CleanupTicket {
    fn drop(&mut self) {
        let mut pending = self
            .cleanups
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = pending.get_mut(&self.owner) {
            *count -= 1;
            if *count == 0 {
                pending.remove(&self.owner);
            }
        }
        drop(pending);
        self.cleanups.finished.notify_waiters();
    }
}

/// A box already taken out of its slot, whose cleanup is counted as pending
/// for its owner from the moment it is detached. Detaching happens while the
/// lock that removed the box is still held, so no concurrent command can
/// create a new box for the owner before this cleanup is visible.
pub(super) struct Detached {
    resource: String,
    ticket: CleanupTicket,
}

impl Cleanups {
    /// Take `resource` (of `owner`'s box) for cleanup.
    pub(super) fn detach(self: &Arc<Self>, owner: Owner, resource: String) -> Detached {
        Detached {
            resource,
            ticket: self.begin(&owner),
        }
    }
}

/// Let go of a detached box on a task of its own, on `runtime`. The task
/// completes even if the caller that asked for it is dropped; the returned
/// handle may be awaited or dropped.
pub(super) fn spawn_detached<D: BoxDriver>(
    driver: &Arc<D>,
    retained: bool,
    runtime: &tokio::runtime::Handle,
    detached: Detached,
) -> tokio::task::JoinHandle<()> {
    let driver = Arc::clone(driver);
    runtime.spawn(async move {
        let Detached { resource, ticket } = detached;
        let _ticket = ticket;
        let_go_with(driver.as_ref(), retained, &resource).await;
    })
}

/// Let go of `resource` through `driver`: forget it when `retained`, close it
/// otherwise. A failure is logged.
pub(super) async fn let_go_with<D: BoxDriver>(driver: &D, retained: bool, resource: &str) {
    let outcome = if retained {
        driver.forget(resource).await
    } else {
        driver.close(resource).await
    };
    if let Err(error) = outcome {
        tracing::warn!(
            resource = %resource,
            retained,
            error = %error,
            "[sandbox:box] letting go of a box failed"
        );
    }
}
