//! Per-module resolution state, and the wait on it.
//!
//! [`super::ops::ensure_loaded`] used to hold one process-wide lock while it
//! resolved a module, and every caller for every module queued behind it. Three
//! things followed, and all three were visible in the field on a slow link:
//! the memory module waited its turn behind runtime downloads it had nothing to
//! do with; a caller that stopped waiting dropped the resolver's own future,
//! which released the lock with the module half-resolved and let the next
//! caller start a second download; and there was no way to say "still loading",
//! so every memory call ran into its caller's own timeout instead.
//!
//! This table replaces the lock with a slot per module. The first caller to
//! ask claims the slot and runs the resolution as a task with process lifetime,
//! so a caller that gives up cannot cancel it; every other caller waits on a
//! watch channel until the outcome lands. Waiting is cancel-safe, so a caller
//! may give up after a bound and report the module as loading rather than hang.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::watch;

/// Outcome of resolving one module, remembered for the process lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Resolution {
    /// Serving.
    Ready,
    /// Terminal. Carries the sanitised reason, never a path or URL.
    Failed(String),
}

/// What one module's slot holds.
#[derive(Debug, Clone)]
enum Slot {
    InFlight(watch::Receiver<Option<Resolution>>),
    Done(Resolution),
}

/// What [`ResolutionTable::claim`] hands the caller.
pub(super) enum Claim {
    /// Nobody has asked for this module yet. The caller runs the resolution and
    /// reports through the sender; the receiver is its own seat in the queue.
    Run {
        sender: watch::Sender<Option<Resolution>>,
        receiver: watch::Receiver<Option<Resolution>>,
    },
    /// Someone else is resolving it: wait for their outcome.
    Wait(watch::Receiver<Option<Resolution>>),
    /// Already resolved, one way or the other.
    Done(Resolution),
}

/// A module's state as [`ResolutionTable::peek`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResolutionState {
    /// Nothing has asked for it.
    Unresolved,
    /// Being downloaded, verified, or initialised.
    Loading,
    Ready,
    Failed(String),
}

/// How a wait on a slot ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Waited {
    Ready,
    Failed(String),
    /// The bound passed before the outcome landed.
    StillLoading,
}

impl From<Resolution> for Waited {
    fn from(resolution: Resolution) -> Self {
        match resolution {
            Resolution::Ready => Self::Ready,
            Resolution::Failed(reason) => Self::Failed(reason),
        }
    }
}

/// One slot per module, keyed by registry id.
#[derive(Default)]
pub(super) struct ResolutionTable {
    slots: Mutex<HashMap<String, Slot>>,
}

impl ResolutionTable {
    /// Claim `id` for resolution, or learn who already has.
    ///
    /// Atomic with respect to other claims: two concurrent first callers get one
    /// `Run` and one `Wait`, never two `Run`s — that is the property the old
    /// lock existed for, kept without serialising unrelated modules.
    pub(super) fn claim(&self, id: &str) -> Claim {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match slots.get(id) {
            Some(Slot::Done(resolution)) => Claim::Done(resolution.clone()),
            Some(Slot::InFlight(receiver)) => Claim::Wait(receiver.clone()),
            None => {
                let (sender, receiver) = watch::channel(None);
                slots.insert(id.to_string(), Slot::InFlight(receiver.clone()));
                Claim::Run { sender, receiver }
            }
        }
    }

    /// Record the outcome for `id` and wake everyone waiting on it.
    ///
    /// The table is updated before the channel fires, so a waiter that wakes and
    /// looks again sees the settled slot rather than a stale in-flight one.
    pub(super) fn complete(
        &self,
        id: &str,
        resolution: Resolution,
        sender: watch::Sender<Option<Resolution>>,
    ) {
        {
            let mut slots = self
                .slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            slots.insert(id.to_string(), Slot::Done(resolution.clone()));
        }
        let _ = sender.send(Some(resolution));
    }

    /// The state of `id` without touching it.
    pub(super) fn peek(&self, id: &str) -> ResolutionState {
        let slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match slots.get(id) {
            None => ResolutionState::Unresolved,
            Some(Slot::InFlight(_)) => ResolutionState::Loading,
            Some(Slot::Done(Resolution::Ready)) => ResolutionState::Ready,
            Some(Slot::Done(Resolution::Failed(reason))) => ResolutionState::Failed(reason.clone()),
        }
    }

    /// Wait for an outcome on `receiver`, for at most `within` when given.
    ///
    /// Cancel-safe: dropping this future leaves the resolution running and the
    /// slot intact. A sender dropped without an outcome means the resolver task
    /// itself died before reporting; that is reported as a failure rather than
    /// waited on, because nothing will ever complete the slot.
    pub(super) async fn wait(
        mut receiver: watch::Receiver<Option<Resolution>>,
        within: Option<Duration>,
    ) -> Waited {
        let settled = async move {
            loop {
                if let Some(resolution) = receiver.borrow_and_update().clone() {
                    return resolution;
                }
                if receiver.changed().await.is_err() {
                    return Resolution::Failed(
                        "module resolution was abandoned; restart the app to try again".to_string(),
                    );
                }
            }
        };
        match within {
            None => settled.await.into(),
            Some(limit) => match tokio::time::timeout(limit, settled).await {
                Ok(resolution) => resolution.into(),
                Err(_elapsed) => Waited::StillLoading,
            },
        }
    }

    /// Put `id` in flight without a resolver, handing back the sender that
    /// completes it. For tests that need to observe the loading state.
    #[cfg(test)]
    pub(super) fn mark_in_flight_for_test(&self, id: &str) -> watch::Sender<Option<Resolution>> {
        let (sender, receiver) = watch::channel(None);
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slots.insert(id.to_string(), Slot::InFlight(receiver));
        sender
    }

    /// Forget `id` entirely. For tests, so a slot they planted does not outlive
    /// them in the process-wide table.
    #[cfg(test)]
    pub(super) fn reset_for_test(&self, id: &str) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slots.remove(id);
    }
}

/// The process-wide table. One per process because tinybus loads a library at
/// most once per process, so the outcome is a fact about the process.
pub(super) fn table() -> &'static ResolutionTable {
    static TABLE: OnceLock<ResolutionTable> = OnceLock::new();
    TABLE.get_or_init(ResolutionTable::default)
}

#[cfg(test)]
#[path = "resolution_tests.rs"]
mod tests;
