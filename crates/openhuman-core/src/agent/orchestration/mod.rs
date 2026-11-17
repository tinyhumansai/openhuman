//! High-level agent-to-agent orchestration domain.
//!
//! This module owns the control-plane semantics for coordinating multiple
//! agent workers from one parent session. The lower-level
//! [`crate::agent::harness`] module remains responsible for prompt
//! construction, tool filtering, and the actual sub-agent run loop.
//!
//! Agent turns can dispatch live detached sub-agents through TinyAgents'
//! orchestration ports. OpenHuman supplies their definitions, policy,
//! persistence, steering, progress projection, and host-facing tools.
//! [`running_subagents`] mirrors detached-sub-agent lifecycle into a
//! TinyAgents task store.

pub(crate) mod background_completions;
pub(crate) mod background_delivery;
pub(crate) mod busy_guard;
pub(crate) mod completion_notice;
pub(crate) mod completion_owners;
pub(crate) mod completion_target;
pub(crate) mod delivery_drain;
pub(crate) mod running_subagents;
pub(crate) mod subagent_events;
pub(crate) mod subagent_sessions;
pub mod tools;

#[cfg(test)]
#[path = "background_completions_tenant_tests.rs"]
mod background_completions_tenant_tests;

#[cfg(test)]
#[path = "background_delivery_tenant_tests.rs"]
mod background_delivery_tenant_tests;

/// Release the background-completion logs this process holds open. The core
/// calls this on exit so a data reset can delete the workspace directory.
pub async fn release_background_completion_stores() -> usize {
    background_completions::release_all().await
}
