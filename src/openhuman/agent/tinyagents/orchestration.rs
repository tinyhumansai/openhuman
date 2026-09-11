//! Shared orchestration helpers on the `tinyagents` graph layer (issue #4249).
//!
//! OpenHuman's control plane historically hand-rolled fan-out
//! ([`futures_util::future::join_all`]) and a bespoke detached-sub-agent registry
//! (raw `tokio` `AbortHandle`s, `watch` status channels, tombstone sets). This
//! module is the shared seam that re-expresses that work on `tinyagents`
//! primitives so the detached-sub-agent control plane routes through one place:
//!
//! - The `graph::orchestration` task primitives ([`TaskStore`],
//!   [`OrchestrationTaskKind`], …) are re-exported here so the detached-sub-agent
//!   control plane gets typed task lifecycle bookkeeping (Pending → Running →
//!   Completed/Failed/Cancelled/…) instead of bespoke status enums + watch
//!   channels + tombstones. The store tracks durable lifecycle while
//!   [`DetachedTaskRegistry`] owns the process-local status, cancellation,
//!   hard-abort, ownership, and steering mechanics. OpenHuman retains its
//!   product metadata and `RunQueue` compatibility fallback.
//!
//! Graph lifecycle events are mirrored onto tracing via the shared
//! [`GraphTracingSink`](crate::openhuman::agent::tinyagents::observability::GraphTracingSink).

use std::sync::OnceLock;

// Re-export the tinyagents task-orchestration primitives so the detached
// sub-agent control plane imports lifecycle types from one openhuman path.
pub(crate) use tinyagents_graph::orchestration::OrchestrationTaskStatus;
#[allow(unused_imports)]
pub(crate) use tinyagents_graph::orchestration::SteeringRegistry;
pub(crate) use tinyagents_graph::orchestration::{
    open_jsonl_task_store_or_memory, reconcile_orphaned_tasks, DetachedTaskRegistry,
    DetachedTaskRegistryError, DetachedTaskWaitOutcome, InMemoryTaskStore, OrchestrationTaskFilter,
    OrchestrationTaskKind, OrchestrationTaskRecord, OrchestrationTaskResult, OrchestrationTaskSpec,
    TaskStore, TaskStoreRegistry,
};
#[allow(unused_imports)]
pub(crate) use tinyagents_harness::ids::TaskId;
#[allow(unused_imports)]
pub(crate) use tinyagents_harness::steering::{
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
#[path = "orchestration_tests.rs"]
mod tests;
