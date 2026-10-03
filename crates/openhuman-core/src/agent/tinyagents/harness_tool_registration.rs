//! Tool + agent capability-registry projection for
//! [`assemble_turn_harness`](super::harness_assembly::assemble_turn_harness):
//! register every admitted tool onto the harness and project the visible
//! agent set into the [`CapabilityRegistry`] for the health-diagnostics gate.

use std::collections::HashSet;
use std::sync::Arc;

use tinyagents_graph::goals::GoalToolKind;
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::tool::ToolDispatch;
use tinyagents_registry::{
    CapabilityRegistry, ComponentKind, RegistryDiagnostic, RegistrySnapshot,
};

use crate::agent::orchestration::tools::{
    CloseSubagentDispatch, ContinueSubagentDispatch, DelegateGraphDispatch, DelegationDispatch,
    ListSubagentsDispatch, SpawnAsyncSubagentDispatch, SpawnParallelAgentsDispatch,
    SpawnSubagentDispatch, SpawnWorkerThreadDispatch, SteerSubagentDispatch, WaitSubagentDispatch,
};
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::tools::{CanonicalSharedToolAdapter, EarlyExitHook};
use crate::agent::tinyagents::turn_policy::is_subagent_spawn_or_delegate_tool;
use crate::agent::tinyagents::use_skill_dispatch::UseSkillDispatch;
use crate::agent::tools::{DelegateToolDispatch, TodoToolDispatch};
use tinyagents_harness::tool::packs::USE_SKILL;

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
        "delegate" => Arc::new(DelegateToolDispatch::new(adapter)),
        "todo" => Arc::new(TodoToolDispatch::new(adapter)),
        _ => {
            return DelegationDispatch::for_tool(adapter).map(|dispatch| {
                Arc::new(dispatch) as Arc<dyn ToolDispatch<(), OpenHumanRunContext>>
            })
        }
    };
    Some(dispatch)
}

/// Whether `name` is one of the model-facing per-thread goal tools
/// (`goal_get` / `goal_set` / `goal_complete`), named by their owner.
pub(crate) fn is_thread_goal_tool(name: &str) -> bool {
    GoalToolKind::MODEL_FACING
        .iter()
        .any(|kind| kind.name() == name)
}

