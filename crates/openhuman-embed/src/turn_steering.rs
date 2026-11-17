//! Steering one turn: messages a host adds to a turn while it runs.
//!
//! A [`TurnSteering`] handle is taken from a [`Turn`](crate::Turn) before it is
//! sent. Each [`steer`](TurnSteering::steer) queues a user message on the
//! turn's run queue; the core forwards it into the running agent loop, which
//! appends it to the transcript at its next safe checkpoint:
//!
//! - before the first model call, for a steer queued before `send()`;
//! - after a tool batch has settled, never in the middle of one;
//! - when the model has written what would be its final answer. The draft
//!   stays on the transcript as an assistant message, the steer follows it,
//!   and the model is called again, so a steer that arrives during a plain
//!   chat turn's only model call is still answered by that turn.
//!
//! The model sees each steer as a user message framed
//! `[User steering message]: <text>`.
//!
//! Once the turn has ended, steers it never applied are handed back rather
//! than dropped: on success in [`TurnOutcome::unapplied_steers`], and after a
//! failure or cancellation through [`TurnSteering::take_unapplied`]. A steer
//! sent after the turn ended is refused with [`SteerRefused`], which carries the
//! text back. Either way the host decides whether to run it as a new turn.
//!
//! [`TurnOutcome::unapplied_steers`]: crate::TurnOutcome::unapplied_steers

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use openhuman_core::agent::queued_turn::{steer_message, QueuedTurn};
use tinyagents_harness::run_queue::{QueueLane, RunQueue};

/// The run queue a turn is steered through.
pub(crate) type SteeringQueue = Arc<RunQueue<QueuedTurn>>;

/// Whether the turn still takes steers, and what it left behind if not.
#[derive(Default)]
struct State {
    /// Set once the turn has ended; later steers are refused.
    finished: bool,
    /// Steers a failed or cancelled turn did not apply, until the host takes
    /// them.
    unapplied: Vec<String>,
}

struct Shared {
    queue: SteeringQueue,
    state: Mutex<State>,
}

/// A handle that adds messages to one [`Turn`](crate::Turn) while it runs.
///
/// Obtained with [`Turn::steering_handle`](crate::Turn::steering_handle)
/// before the turn is sent. Clones address the same turn, and the handle is
/// `Send + Sync`, so it can be given to whatever receives the host's next
/// message while the turn is still running.
#[derive(Clone)]
pub struct TurnSteering(Arc<Shared>);

impl Default for TurnSteering {
    fn default() -> Self {
        Self(Arc::new(Shared {
            queue: Arc::new(RunQueue::new()),
            state: Mutex::new(State::default()),
        }))
    }
}

impl std::fmt::Debug for TurnSteering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        f.debug_struct("TurnSteering")
            .field("finished", &state.finished)
            .field("unapplied", &state.unapplied.len())
            .finish()
    }
}

impl TurnSteering {
    /// Add `text` to the turn as a user message.
    ///
    /// Before `send()`, the message is applied ahead of the first model call.
    /// While the turn runs, it is applied at the loop's next safe checkpoint:
    /// after a tool batch has settled, or when the model has written what
    /// would be its final answer. It does not interrupt a model call or a
    /// tool that is already running.
    ///
    /// # Errors
    ///
    /// [`SteerRefused`] when the turn has already ended. It carries `text`
    /// back so the host can run it as a new turn.
    pub fn steer(&self, text: impl Into<String>) -> Result<(), SteerRefused> {
        let text = text.into();
        // Checked and pushed under one lock, which `finish` also takes, so a
        // steer is either refused or in the queue that `finish` drains.
        let state = self.state();
        if state.finished {
            log::debug!("[embed][agent] steer refused: the turn has ended");
            return Err(SteerRefused { text });
        }
        self.0.queue.push_now(
            QueueLane::Steer,
            steer_message(uuid::Uuid::new_v4().to_string(), text, "", ""),
        );
        drop(state);
        log::debug!("[embed][agent] steer queued for the running turn");
        Ok(())
    }

    /// Take the steers a turn that **failed or was cancelled** did not apply,
    /// in the order they were sent. Empty while the turn runs, after a
    /// successful turn (whose leftovers are in
    /// [`TurnOutcome::unapplied_steers`](crate::TurnOutcome::unapplied_steers)),
    /// and on a second call.
    pub fn take_unapplied(&self) -> Vec<String> {
        let mut state = self.state();
        if !state.finished {
            return Vec::new();
        }
        let mut taken = std::mem::take(&mut state.unapplied);
        // A cancelled turn may still be unwinding on another task, putting a
        // delivered steer back after the turn was marked finished.
        taken.extend(self.drain_queue());
        taken
    }

    pub(crate) fn queue(&self) -> SteeringQueue {
        Arc::clone(&self.0.queue)
    }

    /// End the turn's steering: refuse later steers and return what is still
    /// queued, in order. Call once the turn future has finished, when every
    /// steer it did not apply is back in the queue.
    pub(crate) fn finish(&self) -> Vec<String> {
        let mut state = self.state();
        state.finished = true;
        let mut left = std::mem::take(&mut state.unapplied);
        left.extend(self.drain_queue());
        left
    }

    fn drain_queue(&self) -> Vec<String> {
        self.0
            .queue
            .drain_now(QueueLane::Steer)
            .into_iter()
            .map(|queued| queued.text)
            .collect()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Ends a turn's steering on every exit path. A successful turn calls
/// [`Self::close`] and reports the leftovers on its outcome; any other exit
/// (an error, a cancellation that drops the turn) keeps them on the handle for
/// [`TurnSteering::take_unapplied`].
pub(crate) struct SteeringScope(Option<TurnSteering>);

impl SteeringScope {
    pub(crate) fn new(steering: Option<TurnSteering>) -> Self {
        Self(steering)
    }

    pub(crate) fn queue(&self) -> Option<SteeringQueue> {
        self.0.as_ref().map(TurnSteering::queue)
    }

    /// End steering and return the steers the turn did not apply.
    pub(crate) fn close(mut self) -> Vec<String> {
        self.0
            .take()
            .map(|steering| steering.finish())
            .unwrap_or_default()
    }
}

impl Drop for SteeringScope {
    fn drop(&mut self) {
        if let Some(steering) = self.0.take() {
            let mut state = steering.state();
            state.finished = true;
            let left = steering.drain_queue();
            if !left.is_empty() {
                log::debug!(
                    "[embed][agent] turn ended without applying {} steer(s); kept for take_unapplied",
                    left.len()
                );
            }
            state.unapplied.extend(left);
        }
    }
}

/// A steer sent after its turn ended. The text comes back so the host can run
/// it as a new turn.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the turn has already ended; send this message as a new turn")]
pub struct SteerRefused {
    text: String,
}

impl SteerRefused {
    /// The refused message.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The refused message, by value.
    pub fn into_text(self) -> String {
        self.text
    }
}

#[cfg(test)]
#[path = "turn_steering_tests.rs"]
mod tests;
