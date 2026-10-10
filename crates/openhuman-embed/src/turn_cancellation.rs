//! A cancellation request and the acknowledgement that the turn has stopped.

use std::sync::Arc;

use openhuman_core::tools::timeout::ProcessCleanup;
use tokio::sync::watch;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Pending,
    Running,
    Finished,
}

struct State {
    requested: watch::Sender<bool>,
    phase: watch::Sender<Phase>,
    cleanup: ProcessCleanup,
}

/// A handle to one [`Turn`](crate::Turn), obtained before sending it.
/// Clones address the same turn; other turns on its agent are unaffected.
#[derive(Clone)]
pub struct TurnCancellation(Arc<State>);

impl Default for TurnCancellation {
    fn default() -> Self {
        Self(Arc::new(State {
            requested: watch::Sender::new(false),
            phase: watch::Sender::new(Phase::Pending),
            cleanup: ProcessCleanup::default(),
        }))
    }
}

impl TurnCancellation {
    /// Request cancellation and wait until the turn future has stopped and
    /// its tracked command waiters have finished. On Unix, built-in shell,
    /// Node, Python and npm tools kill the whole process group before reaping.
    /// Host tools that spawn their own tasks or processes own their cleanup.
    ///
    /// Before send, this returns immediately and the turn is refused when
    /// sent. Calling again, or after the turn ended, is safe. Keep polling the
    /// send future (usually in another Tokio task) while awaiting cancellation.
    pub async fn cancel(&self) {
        self.0.requested.send_replace(true);
        let mut phase = self.0.phase.subscribe();
        if *phase.borrow_and_update() == Phase::Running {
            let _ = phase.wait_for(|phase| *phase == Phase::Finished).await;
        }
        self.0.cleanup.wait().await;
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        *self.0.requested.borrow()
    }

    pub(crate) async fn cancelled(&self) {
        let mut requested = self.0.requested.subscribe();
        let _ = requested.wait_for(|requested| *requested).await;
    }

    pub(crate) fn enter(&self) -> TurnGuard {
        self.0.phase.send_replace(Phase::Running);
        TurnGuard {
            cancellation: self.clone(),
        }
    }

    pub(crate) fn cleanup(&self) -> &ProcessCleanup {
        &self.0.cleanup
    }
}

pub(crate) struct TurnGuard {
    cancellation: TurnCancellation,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        self.cancellation.0.phase.send_replace(Phase::Finished);
    }
}
