//! Project module-owned PollRun events onto OpenHuman events and Langfuse.

use std::collections::{HashSet, VecDeque};
use std::sync::{LazyLock, Mutex};

use serde_json::Value;
use tinyflows::engine::GraphObservation;
use tinyflows_bus::runs::FlowRunTrigger;
use tinyflows_bus::runs::{self, RunEvent};

use crate::core::events::DomainEvent;
use crate::flows::runs_adapter::session::RunSession;
use crate::flows::runs_adapter::RunKey;

const RECENT_FINISHED_KEYS: usize = 4_096;

static FINISHED_KEYS: LazyLock<Mutex<(HashSet<RunKey>, VecDeque<RunKey>)>> =
    LazyLock::new(|| Mutex::new((HashSet::new(), VecDeque::new())));

pub(super) async fn project(session: &RunSession, event: &RunEvent) {
    match event.kind.as_str() {
        "started" => {
            if session.mark_started() {
                crate::core::bus::BUS.publish(DomainEvent::FlowRunStarted {
                    flow_id: session.flow_id().to_owned(),
                    run_id: session.run_id.clone(),
                });
            }
        }
        "resumed" => {
            session.set_trigger(FlowRunTrigger::Resume);
            session.mark_resumed();
        }
        "step_started" => {
            if let Some(node_id) = event.data.get("node_id").and_then(Value::as_str) {
                publish_step(session, node_id, "running");
            }
        }
        "step" => {
            if let (Some(node_id), Some(status)) = (
                event.data.get("node_id").and_then(Value::as_str),
                event.data.get("status").and_then(Value::as_str),
            ) {
                publish_step(session, node_id, status);
            }
        }
        "graph_observation" => {
            match serde_json::from_value::<GraphObservation>(event.data.clone()) {
                Ok(observation) => session.add_observation(observation),
                Err(_) => tracing::warn!(
                    target: "flows",
                    run_id = %session.run_id,
                    "[flows] PollRun graph observation could not be decoded for Langfuse"
                ),
            }
        }
        "agent_progress" => {
            // Detailed agent events remain on the module stream. The OpenHuman
            // workflow overlay consumes the node-level progress events above.
        }
        "parked" => {
            session.mark_parked();
            let parked = match serde_json::from_value::<runs::RunParked>(event.data.clone()) {
                Ok(parked) => parked,
                Err(_) => {
                    tracing::warn!(
                        target: "flows",
                        run_id = %session.run_id,
                        "[flows] PollRun parked outcome could not be decoded"
                    );
                    return;
                }
            };
            let flow = session.flow().await;
            crate::flows::ops::notify_pending_approval(
                &flow,
                &session.run_id,
                &parked.pending_approvals,
            );
            export_segment(session, "pending_approval").await;
        }
        "finished" => {
            let finished = match serde_json::from_value::<runs::RunFinished>(event.data.clone()) {
                Ok(finished) => finished,
                Err(_) => {
                    tracing::warn!(
                        target: "flows",
                        run_id = %session.run_id,
                        "[flows] PollRun terminal outcome could not be decoded"
                    );
                    return;
                }
            };
            if claim_finished(session.key()) {
                crate::core::bus::BUS.publish(DomainEvent::FlowRunFinished {
                    flow_id: session.flow_id().to_owned(),
                    run_id: session.run_id.clone(),
                    status: finished.status.clone(),
                });
                export_segment(session, &finished.status).await;
            }
            session.mark_finished();
        }
        _ => {}
    }
}

/// Project a recovery event when the run no longer has an in-process session.
/// This path publishes only the newly committed terminal event; historical
/// start/progress events are not replayed after a process restart.
pub(crate) fn project_recovered_finished(
    key: &RunKey,
    flow_id: &str,
    run_id: &str,
    event: &RunEvent,
) {
    let Ok(finished) = serde_json::from_value::<runs::RunFinished>(event.data.clone()) else {
        tracing::warn!(target: "flows", run_id, "[flows] recovery PollRun terminal outcome could not be decoded");
        return;
    };
    if claim_finished(key.clone()) {
        crate::core::bus::BUS.publish(DomainEvent::FlowRunFinished {
            flow_id: flow_id.to_owned(),
            run_id: run_id.to_owned(),
            status: finished.status,
        });
    }
}

fn claim_finished(key: RunKey) -> bool {
    let mut finished = FINISHED_KEYS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !finished.0.insert(key.clone()) {
        return false;
    }
    finished.1.push_back(key);
    while finished.1.len() > RECENT_FINISHED_KEYS {
        if let Some(expired) = finished.1.pop_front() {
            finished.0.remove(&expired);
        }
    }
    true
}

fn publish_step(session: &RunSession, node_id: &str, status: &str) {
    crate::core::bus::BUS.publish(DomainEvent::FlowRunProgress {
        run_id: session.run_id.clone(),
        node_id: node_id.to_owned(),
        status: status.to_owned(),
    });
}

async fn export_segment(session: &RunSession, status: &str) {
    let observations = session.take_observations();
    let flow = session.flow().await;
    crate::flows::ops::export_run_to_langfuse(
        &session.config,
        &flow.name,
        &flow.id,
        &session.run_id,
        status,
        bus_trigger(session.trigger()),
        &observations,
    )
    .await;
}

fn bus_trigger(trigger: FlowRunTrigger) -> runs::FlowRunTrigger {
    match trigger {
        FlowRunTrigger::Rpc => runs::FlowRunTrigger::Rpc,
        FlowRunTrigger::Schedule => runs::FlowRunTrigger::Schedule,
        FlowRunTrigger::AppEvent => runs::FlowRunTrigger::AppEvent,
        FlowRunTrigger::Resume => runs::FlowRunTrigger::Resume,
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
