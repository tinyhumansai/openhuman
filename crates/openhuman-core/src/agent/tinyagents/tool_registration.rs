//! Host adapters over the runtime's generic tool registration
//! ([`tinyagents_runtime::turn_driver::ToolSetBuilder`]): which typed dispatch a
//! tool needs, the ambient turn-origin scope every tool execution runs in, and
//! the capability-registry projection behind the fail-closed diagnostics gate.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::tool::{
    CanonicalSharedToolAdapter, ToolDispatch, ToolExecScope, ToolExecution, ToolExecutionContext,
};
use tinyagents_harness_tools::goals::GoalToolKind;
use tinyagents_harness_tools::skills::{PackSelection, UseSkillDispatch, USE_SKILL};
use tinyagents_registry::{CapabilityRegistry, ComponentKind, RegistryDiagnostic};
use tinyagents_runtime::turn_driver::{SharedToolSet, ToolSetBuilder};
use tinyagents_runtime::{project_descriptors, ToolAdmission};
use tinyinference_llm::model::ChatModel;
use tinytools::{Tool, ToolCallOptions, ToolResult, ToolRunContext};

use crate::agent::orchestration::tools::{
    ContinueSubagentDispatch, DelegateGraphDispatch, DelegationDispatch,
    SpawnAsyncSubagentDispatch, SpawnParallelAgentsDispatch, SpawnSubagentDispatch,
};
use crate::agent::tinyagents::host::run_context::HostOperationContext;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::turn_policy::is_subagent_spawn_or_delegate_tool;
use crate::agent::tools::TodoToolDispatch;
use crate::core::runtime::CoreContext;
use crate::tools::delegate::DelegateToolDispatch;
use crate::tools::implementations::threads::SpawnWorkerThreadDispatch;

pub(crate) type RegisteredToolDispatch = Arc<dyn ToolDispatch<(), OpenHumanRunContext>>;

/// Keeps tools that need the live turn's execution context on their original
/// run instead of treating them as sub-agent work. In particular, the memory
/// tool's request budget is keyed to the parent run id.
struct LiveToolContextDispatch {
    tool: Arc<dyn Tool>,
}

impl LiveToolContextDispatch {
    fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), OpenHumanRunContext> for LiveToolContextDispatch {
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    async fn execute(
        &self,
        _state: &(),
        call_id: tinyagents_harness::CallId,
        arguments: serde_json::Value,
        options: ToolCallOptions,
        parent: &tinyagents_harness::context::RunContext<OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let context = ToolExecutionContext::from_run_context(parent, call_id);
        let child = parent.data.child();
        tokio::select! {
            _ = child.cancellation.cancelled() => Ok(ToolResult::error(format!(
                "{}: cancelled before the tool completed",
                self.tool.name(),
            ))),
            result = self.tool.execute_with_context(arguments, options, Some(&context)) => result,
        }
    }
}

/// Collision-free stable scope for controls over a host-owned parent session,
/// workspace, and optional chat thread. JSON tuple encoding distinguishes
/// absent thread from the literal `"no-thread"` and handles delimiters in any
/// value.
pub(crate) fn subagent_owner_key(
    workspace: &std::path::Path,
    parent_session: &str,
    thread_id: Option<&str>,
) -> String {
    format!(
        "openhuman-subagent-owner:{}",
        serde_json::to_string(&(
            workspace.as_os_str().as_encoded_bytes(),
            parent_session,
            thread_id,
        ))
        .expect("owner scope tuple is always JSON serializable")
    )
}

#[path = "subagent_projection.rs"]
mod subagent_projection;
use subagent_projection::{ProjectedSubagentDispatch, SubagentResultProjection};
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
/// used at registration. For synthesized archetype routes, the raw registered
/// tool's `DelegationTarget` host extension is passed separately because the
/// generic adapter intentionally does not preserve OpenHuman-specific
/// extensions.
pub(crate) fn typed_dispatch_for(
    name: &str,
    adapter: Arc<dyn tinytools::Tool>,
    delegation_target: Option<String>,
) -> Option<Arc<dyn ToolDispatch<(), OpenHumanRunContext>>> {
    let dispatch: Arc<dyn ToolDispatch<(), OpenHumanRunContext>> = match name {
        "spawn_parallel_agents" => Arc::new(SpawnParallelAgentsDispatch::new(adapter)),
        "spawn_async_subagent" => Arc::new(SpawnAsyncSubagentDispatch::new(adapter)),
        "spawn_worker_thread" => Arc::new(SpawnWorkerThreadDispatch::new(adapter)),
        "spawn_subagent" => Arc::new(SpawnSubagentDispatch::new(adapter)),
        "continue_subagent" => Arc::new(ContinueSubagentDispatch::new(adapter)),
        "delegate_graph" => Arc::new(DelegateGraphDispatch::new(adapter)),
        "delegate" => Arc::new(DelegateToolDispatch::new(adapter)),
        "memory" => Arc::new(LiveToolContextDispatch::new(adapter)),
        "todo" => Arc::new(TodoToolDispatch::new(adapter)),
        _ => {
            return DelegationDispatch::for_tool(adapter, delegation_target).map(|dispatch| {
                Arc::new(dispatch) as Arc<dyn ToolDispatch<(), OpenHumanRunContext>>
            });
        }
    };
    Some(dispatch)
}

