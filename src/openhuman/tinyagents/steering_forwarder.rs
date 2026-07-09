//! Abort-on-drop steering forwarder guard (issue #4456).
//!
//! `run_turn_via_tinyagents_shared` bridges OpenHuman's session-owned
//! [`RunQueue`] into a running TinyAgents turn by spawning a 50 ms poll loop
//! that drains queued **steer**/**collect** messages and forwards them into the
//! run's [`SteeringHandle`]. The harness applies them at the next iteration
//! checkpoint.
//!
//! The historical cleanup — `forwarder.abort()` + steering-registry
//! `deregister(...)` — only ran when the drive future returned *normally*. But
//! cancellation in this codebase is **drop-based** (the web channel drops the
//! turn future via a `tokio::select!` cancel token; detached sub-agents are
//! hard-aborted through an `AbortHandle`). On those paths the spawned task
//! *detached* and looped `sleep(50 ms) → drain` forever, pinning the
//! `Arc<RunQueue>` + `SteeringHandle` and — because the `RunQueue` is
//! session-owned and reused — **racing the next turn's forwarder**, stealing its
//! steer/collect messages into a dead handle. Registry entries for aborted
//! sub-agent tasks were likewise never removed.
//!
//! [`SteeringForwarderGuard`] fixes this with RAII: its [`Drop`] aborts the poll
//! task, deregisters from the shared steering registry, and drains any residual
//! (delivered-but-unapplied) steers back into the session `RunQueue` so a late
//! steer becomes the *next* turn's input instead of vanishing. Because the guard
//! is held across the drive future, cleanup happens identically on normal
//! return, error, and drop-cancellation.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tinyagents::harness::message::Message as TaMessage;
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};

use super::orchestration::{self, TaskId};

/// Framing prepended to a queued **steer** message when it is injected as a user
/// turn. Kept as a shared const so the residual-requeue path can strip it and
/// avoid double-prefixing when the next turn re-forwards the recovered text.
pub(super) const STEER_PREFIX: &str = "[User steering message]: ";

/// Framing prepended to a queued **collect** message (orchestrator/monitor
/// context lines). See [`STEER_PREFIX`].
pub(super) const COLLECT_PREFIX: &str = "[Additional context from user]: ";

/// Live steering-forwarder poll tasks, incremented when
/// [`SteeringForwarderGuard::new`] spawns a poll loop and decremented on its
/// `Drop`. Exposed for tests asserting the leak fix: after a cancelled/aborted
/// turn this must return to its pre-turn value (zero live forwarders for that
/// turn).
static ACTIVE_FORWARDERS: AtomicUsize = AtomicUsize::new(0);

/// Number of steering-forwarder poll tasks currently live process-wide.
pub(crate) fn active_steering_forwarders() -> usize {
    ACTIVE_FORWARDERS.load(Ordering::SeqCst)
}

/// Milliseconds since the Unix epoch (best-effort; `0` on a pre-epoch clock).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Drain the run queue's pending **steer** messages and forward them to the
/// tinyagents [`SteeringHandle`] as injected user turns (the harness applies
/// them to the working transcript at the next iteration checkpoint). This is the
/// bridge behind the `steer_subagent` / mid-flight-steering feature. Emits a
/// [`DomainEvent::RunQueueMessageDelivered`] when at least one message is
/// delivered so the delivery is visible in the event stream (issue #4456).
pub(super) async fn forward_steers(queue: &RunQueue, handle: &SteeringHandle, thread_label: &str) {
    let drained = queue.drain_steers().await;
    if drained.is_empty() {
        return;
    }
    let delivered = drained.len();
    for msg in drained {
        handle.send(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "{STEER_PREFIX}{}",
            msg.text
        ))));
    }
    tracing::debug!(
        thread_id = thread_label,
        delivered,
        "[run_queue] delivered steer message(s) into running steering handle"
    );
    publish_global(DomainEvent::RunQueueMessageDelivered {
        thread_id: thread_label.to_string(),
        mode: "steer".to_string(),
        delivered,
    });
}

