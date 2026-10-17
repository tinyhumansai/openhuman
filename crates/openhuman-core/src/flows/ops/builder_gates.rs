use super::*;
use crate::core::runtime::DispatchCtx;

/// The single canonical definition of the builder hard-gate stack: the
/// author-time gates that reject (not warn) a graph an agent must not propose
/// or persist — engine compatibility, binding-resolvability, agent-ref
/// resolvability, connection-ref, tool-contract, and required-arg
/// resolvability, in increasing cost order.
///
/// Returns an empty `Vec` when the graph passes; otherwise the first failing
/// gate's node-level error messages (short-circuiting, so an expensive later
/// gate never runs on a graph already known to be broken). Every plane that
/// gates an agent-authored graph — `build_builder_proposal` (propose / revise /
/// edit), `save_workflow`, and the `strict` create/update RPC path — routes
/// through here, so they cannot drift (audit F3: agent saves and UI saves used
/// to validate differently).
///
/// Assumes `graph` is already structurally valid (run
/// `validate_and_migrate_graph` / `validate_all` first) — these gates check
/// resolvability/contracts on a compilable graph.
///
/// Author-gate for `oh:storage_upload_file`: its literal `path` arg must be
/// workspace-relative. Uploads are confined to the agent workspace by the
/// runtime `resolve_upload_path` (a canonicalized path that escapes `action_dir`
/// is rejected), so an absolute path like `/tmp/report.html` or one climbing out
/// with `..` cannot work — it fails mid-run at the upload step. The prompt tells
/// the builder to use a relative path, but the model reliably ignores that and
/// copies an absolute path from a prior flow's example, so this enforces it in
/// code (a hard, actionable author-gate) rather than trusting the prose.
///
/// Only LITERAL paths are checked: a `=`-expression resolves from upstream data
/// at runtime and is out of scope here (the runtime check still applies). An
/// absent `path` is left to the required-arg gate.
pub(crate) fn validate_upload_paths(graph: &WorkflowGraph) -> Vec<String> {
    const UPLOAD_SLUG: &str = "oh:storage_upload_file";
    let mut errors = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        if node.config.get("slug").and_then(Value::as_str) != Some(UPLOAD_SLUG) {
            continue;
        }
        let Some(raw) = node
            .config
            .get("args")
            .and_then(|a| a.get("path"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let path = raw.trim();
        // Dynamic (resolved at runtime) or absent — not a literal we can check here.
        if path.is_empty() || path.starts_with('=') {
            continue;
        }
        let escapes_via_parent = path.split(['/', '\\']).any(|seg| seg == "..");
        if std::path::Path::new(path).is_absolute() || escapes_via_parent {
            errors.push(format!(
                "Node '{}': `oh:storage_upload_file` path `{path}` must be workspace-relative \
                 (e.g. `report.html`). Uploads are confined to the agent workspace, so an \
                 absolute path (`/tmp/...`, `/Users/...`) or one escaping with `..` is rejected \
                 at run time. Use a relative path, and have the producing node write the file to \
                 that same relative path.",
                node.id
            ));
        }
    }
    errors
}

pub(crate) async fn run_builder_gates(
    ctx: &DispatchCtx,
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    // The stack order, short-circuiting and mock-run preflight are
    // `tinyflows::gates::builder_failures`'; the host supplies the stages that
    // depend on its vocabulary (agent registry, connections, tool catalog).
    //
    // Provider connectivity (B45) is deliberately NOT a stage: it is an advisory
    // `inference_status` on the proposal payload and a run-time rejection, so a
    // graph that is only "not runnable yet" can still be shown to the user.
    let host = OpenHumanBuilderGateHost { ctx, config };
    match tinyflows::gates::builder_failures(
        graph,
        &host,
        &[crate::flows::tinyflows::caps::NATIVE_TOOL_PREFIX],
    )
    .await
    {
        Ok(failures) => failures,
        Err(error) => {
            tracing::warn!(
                target: "flows",
                %error,
                "[flows] builder gates: host stage failed — reporting it as a gate failure"
            );
            vec![format!("builder gate could not complete: {error}")]
        }
    }
}

/// Host side of the builder gate stack: the four `HostGateStage`s and the
/// saved-workflow lookup the compatibility walk needs.
struct OpenHumanBuilderGateHost<'a> {
    ctx: &'a DispatchCtx,
    config: &'a Config,
}

