//! Multi-agent harness support — agent definitions, turn graphs, and artifacts.
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
//! Cancellation is handled by the tinyagents steering channel (see
//! `crate::agent::tinyagents`); there is no in-house interrupt fence.

pub mod artifact_offload;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod tool_result_artifacts;

// NOTE: deliberately no flat re-export here. `artifact_offload::ArtifactKind`
// would shadow the unrelated `openhuman::artifacts::ArtifactKind` for anyone
// glob-importing this module; callers import the owning library directly.
