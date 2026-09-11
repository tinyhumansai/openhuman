
#[async_trait]
impl Tool for ValidateWorkflowTool {
    fn name(&self) -> &str {
        "validate_workflow"
    }

    fn description(&self) -> &str {
        "Check a workflow graph WITHOUT proposing or saving it — the same validation the \
         propose/revise/edit/save tools run, surfaced on its own so you can verify a draft mid-\
         build. Provide the graph to check as exactly one of `draft_id` (a working draft), \
         `flow_id` (a saved flow), or inline `graph` (if several are given, draft_id wins, then \
         flow_id). Returns { ok, structurally_valid, errors, error_details:[{code, message, \
         node_id}], gate_errors, warnings }: `errors` lists EVERY structural problem at once; \
         `gate_errors` lists the hard author-gate failures (unresolvable bindings, unreal tool \
         slugs, unwired required args) checked only once the graph is structurally valid; \
         `warnings` are non-fatal. `ok` is true only when there are no errors and no gate_errors. \
         Read-only."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "draft_id": {
                    "type": "string",
                    "description": "A working draft to validate. Provide one of draft_id / flow_id / graph (draft_id wins)."
                },
                "flow_id": {
                    "type": "string",
                    "description": "A saved flow to validate. Provide one of draft_id / flow_id / graph."
                },
                "graph": {
                    "type": "object",
                    "description": "An inline tinyflows WorkflowGraph to validate. Provide one of draft_id / flow_id / graph.",
                    "properties": {
                        "nodes": { "type": "array" },
                        "edges": { "type": "array" }
                    }
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Resolve the graph to check from exactly one of a working draft, a
        // saved flow, or an inline graph — same precedence (draft_id > flow_id >
        // graph) as edit_workflow, so the sibling tools accept the same handles.
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
                        "Could not load draft '{id}' to validate: {e}"
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
                        "Could not load flow '{id}' to validate: {e}"
                    )));
                }
            },
            (None, None, Some(graph)) => graph.clone(),
            (None, None, None) => {
                return Ok(ToolResult::error(
                    "Provide one of `draft_id` (a working draft), `flow_id` (a saved flow), or \
                     `graph` (an inline graph) to validate."
                        .to_string(),
                ));
            }
        };

        tracing::debug!(
            target: "flows",
            from_draft = draft_id.is_some(),
            from_flow = flow_id.is_some(),
            "[flows] validate_workflow: checking graph (read-only)"
        );

        // Structural validation first (every error at once).
        let validation = ops::flows_validate(graph_json.clone()).value;

        // Only run the (expensive) hard gates on a structurally-valid graph.
        // A migrate/deserialize error here must fail CLOSED: `validation.valid`
        // only proves the graph passed structural checks, not that the hard
        // gates (unresolvable bindings, unreal tool slugs, unwired required
        // args) ran. Treating the empty `gate_errors` from a caught `Err` as
        // "gates passed" previously reported `ok: true` while silently
        // skipping every hard gate.
        let (gate_errors, gate_check_failed) = if validation.valid {
            match ops::migrate_and_deserialize_graph(graph_json) {
                Ok(graph) => (ops::run_builder_gates(&self.config, &graph).await, false),
                Err(e) => {
                    tracing::warn!(
                        target: "flows",
                        error = %e,
                        "[flows] validate_workflow: graph passed structural validation but \
                         failed to migrate/deserialize for gate checks; failing closed"
                    );
                    (
                        vec![format!(
                            "hard gates could not run: graph failed to migrate/deserialize ({e})"
                        )],
                        true,
                    )
                }
            }
        } else {
            (Vec::new(), false)
        };

        let ok = validate_workflow_report_is_ok(validation.valid, &gate_errors, gate_check_failed);
        let report = json!({
            "ok": ok,
            "structurally_valid": validation.valid,
            "errors": validation.errors,
            "error_details": validation.error_details,
            "gate_errors": gate_errors,
            "warnings": validation.warnings,
        });
        Ok(ToolResult::success(serde_json::to_string_pretty(&report)?))
    }
}

/// `validate_workflow`'s aggregate verdict (T-m4): `ok` must be true only when
/// the graph is structurally valid, every hard gate ran, AND every hard gate
/// passed. Pulled out as a pure function so the fail-closed invariant — a
/// gate-check failure (e.g. a migrate/deserialize error) must never be
/// reported as `ok: true` — is unit-testable independent of the async gate
/// execution and the (currently unreachable, pending future per-node schema
/// migrations) path that produces `gate_check_failed`.
fn validate_workflow_report_is_ok(
    structurally_valid: bool,
    gate_errors: &[String],
    gate_check_failed: bool,
) -> bool {
    structurally_valid && gate_errors.is_empty() && !gate_check_failed
}

