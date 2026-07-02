//! Sub-agent execution entry points.
//!
//! The public runner lives in [`run_subagent`]. It dispatches to
//! [`runner::run_typed_mode`] (narrow prompt + filtered tools) which builds a
//! brand-new system prompt and a filtered tool list for the requested
//! archetype, then drives the turn through the tinyagents harness
//! ([`graph::run_subagent_via_graph`]) until the model returns without
//! further tool calls (or the iteration budget is exhausted).
//!
//! ## Layout
//!
//! | File                | Contents                                                       |
//! | ------------------- | -------------------------------------------------------------- |
//! | `provider.rs`       | `resolve_subagent_provider`, `user_is_signed_in_to_composio`, `LazyToolkitResolver` |
//! | `prompt.rs`         | Role-contract suffix, `append_subagent_role_contract`, `dedup_tool_specs_by_name` |
//! | `runner.rs`         | `run_subagent`, `run_typed_mode`                               |
//! | `graph.rs`          | `run_subagent_via_graph` — the sub-agent turn graph + tools    |
//! | `usage.rs`          | `AggregatedUsage` (cumulative usage stats)                     |
//! | `handoff_helper.rs` | `apply_handoff`                                                |
//! | `checkpoint.rs`     | `SubagentCheckpoint`, `parse_tool_arguments`                   |

mod checkpoint;
mod graph;
mod handoff_helper;
pub(crate) use handoff_helper::apply_handoff;
mod prompt;
mod provider;
mod runner;
mod usage;

// Public entry point — the primary API surface consumed by the parent module.
pub use runner::run_subagent;

// `user_is_signed_in_to_composio` is the mode-aware "can the user call
// composio at all?" probe added in Wave 2 (#1710). Re-exported here so
// non-composio probe sites (registration gates, heartbeat telemetry)
// can call it as
// `crate::openhuman::agent::harness::subagent_runner::user_is_signed_in_to_composio`
// without reaching into a private sibling module.
pub(crate) use provider::user_is_signed_in_to_composio;

// `resolve_subagent_provider` is called from tests via
// `super::resolve_subagent_provider`. Keep it accessible at the ops
// module boundary.
pub(crate) use provider::resolve_subagent_provider;

// Re-exports for test companion modules that use `use super::*`.
// These provide the same flat namespace the original ops.rs had.
#[cfg(test)]
pub(super) use prompt::{append_subagent_role_contract, dedup_tool_specs_by_name};
#[cfg(test)]
pub(super) use provider::{normalize_slug, LazyToolkitResolver};
// filter_tool_indices lives in tool_prep (sibling of ops).
#[cfg(test)]
pub(super) use super::tool_prep::filter_tool_indices;
// Types used by tests that were previously in scope via the flat ops.rs imports.
#[cfg(test)]
pub(super) use super::types::{
    SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome,
};
#[cfg(test)]
pub(super) use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
#[cfg(test)]
pub(super) use crate::openhuman::agent::harness::fork_context::ParentExecutionContext;
#[cfg(test)]
pub(super) use crate::openhuman::agent::harness::{
    current_spawn_depth, with_spawn_depth, MAX_SPAWN_DEPTH,
};
#[cfg(test)]
pub(super) use crate::openhuman::tools::{Tool, ToolSpec};

// Test companion modules — path references relative to their original location.
#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../ops_dedup_tests.rs"]
mod dedup_tests;

#[cfg(test)]
#[path = "../ops_truncation_tests.rs"]
mod truncation_tests;
