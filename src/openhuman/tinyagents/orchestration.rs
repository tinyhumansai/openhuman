//! Shared orchestration helpers on the `tinyagents` graph layer (issue #4249).
//!
//! openhuman's control plane historically hand-rolled fan-out
//! ([`futures_util::future::join_all`]) and a bespoke detached-sub-agent registry
//! (raw `tokio` `AbortHandle`s, `watch` status channels, tombstone sets). This
//! module is the shared seam that re-expresses that work on `tinyagents`
//! primitives so the detached-sub-agent control plane routes through one place:
//!
//! - The `graph::orchestration` task primitives ([`TaskStore`],
//!   [`OrchestrationTaskKind`], …) are re-exported here so the detached-sub-agent
//!   control plane gets typed task lifecycle bookkeeping (Pending → Running →
//!   Completed/Failed/Cancelled/…) instead of bespoke status enums + watch
//!   channels + tombstones. The store tracks lifecycle; the caller still owns the
//!   executor (the `tokio` task + cooperative cancel + hard abort).
//! - [`SteeringRegistry`] is re-exported as the next bridge for task-id-addressed
//!   steering. Today the live control path still goes through OpenHuman's
//!   `RunQueue`; the registry gives the follow-up patch one local import seam for
//!   registering the TinyAgents [`SteeringHandle`] per detached task.
//!
//! Graph lifecycle events are mirrored onto tracing via the shared
//! [`GraphTracingSink`](crate::openhuman::tinyagents::observability::GraphTracingSink).

use std::sync::OnceLock;

// Re-export the tinyagents task-orchestration primitives so the detached
// sub-agent control plane imports lifecycle types from one openhuman path.
pub(crate) use tinyagents::graph::orchestration::OrchestrationTaskStatus;
#[allow(unused_imports)]
pub(crate) use tinyagents::graph::orchestration::SteeringRegistry;
pub(crate) use tinyagents::graph::orchestration::{
    InMemoryTaskStore, JsonlTaskStore, OrchestrationControlOutcome, OrchestrationTaskFilter,
    OrchestrationTaskKind, OrchestrationTaskRecord, OrchestrationTaskResult, OrchestrationTaskSpec,
    TaskStore,
};
#[allow(unused_imports)]
pub(crate) use tinyagents::harness::ids::TaskId;
#[allow(unused_imports)]
pub(crate) use tinyagents::harness::steering::{
    SteeringCommand, SteeringCommandKind, SteeringHandle, SteeringPolicy,
};

static STEERING_REGISTRY: OnceLock<SteeringRegistry> = OnceLock::new();

/// Process-local registry for TinyAgents steering handles keyed by detached
/// task id. The current product control path still uses OpenHuman's `RunQueue`;
/// this registry is the crate-native lookup seam for the next control-plane
/// migration slice.
pub(crate) fn shared_steering_registry() -> &'static SteeringRegistry {
    STEERING_REGISTRY.get_or_init(SteeringRegistry::new)
}

/// Run class of a TinyAgents turn, used to tighten the steering allowlist.
///
/// The distinction matters for steering safety: an *interactive* turn is the
/// user's own live chat turn, where the only trusted controls are transcript
/// injection (user/orchestrator steering) and cooperative `Pause`. A
/// *background* turn is a detached sub-agent run with no live user transcript of
/// its own, so it can additionally accept crate-native control-flow steering
/// (`Resume`, `Cancel`) and `Redirect` — a graceful, safe-boundary alternative
/// to the hard `AbortHandle` cancel — without ever widening what the interactive
/// path accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SteeringRunClass {
    /// The user's live interactive chat turn.
    Interactive,
    /// A detached / background sub-agent run.
    Background,
}