// ─────────────────────────────────────────────────────────────────────────────
// get_flow_history — read-only: prior graph snapshots (F6)
// ─────────────────────────────────────────────────────────────────────────────

/// `get_flow_history`: read a saved flow's revision history — the prior graph
/// snapshots captured on each update. Lets the agent see what changed and pick
/// a revision to roll back to (the user drives the actual rollback RPC).
pub struct GetFlowHistoryTool {
    config: Arc<Config>,
}

impl GetFlowHistoryTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for GetFlowHistoryTool {
    fn name(&self) -> &str {
        "get_flow_history"
    }

    fn description(&self) -> &str {
        "List a saved flow's revision history — the prior graph snapshots captured automatically \
         on each update (newest first, capped). Read-only. Returns a JSON array of { id, flow_id, \
         graph, name, require_approval, created_at }. Use it to see what a flow looked like before \
         a change, or to find the revision id the user can roll back to."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": { "type": "string", "description": "The saved flow whose history to list." },
                "limit": { "type": "integer", "description": "Max revisions to return (default 20)." }
            },
            "required": ["flow_id"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let flow_id = match args.get("flow_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'flow_id' parameter".to_string())),
        };
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(20);
        tracing::debug!(target: "flows", %flow_id, limit, "[flows] get_flow_history: listing revisions (read-only)");
        match ops::flows_get_history(&self.config, &flow_id, limit) {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &json!({ "revisions": outcome.value }),
            )?)),
            Err(e) => Ok(ToolResult::error(format!(
                "Could not load history for flow '{flow_id}': {e}"
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase 4 — the self-debug loop + gated create (F4, F7)
// ─────────────────────────────────────────────────────────────────────────────

/// `list_flow_runs`: read-only listing of a saved flow's recent runs (id /
/// status / timestamps), so the agent can FIND a failing run to diagnose
/// instead of needing a run_id handed to it externally — the missing first step
/// of the self-debug loop (audit F4).
pub struct ListFlowRunsTool {
    config: Arc<Config>,
}

impl ListFlowRunsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ListFlowRunsTool {
    fn name(&self) -> &str {
        "list_flow_runs"
    }

    fn description(&self) -> &str {
        "List a saved flow's recent runs (newest first) so you can find one to diagnose with \
         get_flow_run. Read-only. Returns a JSON array of runs { id, flow_id, thread_id, status, \
         started_at, finished_at?, error? }. `id`/`thread_id` is the run id you pass to \
         get_flow_run / resume_flow_run / cancel_flow_run."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": { "type": "string", "description": "The saved flow whose runs to list." },
                "limit": { "type": "integer", "description": "Max runs to return (default 20)." }
            },
            "required": ["flow_id"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let flow_id = match args.get("flow_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'flow_id' parameter".to_string())),
        };
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(20);
        tracing::debug!(target: "flows", %flow_id, limit, "[flows] list_flow_runs: listing runs (read-only)");
        match ops::flows_list_runs(&self.config, &flow_id, limit).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &json!({ "runs": outcome.value }),
            )?)),
            Err(e) => Ok(ToolResult::error(format!(
                "Could not list runs for flow '{flow_id}': {e}"
            ))),
        }
    }
}

/// `resume_flow_run`: progress a run parked on a human approval by
/// approving/rejecting its pending node(s). Execute + approval-gated — it
/// advances a REAL run that can fire real outbound effects.
pub struct ResumeFlowRunTool {
    config: Arc<Config>,
}

impl ResumeFlowRunTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ResumeFlowRunTool {
    fn name(&self) -> &str {
        "resume_flow_run"
    }

    fn description(&self) -> &str {
        "Resume a flow run that is paused on a human approval, approving and/or rejecting its \
         pending node(s). This ADVANCES A REAL RUN — approved outbound nodes will fire — so it is \
         approval-gated. Params: { flow_id, run_id, approve?: [node_id...], reject?: [node_id...] }. \
         Use list_flow_runs / get_flow_run to find a run with status pending_approval and its \
         pending node ids first."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": { "type": "string", "description": "The run's flow id." },
                "run_id": { "type": "string", "description": "The run (thread) id to resume (from list_flow_runs)." },
                "approve": { "type": "array", "items": { "type": "string" }, "description": "Node ids to approve." },
                "reject": { "type": "array", "items": { "type": "string" }, "description": "Node ids to reject." }
            },
            "required": ["flow_id", "run_id"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Advances a real run (approved nodes fire) — gate like an execute-class,
        // approval-parked action.
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let flow_id = match args.get("flow_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'flow_id' parameter".to_string())),
        };
        let run_id = match args.get("run_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'run_id' parameter".to_string())),
        };
        let approve = string_array(&args, "approve");
        let reject = string_array(&args, "reject");
        tracing::debug!(target: "flows", %flow_id, %run_id, approve = approve.len(), reject = reject.len(), "[flows] resume_flow_run: resuming parked run");
        match ops::flows_resume(&self.config, &flow_id, &run_id, approve, reject).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &outcome.value,
            )?)),
            Err(e) => Ok(ToolResult::error(format!("Could not resume run: {e}"))),
        }
    }
}

