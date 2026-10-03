//! Multi-agent harness — sub-agent dispatch and parent-context plumbing.
//!
//! The harness provides the infrastructure for an agent to delegate work to
//! specialized sub-agents. It manages the lifecycle of these sub-agents,
//! including prompt construction, tool filtering, and result synthesis.
//!
//! ## Delegation via `spawn_subagent`
//! The system treats specialized agents (planners, code executors, etc.) as tools.
//! An agent can invoke the `spawn_subagent` tool, which looks up a definition
//! in the global [`AgentDefinitionRegistry`] and runs a dedicated tool loop.
//!
//! ## Token Optimization
//! - **Typed Sub-agents**: Skip unnecessary system prompt sections (e.g.,
//!   identity, global skills) to keep sub-agent prompts small.
//!
//! ## Key Sub-modules
//! - **[`definition`]**: Data structures for defining an agent's archetype.
//! - **[`fork_context`]**: Task-local storage for parent context sharing.
//!
//! Cancellation is handled by the tinyagents steering channel (see
//! `crate::agent::tinyagents`); there is no in-house interrupt fence.

pub mod agent_graph;
pub mod artifact_offload;
pub(crate) mod builtin_definitions;
pub mod definition;
pub(crate) mod definition_loader;
pub mod fork_context;
pub(crate) mod graph;
pub(crate) mod memory_context_safety;
pub mod sandbox_context;
pub(crate) mod spawn_depth_context;
pub mod task_recency_context;
pub(crate) mod tool_result_artifacts;

pub use agent_graph::{AgentGraph, AgentTurnRequest, AgentTurnResult, AgentTurnUsage};
// NOTE: deliberately no flat re-export here. `artifact_offload::ArtifactKind`
// would shadow the unrelated `openhuman::agent::artifacts::ArtifactKind` for anyone
// glob-importing this module; callers use the `artifact_offload::` path.
pub use definition::{
    AgentDefinition, AgentDefinitionRegistry, DefinitionSource, ModelSpec, PromptSource,
    SandboxMode, ToolScope,
};
pub use fork_context::{
    current_parent, with_parent_context, AgentContextPreparedSource, ParentExecutionContext,
};
pub use sandbox_context::{current_sandbox_mode, with_current_sandbox_mode};
pub(crate) use spawn_depth_context::{with_spawn_depth, MAX_SPAWN_DEPTH};
pub use task_recency_context::{current_task_recency_window, with_task_recency_window};

pub(crate) use graph::run_channel_turn_via_graph;
