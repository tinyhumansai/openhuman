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

mod types;

use std::sync::Arc;
use tinyagents_harness::run_queue::{QueueLane, RunQueue as TinyAgentsRunQueue};

pub use tinyagents_harness::run_queue::QueueStatus;
pub use types::{QueueMode, QueuedMessage};

/// Thread-safe run queue with three lanes. Wrapped in `Arc` for shared
/// ownership between the web channel producer and the engine consumer.
#[derive(Debug)]
pub struct RunQueue {
    inner: TinyAgentsRunQueue<QueuedMessage>,
}

impl RunQueue {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: TinyAgentsRunQueue::new(),
        })
    }

    /// Push a message into the appropriate lane based on its mode.
    pub async fn push(&self, msg: QueuedMessage) {
        match msg.mode {
            QueueMode::Steer => self.inner.push(QueueLane::Steer, msg).await,
            QueueMode::Followup => self.inner.push(QueueLane::Followup, msg).await,
            QueueMode::Collect => self.inner.push(QueueLane::Collect, msg).await,
            QueueMode::Interrupt => {
                log::warn!(
                    "[run_queue] interrupt-mode message pushed to queue — should have been handled by caller"
                );
            }
            QueueMode::Parallel => {
                log::warn!(
                    "[run_queue] parallel-mode message pushed to queue — should have spawned a forked turn at the caller"
                );
            }
        }
    }

    /// Drain all pending steer messages (FIFO order).
    pub async fn drain_steers(&self) -> Vec<QueuedMessage> {
        self.inner.drain(QueueLane::Steer).await
    }

    /// Drain all pending collect messages (FIFO order).
    pub async fn drain_collects(&self) -> Vec<QueuedMessage> {
        self.inner.drain(QueueLane::Collect).await
    }

    /// Drain all pending followup messages (FIFO order).
    pub async fn drain_followups(&self) -> Vec<QueuedMessage> {
        self.inner.drain(QueueLane::Followup).await
    }

    /// Snapshot the current queue depth per lane.
    pub async fn status(&self) -> QueueStatus {
        self.inner.status().await
    }

    /// Clear all lanes and return the total number of messages dropped.
    pub async fn clear(&self) -> usize {
        self.inner.clear().await
    }
}

#[cfg(test)]
#[path = "run_queue_tests.rs"]
mod tests;