/// `cancel_flow_run`: stop an in-flight or parked run. Write-class — it changes
/// run state but fires no new outbound effect.
///
/// **T-M3 fix.** This tool used to cancel an arbitrary `run_id` with no
/// ownership check at all — combined with `external_effect() == false` (so
/// the approval gate never parked it) and hiding that only covered the two
/// `flows_build` copilot/headless paths (`FLOWS_BUILD_COPILOT_HIDDEN_TOOLS`,
/// not the orchestrator-delegation or main-chat paths that also carry this
/// tool), a prompt-injected turn could cancel ANY user's in-flight or
/// approval-parked automation, unapproved. Two independent closes now apply:
/// 1. **Ownership check** — the caller must name the `flow_id` it believes
///    owns the run (mirrors [`ResumeFlowRunTool`]'s existing `{ flow_id,
///    run_id }` shape); the run row's *actual* `flow_id` is resolved and
///    compared, and a mismatch is refused rather than silently cancelling a
///    run scoped to a different flow.
/// 2. **`external_effect() == true`** — parks for approval on any surface
///    that has a gate, same as `resume_flow_run`.
pub struct CancelFlowRunTool {
    config: Arc<Config>,
}

impl CancelFlowRunTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CancelFlowRunTool {
    fn name(&self) -> &str {
        "cancel_flow_run"
    }

    fn description(&self) -> &str {
        "Cancel an in-flight or approval-parked flow run by its run_id (from list_flow_runs). \
         Stops a runaway or stuck run; fires no new outbound effect. The run_id must belong to \
         the given flow_id — cancelling a run that belongs to a different flow is refused. \
         Approval-gated. Params: { flow_id, run_id }."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": { "type": "string", "description": "The flow that owns the run being cancelled (from list_flow_runs)." },
                "run_id": { "type": "string", "description": "The run (thread) id to cancel." }
            },
            "required": ["flow_id", "run_id"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let flow_id = match args.get("flow_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'flow_id' parameter".to_string())),
        };
        let run_id = match args.get("run_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'run_id' parameter".to_string())),
        };

        // SECURITY (T-M3 fix): verify the run actually belongs to the
        // caller-named flow before cancelling anything — mirrors
        // `resume_flow_run` (`ops::flows_resume`)'s existing `run_record.flow_id
        // != flow_id` guard. Without this, any run_id (guessed, enumerated, or
        // named by a prompt-injected turn that never called list_flow_runs)
        // could cancel a run scoped to a completely different flow.
        let run = match ops::flows_get_run(&self.config, &run_id).await {
            Ok(outcome) => outcome.value,
            Err(e) => return Ok(ToolResult::error(format!("Could not cancel run: {e}"))),
        };
        if run.flow_id != flow_id {
            tracing::warn!(
                target: "flows",
                %flow_id,
                %run_id,
                actual_flow_id = %run.flow_id,
                "[flows] cancel_flow_run: refused — run belongs to a different flow than the one named"
            );
            return Ok(ToolResult::error(format!(
                "run '{run_id}' belongs to flow '{}', not '{flow_id}' — refusing to cancel",
                run.flow_id
            )));
        }

        tracing::debug!(target: "flows", %flow_id, %run_id, "[flows] cancel_flow_run: cancelling run");
        match ops::flows_cancel_run(&self.config, &run_id).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &outcome.value,
            )?)),
            Err(e) => Ok(ToolResult::error(format!("Could not cancel run: {e}"))),
        }
    }
}

/// `create_workflow`: the gated create tool (audit F4/F12). Persists a NEW
/// flow, always **born disabled** (enable stays human-only) and behind the
/// forced `require_approval` floor for side-effect graphs. Write + approval
/// gated. This is the deliberate widening the Phase 3 rails (versioning,
/// events, history) make safe.
pub struct CreateWorkflowTool {
    config: Arc<Config>,
}

impl CreateWorkflowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CreateWorkflowTool {
    fn name(&self) -> &str {
        "create_workflow"
    }

