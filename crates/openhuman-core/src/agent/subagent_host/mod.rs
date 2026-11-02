//! OpenHuman adapters for the neutral TinyAgents subagent lifecycle.
//!
//! Given an [`crate::agent_registry::host_definition::AgentDefinition`] and a task prompt, the
//! runner:
//!
//! 1. Reads the [`fork_context::ParentExecutionContext`] the parent
//!    [`crate::agent::OpenHumanSessionHost::turn`] attached to the run context
//!    (`OpenHumanRunContext::parent`).
//! 2. Resolves the sub-agent's model name (inherit / hint / exact).
//! 3. Filters the parent's tool registry per `definition.tools`,
//!    `disallowed_tools`, and `skill_filter` (or, in `fork` mode,
//!    inherits the parent's tools verbatim).
//! 4. Builds a narrow system prompt that strips the sections the
//!    definition asks to omit (`omit_identity`, `omit_memory_context`,
//!    `omit_safety_preamble`).
//! 5. Runs the child turn on the TinyAgents harness (`ops::graph` →
//!    [`crate::agent::tinyagents::TurnSpec::run`]) using
//!    the parent's [`crate::inference::provider::Provider`], then
//!    mirrors the child transcript/progress and returns one compact tool result
//!    to the parent.
//!
//! This module owns product policy and effects: definition resolution, prompt
//! assembly, tool narrowing, model selection, workspace/sandbox setup,
//! artifacts, checkpoints, and progress. `tinyagents_orchestration::subagent`
//! owns lifecycle ordering, task-key coalescing and mutually-exclusive pause or
//! terminal persistence.
//!
//! ## Layout
//!
//! This is a light `mod.rs`: every item below is declared in a sibling
//! file and re-exported here.
//!
//! | File              | Contents                                                    |
//! | ----------------- | ----------------------------------------------------------- |
//! | `types.rs`        | `SubagentRun{Options,Outcome,Error}`, `SubagentMode`        |
//! | `ops/`            | typed/fork mode and orchestration route                    |
//! | `tool_prep.rs`    | Tool filtering + prompt loading + prompt protocol block     |

mod autonomous;
mod fork_context;
mod lifecycle;
mod ops;
mod tool_prep;
mod types;

#[cfg(test)]
#[path = "tool_ranking_tests.rs"]
mod tool_ranking_tests;

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "persisted_compat_tests.rs"]
mod persisted_compat_tests;

// Public API — the entry point and the shapes it returns.
pub use autonomous::{definition_iter_cap, subagent_iter_cap_with_autonomous_lift};
pub use fork_context::{AgentContextPreparedSource, ParentExecutionContext};
pub(crate) use lifecycle::checkpoint_resume;
pub(crate) use lifecycle::detached_subagent_parent;
pub(crate) use lifecycle::load_subagent_checkpoint;
pub(crate) use lifecycle::load_terminal_outcome;
pub(crate) use lifecycle::resume_async_via_driver_tool;
pub(crate) use lifecycle::spawn_async_via_driver_tool;
#[cfg(test)]
pub(crate) use lifecycle::spawn_async_via_driver_tool_with_jobs;
pub use lifecycle::OpenHumanSubagentHost;
/// Token + cost totals for one sub-agent run. TinyAgents owns the type
/// (`AgentProgress::SubagentCompleted` carries it); re-exported under its
/// historical host path.
pub use tinyagents_harness::progress::SubagentUsage;
pub use types::{
    SubagentCheckpointData, SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome,
    SubagentRunStatus, SubagentUsageExt,
};

pub(crate) use ops::resolve_subagent_source;
pub(crate) use ops::{append_reuse_follow_up, create_worker_thread};
