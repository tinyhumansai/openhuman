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
//!
//! Cancellation is handled by the tinyagents steering channel (see
//! `crate::openhuman::tinyagents`); there is no in-house interrupt fence.

pub mod agent_graph;
pub mod archivist;
pub(crate) mod builtin_definitions;
pub(crate) mod credentials;
pub mod definition;
pub(crate) mod definition_loader;
pub mod fork_context;
pub(crate) mod graph;
mod instructions;
pub(crate) mod memory_context;
pub(crate) mod memory_context_safety;
pub(crate) mod memory_protocol;
pub(crate) mod parse;
pub(crate) mod required_output;
pub mod run_queue;
pub mod sandbox_context;
pub mod session;
mod spawn_depth_context;
pub mod subagent_runner;
pub mod task_recency_context;
pub(crate) mod tool_filter;
pub(crate) mod tool_result_artifacts;
pub mod turn_attachments_context;
pub(crate) mod turn_subagent_usage;

pub use agent_graph::{AgentGraph, AgentTurnRequest, AgentTurnResult, AgentTurnUsage};
pub use definition::{
    AgentDefinition, AgentDefinitionRegistry, DefinitionSource, ModelSpec, PromptSource,
    SandboxMode, ToolScope, TriggerMemoryAgent,
};
pub use fork_context::{
    current_agent_context_prepared_sources, current_parent, push_agent_context_prepared_source,
    with_agent_context_prepared_sources, with_parent_context, AgentContextPreparedSource,
    ParentExecutionContext,
};
pub use sandbox_context::{current_sandbox_mode, with_current_sandbox_mode};
pub(crate) use spawn_depth_context::{current_spawn_depth, with_spawn_depth, MAX_SPAWN_DEPTH};
pub use subagent_runner::{run_subagent, SubagentRunError, SubagentRunOptions};
pub use task_recency_context::{current_task_recency_window, with_task_recency_window};

pub(crate) use graph::run_channel_turn_via_graph;
pub(crate) use instructions::build_tool_instructions_filtered;
pub(crate) use parse::{parse_tool_calls, parse_tool_calls_with_pformat};

#[cfg(test)]
mod harness_gap_tests;
#[cfg(test)]
mod tests;