/// Forward any queued **collect** messages (orchestrator/monitor lines enqueued
/// via `QueueMode::Collect`) into the run as injected user turns so they reach
/// the next LLM call as additional context. Mirrors the legacy
/// `[Additional context from user]:` framing the model was taught to read. Emits
/// a [`DomainEvent::RunQueueMessageDelivered`] on delivery (issue #4456).
pub(super) async fn forward_collects(
    queue: &RunQueue,
    handle: &SteeringHandle,
    thread_label: &str,
) {
    let drained = queue.drain_collects().await;
    if drained.is_empty() {
        return;
    }
    let delivered = drained.len();
    for msg in drained {
        handle.send(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "{COLLECT_PREFIX}{}",
            msg.text
        ))));
    }
    tracing::debug!(
        thread_id = thread_label,
        delivered,
        "[run_queue] delivered collect message(s) into running steering handle"
    );
    publish_global(DomainEvent::RunQueueMessageDelivered {
        thread_id: thread_label.to_string(),
        mode: "collect".to_string(),
        delivered,
    });
}

/// Abort-on-drop guard around the 50 ms steering-forwarder poll task.
///
/// Held across the harness drive future so its [`Drop`] runs on **every** exit
/// path (normal return, error, and drop-cancellation), aborting the poll task,
/// deregistering the sub-agent steering handle, and requeuing residual steers.
pub(super) struct SteeringForwarderGuard {
    /// The spawned poll task; `abort()`-ed on drop. `None` after the abort so a
    /// double-drop is a no-op.
    forwarder: Option<tokio::task::JoinHandle<()>>,
    /// A clone of the run's steering handle, drained on drop to recover
    /// delivered-but-unapplied steers.
    handle: SteeringHandle,
    /// The session-owned run queue, used to requeue residual steers on drop.
    /// `None` after the requeue so a double-drop is a no-op.
    run_queue: Option<Arc<RunQueue>>,
    /// The steering-registry key to deregister on drop (sub-agent runs only).
    registry_task_id: Option<TaskId>,
    /// Best-effort thread label for observability + requeued-message metadata.
    thread_label: String,
}

impl SteeringForwarderGuard {
    /// Arm the guard: spawn the 50 ms poll loop when a `run_queue` is present and
    /// wrap all cleanup in an abort-on-drop scope.
    ///
    /// `run_queue` is `None` for a steering-only run (a sub-agent whose handle is
    /// controlled purely through the steering registry, with no queue lane): no
    /// poll task is spawned, but the guard still deregisters the handle on every
    /// exit path so an aborted run cannot leak a registry entry. `registry_task_id`
    /// is `Some` for sub-agent runs (whose handle is registered in the shared
    /// steering registry) and `None` for the interactive parent turn.
    pub(super) fn new(
        handle: SteeringHandle,
        run_queue: Option<Arc<RunQueue>>,
        registry_task_id: Option<TaskId>,
        thread_label: String,
    ) -> Self {
        let forwarder = run_queue.as_ref().map(|queue| {
            ACTIVE_FORWARDERS.fetch_add(1, Ordering::SeqCst);
            let loop_queue = queue.clone();
            let loop_handle = handle.clone();
            let loop_label = thread_label.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    forward_steers(&loop_queue, &loop_handle, &loop_label).await;
                    forward_collects(&loop_queue, &loop_handle, &loop_label).await;
                }
            })
        });
        tracing::debug!(
            thread_id = thread_label.as_str(),
            has_queue = run_queue.is_some(),
            "[run_queue] steering forwarder guard armed (abort-on-drop)"
        );
        Self {
            forwarder,
            handle,
            run_queue,
            registry_task_id,
            thread_label,
        }
    }
}