#[async_trait::async_trait]
impl tinyflows::caps::WorkflowResolver for OpenHumanBuilderGateHost<'_> {
    async fn resolve(&self, workflow_id: &str) -> tinyflows::error::Result<WorkflowGraph> {
        let config = self.config.clone();
        let id = workflow_id.to_string();
        let loaded = tokio::task::spawn_blocking(move || load_flow_graph(&config, &id))
            .await
            .map_err(|e| tinyflows::error::EngineError::Capability(e.to_string()))?;
        match loaded {
            Ok(Some(graph)) => Ok(graph),
            Ok(None) => Err(tinyflows::error::EngineError::Capability(format!(
                "no saved workflow '{workflow_id}'"
            ))),
            Err(e) => Err(tinyflows::error::EngineError::Capability(e.to_string())),
        }
    }
}

#[async_trait::async_trait]
impl tinyflows::gates::BuilderGateHost for OpenHumanBuilderGateHost<'_> {
    async fn check(
        &self,
        stage: tinyflows::gates::HostGateStage,
        graph: &WorkflowGraph,
    ) -> tinyflows::error::Result<Vec<String>> {
        use tinyflows::gates::HostGateStage;
        Ok(match stage {
            HostGateStage::ToolPolicy => validate_upload_paths(graph),
            HostGateStage::AgentReferences => {
                validate_agent_refs(self.ctx, self.config, graph).await
            }
            HostGateStage::Connections => {
                validate_connection_refs(self.ctx, self.config, graph).await
            }
            HostGateStage::ToolContracts => validate_tool_contracts(self.config, graph).await,
        })
    }
}

/// Strict-mode gate for the create/update RPC path (audit F3): validates
/// `graph_json` structurally (surfacing every error at once) and then runs the
/// same [`run_builder_gates`] the agent tools enforce, returning `Err` with a
/// combined, model-consumable message if anything fails.
///
/// The UI/RPC create/update path stays permissive by default (a human editing
/// on the canvas may save a work-in-progress graph); passing `strict: true`
/// opts that call into the *same* gates an agent save must pass, so the two
/// planes converge on one definition instead of diverging.
pub(crate) async fn strict_gate(
    ctx: &DispatchCtx,
    config: &Config,
    graph_json: &Value,
) -> Result<(), String> {
    let graph = tinyflows::migrate::deserialize_graph(graph_json.clone())?;
    let structural = tinyflows::validate::validate_all(&graph);
    if !structural.is_empty() {
        let messages: Vec<String> = structural.iter().map(ToString::to_string).collect();
        return Err(format!(
            "strict validation failed — the graph is structurally invalid:\n{}",
            messages.join("\n")
        ));
    }
    let gate_errors = run_builder_gates(ctx, config, &graph).await;
    if !gate_errors.is_empty() {
        return Err(format!(
            "strict validation failed:\n{}",
            gate_errors.join("\n\n")
        ));
    }
    Ok(())
}