/// Register every admitted tool from `tool_sets` onto `harness` (and its
/// `capability_registry` projection), project the visible agent set as
/// name-only descriptors, and return `(tool_count, registry_diagnostics,
/// registry_snapshot)`.
///
/// Allowlist semantics are **fail-closed** (issue #4452): `allowed == None` →
/// no filter, every visible tool registers; `allowed == Some(set)` → register
/// *exactly* the named tools, so `Some(empty)` denies all. This is what keeps
/// a deliberately tool-less sub-agent (`ToolScope::Named([])`, a zero-match
/// `skill_filter`, or a `named` list that resolves to nothing) from silently
/// inheriting the parent's full tool surface (shell/file-write/spawn) — the
/// old `allowed.is_empty() || allowed.contains(name)` predicate was
/// fail-open.
///
/// `has_thread == false` drops the per-thread goal tools, which cannot run
/// without a chat thread (issue #6956).
#[allow(clippy::too_many_arguments)]
pub(super) fn register_turn_tools_and_agents(
    harness: &mut AgentHarness<(), OpenHumanRunContext>,
    capability_registry: &mut CapabilityRegistry<()>,
    tool_sets: &[Arc<Vec<Box<dyn tinytools::Tool>>>],
    allowed: &Option<HashSet<String>>,
    early_exit_set: &HashSet<&str>,
    early_exit_hook: Option<&EarlyExitHook>,
    is_subagent_run: bool,
    has_thread: bool,
    session_deferred: &HashSet<String>,
) -> (
    usize,
    Vec<String>,
    Vec<RegistryDiagnostic>,
    RegistrySnapshot,
) {
    if let Some(set) = allowed {
        if set.is_empty() {
            tracing::warn!(
                subagent = is_subagent_run,
                "[subagent] tool allowlist resolved empty — registering no tools"
            );
        }
    }
    let mut seen_candidates: HashSet<String> = HashSet::new();
    let candidate_names: Vec<String> = tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .map(|tool| tool.name())
        .filter(|&name| seen_candidates.insert(name.to_string()))
        // The per-thread goal tools resolve their target from the run's thread
        // and refuse every call without one ("thread goal tools require an
        // active chat thread"), so a thread-less turn is not offered them
        // (issue #6956). Dropped as candidates, not just at registration, so
        // the shadow exposure layer's reference matches what registers.
        .filter(|&name| {
            let keep = has_thread || !is_thread_goal_tool(name);
            if !keep {
                tracing::debug!(
                    tool = name,
                    "[goals] not registering thread goal tool on a turn without a chat thread"
                );
            }
            keep
        })
        .map(|name| name.to_string())
        .collect();
    let mut registered: HashSet<String> = HashSet::new();
    for name in candidate_names.iter().map(String::as_str) {
        // Fail-closed allowlist: `None` admits everything, `Some(set)` admits only
        // its members (empty set → nothing).
        let admitted = match allowed {
            None => true,
            Some(set) => set.contains(name),
        };
        // Defense-in-depth (issue #4452): a sub-agent must NEVER be handed a
        // spawn/delegate tool whatever the allowlist says — re-assert it here at
        // registration, not just on the caller's `allowed_indices`. Warn only when
        // the allowlist actually readmitted one (issue #6157); the caller strips
        // them first, so a bare `spawn_stripped` warn fired on every healthy run.
        let spawn_stripped = is_subagent_run && is_subagent_spawn_or_delegate_tool(name);
        if spawn_stripped && admitted {
            tracing::warn!(
                tool = name,
                "[subagent] refusing to register spawn/delegate tool on sub-agent run"
            );
        }
        if !registered.contains(name) && admitted && !spawn_stripped {
            if let Some(mut adapter) =
                CanonicalSharedToolAdapter::for_name(tool_sets.to_vec(), name)
            {
                if early_exit_set.contains(name) {
                    if let Some(hook) = early_exit_hook {
                        adapter = adapter.with_early_exit(hook.clone());
                    }
                }
                if session_deferred.contains(name) {
                    adapter = adapter.deferred();
                }
                registered.insert(name.to_string());
                let adapter = Arc::new(adapter);
                capability_registry.replace_tool(adapter.clone());
                if name == USE_SKILL {
                    // `use_skill` needs its own typed dispatch (regression
                    // R3): it is the proxy every packed archetype delegation
                    // (`create_image`, `do_crypto`, `make_presentation`, …)
                    // is reached through, and it must resolve the SAME live
                    // parent `typed_dispatch_for` gives a natively advertised
                    // delegate tool. The pack-registry handle comes off the
                    // raw registered tool (not this adapter, which has no
                    // erased host extension of its own).
                    let handle = tool_sets
                        .iter()
                        .flat_map(|set| set.iter())
                        .find(|tool| tool.name() == name)
                        .and_then(|tool| {
                            crate::tools::host_extensions::pack_registry_handle(tool.as_ref())
                        })
                        .cloned();
                    match handle {
                        Some(handle) => harness.register_tool_dispatch(Arc::new(
                            UseSkillDispatch::new(adapter, handle),
                        )),
                        None => harness.register_tool(adapter),
                    };
                } else if let Some(dispatch) = typed_dispatch_for(name, adapter.clone()) {
                    harness.register_tool_dispatch(dispatch);
                } else {
                    harness.register_tool(adapter);
                }
            }
        }
    }
    let tool_count = registered.len();
    for report in crate::agent::tinyagents::topology::all_graph_topologies() {
        let _ = capability_registry.register_descriptor(ComponentKind::Graph, report.name);
    }

    // Project the agents visible to this turn into the registry as name-only
    // `ComponentKind::Agent` descriptors (issue #4249, Workstream 10.1). This is
    // metadata only: no executable `HarnessAgent` is attached (sub-agent
    // dispatch still flows through the openhuman sub-agent runner), so the
    // registration is cheap and leaves the turn hot path unchanged. Agents are
    // sourced from BOTH the runtime `AgentDefinitionRegistry` global (built-ins
    // plus any workspace/config custom overrides, already merged by id) AND the
    // `agent_registry` built-in loader, deduped by id — the runtime registry is
    // registered first and wins, since it carries the richer, override-aware
    // `when_to_use`. Registering here keeps the ids in
    // `capability_registry.snapshot()` so the 10.3 `agent.registry_snapshot` RPC
    // and the fail-closed diagnostics (10.2) observe them.
    //
    // DEFERRED (rich metadata): tinyagents 1.3.0 exposes no public API to attach
    // a `ComponentMetadata` description/tags to a name-only descriptor — only
    // `register_agent(Arc<dyn HarnessAgent>)` carries a full executable blueprint
    // we do not have at this layer. Until the crate grows a
    // `register_descriptor_with_meta` (or we thread real `HarnessAgent`s through
    // `assemble_turn_harness`), the `when_to_use` descriptions and
    // `display_name`/`source` tags cannot be persisted onto the snapshot entry;
    // only the ids are projected. The runtime-registry → executable-agent
    // projection (so `register_agent`/`.rag` sub-agent resolution can bind these)
    // remains the deferred follow-up.
    let mut registered_agents: HashSet<String> = HashSet::new();
    let mut runtime_agent_count = 0usize;
    if let Some(runtime) = crate::agent::harness::definition::AgentDefinitionRegistry::global() {
        for def in runtime.list() {
            if registered_agents.insert(def.id.clone()) {
                let _ =
                    capability_registry.register_descriptor(ComponentKind::Agent, def.id.clone());
                runtime_agent_count += 1;
            }
        }
    }
    // agent_registry built-ins as a supplement/fallback (deduped by id). When the
    // runtime global is uninitialised this is the sole source; otherwise it only
    // contributes ids the runtime registry did not already cover.
    let mut builtin_supplement_count = 0usize;
    match crate::agent::registry::agents::load_builtins() {
        Ok(builtins) => {
            for def in builtins {
                if registered_agents.insert(def.id.clone()) {
                    let _ = capability_registry
                        .register_descriptor(ComponentKind::Agent, def.id.clone());
                    builtin_supplement_count += 1;
                }
            }
        }
        Err(err) => {
            tracing::debug!(
                %err,
                "[registry] agent_registry builtin load failed; \
                 registered runtime-registry agents only"
            );
        }
    }

    let registry_diagnostics = capability_registry.diagnostics();
    let registry_snapshot = capability_registry.snapshot();

    // Validation/projection pass (issue #4249, Workstream 10.1): exercise the
    // model/tool projection helpers that are slated to eventually replace the
    // live `harness.register_model`/`register_tool` glue. Today they are
    // infallible projections — they cannot themselves surface diagnostics — so
    // invoking them here is a non-fatal cross-check that every registered model
    // and tool projects into a harness registry cleanly. The authoritative,
    // fail-closed health signal stays `capability_registry.diagnostics()`
    // (captured in `registry_diagnostics` above and enforced by 10.2); a benign
    // projection difference must never abort a turn, so nothing here is folded
    // into that stream. The harness is deliberately NOT switched over to these
    // projections yet — that glue swap is explicitly deferred.
    let projected_models = capability_registry.to_model_registry();
    let projected_tools = capability_registry.to_tool_registry::<OpenHumanRunContext>();
    tracing::debug!(
        models = projected_models.names().len(),
        tools = projected_tools.names().len(),
        graphs = capability_registry.names(ComponentKind::Graph).len(),
        agents = registered_agents.len(),
        runtime_agents = runtime_agent_count,
        builtin_supplement_agents = builtin_supplement_count,
        diagnostics = registry_diagnostics.len(),
        "[registry] per-turn capability projection summary"
    );

    (
        tool_count,
        candidate_names,
        registry_diagnostics,
        registry_snapshot,
    )
}

#[cfg(test)]
#[path = "harness_tool_registration_tests.rs"]
mod tests;
