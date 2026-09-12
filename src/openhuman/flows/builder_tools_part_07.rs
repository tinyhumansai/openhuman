
#[async_trait]
impl Tool for SaveWorkflowTool {
    fn name(&self) -> &str {
        "save_workflow"
    }

    fn description(&self) -> &str {
        "Save a workflow graph onto an EXISTING saved flow (by `flow_id`), persisting it. \
         This is the ONLY builder tool that writes onto a saved flow — edit/validate/dry_run \
         never do. Use it after the user asked you to build/update a workflow and you have \
         dry-run-verified the graph. The graph source is either `draft_id` (a working draft — \
         the usual case after editing with edit_workflow; draft_id wins if both are given) or \
         an inline `graph`; `flow_id` is always required as the persistence TARGET. It \
         validates and writes the graph (and optional new `name`) to that flow. It can NOT \
         create a new flow, and it never touches the approval gate — but it CAN \
         auto-disable the flow when the trigger transitions from manual to automatic \
         (schedule/webhook/app_event), so a save never silently arms a trigger that wasn't \
         already live; the returned `warnings` will explain it when that happens. NOTE: if \
         the flow was ALREADY enabled with an automatic trigger and stays automatic, saving \
         re-arms it live — it will start firing on its own. Always tell the user what you \
         saved (including any auto-disable). Params: { flow_id, draft_id? | graph?, name? }."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": {
                    "type": "string",
                    "description": "Id of the EXISTING saved flow to write the graph to (the persistence target — always required)."
                },
                "draft_id": {
                    "type": "string",
                    "description": "A working draft whose graph to persist onto the flow. Provide this OR inline `graph`; if both are given, draft_id wins."
                },
                "graph": {
                    "type": "object",
                    "description": "The full tinyflows WorkflowGraph to persist: { name?, nodes: [...], edges: [...] }. Provide this OR `draft_id`. Same shape as propose_workflow.",
                    "properties": {
                        "nodes": { "type": "array" },
                        "edges": { "type": "array" }
                    },
                    "required": ["nodes", "edges"]
                },
                "name": {
                    "type": "string",
                    "description": "Optional new human-readable name for the flow."
                }
            },
            "required": ["flow_id"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Persists a flow definition; on an enabled flow this can arm a
        // self-firing trigger — gate like a Write-class action.
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        // Persistence is local (no message/HTTP/code fires at save time); the
        // flow's own runs — and their approval gate — govern real effects.
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let flow_id = match args.get("flow_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing 'flow_id' — save_workflow only updates an EXISTING saved flow. \
                     If there is no flow yet, return the proposal and let the user save it."
                        .to_string(),
                ))
            }
        };
        // Graph source: a working draft (the usual post-edit_workflow handle) or
        // an inline graph. `flow_id` above is the persistence TARGET, always
        // required; the draft only supplies the graph to write. If both a
        // draft_id and an inline graph are given, the draft wins (it is the
        // durable working copy the agent just iterated on).
        let draft_id = args
            .get("draft_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let graph_json =
            if let Some(id) = draft_id {
                match ops::flows_draft_get(&self.config, id) {
                    Ok(outcome) => outcome.value.graph,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not load draft '{id}' to save: {e}"
                        )));
                    }
                }
            } else {
                match args.get("graph") {
                    Some(v) if !v.is_null() => v.clone(),
                    _ => return Ok(ToolResult::error(
                        "Provide `draft_id` (a working draft) or inline `graph` to save onto the \
                         flow."
                            .to_string(),
                    )),
                }
            };
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        // Same migrate/validate + enforcing binding-resolvability gate as
        // propose_workflow/revise_workflow, run HERE at the tool level (not
        // inside `ops::flows_update`, which the UI/RPC also call for a
        // human's own edits and which must stay permissive) — so an agent
        // can never persist a graph with an unresolvable `tool_call` binding
        // either. See `ops::validate_binding_resolvability`.
        let graph = match validate_and_migrate_graph(graph_json.clone()) {
            Ok(graph) => graph,
            Err(e) => {
                tracing::debug!(target: "flows", %flow_id, error = %e, "[flows] save_workflow: validation failed");
                return Ok(ToolResult::error(format!(
                    "Workflow graph is invalid: {e}. Fix the graph and call save_workflow again."
                )));
            }
        };
        // The full builder hard-gate stack, run through the single canonical
        // runner shared with propose/revise/edit and the strict create/update
        // RPC path (F3) — so an agent can never persist a graph that would fail
        // gates the other planes enforce.
        let gate_errors = ops::run_builder_gates(&self.config, &graph).await;
        if !gate_errors.is_empty() {
            tracing::debug!(
                target: "flows",
                %flow_id,
                error_count = gate_errors.len(),
                "[flows] save_workflow: a hard gate rejected the graph"
            );
            return Ok(ToolResult::error(format!(
                "{}\n\nFix these and call save_workflow again.",
                gate_errors.join("\n\n")
            )));
        }
        // Author-time warnings (unfired trigger kinds + unwired REQUIRED
        // Composio args) were previously computed by propose/revise but never
        // surfaced again at save time — add them here so the agent sees any
        // non-fatal wiring gaps that remain in the final persisted graph.
        let mut warnings = ops::graph_trigger_warnings(&graph);
        warnings.extend(ops::graph_wiring_warnings(&self.config, &graph).await);

        tracing::info!(
            target: "flows",
            %flow_id,
            renaming = name.is_some(),
            "[flows] save_workflow: agent-initiated save to existing flow"
        );

        match ops::flows_update(&self.config, &flow_id, name, Some(graph_json), None, None).await {
            Ok(outcome) => {
                let flow = outcome.value;
                tracing::info!(
                    target: "flows",
                    %flow_id,
                    node_count = flow.graph.nodes.len(),
                    enabled = flow.enabled,
                    "[flows] save_workflow: persisted"
                );
                // Surface any explanatory logs `flows_update` produced — most
                // notably the manual→automatic auto-disarm message (#4889) —
                // to the agent. Skip the boilerplate "flow updated: <id>" line,
                // which just duplicates the `persisted`/`flow_id` fields this
                // response already carries.
                let flow_updated_boilerplate = format!("flow updated: {flow_id}");
                warnings.extend(
                    outcome
                        .logs
                        .into_iter()
                        .filter(|log| *log != flow_updated_boilerplate),
                );
                // Issue B29 (save/enable safety), Rule 3: `flows_create` only
                // gates the FIRST creation of a flow — an agent `save_workflow`
                // targets an EXISTING flow via `flows_update`, which (since
                // #4889) force-disables the flow whenever the trigger
                // transitions from manual to automatic (schedule/webhook/
                // app_event) — so a save can never silently arm a trigger that
                // wasn't already live (see the `warnings.extend` above for the
                // explanatory log). Short of that transition, `flows_update`
                // preserves whatever `enabled` state the flow already had: if
                // it was ALREADY enabled with an automatic trigger and stays
                // automatic, saving a new graph onto it re-arms it live with no
                // further confirmation. Surface that loudly so the copilot
                // relays it to the user instead of staying silent.
                if flow.enabled && ops::trigger_is_automatic(&flow.graph) {
                    let trigger_desc = flow
                        .graph
                        .trigger()
                        .map(tools::describe_trigger)
                        .unwrap_or_else(|| "automatic".to_string());
                    let warning = format!(
                        "WARNING: this flow is ENABLED with an automatic trigger \
                         ({trigger_desc}). It is now LIVE and will fire on its own — tell the \
                         user, and offer to disable it (flows_set_enabled) if that's not what \
                         they intended."
                    );
                    tracing::warn!(
                        target: "flows",
                        %flow_id,
                        trigger = %trigger_desc,
                        "[flows] save_workflow: saved onto an enabled auto-trigger flow — now LIVE"
                    );
                    warnings.push(warning);
                }
                Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "type": "workflow_saved",
                    // Explicit counterpart to a proposal's persisted:false — this
                    // graph IS now written onto the saved flow.
                    "persisted": true,
                    "flow_id": flow.id,
                    "name": flow.name,
                    "enabled": flow.enabled,
                    "require_approval": flow.require_approval,
                    "node_count": flow.graph.nodes.len(),
                    "warnings": warnings,
                }))?))
            }
            Err(e) => {
                tracing::debug!(target: "flows", %flow_id, error = %e, "[flows] save_workflow: failed");
                Ok(ToolResult::error(format!(
                    "Could not save workflow to flow '{flow_id}': {e}"
                )))
            }
        }
    }
}
