
#[async_trait]
impl Tool for DryRunWorkflowTool {
    fn name(&self) -> &str {
        "dry_run_workflow"
    }

    fn description(&self) -> &str {
        "Dry-run a workflow graph in a SANDBOX to self-verify it before \
         proposing. Compiles the graph and executes it against MOCK capabilities \
         — every LLM / tool_call / http_request / code node returns a deterministic \
         echo, so NOTHING real happens (no messages sent, no code run). Returns the \
         simulated per-node output labeled as sandbox output. Use it to catch \
         wiring/routing mistakes; it does NOT prove real integrations work. Provide \
         the graph as exactly one of `draft_id` (a working draft), `flow_id` (a saved \
         flow), or inline `graph` (draft_id wins, then flow_id), plus an optional \
         `input`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "draft_id": {
                    "type": "string",
                    "description": "A working draft to simulate. Provide one of draft_id / flow_id / graph (draft_id wins)."
                },
                "flow_id": {
                    "type": "string",
                    "description": "A saved flow to simulate. Provide one of draft_id / flow_id / graph."
                },
                "graph": {
                    "type": "object",
                    "description": "An inline tinyflows WorkflowGraph to simulate: { nodes: [...], edges: [...] }. Provide one of draft_id / flow_id / graph.",
                    "properties": {
                        "nodes": { "type": "array" },
                        "edges": { "type": "array" }
                    },
                    "required": ["nodes", "edges"]
                },
                "input": {
                    "description": "Optional trigger input passed to the run (defaults to {})."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Mock-only and side-effect-free: nothing external ever fires (all
        // capabilities are echo stubs). So it needs no elevated permission and
        // is available on EVERY tier, read-only included (audit F7) — a
        // read-only agent must be able to self-verify its own proposal.
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        // Mock capabilities only — no real outbound effect.
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Graph source: exactly one of a working draft, a saved flow, or an
        // inline graph — same precedence (draft_id > flow_id > graph) as the
        // sibling validate/edit tools, so they all accept the same handles.
        let draft_id = args
            .get("draft_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let flow_id = args
            .get("flow_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let inline_graph = args.get("graph").filter(|v| !v.is_null());

        let graph_json = match (draft_id, flow_id, inline_graph) {
            (Some(id), _, _) => match ops::flows_draft_get(&self.config, id) {
                Ok(outcome) => outcome.value.graph,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Could not load draft '{id}' to dry-run: {e}"
                    )));
                }
            },
            (None, Some(id), _) => match ops::load_flow_graph(&self.config, id) {
                Ok(Some(graph)) => serde_json::to_value(&graph)?,
                Ok(None) => {
                    return Ok(ToolResult::error(format!("flow '{id}' not found")));
                }
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Could not load flow '{id}' to dry-run: {e}"
                    )));
                }
            },
            (None, None, Some(v)) => v.clone(),
            (None, None, None) => {
                return Ok(ToolResult::error(
                    "Provide one of `draft_id` (a working draft), `flow_id` (a saved flow), or \
                     `graph` (an inline graph) to dry-run."
                        .to_string(),
                ));
            }
        };
        let input = args.get("input").cloned().unwrap_or_else(|| json!({}));

        let graph: WorkflowGraph = match validate_and_migrate_graph(graph_json) {
            Ok(graph) => graph,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Cannot dry-run an invalid graph: {e}. Fix the graph first."
                )))
            }
        };

        tracing::debug!(
            target: "flows",
            node_count = graph.nodes.len(),
            "[flows] dry_run_workflow: compiling + running draft against MOCK capabilities"
        );

        let compiled = match tinyflows::compiler::compile(&graph) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Draft graph failed to compile: {e}"
                )))
            }
        };

        // Wire the schema-aware mock `AgentRunner` so a draft with `agent`
        // nodes exercises the agent-node path during the dry run instead of
        // erroring on a missing capability — the plain `mock_capabilities()`
        // leaves `agent: None`. No real agent turn fires; the mock runner is a
        // deterministic echo, same contract as the other sandbox mocks, except
        // it additionally honors `config.output_parser.schema` (see its doc)
        // so the null-resolution check below doesn't false-positive on an
        // agent node that correctly declared a schema.
        let mut caps = tinyflows::caps::mock::mock_capabilities_with_agent(
            crate::openhuman::flows::tinyflows::caps::SchemaAwareMockAgentRunner,
        );
        // Plain agent nodes (no `agent_ref`) never reach the runner above —
        // the vendored `agent` node routes them to the `llm` slot instead (see
        // `SchemaAwareMockLlm`'s doc). Swap the vendored `MockLlm` echo for the
        // schema-aware mock so their `output_parser.schema` is honored too,
        // instead of the echo shape failing the sub-port's validation.
        caps.llm =
            std::sync::Arc::new(crate::openhuman::flows::tinyflows::caps::SchemaAwareMockLlm);
        // Wiring preflight over the echo mocks (see the struct doc): required
        // Composio args must be present and non-null even in the sandbox.
        caps.tools = std::sync::Arc::new(
            crate::openhuman::flows::tinyflows::caps::PreflightToolInvoker {
                config: self.config.clone(),
                inner: caps.tools.clone(),
            },
        );

        // Which node ids are `tool_call` nodes — the null-resolution check
        // below is scoped to just these (see the struct doc: a null in an
        // `agent`'s prompt is not execution-breaking the way a null tool arg
        // is, so only `tool_call` diagnostics fail the dry run).
        let tool_call_node_ids: std::collections::HashSet<&str> = graph
            .nodes
            .iter()
            .filter(|node| node.kind == tinyflows::model::NodeKind::ToolCall)
            .map(|node| node.id.as_str())
            .collect();

        // Which node ids are `agent` nodes — scoped narrowly to the ONE
        // execution-breaking agent diagnostic: a null-resolved `prompt`
        // itself (see the struct doc's "agent prompt nulls" section). Every
        // OTHER agent-config subfield (e.g. a null inside `tools` args) stays
        // non-fatal here, same as before.
        let agent_node_ids: std::collections::HashSet<&str> = graph
            .nodes
            .iter()
            .filter(|node| node.kind == tinyflows::model::NodeKind::Agent)
            .map(|node| node.id.as_str())
            .collect();

        // Capture every node's execution diagnostics (null-resolved
        // `=`-expressions the engine itself traced — see
        // `tinyflows::expr::resolve_traced`) as the sandbox run executes, so
        // they can be inspected once the run settles.
        let observer = Arc::new(CapturingObserver::default());
        let observer_dyn: Arc<dyn tinyflows::observability::RunObserver> = observer.clone();
        let run = tinyflows::engine::run_with_observer(&compiled, input, &caps, &observer_dyn);
        let outcome = match tokio::time::timeout(
            std::time::Duration::from_secs(DRY_RUN_TIMEOUT_SECS),
            run,
        )
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(e)) => {
                // A `stop`-policy `tool_call` whose required arg resolved null
                // aborts the WHOLE run here (via `PreflightToolInvoker`), so
                // the honest per-field diagnostic never reaches the settled-run
                // `null_resolutions` path below. Recover it from the observer:
                // if the abort was caused by a required arg bound to an upstream
                // Composio `tool_call`'s output, the echo mock simply CAN'T
                // produce that field — so surface it as `unverifiable` rather
                // than letting the generic "required arg missing/null" text
                // (which sent the transcript agent re-wiring a correct binding
                // three times) stand alone. WS6.
                let unverifiable_bindings: Vec<Value> =
                    tool_call_arg_null_entries(&observer.steps(), &graph, &tool_call_node_ids)
                        .into_iter()
                        .filter(|entry| {
                            entry.get("unverifiable").and_then(Value::as_bool) == Some(true)
                        })
                        .collect();
                if !unverifiable_bindings.is_empty() {
                    tracing::debug!(
                        target: "flows",
                        error = %e,
                        unverifiable_count = unverifiable_bindings.len(),
                        "[flows] dry_run_workflow: sandbox run aborted on a Composio-upstream \
                         binding the echo mock cannot verify — surfacing it honestly"
                    );
                    return Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                        "sandbox": true,
                        "ok": false,
                        "error": e.to_string(),
                        "unverifiable_bindings": unverifiable_bindings,
                        "note": "SANDBOX (mock) output — a tool_call node aborted because a \
                            required arg binds to the output of an upstream Composio tool_call, \
                            which the sandbox can only ECHO (it never produces real tool output \
                            fields). See unverifiable_bindings: each MAY already be wired \
                            correctly — confirm the path with get_tool_contract {{ slug }} \
                            (output_fields / primary_array_path; Composio results nest under \
                            .item.json.data.) or get_tool_output_sample {{ slug, args }} instead \
                            of re-wiring blindly. No real side effects occurred.",
                    }))?));
                }
                tracing::debug!(target: "flows", error = %e, "[flows] dry_run_workflow: sandbox run errored");
                return Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "sandbox": true,
                    "ok": false,
                    "error": e.to_string(),
                    "note": "SANDBOX (mock) output — a node errored during simulation. No real side effects occurred.",
                }))?));
            }
            Err(_elapsed) => {
                return Ok(ToolResult::error(format!(
                    "Sandbox dry-run timed out after {DRY_RUN_TIMEOUT_SECS}s"
                )))
            }
        };

        // Collect every null-resolved `=`-expression that landed on a
        // `tool_call` node's `args.*` config path — the class of binding
        // mistake that "builds" (compiles, dry-runs against echo mocks) but
        // does nothing at runtime because the wired field never had a value.
        // Each entry is honest about WHY it resolved null: a binding to an
        // upstream Composio `tool_call`'s output is flagged `unverifiable`
        // (the echo mock can't produce real tool output fields) rather than
        // reported as a plain wiring mistake — see [`build_null_resolution_entry`].
        let null_resolutions: Vec<Value> =
            tool_call_arg_null_entries(&observer.steps(), &graph, &tool_call_node_ids);

        // Collect every null-resolved `agent`-node `prompt` — execution-
        // breaking in the same way a null `tool_call` arg is: `prompt` is the
        // node's ONLY input channel to the completion, so a `null` there
        // means the agent runs with an EMPTY prompt (the exact root-cause bug
        // `input_context` — and the static gate in
        // `ops::validate_binding_resolvability` — exist to prevent). Scoped
        // to the `location == "prompt"` diagnostic specifically: other
        // agent-config subfields (e.g. a null buried in `tools` args) stay
        // non-fatal here, same as before this check existed.
        let agent_prompt_nulls: Vec<Value> = observer
            .steps()
            .iter()
            .filter(|step| agent_node_ids.contains(step.node_id.as_str()))
            .flat_map(|step| {
                step.diagnostics
                    .iter()
                    .filter(|&diag| diag.location == "prompt")
                    .map(|diag| {
                        json!({
                            "node_id": step.node_id,
                            "location": diag.location,
                            "expression": diag.expression,
                            "suggestion": "Feed upstream data via input_context:\"=item\" and \
                                make the prompt a plain instruction.",
                        })
                    })
            })
            .collect();

        // Collect every null-resolved `agent`-node `input_context` — mirrors
        // `agent_prompt_nulls` exactly (see the struct doc's "Agent-
        // `input_context` null check" section): `input_context` has been the
        // agent's primary upstream-data channel since #4590, so a null
        // resolution here is just as execution-breaking as a null `prompt` —
        // the agent runs with no upstream data at all.
        let agent_input_context_nulls: Vec<Value> = observer
            .steps()
            .iter()
            .filter(|step| agent_node_ids.contains(step.node_id.as_str()))
            .flat_map(|step| {
                step.diagnostics
                    .iter()
                    .filter(|&diag| diag.location == "input_context")
                    .map(|diag| {
                        json!({
                            "node_id": step.node_id,
                            "location": diag.location,
                            "expression": diag.expression,
                            "suggestion": "Wire input_context from a real upstream field, e.g. \
                                \"=nodes.<node_id>.item.json.<field>\" (or \"=item\" off the \
                                trigger), not an expression that resolves to null.",
                        })
                    })
            })
            .collect();

        // Collect every `tool_call` node whose EXECUTOR errored (e.g. the
        // Composio required-arg preflight rejecting a missing/null arg) —
        // regardless of that node's `on_error` policy. A `"continue"`/`"route"`
        // policy converts the failure into a routed error ITEM and the run
        // still completes successfully (`Ok(outcome)`), so the naive
        // `null_resolutions` check above misses it entirely: the failing
        // node's `ExecutionStep` carries an EMPTY `diagnostics` (the engine
        // never got far enough to trace an `=`-expression — see
        // `tinyflows::engine`'s error-item path) even though the node
        // genuinely failed. Only `"stop"` (the default) fails the whole run —
        // and that's already caught above via `Ok(Err(e))` before this point,
        // so every `StepStatus::Error` step reachable here is exactly the
        // continue/route case. The error text itself isn't on the step (the
        // engine only attaches it to the routed error item), so it's read
        // back out of `outcome.output`.
        let node_errors: Vec<Value> = observer
            .steps()
            .iter()
            .filter(|step| {
                tool_call_node_ids.contains(step.node_id.as_str())
                    && matches!(step.status, tinyflows::observability::StepStatus::Error)
            })
            .map(|step| {
                let error =
                    tool_call_error_message(&outcome.output, &step.node_id).unwrap_or_else(|| {
                        format!(
                            "tool_call node '{}' failed during the sandbox run — its `on_error` \
                             policy turned the failure into routed/continued data instead of \
                             failing the whole dry run, but the underlying error still means the \
                             node is broken.",
                            step.node_id
                        )
                    });
                json!({ "node_id": step.node_id, "error": error })
            })
            .collect();

        // Routing-divergence blind spot (B15): an `agent`/`tool_call` node that
        // did NOT execute during the sandbox run at all — because an upstream
        // `condition` routed the mock trigger payload onto its OTHER branch —
        // is invisible to every check above (`null_resolutions` etc. only
        // inspect steps that ran). But the mock input's *shape* need not match
        // a real trigger's shape (a webhook's real JSON vs. the dry run's `{}`
        // default, say), so a condition that took the `false` branch under mock
        // data may well take `true` at runtime with real data — or vice versa.
        // Either way, the dry run silently never exercised the very node whose
        // wiring most needed checking. This is a WARNING, not a hard reject
        // (an unexercised branch can be entirely intentional), surfaced
        // alongside the other diagnostics so the caller can double-check the
        // wiring by hand.
        let executed_steps = observer.steps();
        let executed_node_ids: std::collections::HashSet<&str> = executed_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect();
        let routing_divergence_warnings: Vec<Value> = graph
            .nodes
            .iter()
            .filter(|node| {
                node.kind != tinyflows::model::NodeKind::Trigger
                    && (agent_node_ids.contains(node.id.as_str())
                        || tool_call_node_ids.contains(node.id.as_str()))
                    && !executed_node_ids.contains(node.id.as_str())
            })
            .map(|node| {
                let condition_node_id = find_upstream_condition(&graph, &node.id);
                let message = match &condition_node_id {
                    Some(cid) => format!(
                        "Node '{}' did not execute in the dry run (condition '{}' routed to \
                         the other branch under mock data); verify the wiring — at runtime \
                         with real data it may route differently.",
                        node.id, cid
                    ),
                    None => format!(
                        "Node '{}' did not execute in the dry run (an upstream branch routed \
                         the mock data away from it); verify the wiring — at runtime with real \
                         data it may route differently.",
                        node.id
                    ),
                };
                json!({
                    "node_id": node.id,
                    "condition_node_id": condition_node_id,
                    "message": message,
                })
            })
            .collect();

        // Quiet, informational only (never a prompt, never a gate): the
        // ApprovalGate permissions a real run of this graph will need, so the
        // builder agent can tell the user what the save+enable card will ask
        // for — the card itself fires at save+enable, NOT during dry runs.
        let permissions_manifest =
            crate::openhuman::flows::ops::compute_approval_manifest(&self.config, &graph).await;

        tracing::info!(
            target: "flows",
            node_count = graph.nodes.len(),
            pending_approvals = outcome.pending_approvals.len(),
            null_resolution_count = null_resolutions.len(),
            agent_prompt_null_count = agent_prompt_nulls.len(),
            agent_input_context_null_count = agent_input_context_nulls.len(),
            node_error_count = node_errors.len(),
            routing_divergence_warning_count = routing_divergence_warnings.len(),
            permissions_manifest_count = permissions_manifest.len(),
            "[flows] dry_run_workflow: sandbox run finished"
        );

        if !null_resolutions.is_empty()
            || !agent_prompt_nulls.is_empty()
            || !agent_input_context_nulls.is_empty()
            || !node_errors.is_empty()
        {
            tracing::debug!(
                target: "flows",
                ?null_resolutions,
                ?agent_prompt_nulls,
                ?agent_input_context_nulls,
                ?node_errors,
                "[flows] dry_run_workflow: tool_call/agent-prompt/agent-input_context issue(s) \
                 found — failing the dry run"
            );
            return Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                "sandbox": true,
                "ok": false,
                "null_resolutions": null_resolutions,
                "agent_prompt_nulls": agent_prompt_nulls,
                "agent_input_context_nulls": agent_input_context_nulls,
                "node_errors": node_errors,
                "routing_divergence_warnings": routing_divergence_warnings,
                "permissions_manifest": permissions_manifest,
                "message": "These tool_call args resolved to null, an agent node's prompt or \
                    input_context resolved to null (an EMPTY prompt — see agent_prompt_nulls — \
                    or no upstream data at all — see agent_input_context_nulls), or a tool_call \
                    node failed during the sandbox run (even one recovered via on_error: \
                    continue/route) — wire null-resolved args from an upstream node's real \
                    output (give any agent node an output_parser.schema so its fields are \
                    addressable), feed upstream data into a null-resolved agent prompt/ \
                    input_context from a real upstream field instead of a jq expression inside \
                    the prompt text, and fix or rewire whatever tool_call node_errors names. Also \
                    check routing_divergence_warnings: any agent/tool_call node listed there \
                    never ran in this sandbox at all because an upstream condition routed the \
                    mock data past it — verify that wiring by hand too.",
            }))?));
        }

        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "sandbox": true,
            "ok": true,
            "output": outcome.output,
            "pending_approvals": outcome.pending_approvals,
            "null_resolutions": null_resolutions,
            "agent_prompt_nulls": agent_prompt_nulls,
            "agent_input_context_nulls": agent_input_context_nulls,
            "node_errors": node_errors,
            "routing_divergence_warnings": routing_divergence_warnings,
            "permissions_manifest": permissions_manifest,
            "note": "SANDBOX (mock) output — LLM/tool/HTTP/code nodes returned deterministic echoes; NO real side effects occurred. This checks wiring/routing only, not whether real integrations work. \
                If routing_divergence_warnings is non-empty, an agent/tool_call node never ran in \
                this sandbox because an upstream condition routed the mock data past it — that \
                node's wiring is unverified; check it by hand.",
        }))?))
    }
}