/// Runs the full builder hard-gate stack on an already structurally-valid
/// `graph` and, if it passes, builds the `workflow_proposal` payload the
/// propose/revise/edit tools all return.
///
/// The single home for the gate sequence (`tinyflows::gates::builder_failures`) plus
/// summary/warning assembly,
/// so `revise_workflow` and `edit_workflow` cannot drift. `retry_tool` names
/// the tool in the "fix … and call `<tool>` again" guidance so each caller's
/// error text points the agent back at the right tool.
///
/// `draft_id` / `flow_id` are OPTIONAL persistence-state context echoed onto
/// the payload (the draft this proposal's edit lives on, and the saved flow it
/// derives from / targets). The payload ALWAYS carries `"persisted": false` so
/// a proposal can never be mistaken for a save confirmation — the exact false
/// belief the WS2 audit caught (an agent read a proposal as "written onto the
/// saved flow"). Actual persistence only happens via `save_workflow` /
/// `create_workflow` / `flows_draft_promote`.
///
/// Returns `Ok(payload)` on success, or `Err(message)` with a
/// model-consumable, fix-and-retry error when a gate rejects the graph. The
/// caller is responsible for structural validation (`validate_and_migrate_graph`
/// / `validate_all`) *before* calling this — these gates assume a compilable
/// graph.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_builder_proposal(
    ctx: &DispatchCtx,
    config: &Config,
    retry_tool: &str,
    name: &str,
    graph: &WorkflowGraph,
    require_approval: bool,
    revision: bool,
    instruction: Option<String>,
    draft_id: Option<String>,
    flow_id: Option<String>,
) -> Result<Value, String> {
    // The full builder hard-gate stack, run through the single canonical
    // runner so every proposal/save/strict-RPC path gates identically (F3).
    let gate_errors = run_builder_gates(ctx, config, graph).await;
    if !gate_errors.is_empty() {
        return Err(format!(
            "{}\n\nFix these and call {retry_tool} again.",
            gate_errors.join("\n\n")
        ));
    }

    let summary = tinyflows::summary::summarize(graph);
    let mut warnings = graph_trigger_warnings(graph);
    warnings.extend(graph_wiring_warnings(config, graph).await);
    // Connector onboarding (Phase 5, item 18): tell the proposal card which
    // toolkits this graph needs and whether they're connected, so it can render
    // "Connect <toolkit>" CTAs instead of a bare gate error later.
    let required_connections = compute_required_connections(ctx, config, graph).await;
    // B45 (design correction): the LLM-provider-connectivity evaluation is
    // ADVISORY here, never a rejection — `run_builder_gates` above no longer
    // includes it (that used to hard-block `propose_workflow`/`edit_workflow`
    // on a graph the copilot couldn't then show the user at all — judge
    // finding on live run 104aab90). So `evaluation.status` here can
    // legitimately be `"ready"`, `"signed_out"`, `"provider_not_configured"`,
    // or `"error"` — the UI renders a "Connect a provider" / "Sign in" CTA
    // for the non-ready cases, alongside the toolkit-connection CTAs above.
    // The graph is proposed regardless of this value. Computed via the same
    // shared, cached evaluator the run-time preflight (`validate_inference_readiness`
    // in `run_flow_body`) consumes, so a run right after this proposal reads
    // the cached result instead of re-probing the network.
    let inference_readiness = evaluate_inference_readiness(ctx, config, graph).await;
    let graph_value = serde_json::to_value(graph).map_err(|e| e.to_string())?;

    tracing::info!(
        target: "flows",
        %name,
        node_count = graph.nodes.len(),
        require_approval,
        warning_count = warnings.len(),
        revision,
        "[flows] build_builder_proposal: proposal ready for user review"
    );

    let mut payload = json!({
        "type": "workflow_proposal",
        "revision": revision,
        // A proposal is NEVER a persisted flow — it is a candidate the user
        // still has to accept/save. Stamp this unconditionally so the payload
        // can't be misread as a save confirmation (WS2 audit).
        "persisted": false,
        "name": name,
        "graph": graph_value,
        "require_approval": require_approval,
        "summary": summary,
        "warnings": warnings,
        "required_connections": required_connections,
    });
    // Only present when the graph has at least one applicable `agent` node;
    // a tool_call-only graph omits both fields entirely rather than claiming
    // a meaningless "ready".
    if let Some(evaluation) = inference_readiness {
        payload["inference_status"] = json!(evaluation.status);
        if let Some(message) = evaluation.message {
            payload["inference_message"] = json!(message);
        }
    }
    if let Some(instruction) = instruction {
        payload["instruction"] = json!(instruction);
    }
    // Echo the persistence-state handles so the agent can iterate/persist
    // against the right ids (the draft the edit lives on; the flow it targets).
    if let Some(draft_id) = draft_id {
        payload["draft_id"] = json!(draft_id);
    }
    if let Some(flow_id) = flow_id {
        payload["flow_id"] = json!(flow_id);
    }
    Ok(payload)
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent-ref resolvability gate: an `agent` node's `agent_ref` must name a
// real agent, not the runtime's `RegistryFallback` "unknown agent_ref" case
// ─────────────────────────────────────────────────────────────────────────────
//
// `run_via_registry_fallback` (`tinyflows/caps.rs`) hard-errors mid-run with
// "unknown agent_ref '…'" the moment an `agent` node's `config.agent_ref`
// doesn't resolve to either a harness `AgentDefinition` or a custom agent
// registry entry. Today that is the FIRST time an author finds out — the
// graph proposes, saves, and even passes every other builder gate, then
// fails on the very node whose whole job was to run. This gate moves that
// same check to propose/edit/save time so a broken `agent_ref` is rejected
// before it's ever persisted, using the exact resolution the runtime uses
// (`route_for_agent_ref` + `agent_registry::get_agent`) rather than
// re-implementing it.
//
// A plain `agent` node with NO `agent_ref` is unaffected (and must stay
// that way) — it runs on the default LLM completion (`caps.llm`), never
// touches `OpenHumanAgentRunner`'s routing at all, so there is nothing to
// resolve.

/// Rejects an `agent` node whose `config.agent_ref` would hit the runtime's
/// `RegistryFallback` "unknown agent_ref" hard error mid-run
/// (`run_via_registry_fallback` in `tinyflows/caps.rs`) — a real ref is one
/// that resolves via [`crate::flows::tinyflows::caps::route_for_agent_ref`]
/// to a harness [`AgentDefinition`](crate::agent::harness::host_definition::AgentDefinition)
/// (`AgentRoute::Harness`), OR — when it routes to `AgentRoute::RegistryFallback`
/// — resolves to an *enabled*
/// [`AgentRegistryEntry`](crate::agent::registry::AgentRegistryEntry)
/// via [`crate::agent::registry::get_agent`]. Both are exactly the
/// checks `OpenHumanAgentRunner::run_agent` performs at run time, reused here
/// rather than duplicated so the two planes cannot drift.
///
/// A node with no `agent_ref` (or a blank one) is a plain agent node — it
/// runs on the default LLM completion, never reaches this routing at all —
/// and is skipped, not rejected. A registry lookup failure (e.g. config
/// unavailable) fails OPEN (skipped, logged) like the sibling
/// `validate_connection_refs` gate: this gate must never false-reject a
/// graph because of a transient local read.
///
/// Takes `config` for two reasons. First (CodeRabbit/Codex review on #5114):
/// one-shot contexts — the generic `openhuman <namespace> <function>` CLI
/// dispatcher (`default_state()`, no bootstrap), cron, tests — may reach this
/// gate before the full server bootstrap has called
/// [`AgentDefinitionRegistry::init_global`]. Without it, `route_for_agent_ref`
/// sees an empty global registry and routes EVERY ref — including a real
/// workspace-TOML harness definition — to `RegistryFallback`, which then only
/// checks the custom agent registry and would reject a valid harness agent
/// as unknown. So this gate defensively (re-)initialises the harness registry
/// itself, same idempotent (`OnceLock`) idiom as
/// other lazy registry users, before resolving any ref — the two planes
/// (author-time gate and `OpenHumanAgentRunner::run_agent` at actual run
/// time) then always see the same registry state. Second, it threads through
/// to `agent_registry::get_agent`'s underlying config load.
///
/// Also lazily caches the custom agent registry snapshot on the first
/// `RegistryFallback` node (CodeRabbit nitpick): a graph with several
/// non-harness `agent_ref`s previously triggered one `config_rpc::
/// load_config_with_timeout` per node; an all-`Harness`/no-custom-ref graph
/// still never reads it at all.
pub(crate) async fn validate_agent_refs(
    ctx: &DispatchCtx,
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    use crate::agent::harness::host_definition::AgentDefinitionRegistry;
    use crate::agent::registry::AgentRegistryEntry;
    use crate::flows::tinyflows::caps::{route_for_agent_ref, AgentRoute};

    let mut errors = Vec::new();
    let mut harness_registry_init_attempted = false;
    let mut custom_registry: Option<Result<Vec<AgentRegistryEntry>, String>> = None;

    for node in &graph.nodes {
        if node.kind != NodeKind::Agent {
            continue;
        }
        let Some(agent_ref) = node.config.get("agent_ref").and_then(Value::as_str) else {
            continue;
        };
        let agent_ref = agent_ref.trim();
        if agent_ref.is_empty() {
            continue;
        }

        if !harness_registry_init_attempted && AgentDefinitionRegistry::global().is_none() {
            harness_registry_init_attempted = true;
            if let Err(e) = AgentDefinitionRegistry::init_global(&config.workspace_dir) {
                tracing::debug!(
                    target: "flows",
                    error = %e,
                    "[flows] agent-ref check: harness registry init failed — falling through \
                     to route resolution with whatever state is available"
                );
            }
        }

        match route_for_agent_ref(agent_ref) {
            AgentRoute::Harness | AgentRoute::HostAgent => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %agent_ref,
                    "[flows] agent-ref check: resolves to a harness or host agent definition"
                );
            }
            AgentRoute::RegistryFallback => {
                if custom_registry.is_none() {
                    custom_registry = Some(crate::agent::registry::list_agents(ctx, true).await);
                }
                match custom_registry.as_ref().expect("just populated") {
                    Ok(entries) => match entries.iter().find(|entry| entry.id == agent_ref) {
                        Some(entry) if entry.enabled => {
                            tracing::debug!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: resolves to an enabled custom agent \
                                 registry entry"
                            );
                        }
                        Some(_disabled) => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: agent_ref is registered but disabled — \
                                 rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is registered but currently \
                                 disabled — enable it (or pick another agent_ref via \
                                 list_agent_definitions) before this node can run.",
                                node.id
                            ));
                        }
                        None => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: unknown agent_ref — neither a harness \
                                 definition nor a custom agent registry entry — rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is not a real agent — it \
                                 names neither a built-in agent definition nor a custom agent \
                                 registry entry, and would fail at run time with an \"unknown \
                                 agent_ref\" error. Call list_agent_definitions to see the real, \
                                 selectable agent_ref values.",
                                node.id
                            ));
                        }
                    },
                    Err(e) => {
                        tracing::debug!(
                            target: "flows",
                            node = %node.id,
                            %agent_ref,
                            error = %e,
                            "[flows] agent-ref check: custom agent registry lookup unavailable — \
                             skipping (fail-open)"
                        );
                    }
                }
            }
        }
    }
    errors
}