    fn description(&self) -> &str {
        "Create a NEW saved flow from a graph. Approval-gated. The flow is ALWAYS created DISABLED \
         (only the user can enable it via the UI) and inherits the forced approval gate for any \
         outbound action — so a created flow can never fire on its own without an explicit human \
         enable. Runs the same author hard-gates as save. Params: { name, graph, require_approval? }. \
         Prefer propose_workflow when the user just wants to review a design; use this when they've \
         explicitly asked you to create the flow."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Human-readable flow name." },
                "graph": {
                    "type": "object",
                    "description": "The tinyflows WorkflowGraph: { nodes: [...], edges: [...] }.",
                    "properties": { "nodes": { "type": "array" }, "edges": { "type": "array" } },
                    "required": ["nodes", "edges"]
                },
                "require_approval": { "type": "boolean", "description": "Force the approval gate (defaults true)." }
            },
            "required": ["name", "graph"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        // Persists a new flow definition.
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let name = match args.get("name").and_then(Value::as_str).map(str::trim) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => return Ok(ToolResult::error("Missing 'name' parameter".to_string())),
        };
        let graph_json = match args.get("graph") {
            Some(v) if !v.is_null() => v.clone(),
            _ => return Ok(ToolResult::error("Missing 'graph' parameter".to_string())),
        };
        let require_approval = args
            .get("require_approval")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        // Same structural + hard-gate stack an agent save must pass.
        if let Err(msg) = ops::strict_gate(&self.config, &graph_json).await {
            return Ok(ToolResult::error(format!(
                "{msg}\n\nFix the graph and call create_workflow again."
            )));
        }

        tracing::info!(target: "flows", %name, "[flows] create_workflow: agent-initiated create (born disabled)");
        let flow = match ops::flows_create(&self.config, name, graph_json, require_approval).await {
            Ok(outcome) => outcome.value,
            Err(e) => return Ok(ToolResult::error(format!("Could not create flow: {e}"))),
        };

        // Force born-disabled: enable stays human-only, even for a manual-trigger
        // graph that flows_create would otherwise create enabled. `flows_create`
        // and this force-disable are two separate writes — not one transaction —
        // so there is necessarily a brief window between them where the row is
        // persisted `enabled: true` before this call disables it. This fix does
        // not close that window; it only stops MISREPORTING the outcome when the
        // disable itself fails.
        //
        // T-m3: `flows_set_enabled(.., false)` can fail (store error, flow
        // deleted concurrently, …). That used to be only `warn!`-logged while
        // the response unconditionally claimed `"enabled": false` — so a
        // manual-trigger flow that flows_create left enabled would stay
        // enabled while the agent told the user it was disabled. Track the
        // real post-attempt state and report THAT.
        let mut disable_succeeded = true;
        if flow.enabled {
            match ops::flows_set_enabled(&self.config, &flow.id, false).await {
                Ok(_) => {}
                Err(e) => {
                    disable_succeeded = false;
                    tracing::warn!(
                        target: "flows",
                        flow_id = %flow.id,
                        error = %e,
                        "[flows] create_workflow: could not force-disable the new flow — it \
                         remains ENABLED; reporting the true state, not the intended one"
                    );
                }
            }
        }
        let (enabled, note) = create_workflow_report(flow.enabled, disable_succeeded);

        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "type": "workflow_created",
            "flow_id": flow.id,
            "name": flow.name,
            "enabled": enabled,
            "require_approval": flow.require_approval,
            "note": note,
        }))?))
    }
}

/// `create_workflow`'s reported `enabled` state + note (T-m3): derived from
/// whether the flow was born enabled (`born_enabled`, from `flows_create`'s
/// Rule 1) and whether the subsequent force-disable attempt succeeded
/// (`disable_succeeded`, ignored when no attempt was made). Pulled out as a
/// pure function so the fail-HONEST invariant — the response must reflect
/// the flow's real post-attempt state, not the intended one — is
/// unit-testable without forcing a genuine concurrent store failure between
/// `flows_create` and `flows_set_enabled`.
fn create_workflow_report(born_enabled: bool, disable_succeeded: bool) -> (bool, &'static str) {
    let enabled = born_enabled && !disable_succeeded;
    let note = if enabled {
        "Flow created, but it could NOT be force-disabled (see the tool result for the \
         underlying error) — it is currently ENABLED. Tell the user and ask them to disable it \
         manually if that was not intended."
    } else {
        "Flow created DISABLED. The user must enable it explicitly before it can run."
    };
    (enabled, note)
}

/// `duplicate_flow`: create an independent, DISABLED copy of a saved flow — the
/// clone-then-edit pattern. Write-class.
pub struct DuplicateFlowTool {
    config: Arc<Config>,
}

impl DuplicateFlowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