impl Drop for SteeringForwarderGuard {
    fn drop(&mut self) {
        // 1. Stop the poll loop so it can no longer race the next turn's
        //    forwarder for the shared, session-owned run queue.
        if let Some(forwarder) = self.forwarder.take() {
            forwarder.abort();
            ACTIVE_FORWARDERS.fetch_sub(1, Ordering::SeqCst);
            tracing::debug!(
                thread_id = self.thread_label.as_str(),
                "[run_queue] aborted steering forwarder (guard drop)"
            );
        }

        // 2. Deregister the sub-agent steering handle so an aborted run does not
        //    leak a registry entry keyed by a dead handle.
        if let Some(task_id) = self.registry_task_id.take() {
            orchestration::shared_steering_registry().deregister(&task_id);
            tracing::debug!(
                task_id = task_id.as_str(),
                "[tinyagents] deregistered subagent steering handle (guard drop)"
            );
        }

        // 3. Recover residual steers: messages the poll loop already delivered
        //    into the handle but that the harness ended/cancelled before
        //    applying at a checkpoint. Strip the delivery prefix so the next
        //    turn's forwarder re-frames them cleanly (no double prefix).
        //    Control-flow-only commands (Pause/Resume/Cancel/…) are meaningless
        //    once the run is gone and are intentionally dropped.
        let residual = self.handle.drain();
        let requeue_texts: Vec<(String, QueueMode)> = residual
            .into_iter()
            .filter_map(|cmd| match cmd {
                SteeringCommand::InjectMessage(msg) => {
                    let text = msg.text();
                    // The prefix that matched tells us the lane — preserve it so
                    // a delivered-but-unapplied collect line re-enters as Collect
                    // (framed `[Additional context from user]:`) rather than being
                    // re-labeled as user Steer. Default to Steer when neither
                    // prefix is present (a raw steer that was never framed).
                    if let Some(rest) = text.strip_prefix(STEER_PREFIX) {
                        Some((rest.to_string(), QueueMode::Steer))
                    } else if let Some(rest) = text.strip_prefix(COLLECT_PREFIX) {
                        Some((rest.to_string(), QueueMode::Collect))
                    } else {
                        Some((text.to_string(), QueueMode::Steer))
                    }
                }
                _ => None,
            })
            .collect();

        let Some(queue) = self.run_queue.take() else {
            return;
        };
        if requeue_texts.is_empty() {
            return;
        }
        let requeued = requeue_texts.len();
        let thread_label = self.thread_label.clone();

        // `RunQueue::push` is async (tokio `Mutex`); `Drop` is synchronous. Push
        // the recovered steers back on a detached task so they land in the
        // session queue and become the next turn's input. The forwarder poll
        // loop keeps re-draining the queue, so even if the requeue completes
        // after the next turn starts, its next 50 ms tick picks them up.
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                let label = thread_label.clone();
                rt.spawn(async move {
                    for (text, mode) in requeue_texts {
                        queue
                            .push(QueuedMessage {
                                text,
                                mode,
                                client_id: String::new(),
                                thread_id: label.clone(),
                                queued_at_ms: now_ms(),
                                model_override: None,
                                temperature: None,
                                profile_id: None,
                                locale: None,
                            })
                            .await;
                    }
                });
            }
            Err(_) => {
                // No runtime to spawn on (should not happen on the live paths,
                // which always drop inside a tokio task). The steers are lost
                // rather than silently mis-handled — log loudly.
                tracing::warn!(
                    thread_id = thread_label.as_str(),
                    requeued,
                    "[run_queue] could not requeue residual steers: no tokio runtime at guard drop"
                );
                return;
            }
        }

        tracing::debug!(
            thread_id = thread_label.as_str(),
            requeued,
            "[run_queue] requeued residual steer(s) as next-turn input (guard drop)"
        );
        publish_global(DomainEvent::RunQueueSteerRequeued {
            thread_id: thread_label,
            requeued,
        });
    }
}
