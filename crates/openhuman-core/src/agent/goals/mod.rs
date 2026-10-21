//! Thin OpenHuman host adapters for [`tinyagents_harness_tools::goals`].
//!
//! Tinyagents owns the goal types, lifecycle, persistence, prompt rendering,
//! graph continuation, and native harness tools. This module contains the
//! remaining OpenHuman-specific adapters: workspace-store resolution, domain
//! events, and bridges for
//! the shared tool contract and TinyAgents' stop-hook contract.

pub mod runtime;
pub mod tools;

use tinyagents_harness_tools::goals::ThreadGoal;
pub use tools::goal_tools;

/// Serialize a [`ThreadGoal`] for the `goal` field on `ThreadGoalUpdated` /
/// `threads.goal_get`. `ThreadGoal` is owned by `tinyagents-graph`, so this is
/// kept as a raw `Value` rather than a typed field on the event. Falls back to
/// `Value::Null` on an (unexpected) serialization failure rather than
/// panicking or dropping the event.
#[must_use]
pub fn goal_to_value(goal: &ThreadGoal) -> serde_json::Value {
    serde_json::to_value(goal).unwrap_or(serde_json::Value::Null)
}
