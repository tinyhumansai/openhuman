//! Injecting messages and crate-native control-flow directives into a running
//! sub-agent: the `RunQueue` compatibility lane, and the TinyAgents
//! `SteeringHandle` fast path for runs that have registered one.

use tinyagents_harness::ids::TaskId;
use tinyagents_harness::run_queue::QueueLane;
use tinyagents_harness::steering::SteeringCommand;
use tinyagents_tasks::DetachedTaskRegistryError;
use tinyinference_llm::message::Message as TaMessage;

use super::cancel::now_ms;
use super::registry::{control_admits, owner_key, registry};

/// Why a steer could not be delivered.
#[derive(Debug, PartialEq, Eq)]
pub enum SteerError {
    /// No such sub-agent — never existed, or already finished and pruned.
    Unknown,
    /// The caller's `parent_session` does not own this sub-agent.
    NotOwned,
    /// The sub-agent already reached a terminal status.
    AlreadyDone,
    /// Detached sub-agents can only receive an injected instruction or
    /// collected context. Follow-up work is a web-turn concern and cannot be
    /// meaningfully dispatched from this fallback queue.
    UnsupportedLane,
}

pub(crate) fn steer_error_from_registry(error: DetachedTaskRegistryError) -> SteerError {
    match error {
        DetachedTaskRegistryError::NotOwned => SteerError::NotOwned,
        DetachedTaskRegistryError::AlreadyDone => SteerError::AlreadyDone,
        _ => SteerError::Unknown,
    }
}

fn steering_command_for_lane(lane: QueueLane, text: String) -> Option<SteeringCommand> {
    match lane {
        QueueLane::Steer => Some(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "[User steering message]: {text}"
        )))),
        QueueLane::Collect => Some(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "[Additional context from user]: {text}"
        )))),
        QueueLane::Followup => None,
    }
}

fn queue_lane_name(lane: QueueLane) -> &'static str {
    match lane {
        QueueLane::Steer => "steer",
        QueueLane::Followup => "followup",
        QueueLane::Collect => "collect",
    }
}

fn send_registered_steering(
    handle: &tinyagents_harness::steering::SteeringHandle,
    text: String,
    lane: QueueLane,
) -> bool {
    let Some(command) = steering_command_for_lane(lane, text) else {
        return false;
    };
    handle.send(command);
    true
}

/// Inject a message into a running sub-agent. Prefer the crate-native
/// TinyAgents steering registry when the child run has registered its live
/// handle, and fall back to the OpenHuman `RunQueue` compatibility path.
pub async fn steer(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    parent_session: &str,
    text: String,
    lane: QueueLane,
) -> Result<(), SteerError> {
    if !matches!(lane, QueueLane::Steer | QueueLane::Collect) {
        log::warn!(
            "[running_subagents] rejected unsupported queue lane task_id={} lane={}",
            task_id,
            queue_lane_name(lane)
        );
        return Err(SteerError::UnsupportedLane);
    }
    let task_id_key = TaskId::new(task_id);
    let owner = owner_key(ctx, parent_session).map_err(|_| SteerError::NotOwned)?;
    let snapshot = registry()
        .snapshot(&task_id_key, &owner)
        .map_err(steer_error_from_registry)?;
    if snapshot.status.is_terminal() {
        return Err(SteerError::AlreadyDone);
    }

    let steered_via_registry = registry()
        .steering_handle(&task_id_key, &owner)
        .map(|handle| send_registered_steering(&handle, text.clone(), lane))
        .unwrap_or(false);
    if steered_via_registry {
        log::info!(
            "[running_subagents] steered task_id={} lane={} via=tinyagents_registry",
            task_id,
            queue_lane_name(lane)
        );
        return Ok(());
    }

    snapshot
        .metadata
        .run_queue
        .push(
            lane,
            crate::agent::queued_turn::QueuedTurn {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                client_id: "steer_subagent".to_string(),
                thread_id: task_id.to_string(),
                queued_at_ms: now_ms(),
                model_override: None,
                temperature: None,
                locale: None,
            },
        )
        .await;
    log::info!(
        "[running_subagents] steered task_id={} lane={}",
        task_id,
        queue_lane_name(lane)
    );
    Ok(())
}

/// User-control variant used by JSON-RPC sub-agent controls.
///
/// This intentionally does not require the caller to provide `parent_session`:
/// the captured profile is checked against the caller in SaaS. Desktop controls
/// retain authority over the process user's workers. Unknown, foreign or
/// terminal tasks are refused, and steered text is never logged.
pub(crate) async fn steer_control(
    ctx: &crate::core::runtime::DispatchCtx,
    task_id: &str,
    text: String,
    lane: QueueLane,
) -> Result<(), SteerError> {
    if !matches!(lane, QueueLane::Steer | QueueLane::Collect) {
        log::warn!(
            "[running_subagents] rejected unsupported control queue lane task_id={} lane={}",
            task_id,
            queue_lane_name(lane)
        );
        return Err(SteerError::UnsupportedLane);
    }
    let task_id_key = TaskId::new(task_id);
    let snapshot = registry()
        .snapshot_trusted(&task_id_key)
        .map_err(steer_error_from_registry)?;
    if !control_admits(ctx, &snapshot.metadata) {
        return Err(SteerError::NotOwned);
    }
    if snapshot.status.is_terminal() {
        return Err(SteerError::AlreadyDone);
    }

    let steered_via_registry = registry()
        .steering_handle(&task_id_key, &snapshot.owner_id)
        .map(|handle| send_registered_steering(&handle, text.clone(), lane))
        .unwrap_or(false);
    if steered_via_registry {
        log::info!(
            "[running_subagents] control_steered task_id={} lane={} via=tinyagents_registry",
            task_id,
            queue_lane_name(lane)
        );
        return Ok(());
    }

    snapshot
        .metadata
        .run_queue
        .push(
            lane,
            crate::agent::queued_turn::QueuedTurn {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                client_id: "subagent_control_rpc".to_string(),
                thread_id: task_id.to_string(),
                queued_at_ms: now_ms(),
                model_override: None,
                temperature: None,
                locale: None,
            },
        )
        .await;
    log::info!(
        "[running_subagents] control_steered task_id={} lane={}",
        task_id,
        queue_lane_name(lane)
    );
    Ok(())
}