/// Resolves the Jobs/Message adapter from the actual harness execution
/// context. Live voice harnesses build their tool set before a parent turn
/// exists, so their controls cannot capture the per-turn owner at registration.
struct LiveSubagentControlDispatch {
    name: String,
    adapter: Arc<dyn Tool>,
    tool_sets: Vec<SharedToolSet>,
}

/// Preserves the product control contract where a nonempty `task_id` wins
/// over `subagent_session_id`, while routing the selected value through the
/// generic MessageTool's durable-session lookup. The session port then binds
/// it to the authenticated full lifecycle key.
pub(crate) struct TaskFirstSessionMessageDispatch {
    inner: RegisteredToolDispatch,
    tool: Arc<dyn Tool>,
}

impl TaskFirstSessionMessageDispatch {
    pub(crate) fn new(inner: RegisteredToolDispatch) -> Self {
        Self {
            tool: inner.tool(),
            inner,
        }
    }
}

#[async_trait]
impl ToolDispatch<(), OpenHumanRunContext> for TaskFirstSessionMessageDispatch {
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    fn output_origin(&self) -> tinyagents_harness::host::ContentOrigin {
        self.inner.output_origin()
    }

    async fn execute(
        &self,
        state: &(),
        call_id: tinyagents_harness::CallId,
        mut arguments: serde_json::Value,
        options: ToolCallOptions,
        parent: &tinyagents_harness::context::RunContext<OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if let Some(object) = arguments.as_object_mut() {
            let Some(message) = object
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|message| !message.is_empty())
            else {
                return Ok(ToolResult::error("steer_subagent: `message` is required"));
            };
            object.insert(
                "message".into(),
                serde_json::Value::String(message.to_owned()),
            );
            let selected = object
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    object
                        .get("subagent_session_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                });
            if let Some(selected) = selected {
                object.insert(
                    "subagent_session_id".into(),
                    serde_json::Value::String(selected.to_owned()),
                );
                object.remove("task_id");
            }
        }
        self.inner
            .execute(state, call_id, arguments, options, parent)
            .await
    }
}

#[async_trait]
impl ToolDispatch<(), OpenHumanRunContext> for LiveSubagentControlDispatch {
    fn tool(&self) -> Arc<dyn Tool> {
        self.adapter.clone()
    }

    async fn execute(
        &self,
        state: &(),
        call_id: tinyagents_harness::CallId,
        arguments: serde_json::Value,
        options: ToolCallOptions,
        parent: &tinyagents_harness::context::RunContext<OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let dispatches = subagent_control_dispatches(&self.tool_sets, &parent.data);
        let Some(dispatch) = dispatches.get(&self.name) else {
            return Ok(ToolResult::error(format!(
                "{} requires an authenticated parent session",
                self.name
            )));
        };
        dispatch
            .execute(state, call_id, arguments, options, parent)
            .await
    }
}

fn live_dispatch_selector(tool_sets: Vec<SharedToolSet>) -> DispatchSelector {
    let fallback = dispatch_selector(tool_sets.clone());
    Arc::new(move |name, adapter| {
        if matches!(
            name,
            "list_subagents" | "wait_subagent" | "steer_subagent" | "close_subagent"
        ) {
            return Some(Arc::new(LiveSubagentControlDispatch {
                name: name.to_owned(),
                adapter,
                tool_sets: tool_sets.clone(),
            }));
        }
        fallback(name, adapter)
    })
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
/// [`AgentTurnOrigin`]: crate::security::turn_origin::AgentTurnOrigin
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
            let delegation_target = tool_sets
                .iter()
                .flat_map(|set| set.iter())
                .find(|tool| tool.name() == name)
                .and_then(|tool| crate::tools::host_extensions::delegation_target(tool.as_ref()))
                .map(str::to_string);
            return typed_dispatch_for(name, adapter, delegation_target);
        }
        let selector: Arc<
            tinyagents_harness_tools::skills::PackDispatchSelector<(), OpenHumanRunContext>,
        > = Arc::new(select_packed_tool_dispatch);
        Some(Arc::new(UseSkillDispatch::new(adapter, selector)))
    })
}

