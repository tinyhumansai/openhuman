//! Declares the LLM-callable orchestration tools kept in `tools/` (via
//! `#[path]`, since this file lives in `orchestration/` rather than a
//! `tools/mod.rs`).
//!
//! Tools, by role:
//! - **Spawn**: `spawn_subagent`, `spawn_async_subagent`,
//!   `spawn_parallel_agents`, `spawn_worker_thread`.
//! - **Control**: `steer_subagent`, `continue_subagent`, `close_subagent`,
//!   `wait_subagent`, `wait` / `wait_loop`, `list_subagents`.
//! - **Delegation**: `DelegateGraphTool`, `ArchetypeDelegationTool`,
//!   `CollapsedDelegationTool` (`delegate_to`), and `agent_prepare_context`.
//!
//! `dispatch.rs`, `awaiting_user.rs`, and `worker_thread.rs` are `pub(crate)`
//! helpers shared by the tools above (the common spawn path, the awaiting-user
//! envelope, and worker thread creation), not tools themselves.
//!
//! All tools are re-exported through `crate::tools` (`tools/mod.rs`:
//! `pub use crate::agent::orchestration::tools::*`), which is how the agent
//! tool-calling loop discovers them. Execution itself goes through
//! `agent::subagent_host::run_subagent`; this module only owns the tool-call
//! surface (schema, argument parsing, response formatting).

#[path = "tools/agent_prepare_context.rs"]
mod agent_prepare_context;
#[path = "tools/archetype_delegation.rs"]
mod archetype_delegation;
#[path = "tools/awaiting_user.rs"]
pub(crate) mod awaiting_user;
#[path = "tools/close_subagent.rs"]
mod close_subagent;
#[path = "tools/collapsed_delegation.rs"]
mod collapsed_delegation;
#[path = "tools/continue_subagent.rs"]
mod continue_subagent;
#[path = "tools/delegate_graph.rs"]
mod delegate_graph;
#[path = "tools/dispatch.rs"]
mod dispatch;
#[path = "tools/list_subagents.rs"]
mod list_subagents;
#[path = "tools/spawn_async_subagent.rs"]
mod spawn_async_subagent;
#[path = "tools/spawn_parallel_agents.rs"]
mod spawn_parallel_agents;
#[path = "tools/spawn_subagent.rs"]
mod spawn_subagent;
#[path = "tools/spawn_worker_thread.rs"]
pub mod spawn_worker_thread;
#[path = "tools/steer_subagent.rs"]
mod steer_subagent;
#[path = "tools/subagent_abort_report.rs"]
mod subagent_abort_report;
#[cfg(test)]
#[path = "tools/tools_e2e_tests.rs"]
mod tools_e2e_tests;
#[path = "tools/wait.rs"]
mod wait;
#[path = "tools/wait_subagent.rs"]
mod wait_subagent;
#[path = "tools/worker_thread.rs"]
mod worker_thread;

pub(crate) use dispatch::DelegationDispatch;

/// Recreate the minimal live TinyAgents carrier for callers that invoke a
/// concrete tool directly inside `with_parent_context`. Normal agent turns
/// always arrive through the typed dispatchers with their original carrier;
/// this compatibility path keeps controller/test callers inside an explicit
/// parent context from losing their recursive delegation authority.
pub(crate) fn ambient_parent_run_context(
    kind: &str,
) -> Option<
    tinyagents_harness::context::RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
> {
    crate::agent::harness::current_parent().map(|parent| {
        crate::agent::tinyagents::host::OpenHumanRunContext::new()
            .with_parent(parent)
            .into_tinyagents(tinyagents_harness::context::RunConfig::new(kind))
    })
}

pub(crate) use agent_prepare_context::AgentPrepareContextDispatch;
pub use agent_prepare_context::{
    run_context_scout, run_context_scout_with_catalog, AgentPrepareContextTool,
};
pub use archetype_delegation::{ArchetypeDelegationTool, DelegationTarget};
pub(crate) use close_subagent::CloseSubagentDispatch;
pub use close_subagent::CloseSubagentTool;
pub use collapsed_delegation::{CollapsedDelegationTool, DelegateTarget, DELEGATE_TO_TOOL_NAME};
pub(crate) use continue_subagent::ContinueSubagentDispatch;
pub use continue_subagent::ContinueSubagentTool;
pub(crate) use delegate_graph::DelegateGraphDispatch;
pub use delegate_graph::DelegateGraphTool;
pub(crate) use list_subagents::ListSubagentsDispatch;
pub use list_subagents::ListSubagentsTool;
pub(crate) use spawn_async_subagent::SpawnAsyncSubagentDispatch;
pub use spawn_async_subagent::{scope_spawn_async_subagent_spec, SpawnAsyncSubagentTool};
pub(crate) use spawn_parallel_agents::SpawnParallelAgentsDispatch;
pub use spawn_parallel_agents::SpawnParallelAgentsTool;
pub(crate) use spawn_subagent::SpawnSubagentDispatch;
pub use spawn_subagent::SpawnSubagentTool;
pub(crate) use spawn_worker_thread::SpawnWorkerThreadDispatch;
pub use spawn_worker_thread::SpawnWorkerThreadTool;
pub(crate) use steer_subagent::SteerSubagentDispatch;
pub use steer_subagent::SteerSubagentTool;
pub use wait::{WaitLoopTool, WaitTool};
pub(crate) use wait_subagent::WaitSubagentDispatch;
pub use wait_subagent::WaitSubagentTool;
