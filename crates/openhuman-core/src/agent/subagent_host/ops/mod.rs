//! Sub-agent execution entry points.
//!
//! The registered dispatcher reaches this adapter, which dispatches to
//! [`runner::run_typed_mode`] (narrow prompt + filtered tools) to build a
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
//! | `prompt.rs`         | Child role framing and artifact-offload contract |
//! | `runner.rs`         | direct host execution and `run_typed_mode`                   |
//! | `graph.rs`          | `run_subagent_via_graph` — the sub-agent turn graph + tools    |
//! | `checkpoint.rs`     | `SubagentCheckpoint` — cap-hit summary                         |
//! | `pause_checkpoint.rs` | `is_safe_task_id` — validates model-provided checkpoint names |

mod checkpoint;
mod graph;
mod pause_checkpoint;
// The checkpoint filename validator rejects model-authored task ids before the
// continue path resolves a checkpoint.
pub(crate) use pause_checkpoint::is_safe_task_id;
mod prompt;
mod provider;
mod runner;
mod runner_artifacts;
mod worker_mirror;
mod worker_thread;

pub(crate) use worker_mirror::{mirror_worker_thread, mirror_worker_thread_from_history};
pub(crate) use worker_thread::{append_reuse_follow_up, create_worker_thread};

#[cfg(test)]
#[path = "worker_thread_tests.rs"]
mod worker_thread_tests;

// The lifecycle adapter owns the public entry point.  This is its host-only
// execution leaf after a neutral driver has prepared and admitted a request.
pub(crate) use runner::run_subagent_direct_with_native_context;

pub(crate) use provider::resolve_subagent_source;

// Re-exports for test companion modules that use `use super::*`.
// These provide the same flat namespace the original ops.rs had.
// filter_tool_indices lives in tool_prep (sibling of ops).
#[cfg(test)]
pub(super) use super::tool_prep::filter_tool_indices;
// Types used by tests that were previously in scope via the flat ops.rs imports.
#[cfg(test)]
pub(super) use super::types::{SubagentMode, SubagentRunError, SubagentRunOptions};
#[cfg(test)]
pub(super) use crate::agent::subagent_host::ParentExecutionContext;
#[cfg(test)]
pub(super) use crate::agent::tinyagents::budget::MAX_SPAWN_DEPTH;
#[cfg(test)]
pub(super) use crate::agent_registry::host_definition::AgentDefinition;
#[cfg(test)]
use tinyagents_definition::PromptSource;
// Test companion modules — path references relative to their original location.
#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
