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
//! | `provider.rs`       | `resolve_subagent_source`, `user_is_signed_in_to_composio`       |
//! | `prompt.rs`         | Role-contract suffix, `append_subagent_role_contract`, `dedup_tool_specs_by_name` |
//! | `runner.rs`         | `run_subagent`, `run_typed_mode`                               |
//! | `graph.rs`          | `run_subagent_via_graph` — the sub-agent turn graph + tools    |
//! | `checkpoint.rs`     | `SubagentCheckpoint` — cap-hit summary                         |
//! | `pause_checkpoint.rs` | `is_safe_task_id` — the guard for model-authored task ids        |

mod checkpoint;
mod graph;
mod pause_checkpoint;
// The checkpoint filename validator, shared with `continue_subagent`: the
// model-authored `task_id` must be rejected at the tool boundary as well as at
// the write, so the read path cannot traverse either.
pub(crate) use pause_checkpoint::is_safe_task_id;
mod prompt;
mod provider;
mod runner;

// The lifecycle adapter owns the public entry point.  This is its host-only
// execution leaf after a neutral driver has prepared and admitted a request.
pub(crate) use runner::run_subagent_direct;

// `user_is_signed_in_to_composio` is the mode-aware "can the user call
// composio at all?" probe added in Wave 2 (#1710). Re-exported here so
// non-composio probe sites (registration gates, telemetry)
// can call it as
// `crate::agent::subagent_host::user_is_signed_in_to_composio`
// without reaching into a private sibling module.
pub(crate) use provider::user_is_signed_in_to_composio;

pub(crate) use prompt::append_subagent_role_contract;
pub(crate) use provider::resolve_subagent_source;

// Re-exports for test companion modules that use `use super::*`.
// These provide the same flat namespace the original ops.rs had.
// `append_subagent_role_contract` is already re-exported (pub(crate)) above.
// `dedup_tool_specs_by_name` is private in its source
// module and cannot be `pub`-re-exported; a plain `use` still makes them
// reachable from the `#[cfg(test)]` companion modules via their `use super::*`
// (descendants can name ancestor-private imports).
#[cfg(test)]
use prompt::dedup_tool_specs_by_name;
// filter_tool_indices lives in tool_prep (sibling of ops).
#[cfg(test)]
pub(super) use super::tool_prep::filter_tool_indices;
// Types used by tests that were previously in scope via the flat ops.rs imports.
#[cfg(test)]
pub(super) use super::types::{SubagentMode, SubagentRunError, SubagentRunOptions};
#[cfg(test)]
pub(super) use crate::agent::harness::fork_context::ParentExecutionContext;
#[cfg(test)]
pub(super) use crate::agent::harness::host_definition::AgentDefinition;
#[cfg(test)]
pub(super) use crate::agent::tinyagents::budget::MAX_SPAWN_DEPTH;
#[cfg(test)]
use tinyagents_definition::PromptSource;
#[cfg(test)]
use tinytools::ToolSpec;

// Test companion modules — path references relative to their original location.
#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../ops_dedup_tests.rs"]
mod dedup_tests;
