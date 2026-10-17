//! Host adapters over the runtime's generic tool registration
//! ([`tinyagents_runtime::turn_driver::ToolSetBuilder`]): which typed dispatch a
//! tool needs, the ambient turn-origin scope every tool execution runs in, and
//! the capability-registry projection behind the fail-closed diagnostics gate.

use std::collections::HashSet;
use std::sync::Arc;

use tinyagents_graph::goals::GoalToolKind;
use tinyagents_harness::tool::packs::USE_SKILL;
use tinyagents_harness::tool::{
    CanonicalSharedToolAdapter, ToolDispatch, ToolExecScope, ToolExecution, ToolExecutionContext,
};
use tinyagents_registry::{CapabilityRegistry, ComponentKind, RegistryDiagnostic};
use tinyagents_runtime::turn_driver::{SharedToolSet, ToolSetBuilder};
use tinyagents_runtime::{project_descriptors, ToolAdmission};
use tinyinference_llm::model::ChatModel;
use tinytools::ToolRunContext;

use crate::agent::orchestration::tools::{
    CloseSubagentDispatch, ContinueSubagentDispatch, DelegateGraphDispatch, DelegationDispatch,
    ListSubagentsDispatch, SpawnAsyncSubagentDispatch, SpawnParallelAgentsDispatch,
    SpawnSubagentDispatch, SpawnWorkerThreadDispatch, SteerSubagentDispatch, WaitSubagentDispatch,
};
use crate::agent::tinyagents::host::run_context::HostOperationContext;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::turn_policy::is_subagent_spawn_or_delegate_tool;
use crate::agent::tinyagents::use_skill_dispatch::UseSkillDispatch;
use crate::agent::tools::{DelegateToolDispatch, TodoToolDispatch};
use crate::core::runtime::CoreContext;

type RegisteredToolDispatch = Arc<dyn ToolDispatch<(), OpenHumanRunContext>>;
type ToolDispatchSelectorFn =
    dyn Fn(&str, Arc<dyn tinytools::Tool>) -> Option<RegisteredToolDispatch> + Send + Sync;
type ToolDispatchSelector = Arc<ToolDispatchSelectorFn>;

/// Typed-dispatch selection shared by the direct per-turn registration below
/// and by [`UseSkillDispatch`], which must resolve the SAME live-parent
/// dispatch for a packed archetype delegation (`create_image`, `do_crypto`,
/// `make_presentation`, …) reached through `use_skill` instead of natively
/// advertised (regression R3: `use_skill` used to hand every packed tool to
/// plain `Tool::execute_with_context`, which has no live parent, so a packed
/// delegation always failed with "delegation requires a live harness run
/// context.").
///
/// `adapter` is expected to be the same `CanonicalSharedToolAdapter` seam
/// used at registration: dispatch selection keys off `name` and the tool's
/// own schema (via [`DelegationDispatch::for_tool`]'s fallback), not object
/// identity, so a freshly built adapter over the resolved tool's registry
/// slot is equivalent to the one the harness itself would have registered.
pub(crate) fn typed_dispatch_for(
    name: &str,
    adapter: Arc<dyn tinytools::Tool>,
) -> Option<Arc<dyn ToolDispatch<(), OpenHumanRunContext>>> {
    let dispatch: Arc<dyn ToolDispatch<(), OpenHumanRunContext>> = match name {
        "spawn_parallel_agents" => Arc::new(SpawnParallelAgentsDispatch::new(adapter)),
        "spawn_async_subagent" => Arc::new(SpawnAsyncSubagentDispatch::new(adapter)),
        "spawn_worker_thread" => Arc::new(SpawnWorkerThreadDispatch::new(adapter)),
        "spawn_subagent" => Arc::new(SpawnSubagentDispatch::new(adapter)),
        "continue_subagent" => Arc::new(ContinueSubagentDispatch::new(adapter)),
        "wait_subagent" => Arc::new(WaitSubagentDispatch::new(adapter)),
        "steer_subagent" => Arc::new(SteerSubagentDispatch::new(adapter)),
        "close_subagent" => Arc::new(CloseSubagentDispatch::new(adapter)),
        "list_subagents" => Arc::new(ListSubagentsDispatch::new(adapter)),
        "delegate_graph" => Arc::new(DelegateGraphDispatch::new(adapter)),
        "delegate" | "memory" => Arc::new(DelegateToolDispatch::new(adapter)),
        "todo" => Arc::new(TodoToolDispatch::new(adapter)),
        _ => {
            return DelegationDispatch::for_tool(adapter).map(|dispatch| {
                Arc::new(dispatch) as Arc<dyn ToolDispatch<(), OpenHumanRunContext>>
            })
        }
    };
    Some(dispatch)
}

