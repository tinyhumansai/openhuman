//! Multi-agent harness — sub-agent dispatch and parent-context plumbing.
//!
//! The harness provides the infrastructure for an agent to delegate work to
//! specialized sub-agents. It manages the lifecycle of these sub-agents,
//! including prompt construction, tool filtering, and result synthesis.
//!
//! ## Delegation via `spawn_subagent`
//! The system treats specialized agents (researchers, planners, etc.) as tools.
//! An agent can invoke the `spawn_subagent` tool, which looks up a definition
//! in the global [`AgentDefinitionRegistry`] and runs a dedicated tool loop.
//!
//! ## Token Optimization
//! - **Typed Sub-agents**: Skip unnecessary system prompt sections (e.g.,
//!   identity, global skills) to keep sub-agent prompts small.
//!
//! ## Key Sub-modules
//! - **[`subagent_runner`]**: The core logic for executing a sub-agent.
//! - **[`definition`]**: Data structures for defining an agent's archetype.
//! - **[`fork_context`]**: Task-local storage for parent context sharing.
//! - **[`interrupt`]**: Infrastructure for graceful cancellation of agent loops.

pub mod archivist;
pub(crate) mod builtin_definitions;
pub(crate) mod compaction;
mod credentials;
pub mod definition;
pub(crate) mod definition_loader;
pub(crate) mod engine;
pub mod fork_context;
mod instructions;
pub mod interrupt;
pub(crate) mod memory_context;
pub(crate) mod memory_context_safety;
pub mod model_vision_context;
mod parse;
pub(crate) mod payload_summarizer;
pub mod run_queue;
pub mod sandbox_context;
pub(crate) mod self_healing;
pub mod session;
pub(crate) mod session_queue;
pub(crate) mod spawn_depth_context;
pub mod subagent_runner;
pub mod task_recency_context;
pub(crate) mod token_budget;
pub(crate) mod tool_filter;
mod tool_loop;
pub(crate) mod tool_result_artifacts;
pub mod turn_attachments_context;
pub mod worktree_context;

pub use definition::{
    AgentDefinition, AgentDefinitionRegistry, DefinitionSource, ModelSpec, PromptSource,
    SandboxMode, ToolScope, TriggerMemoryAgent,
};
pub use fork_context::{current_parent, with_parent_context, ParentExecutionContext};
pub use interrupt::{check_interrupt, InterruptFence, InterruptedError};
pub use model_vision_context::{current_model_vision, with_current_model_vision};
pub use sandbox_context::{current_sandbox_mode, with_current_sandbox_mode};
pub(crate) use spawn_depth_context::{current_spawn_depth, with_spawn_depth, MAX_SPAWN_DEPTH};
pub use subagent_runner::{run_subagent, SubagentRunError, SubagentRunOptions};
pub use task_recency_context::{current_task_recency_window, with_task_recency_window};
pub use worktree_context::{current_action_dir_override, with_action_dir_override};

pub(crate) use instructions::build_tool_instructions_filtered;
pub(crate) use parse::parse_tool_calls;
pub(crate) use tool_loop::run_tool_call_loop;

#[cfg(test)]
mod bughunt_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod test_support_tests;

#[cfg(test)]
mod harness_gap_tests;
#[cfg(test)]
mod tests;
