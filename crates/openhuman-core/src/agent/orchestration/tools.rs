//! OpenHuman's single asynchronous sub-agent spawn surface.

#[path = "tools/awaiting_user.rs"]
pub(crate) mod awaiting_user;
#[path = "tools/dispatch_outcomes.rs"]
mod dispatch;
#[path = "tools/spawn_subagent_surface.rs"]
mod spawn_subagent;
#[path = "tools/structured_payload.rs"]
pub(crate) mod structured_payload;
#[path = "tools/subagent_abort_report.rs"]
mod subagent_abort_report;
#[cfg(test)]
#[path = "tools/tools_e2e_tests.rs"]
mod tools_e2e_tests;

/// A minimal live TinyAgents carrier over `parent`, for tests that drive a
/// concrete tool directly under an explicit parent execution context.
#[cfg(test)]
pub(crate) fn live_parent_run_context(
    kind: &str,
    parent: crate::agent::subagent_host::ParentExecutionContext,
) -> tinyagents_harness::context::RunContext<crate::agent::tinyagents::host::OpenHumanRunContext> {
    crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_host_env(&parent.host_env)
        .with_parent(parent)
        .into_tinyagents(tinyagents_harness::context::RunConfig::new(kind))
}

pub(crate) use spawn_subagent::SpawnSubagentDispatch;
pub use spawn_subagent::{scope_spawn_subagent_spec, SpawnSubagentTool};
