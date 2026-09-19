//! Injecting messages and crate-native control-flow directives into a running
//! sub-agent: the `RunQueue` compatibility lane, and the TinyAgents
//! `SteeringHandle` fast path for runs that have registered one.

use tinyagents_graph::orchestration::DetachedTaskRegistryError;
use tinyagents_harness::ids::TaskId;
use tinyagents_harness::run_queue::QueueLane;
use tinyagents_harness::steering::{SteeringCommand, SteeringCommandKind};
use tinyinference_llm::message::Message as TaMessage;

use super::cancel::now_ms;
use super::registry::registry;

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

/// Crate-native steering directives beyond the `InjectMessage`/collect lanes.
///
/// These map 1:1 onto the tinyagents [`SteeringCommand`] control variants that
/// the crate exposes (`Redirect`, `Pause`, `Resume`, `Cancel`). They are
/// delivered **only** through a registered `SteeringHandle` and therefore land
/// only at a safe loop boundary (the crate drains before each model call) —
/// never mid-stream, and never through the `RunQueue` fallback (which has no
/// equivalent lane). Approval/security is never bypassed: `Redirect` lowers to a
/// system instruction the normal approval-gated loop still governs, and
/// `Pause`/`Resume`/`Cancel` are pure control-flow.
///
/// The crate's `SetMetadata` command is intentionally *not* mapped here: no
/// OpenHuman control surface owns run-metadata mutation yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SteeringDirective {
    /// Redirect the run toward a new instruction (`SteeringCommand::Redirect`).
    Redirect(String),
    /// Cooperatively pause at the next checkpoint (`SteeringCommand::Pause`).
    Pause,
    /// Clear a pending pause (`SteeringCommand::Resume`).
    Resume,
    /// Cooperatively terminate at the next checkpoint (`SteeringCommand::Cancel`) —
    /// a graceful, safe-boundary alternative to the hard `AbortHandle` cancel.
    Cancel,
}

impl SteeringDirective {
    fn kind(&self) -> SteeringCommandKind {
        match self {
            SteeringDirective::Redirect(_) => SteeringCommandKind::Redirect,
            SteeringDirective::Pause => SteeringCommandKind::Pause,
            SteeringDirective::Resume => SteeringCommandKind::Resume,
            SteeringDirective::Cancel => SteeringCommandKind::Cancel,
        }
    }

    fn into_command(self) -> SteeringCommand {
        match self {
            SteeringDirective::Redirect(instruction) => SteeringCommand::Redirect { instruction },
            SteeringDirective::Pause => SteeringCommand::Pause,
            SteeringDirective::Resume => SteeringCommand::Resume,
            SteeringDirective::Cancel => SteeringCommand::Cancel,
        }
    }
}

/// Why a crate-native steering directive could not be delivered.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SteerDirectiveError {
    /// No such sub-agent — never existed, or already finished and pruned.
    Unknown,
    /// The caller's `parent_session` does not own this sub-agent.
    NotOwned,
    /// The sub-agent already reached a terminal status.
    AlreadyDone,
    /// The sub-agent has no live crate-native `SteeringHandle` registered
    /// (e.g. a legacy `RunQueue`-only run), so control-flow steering that has no
    /// `RunQueue` lane cannot be delivered.
    NoRegisteredHandle,
    /// The run's `SteeringPolicy` does not permit this directive's command
    /// kind. Enqueuing it anyway would abort the run with
    /// `TinyAgentsError::Steering`, so we refuse up front.
    PolicyRejected,
}

pub(crate) fn steer_directive_error_from_registry(
    error: DetachedTaskRegistryError,
) -> SteerDirectiveError {
    match error {
        DetachedTaskRegistryError::NotOwned => SteerDirectiveError::NotOwned,
        DetachedTaskRegistryError::AlreadyDone => SteerDirectiveError::AlreadyDone,
        DetachedTaskRegistryError::NoSteeringHandle => SteerDirectiveError::NoRegisteredHandle,
        _ => SteerDirectiveError::Unknown,
    }
}

/// Deliver a crate-native control-flow [`SteeringDirective`] to a running
/// sub-agent through its registered TinyAgents `SteeringHandle`.
///
/// Unlike [`steer`], this has **no** `RunQueue` fallback: the crate control
/// variants (`Redirect`/`Pause`/`Resume`/`Cancel`) have no OpenHuman queue lane,
/// so a run must have a live registered handle to receive them. The directive's
/// command kind is checked against the run's own `SteeringPolicy` *before*
/// enqueue — a disallowed command would otherwise abort the run — so this can
/// never smuggle a control kind past a policy that a tighter run class installed.
pub(crate) fn steer_directive(
    task_id: &str,
    parent_session: &str,
    directive: SteeringDirective,
) -> Result<(), SteerDirectiveError> {
    let handle = registry()
        .steering_handle(&TaskId::new(task_id), parent_session)
        .map_err(steer_directive_error_from_registry)?;
    let kind = directive.kind();
    if !handle.policy().is_allowed(kind) {
        log::warn!(
            "[running_subagents] directive rejected by run policy task_id={} kind={}",
            task_id,
            kind.as_str()
        );
        return Err(SteerDirectiveError::PolicyRejected);
    }
    handle.send(directive.into_command());
    log::info!(
        "[running_subagents] steered task_id={} directive={} via=tinyagents_registry",
        task_id,
        kind.as_str()
    );
    Ok(())
}

/// Inject a message into a running sub-agent. Prefer the crate-native
/// TinyAgents steering registry when the child run has registered its live
/// handle, and fall back to the OpenHuman `RunQueue` compatibility path.
pub async fn steer(
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
    let snapshot = registry()
        .snapshot(&task_id_key, parent_session)
        .map_err(steer_error_from_registry)?;
    if snapshot.status.is_terminal() {
        return Err(SteerError::AlreadyDone);
    }

    let steered_via_registry = registry()
        .steering_handle(&task_id_key, parent_session)
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

/// Trusted-control variant used by JSON-RPC sub-agent controls.
///
/// This intentionally does not require the caller to provide `parent_session`:
/// the RPC layer is already bearer-protected and mirrors the existing
/// `subagent_cancel` control surface, which can abort a task by id. The function
/// still refuses unknown or terminal tasks and never logs the steered text.
pub(crate) async fn steer_control(
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
    if snapshot.status.is_terminal() {
        return Err(SteerError::AlreadyDone);
    }

    let steered_via_registry = registry()
        .steering_handle_trusted(&task_id_key)
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