/// Walks a graph backward from `node_id`'s predecessors (any number of hops)
/// to find the nearest ancestor that is a `condition` node — used to name the
/// branch responsible for a routing-divergence warning (see
/// [`DryRunWorkflowTool::execute`]'s routing-divergence check, just above).
/// Returns `None` if no predecessor chain reaches a `condition` node (e.g. the
/// node simply has no predecessors, or none of them is a condition) — the
/// warning is still emitted, just without a named culprit node.
fn find_upstream_condition(graph: &WorkflowGraph, node_id: &str) -> Option<String> {
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<&str> = graph
        .edges
        .iter()
        .filter(|edge| edge.to_node == node_id)
        .map(|edge| edge.from_node.as_str())
        .collect();
    while let Some(current) = queue.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        if let Some(node) = graph.nodes.iter().find(|n| n.id == current) {
            if node.kind == tinyflows::model::NodeKind::Condition {
                return Some(node.id.clone());
            }
        }
        for edge in graph.edges.iter().filter(|edge| edge.to_node == current) {
            queue.push_back(edge.from_node.as_str());
        }
    }
    None
}

/// Best-effort extraction of the human-readable error message the engine
/// recorded for a `tool_call` node whose `on_error` policy is `"continue"` or
/// `"route"`. Such a node's failure is converted into an error ITEM on its
/// output (`{ "error": { "message", "node" } }` — see `tinyflows::engine`'s
/// `error_item`) rather than failing the whole run, so the message lives in
/// the run's `output` state, not on the [`tinyflows::observability::ExecutionStep`]
/// itself (whose `diagnostics` stays empty for an error step — see
/// [`DryRunWorkflowTool::execute`]'s `node_errors` collection).
fn tool_call_error_message(output: &Value, node_id: &str) -> Option<String> {
    output
        .get("nodes")?
        .get(node_id)?
        .get("items")?
        .as_array()?
        .iter()
        .find_map(|item| {
            item.get("json")?
                .get("error")?
                .get("message")?
                .as_str()
                .map(str::to_string)
        })
}