/// Build the host's usual typed dispatch for the exact pack selection pinned
/// by TinyAgents before authorization. Only tool-specific dispatch selection
/// remains host-owned; the owner binds and executes the packed target.
fn select_packed_tool_dispatch(selection: PackSelection) -> Option<RegisteredToolDispatch> {
    let name = selection.tool_name().to_owned();
    let registry = selection.registry();
    let delegation_target = registry
        .iter()
        .find(|tool| tool.name() == name)
        .and_then(|tool| crate::tools::host_extensions::delegation_target(tool.as_ref()))
        .map(str::to_string);
    let adapter = shared_tool_adapter(vec![registry], &name)?;
    typed_dispatch_for(&name, Arc::new(adapter), delegation_target)
}

fn subagent_control_dispatches(
    tool_sets: &[SharedToolSet],
    run_context: &OpenHumanRunContext,
) -> std::collections::HashMap<String, RegisteredToolDispatch> {
    use tinyagents_harness_tools::subagents::{
        SubAgentJobAction, SubAgentJobsTool, SubAgentMessageTool,
    };
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;

    let Some(parent) = run_context.parent.as_ref() else {
        return std::collections::HashMap::new();
    };
    let registry =
        crate::core::runtime::slot_for::<SubAgentJobRegistry>(&run_context.dispatch_ctx());
    let jobs = (*registry).clone();
    let store: Arc<dyn tinyagents_orchestration::subagent::SubagentSessionStore> = Arc::new(
        crate::agent::orchestration::subagent_sessions::OpenHumanTinySubagentSessionStore::new(
            parent.workspace_dir.clone(),
            parent.session_id.clone(),
            run_context.thread_id.clone(),
        )
        .with_jobs(jobs.clone()),
    );
    let owner = subagent_owner_key(
        &parent.workspace_dir,
        &parent.session_id,
        run_context.thread_id.as_deref(),
    );
    let parent_session = parent.session_id.clone();
    let owner_scope: Arc<dyn Fn(u64) -> String + Send + Sync> = Arc::new(move |_| owner.clone());

    let find_tool = |name: &str| -> Option<Arc<dyn tinytools::Tool>> {
        shared_tool_adapter(tool_sets.to_vec(), name)
            .map(|adapter| Arc::new(adapter) as Arc<dyn tinytools::Tool>)
    };
    let make_jobs_alias =
        |name: &str, action: SubAgentJobAction| -> Option<RegisteredToolDispatch> {
            let adapter = find_tool(name)?;
            let parent_session = parent_session.clone();
            let dispatch = SubAgentJobsTool::new(jobs.clone())
                .recover_selector_errors()
                .with_tool_name(name)
                .with_description(adapter.description())
                .with_parameters(adapter.parameters_schema())
                .with_fixed_action(action)
                .with_metadata(
                    adapter.permission_level(),
                    adapter.scope(),
                    adapter.category(),
                    tinytools::ToolTimeout::Inherit,
                )
                .with_owner_scope(owner_scope.clone())
                .with_sessions(store.clone(), move |_| parent_session.clone());
            Some(Arc::new(dispatch))
        };
    let mut dispatches = std::collections::HashMap::new();
    if let Some(dispatch) = make_jobs_alias("list_subagents", SubAgentJobAction::Query) {
        dispatches.insert(
            "list_subagents".into(),
            Arc::new(ProjectedSubagentDispatch {
                tool: dispatch.tool(),
                inner: dispatch,
                projection: SubagentResultProjection::List,
            }) as RegisteredToolDispatch,
        );
    }
    if let Some(adapter) = find_tool("wait_subagent") {
        let parent_session = parent_session.clone();
        let task_parent_session = parent_session.clone();
        let parent_thread_id = run_context.thread_id.clone();
        let task_store = crate::agent::orchestration::running_subagents::task_store_for_workspace(
            &parent.workspace_dir,
        );
        let wait_dispatch: RegisteredToolDispatch = Arc::new(
            SubAgentJobsTool::new(jobs.clone())
                .recover_selector_errors()
                .with_tool_name("wait_subagent")
                .with_description(adapter.description())
                .with_parameters(adapter.parameters_schema())
                .with_fixed_action(SubAgentJobAction::Await)
                .with_identifier_fields("task_id", "subagent_session_id")
                .with_await_timeout_field("timeout_secs", 120, 3600)
                .with_required_identifier()
                .with_metadata(
                    adapter.permission_level(),
                    adapter.scope(),
                    adapter.category(),
                    tinytools::ToolTimeout::Inherit,
                )
                .with_owner_scope(owner_scope.clone())
                .with_sessions(store.clone(), move |_| parent_session.clone())
                .with_task_store(task_store, move |_| {
                    tinyagents_harness::ids::RunId::new(task_parent_session.clone())
                })
                .with_task_thread_scope(move |_| parent_thread_id.clone()),
        );
        dispatches.insert(
            "wait_subagent".into(),
            Arc::new(ProjectedSubagentDispatch {
                tool: wait_dispatch.tool(),
                inner: wait_dispatch,
                projection: SubagentResultProjection::Wait,
            }),
        );
    }
    if let Some(adapter) = find_tool("close_subagent") {
        let parent_session = parent_session.clone();
        dispatches.insert(
            "close_subagent".into(),
            Arc::new(
                SubAgentJobsTool::new(jobs.clone())
                    .recover_selector_errors()
                    .with_tool_name("close_subagent")
                    .with_description(adapter.description())
                    .with_parameters(adapter.parameters_schema())
                    .with_fixed_action(SubAgentJobAction::Close)
                    .with_identifier_field("subagent_session_id")
                    .with_required_identifier()
                    .with_metadata(
                        adapter.permission_level(),
                        adapter.scope(),
                        adapter.category(),
                        adapter.timeout_policy(&serde_json::Value::Null),
                    )
                    .with_owner_scope(owner_scope.clone())
                    .with_sessions(store.clone(), move |_| parent_session.clone()),
            ),
        );
    }
    if let Some(adapter) = find_tool("steer_subagent") {
        let parent_session = parent_session.clone();
        let dispatch: RegisteredToolDispatch = Arc::new(
            SubAgentMessageTool::new(jobs)
                .recover_selector_errors()
                .with_tool_name("steer_subagent")
                .with_description(adapter.description())
                .with_parameters(adapter.parameters_schema())
                .with_argument_fields("task_id", "subagent_session_id", "message", "__request_id")
                .with_lane_field("mode")
                .with_metadata(
                    adapter.permission_level(),
                    adapter.scope(),
                    adapter.category(),
                    adapter.timeout_policy(&serde_json::Value::Null),
                )
                .with_owner_scope(owner_scope)
                .with_sessions(store, move |_| parent_session.clone()),
        );
        dispatches.insert(
            "steer_subagent".into(),
            Arc::new(TaskFirstSessionMessageDispatch::new(dispatch)),
        );
    }
    dispatches
}

