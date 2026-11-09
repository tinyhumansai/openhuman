use std::sync::Arc;
use std::time::Duration;

use super::super::progress::{ProgressProjection, INFERENCE_HEARTBEAT_SECS};
use super::super::turn_activity::TurnActivity;
use super::{TurnSignal, WorkTracker};
use crate::agent::queued_turn::QueuedPayload;
use crate::core::runtime::DispatchCtx;
use tinyagents_harness::progress::AgentProgress;
use tinyagents_harness::turn_runner::TurnRequest;
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex};

/// Aborts a task when dropped.
pub(super) struct AbortOnDrop(pub(super) tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Settle the turn's own snapshot now the turn is over.
///
/// The projection marks the snapshot terminal on its way out, but it only
/// finishes once the terminal event has been published, and a detached
/// sub-agent may still hold a progress sender past the turn. A snapshot left
/// `Streaming` on disk would hydrate a live "Thinking..." indicator under a
/// reply that was already delivered. The turn has ended here by construction,
/// so record that; a projection that later observes `TurnCompleted` overwrites
/// it with `Completed`, terminal either way.
pub(super) fn settle_turn_snapshot(
    config: &crate::config::Config,
    request: &TurnRequest<QueuedPayload>,
    completed: bool,
) {
    let lifecycle = if completed {
        tinyagents_session::turn_state::TurnLifecycle::Completed
    } else {
        tinyagents_session::turn_state::TurnLifecycle::Interrupted
    };
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(err) = crate::storage::sessions::turn_states_for(&config.workspace_dir).settle_turn(
        &request.thread_id,
        &request.request_id,
        lifecycle,
        &now,
    ) {
        log::warn!(
            "[web-channel] failed to settle turn snapshot client={} thread={} request_id={}: {err}",
            request.payload.client_id,
            request.thread_id,
            request.request_id
        );
    }
}

/// Forward the agent's progress to the runner, folding each event into the
/// idle record and the work tracker on the way. Signals `drained` once the
/// parent's `TurnCompleted` was forwarded or the stream ended.
///
/// A detached sub-agent can outlive its parent turn and still report (a
/// cancelled child settles its card with `subagent_failed`). Once the runner has
/// finished with the turn it no longer reads the stream, so with a `late` sink
/// the tap keeps running and publishes only the sub-agent events straight to
/// the web event stream, under the turn's ids; without one it stops, as before.
pub(super) fn tap_progress(
    ctx: &DispatchCtx,
    mut rx: mpsc::Receiver<AgentProgress>,
    progress: mpsc::Sender<TurnSignal>,
    activity: TurnActivity,
    drained: watch::Sender<bool>,
    late: Option<LateSubagentSink>,
) -> tokio::task::JoinHandle<()> {
    let publish_ctx = ctx.clone();
    ctx.spawn(async move {
        let mut work = WorkTracker::default();
        let mut runner_gone = false;
        while let Some(event) = rx.recv().await {
            if runner_gone {
                if let Some(late) = late.as_ref() {
                    late.publish(&publish_ctx, event).await;
                }
                continue;
            }
            activity.observe(&event);
            let completed = matches!(event, AgentProgress::TurnCompleted { .. });
            if let Some(signal) = work.observe(&event) {
                if progress.send(signal).await.is_err() {
                    runner_gone = true;
                    if late.is_none() {
                        break;
                    }
                }
            }
            if !runner_gone
                && progress
                    .send(TurnSignal::Progress(Box::new(event.clone())))
                    .await
                    .is_err()
            {
                runner_gone = true;
                if late.is_none() {
                    break;
                }
            }
            if runner_gone {
                if let Some(late) = late.as_ref() {
                    late.publish(&publish_ctx, event).await;
                }
            }
            if completed {
                let _ = drained.send(true);
            }
        }
        let _ = drained.send(true);
    })
}

/// Where a finished turn's late sub-agent events go: the web event stream,
/// addressed like the turn.
#[derive(Clone)]
pub(super) struct LateSubagentSink {
    pub(super) client_id: String,
    pub(super) thread_id: String,
    pub(super) request_id: String,
    pub(super) config: crate::config::Config,
}

impl LateSubagentSink {
    async fn publish(&self, ctx: &DispatchCtx, event: AgentProgress) {
        // Only the events that settle a sub-agent card can matter after the
        // turn: a detached child usually finishes after its parent replied,
        // and one aborted by `subagent_cancel` still reports its failure.
        // Everything else is turn-scoped output the finished turn no longer
        // renders.
        let kind = match &event {
            AgentProgress::SubagentCompleted { task_id, .. } => ("completed", task_id),
            AgentProgress::SubagentFailed { task_id, .. } => ("failed", task_id),
            AgentProgress::SubagentAwaitingUser { task_id, .. } => ("awaiting_user", task_id),
            _ => return,
        };
        log::debug!(
            "[web-channel] late subagent_{} task_id={} request_id={}",
            kind.0,
            kind.1,
            self.request_id
        );
        let Some(output) = super::super::progress_map::subagent_event(event, 0, None) else {
            return;
        };
        // The turn's own projection is closed, so the event gets a projection
        // turn of its own: the module still owns the wire form, including the
        // child's output and spend a detached completion carries. Its restarted
        // sequence would fall below the turn's, so it is published without one.
        let surface = super::super::surface::Surface::new(
            ctx,
            super::super::surface::TurnIds {
                client_id: self.client_id.clone(),
                thread_id: self.thread_id.clone(),
                request_id: self.request_id.clone(),
            },
            self.config.clone(),
        )
        .await
        .unsequenced();
        surface.output(output).await;
        surface.close().await;
    }
}

/// The `inference_heartbeat` liveness beat for the turn's projection.
pub(super) fn spawn_heartbeat(
    ctx: &DispatchCtx,
    projection: Arc<AsyncMutex<ProgressProjection>>,
) -> tokio::task::JoinHandle<()> {
    spawn_heartbeat_with_interval(
        ctx,
        projection,
        Duration::from_secs(INFERENCE_HEARTBEAT_SECS),
    )
}

pub(super) fn spawn_heartbeat_with_interval(
    ctx: &DispatchCtx,
    projection: Arc<AsyncMutex<ProgressProjection>>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    ctx.spawn(async move {
        let mut beat = tokio::time::interval(interval);
        // Wall-clock cadence: a slow turn must not produce a burst of catch-up
        // beats once it finally yields control.
        beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick resolves at once; the first real beat lands one full
        // interval after the turn begins.
        beat.tick().await;
        loop {
            beat.tick().await;
            projection.lock().await.heartbeat().await;
        }
    })
}
