//! Multi-agent harness — sub-agent dispatch and fork-cache support.
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
//! - **Typed Sub-agents**: Skips unnecessary system prompt sections (e.g.,
//!   identity, global skills) to keep sub-agent prompts small.
//! - **Fork Mode**: Allows sub-agents to replay the parent's exact context
//!   to leverage KV-cache reuse on the inference backend.
//!
//! ## Key Sub-modules
//! - **[`subagent_runner`]**: The core logic for executing a sub-agent.
//! - **[`definition`]**: Data structures for defining an agent's archetype.
//! - **[`fork_context`]**: Task-local storage for parent context sharing.
//! - **[`interrupt`]**: Infrastructure for graceful cancellation of agent loops.

pub(crate) mod archivist;
pub(crate) mod builtin_definitions;
mod credentials;
pub mod definition;
pub(crate) mod definition_loader;
pub mod fork_context;
mod instructions;
pub mod interrupt;
pub(crate) mod memory_context;
mod parse;
pub(crate) mod self_healing;
pub mod session;
pub(crate) mod session_queue;
pub mod subagent_runner;
pub(crate) mod tool_filter;
mod tool_loop;

pub use definition::{
    AgentDefinition, AgentDefinitionRegistry, DefinitionSource, ModelSpec, PromptSource,
    SandboxMode, ToolScope,
};
pub use fork_context::{
    current_fork, current_parent, with_fork_context, with_parent_context, ForkContext,
    ParentExecutionContext,
};
pub use interrupt::{check_interrupt, InterruptFence, InterruptedError};
pub use subagent_runner::{run_subagent, SubagentRunError, SubagentRunOptions};

pub(crate) use instructions::build_tool_instructions_filtered;
pub(crate) use parse::parse_tool_calls;
pub(crate) use tool_loop::run_tool_call_loop;

#[cfg(test)]
mod tests;