fn turn_dispatch_selector(
    tool_sets: Vec<SharedToolSet>,
    run_context: &OpenHumanRunContext,
) -> DispatchSelector {
    let aliases = subagent_control_dispatches(&tool_sets, run_context);
    let fallback = dispatch_selector(tool_sets);
    Arc::new(move |name, adapter| {
        aliases
            .get(name)
            .cloned()
            .or_else(|| fallback(name, adapter))
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
        .dispatch(live_dispatch_selector(tool_sets.to_vec()))
        .exec_scope(Arc::new(TurnOriginScope))
}

pub(crate) fn host_tool_set_builder_for_turn(
    tool_sets: &[SharedToolSet],
    allowed: &Option<HashSet<String>>,
    has_thread: bool,
    session_deferred: &HashSet<String>,
    run_context: &OpenHumanRunContext,
) -> ToolSetBuilder<(), OpenHumanRunContext> {
    ToolSetBuilder::new()
        .sets(tool_sets.iter().cloned())
        .admission(turn_admission(tool_sets, allowed, has_thread))
        .deferred(session_deferred.iter().cloned())
        .dispatch(turn_dispatch_selector(tool_sets.to_vec(), run_context))
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
        crate::agent_registry::host_definition::AgentDefinitionRegistry::current()
            .map(|runtime| runtime.list().iter().map(|def| def.id.clone()).collect())
            .unwrap_or_default();
    let runtime_agent_count = project_descriptors(&mut registry, ComponentKind::Agent, runtime_ids);
    let builtin_supplement_count = match crate::agent_registry::agents::load_builtins() {
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

#[cfg(test)]
#[path = "use_skill_dispatch_tests.rs"]
mod use_skill_dispatch_tests;