/// The engine's own step-capturing observer, re-exported under the name
/// [`DryRunWorkflowTool`]'s call sites already use.
///
/// It is upstream because what it captures is the engine's:
/// `ExecutionStep::diagnostics` holds the `=`-expressions that resolved to null
/// while a node's config was being assembled, which is the only place a graph's
/// real wiring failure is visible. This host used to declare an identical copy.
pub(crate) use tinyflows::observability::CapturingObserver;

// ─────────────────────────────────────────────────────────────────────────────
// save_workflow — persist a built graph onto an EXISTING saved flow
// ─────────────────────────────────────────────────────────────────────────────

/// `save_workflow`: persist a validated graph (and optionally a new name) onto
/// an **existing, already-saved** flow via [`ops::flows_update`] — the same
/// validate-and-migrate path the UI's Save uses.
///
/// It was originally added as a narrow, deliberate exception to the belt's
/// "propose, never persist" invariant (for the Flows prompt bar's
/// instant-create path, where the host creates the flow *before* delegating
/// and hands the agent its `flow_id`) — before [`CreateWorkflowTool`] and
/// [`DuplicateFlowTool`] existed, this was the belt's only write. Both now
/// exist, so `save_workflow` is one of three persistence tools, not the sole
/// one. Its own remaining boundaries:
///
/// - **Update-only.** It requires an existing `flow_id`; it never fabricates
///   one. Creating a flow is [`CreateWorkflowTool`]/[`DuplicateFlowTool`]'s
///   job — `save_workflow` can only write onto a flow that already exists
///   (whether the host, the user, or an earlier `create_workflow`/
///   `duplicate_flow` call made it).
/// - **Never touches enablement or the approval gate.** `enabled` and
///   `require_approval` are not parameters; whatever the user set stays —
///   except that saving a graph whose trigger just transitioned from manual
///   to automatic on an already-enabled flow auto-disables it (see
///   [`ops::flows_update`]'s own doc for that guard).
/// - **Real persistence, real consequences.** Saving a `schedule`/`app_event`
///   trigger onto an ENABLED flow arms it (the trigger binds and will fire on
///   its own) — hence `PermissionLevel::Write`. The description tells the agent
///   to dry-run first and to say what it saved.
pub struct SaveWorkflowTool {
    config: Arc<Config>,
}

impl SaveWorkflowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
