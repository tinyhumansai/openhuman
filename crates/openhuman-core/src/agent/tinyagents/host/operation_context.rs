//! The immutable authority view the run exposes to shared tools through the
//! harness's typed state-view seam, and the readers tools use on it.

use crate::agent::turn_origin::AgentTurnOrigin;
use tinyagents_definition::SandboxMode;

use std::{sync::Arc, time::Duration};

/// OpenHuman product values that tools read from the run through
/// `ToolExecutionContext::host::<OpenHumanRunExt>()`, installed by
/// [`OpenHumanRunContext::into_tinyagents`] with `RunContext::with_host`.
/// It replaces the former per-value task-locals, which do not cross a module
/// boundary.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OpenHumanRunExt {
    /// Recency window narrowing integration task-fetch tools (morning
    /// briefing only); `None` leaves them unbounded.
    pub task_recency_window: Option<Duration>,
    /// Explicit child-process environment supplied by the embedding host.
    pub command_environment: Option<Arc<std::collections::BTreeMap<String, String>>>,
}

impl std::fmt::Debug for OpenHumanRunExt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenHumanRunExt")
            .field("task_recency_window", &self.task_recency_window)
            .field("command_environment", &self.command_environment.is_some())
            .finish()
    }
}

impl OpenHumanRunExt {
    pub(super) fn from_run_context(context: &super::run_context::OpenHumanRunContext) -> Self {
        Self {
            task_recency_window: context.task_recency_window,
            command_environment: context.command_environment.clone(),
        }
    }
}

/// Reads the explicit command environment from a tool's run context.
pub(crate) fn command_environment_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> Option<Arc<std::collections::BTreeMap<String, String>>> {
    context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.host::<OpenHumanRunExt>())
        .and_then(|ext| ext.command_environment.clone())
}

/// Reads the run's task-recency window from the explicit tool carrier.
/// Direct tool dispatch (CLI, JSON-RPC, tests) has none.
pub(crate) fn task_recency_window_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> Option<Duration> {
    let window = context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.host::<OpenHumanRunExt>())
        .and_then(|ext| ext.task_recency_window);
    tracing::trace!(
        has_window = window.is_some(),
        "[composio][task-window] read run window"
    );
    window
}

/// Minimal immutable authority view exposed to shared tools through the
/// harness's typed state-view seam.
#[derive(Clone, Debug, Default)]
pub(crate) struct HostOperationContext {
    pub(crate) origin: Option<AgentTurnOrigin>,
    /// Where this run's approvals are surfaced; see
    /// [`OpenHumanRunContext::approval`](super::OpenHumanRunContext::approval).
    pub(crate) approval: crate::security::approval::ApprovalTurnContext,
    pub(crate) sandbox_mode: Option<SandboxMode>,
    /// The agent definition the run's parent execution context belongs to, for
    /// tools that act on behalf of the spawning agent when no memory identity
    /// is scoped (see `memory::tools::CallFacts::gather`).
    pub(crate) parent_agent_id: Option<String>,
    /// Who the run acts as on memory; see
    /// [`OpenHumanRunContext::memory_identity`](super::OpenHumanRunContext::memory_identity).
    pub(crate) memory_identity: Option<crate::memory::scope::MemoryIdentity>,
    /// The turn's memory, read live for the thread's turns still verbatim in
    /// the run's prompt (see [`MemoryTurn::in_prompt`](crate::memory::lifecycle::hooks::MemoryTurn::in_prompt)).
    /// `None` when the turn has no memory, as a sub-agent's run does.
    pub(crate) memory_turn: Option<std::sync::Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
    /// The context the run acts under; see
    /// [`OpenHumanRunContext::core`](super::OpenHumanRunContext::core).
    pub(crate) core: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
}

/// The dispatch context of the run a tool executes in, from the explicit tool
/// carrier. Direct tool dispatch (CLI, JSON-RPC, tests) has no run, so it
/// answers with the calling task's own context, which is what such a call has
/// always acted under.
pub(crate) fn dispatch_ctx_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> crate::core::runtime::DispatchCtx {
    let weak = context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.state::<HostOperationContext>())
        .and_then(|host| host.core.clone());
    crate::core::runtime::DispatchCtx::from_weak(weak.as_ref())
}

/// The origin of the turn a tool executes in, from the explicit tool carrier.
/// Direct tool dispatch (CLI, JSON-RPC, tests) has no run, so it answers with
/// the origin the calling task's turn scope carries, if any.
pub(crate) fn turn_origin_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> Option<AgentTurnOrigin> {
    context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.state::<HostOperationContext>())
        .and_then(|host| host.origin.clone())
        .or_else(|| crate::core::runtime::DispatchCtx::ambient_turn().turn_origin())
}

/// Reads the spawning agent's definition id from the explicit tool carrier.
/// Direct tool dispatch has no parent.
pub(crate) fn parent_agent_id_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> Option<String> {
    context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.state::<HostOperationContext>())
        .and_then(|host| host.parent_agent_id.clone())
}

/// Reads the run's memory identity from the explicit tool carrier. Direct tool
/// dispatch has none.
pub(crate) fn memory_identity_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> Option<crate::memory::scope::MemoryIdentity> {
    context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.state::<HostOperationContext>())
        .and_then(|host| host.memory_identity.clone())
}

/// Reads the thread's turns still verbatim in the run's prompt from the
/// explicit tool carrier, as they stand now: a compaction earlier in the turn
/// has already moved them. Direct tool dispatch has none.
pub(crate) fn memory_in_prompt_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> Option<tinymemory_api::ThreadWindow> {
    context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.state::<HostOperationContext>())
        .and_then(|host| host.memory_turn.as_ref().map(|turn| turn.in_prompt()))
}

/// Reads the calling agent's sandbox policy from the explicit tool carrier.
/// Direct tool dispatch has no agent-specific mode.
pub(crate) fn sandbox_mode_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> Option<SandboxMode> {
    context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.state::<HostOperationContext>())
        .and_then(|host| host.sandbox_mode)
}

/// Reads the run's approval context from the explicit tool carrier. Direct tool
/// dispatch (CLI, JSON-RPC, tests) has none, so a tool called that way sees the
/// non-interactive default.
pub(crate) fn approval_for_tool(
    context: Option<&dyn tinytools::ToolRunContext>,
) -> crate::security::approval::ApprovalTurnContext {
    context
        .and_then(tinytools::ToolRunContext::host_extension)
        .and_then(|extension| {
            extension.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>()
        })
        .and_then(|context| context.state::<HostOperationContext>())
        .map(|host| host.approval.clone())
        .unwrap_or_default()
}

/// A tool execution context for a run whose approval context is `approval`: what
/// the harness hands a tool that is dispatched inside that run. Tests drive a
/// tool through it to prove the turn's approval context reaches the tool.
#[cfg(test)]
pub(crate) fn tool_context_with_approval(
    approval: crate::security::approval::ApprovalTurnContext,
) -> tinyagents_harness::tool::ToolExecutionContext {
    let mut host = super::OpenHumanRunContext::new();
    host.approval = approval;
    let run = host.into_tinyagents(tinyagents_harness::context::RunConfig::new("approval-test"));
    tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("approval-test-call"),
    )
}