/// Names among the candidate tools that spawn or delegate to a sub-agent.
fn candidate_spawn_tools(tool_sets: &[SharedToolSet]) -> HashSet<String> {
    tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .map(|tool| tool.name().to_string())
        .filter(|name| is_subagent_spawn_or_delegate_tool(name))
        .collect()
}

/// Whether `name` is one of the model-facing per-thread goal tools
/// (`goal_get` / `goal_set` / `goal_complete`), named by their owner.
pub(crate) fn is_thread_goal_tool(name: &str) -> bool {
    GoalToolKind::MODEL_FACING
        .iter()
        .any(|kind| kind.name() == name)
}

/// Runs every tool execution inside the turn's [`AgentTurnOrigin`] scope.
///
/// The origin rides the run's typed host operation context; the process-wide
/// `CoreContext` is re-scoped with it so a tool that asks "who is acting"
/// answers for this turn and not for whichever turn last set a global.
///
/// [`AgentTurnOrigin`]: crate::agent::turn_origin::AgentTurnOrigin
pub(crate) struct TurnOriginScope;

impl ToolExecScope for TurnOriginScope {
    fn scope<'a>(
        &'a self,
        context: Option<&'a dyn ToolRunContext>,
        execution: ToolExecution<'a>,
    ) -> ToolExecution<'a> {
        let origin = context
            .and_then(ToolRunContext::host_extension)
            .and_then(|extension| extension.downcast_ref::<ToolExecutionContext>())
            .and_then(|context| context.state::<HostOperationContext>())
            .and_then(|context| context.origin.clone());
        match (
            origin,
            super::host::run_context::dispatch_ctx_for_tool(context).into_core(),
        ) {
            (Some(origin), Some(core_context)) => Box::pin(CoreContext::scope_with_turn_origin(
                core_context,
                Some(origin),
                execution,
            )),
            _ => execution,
        }
    }
}

/// The adapter for tool `name` over `sets`, scoped to the turn origin.
///
/// This is the same adapter the registration builds, so a dispatch that
/// resolves a tool later (`use_skill`) executes it exactly as a natively
/// registered one.
pub(crate) fn shared_tool_adapter(
    sets: Vec<SharedToolSet>,
    name: &str,
) -> Option<CanonicalSharedToolAdapter> {
    Some(
        CanonicalSharedToolAdapter::for_name(sets, name)?
            .with_exec_scope(Arc::new(TurnOriginScope)),
    )
}

type DispatchSelector = Arc<
    dyn Fn(&str, Arc<dyn tinytools::Tool>) -> Option<Arc<dyn ToolDispatch<(), OpenHumanRunContext>>>
        + Send
        + Sync,
>;

/// Chooses the typed dispatch for a registered tool: `use_skill` gets its own
/// (it is the proxy every packed archetype delegation is reached through and
/// must resolve the SAME live parent a natively advertised delegate tool does,
/// regression R3), everything else goes through [`typed_dispatch_for`].
fn dispatch_selector(tool_sets: Vec<SharedToolSet>) -> DispatchSelector {
    Arc::new(move |name, adapter| {
        if name != USE_SKILL {
            return typed_dispatch_for(name, adapter);
        }
        // The pack-registry handle comes off the raw registered tool, not the
        // adapter.
        let handle = tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|tool| tool.name() == name)
            .and_then(|tool| crate::tools::host_extensions::pack_registry_handle(tool.as_ref()))
            .cloned()?;
        Some(Arc::new(UseSkillDispatch::new(adapter, handle)))
    })
}