/// Steering handle policy for OpenHuman's shared TinyAgents turn path, tightened
/// per [`SteeringRunClass`].
///
/// Interactive turns send `InjectMessage` for user/orchestrator steering and
/// `Pause` for cooperative early-exit, cap, stop-hook, and repeated-failure
/// halts — nothing else, matching the prior behavior exactly. Background
/// (detached sub-agent) runs additionally accept `Resume`, `Cancel`, and
/// `Redirect`: control-flow steering that never injects untrusted transcript and
/// still lands only at a safe loop boundary (the crate drains before each model
/// call). A command whose kind is not in the allowlist is *rejected* by the
/// crate and aborts the run with `TinyAgentsError::Steering`, so callers must
/// only enqueue kinds this policy permits (see `running_subagents::steer_directive`).
pub(crate) fn openhuman_steering_handle(run_class: SteeringRunClass) -> SteeringHandle {
    let mut policy = SteeringPolicy::new()
        .allow(SteeringCommandKind::InjectMessage)
        .allow(SteeringCommandKind::Pause);
    if run_class == SteeringRunClass::Background {
        // Background-only widening: accept graceful control-flow steering without
        // also accepting transcript injection beyond the shared `InjectMessage`
        // lane. `Cancel` is the crate-native, safe-boundary equivalent of the
        // hard abort; `Resume` lifts a `Pause`; `Redirect` lowers to a system
        // instruction the normal approval-gated loop still governs.
        policy = policy
            .allow(SteeringCommandKind::Resume)
            .allow(SteeringCommandKind::Cancel)
            .allow(SteeringCommandKind::Redirect);
    }
    SteeringHandle::new(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_store_tracks_lifecycle() {
        // Smoke the re-exported orchestration primitives: a task moves
        // Pending → Running → Completed and is readable back by id.
        let store = InMemoryTaskStore::new();
        let spec = OrchestrationTaskSpec::new(
            "task-1",
            OrchestrationTaskKind::SubAgent {
                agent: "researcher".to_string(),
            },
        );
        let rec = store.insert(spec).expect("insert");
        assert_eq!(rec.status, OrchestrationTaskStatus::Pending);

        store.mark_running(rec.task_id()).expect("running");
        let done = store
            .complete(rec.task_id(), OrchestrationTaskResult::text("done"))
            .expect("complete");
        assert_eq!(done.status, OrchestrationTaskStatus::Completed);
        assert_eq!(
            store.get(rec.task_id()).map(|r| r.status),
            Some(OrchestrationTaskStatus::Completed)
        );
    }

    #[test]
    fn steering_registry_reexport_registers_task_handles() {
        let registry = shared_steering_registry();
        let handle = openhuman_steering_handle(SteeringRunClass::Background);
        let task_id = TaskId::new("task-steer");

        registry.register(task_id.clone(), handle);
        assert!(registry.get(&task_id).is_some());
        assert!(registry.deregister(&task_id).is_some());
        assert!(registry.get(&task_id).is_none());
    }

    #[test]
    fn steering_policy_tightens_by_run_class() {
        // Interactive: only the two long-standing kinds; control-flow steering
        // stays closed so the user's live turn can't be cancelled/redirected
        // out from under it via a rogue steer.
        let interactive = openhuman_steering_handle(SteeringRunClass::Interactive);
        let policy = interactive.policy();
        assert!(policy.is_allowed(SteeringCommandKind::InjectMessage));
        assert!(policy.is_allowed(SteeringCommandKind::Pause));
        assert!(!policy.is_allowed(SteeringCommandKind::Cancel));
        assert!(!policy.is_allowed(SteeringCommandKind::Resume));
        assert!(!policy.is_allowed(SteeringCommandKind::Redirect));
        assert!(!policy.is_allowed(SteeringCommandKind::SetMetadata));

        // Background: additionally accepts graceful control-flow steering.
        let background = openhuman_steering_handle(SteeringRunClass::Background);
        let policy = background.policy();
        assert!(policy.is_allowed(SteeringCommandKind::InjectMessage));
        assert!(policy.is_allowed(SteeringCommandKind::Pause));
        assert!(policy.is_allowed(SteeringCommandKind::Cancel));
        assert!(policy.is_allowed(SteeringCommandKind::Resume));
        assert!(policy.is_allowed(SteeringCommandKind::Redirect));
        // Metadata replacement stays closed on every class until a control
        // surface owns it.
        assert!(!policy.is_allowed(SteeringCommandKind::SetMetadata));
    }
}
