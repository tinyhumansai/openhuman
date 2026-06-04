//! Active-run queue for mid-turn message steering.
//!
//! When an agent turn is in flight, incoming messages can be routed into
//! one of three lanes instead of aborting the turn:
//!
//! - **steers** — injected at the next iteration boundary as a new user
//!   instruction the agent must address immediately.
//! - **followups** — dispatched as a fresh turn after the current one completes.
//! - **collects** — injected as additional context at the next iteration
//!   boundary without being a distinct instruction.
//!
//! The engine drains steers and collects at safe points (after tool results are
//! committed to history), preserving the tool-call / tool-result pairing invariant.

pub mod types;

use std::sync::Arc;
use tokio::sync::Mutex;

pub use types::{QueueMode, QueueStatus, QueuedMessage};

/// Thread-safe run queue with three lanes. Wrapped in `Arc` for shared
/// ownership between the web channel producer and the engine consumer.
#[derive(Debug)]
pub struct RunQueue {
    inner: Mutex<RunQueueInner>,
}

#[derive(Debug, Default)]
struct RunQueueInner {
    steers: Vec<QueuedMessage>,
    followups: Vec<QueuedMessage>,
    collects: Vec<QueuedMessage>,
}

impl RunQueue {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(RunQueueInner::default()),
        })
    }

    /// Push a message into the appropriate lane based on its mode.
    pub async fn push(&self, msg: QueuedMessage) {
        let mut inner = self.inner.lock().await;
        match msg.mode {
            QueueMode::Steer => inner.steers.push(msg),
            QueueMode::Followup => inner.followups.push(msg),
            QueueMode::Collect => inner.collects.push(msg),
            QueueMode::Interrupt => {
                log::warn!(
                    "[run_queue] interrupt-mode message pushed to queue — should have been handled by caller"
                );
            }
        }
    }

    /// Drain all pending steer messages (FIFO order).
    pub async fn drain_steers(&self) -> Vec<QueuedMessage> {
        let mut inner = self.inner.lock().await;
        std::mem::take(&mut inner.steers)
    }

    /// Drain all pending collect messages (FIFO order).
    pub async fn drain_collects(&self) -> Vec<QueuedMessage> {
        let mut inner = self.inner.lock().await;
        std::mem::take(&mut inner.collects)
    }

    /// Drain all pending followup messages (FIFO order).
    pub async fn drain_followups(&self) -> Vec<QueuedMessage> {
        let mut inner = self.inner.lock().await;
        std::mem::take(&mut inner.followups)
    }

    /// Snapshot the current queue depth per lane.
    pub async fn status(&self) -> QueueStatus {
        let inner = self.inner.lock().await;
        let steers = inner.steers.len();
        let followups = inner.followups.len();
        let collects = inner.collects.len();
        QueueStatus {
            steers,
            followups,
            collects,
            total: steers + followups + collects,
        }
    }

    /// Clear all lanes and return the total number of messages dropped.
    pub async fn clear(&self) -> usize {
        let mut inner = self.inner.lock().await;
        let total = inner.steers.len() + inner.followups.len() + inner.collects.len();
        inner.steers.clear();
        inner.followups.clear();
        inner.collects.clear();
        total
    }

    /// Check whether any steers or collects are pending (engine poll).
    pub async fn has_pending_injections(&self) -> bool {
        let inner = self.inner.lock().await;
        !inner.steers.is_empty() || !inner.collects.is_empty()
    }
}

#[cfg(test)]
#[path = "run_queue_tests.rs"]
mod tests;