/// The admission rules for one turn.
///
/// Allowlist semantics are **fail-closed** (issue #4452): `None` admits every
/// visible tool; `Some(set)` admits *exactly* the named tools, so `Some(empty)`
/// denies all. That keeps a deliberately tool-less sub-agent from silently
/// inheriting the parent's full tool surface. `has_thread == false` drops the
/// per-thread goal tools, which cannot run without a chat thread (#6956).
pub(crate) fn turn_admission(
    tool_sets: &[SharedToolSet],
    allowed: &Option<HashSet<String>>,
    has_thread: bool,
) -> ToolAdmission {
    ToolAdmission {
        allowed: allowed.clone(),
        // A sub-agent scope on the turn forces this on in the driver.
        is_subagent_run: false,
        has_thread,
        thread_scoped: GoalToolKind::MODEL_FACING
            .iter()
            .map(|kind| kind.name().to_string())
            .collect(),
        spawn_tools: candidate_spawn_tools(tool_sets),
    }
}

/// The host's tool registration: shared sets, fail-closed admission, session
/// deferral, typed dispatches and the turn-origin execution scope.
pub(crate) fn host_tool_set_builder(
    tool_sets: &[SharedToolSet],
    allowed: &Option<HashSet<String>>,
    has_thread: bool,
    session_deferred: &HashSet<String>,
) -> ToolSetBuilder<(), OpenHumanRunContext> {
    ToolSetBuilder::new()
        .sets(tool_sets.iter().cloned())
        .admission(turn_admission(tool_sets, allowed, has_thread))
        .deferred(session_deferred.iter().cloned())
        .dispatch(dispatch_selector(tool_sets.to_vec()))
        .exec_scope(Arc::new(TurnOriginScope))
}

/// Projects the turn's models, the `admitted` tools, graph topologies and visible
/// agents into a [`CapabilityRegistry`] and returns its health diagnostics.
///
/// The turn path enforces them fail-closed before the first model dispatch, so
/// an ambiguous tool surface (a name shared across native/MCP/Composio/generated
/// tools, a dangling alias) aborts instead of silently resolving to an
/// unintended component mid-call. The live run keeps using the harness's own
/// registries; this projection is metadata only.
pub(crate) fn capability_diagnostics(
    models: &[(String, Arc<dyn ChatModel<()>>)],
    tool_sets: &[SharedToolSet],
    admitted: &[String],
) -> Vec<RegistryDiagnostic> {
    let mut registry: CapabilityRegistry<()> = CapabilityRegistry::new();
    for (name, model) in models {
        registry.replace_model(name.as_str(), model.clone());
    }
    for name in admitted {
        if let Some(adapter) = shared_tool_adapter(tool_sets.to_vec(), name) {
            registry.replace_tool(Arc::new(adapter));
        }
    }
    project_descriptors(
        &mut registry,
        ComponentKind::Graph,
        tinyagents_orchestration::topology::all_graph_topologies()
            .into_iter()
            .map(|report| report.name),
    );

    // The agents visible to this turn, as name-only `ComponentKind::Agent`
    // descriptors (issue #4249, Workstream 10.1): metadata only, no executable
    // agent is attached (sub-agent dispatch flows through the OpenHuman
    // sub-agent runner). Sourced from the runtime `AgentDefinitionRegistry`
    // (built-ins plus workspace/config overrides, already merged by id) first,
    // then the `agent_registry` built-in loader, deduped by id.
    let runtime_ids: Vec<String> =
        crate::agent::harness::host_definition::AgentDefinitionRegistry::current()
            .map(|runtime| runtime.list().iter().map(|def| def.id.clone()).collect())
            .unwrap_or_default();
    let runtime_agent_count = project_descriptors(&mut registry, ComponentKind::Agent, runtime_ids);
    let builtin_supplement_count = match crate::agent::registry::agents::load_builtins() {
        Ok(builtins) => project_descriptors(
            &mut registry,
            ComponentKind::Agent,
            builtins.into_iter().map(|def| def.id),
        ),
        Err(err) => {
            tracing::debug!(
                %err,
                "[registry] agent_registry builtin load failed; \
                 registered runtime-registry agents only"
            );
            0
        }
    };

    let diagnostics = registry.diagnostics();
    tracing::debug!(
        models = registry.to_model_registry().names().len(),
        tools = registry.names(ComponentKind::Tool).len(),
        graphs = registry.names(ComponentKind::Graph).len(),
        agents = runtime_agent_count + builtin_supplement_count,
        diagnostics = diagnostics.len(),
        "[registry] per-turn capability projection summary"
    );
    diagnostics
}

#[cfg(test)]
#[path = "tool_registration_tests.rs"]
mod tests;
