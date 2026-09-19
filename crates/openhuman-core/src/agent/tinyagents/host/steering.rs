//! OpenHuman's steering policy over TinyAgents' generic steering queue.

use std::sync::OnceLock;

use tinyagents_graph::orchestration::SteeringRegistry;
#[cfg(test)]
use tinyagents_graph::{
    InMemoryTaskStore, OrchestrationTaskKind, OrchestrationTaskResult, OrchestrationTaskSpec,
    OrchestrationTaskStatus, TaskStore,
};
#[cfg(test)]
use tinyagents_harness::ids::TaskId;
use tinyagents_harness::steering::{SteeringCommandKind, SteeringHandle, SteeringPolicy};

static STEERING_REGISTRY: OnceLock<SteeringRegistry> = OnceLock::new();

/// Process-local registry for TinyAgents steering handles keyed by detached task id.
pub(crate) fn shared_steering_registry() -> &'static SteeringRegistry {
    STEERING_REGISTRY.get_or_init(SteeringRegistry::new)
}

/// Run class used to tighten OpenHuman's steering allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SteeringRunClass {
    /// The user's live interactive chat turn.
    Interactive,
    /// A detached/background sub-agent turn.
    Background,
}

/// Creates a steering handle restricted to controls OpenHuman authorizes.
pub(crate) fn openhuman_steering_handle(run_class: SteeringRunClass) -> SteeringHandle {
    let mut policy = SteeringPolicy::new()
        .allow(SteeringCommandKind::InjectMessage)
        .allow(SteeringCommandKind::Pause);
    if run_class == SteeringRunClass::Background {
        policy = policy
            .allow(SteeringCommandKind::Resume)
            .allow(SteeringCommandKind::Cancel)
            .allow(SteeringCommandKind::Redirect);
    }
    SteeringHandle::new(policy)
}

#[cfg(test)]
#[path = "../orchestration_tests.rs"]
mod tests;
