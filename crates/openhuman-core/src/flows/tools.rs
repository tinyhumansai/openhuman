//! Agent-facing tool for the `flows::` domain (issue B4 — agent-first
//! Workflow authoring): [`ProposeWorkflowTool`] ("propose_workflow").
//!
//! The user asks the assistant in chat to build an automation; the agent
//! calls this tool with a candidate `tinyflows::model::WorkflowGraph`. The
//! tool runs the graph through the exact same
//! [`tinyflows::authoring::validate_and_migrate_graph`] path
//! `flows_create` uses, and returns a `workflow_proposal` summary for the
//! chat UI's `WorkflowProposalCard` — it never persists anything itself.
//!
//! **Human-in-the-loop invariant:** this tool must NEVER call
//! [`crate::flows::ops::flows_create`] (or any other persistence
//! path). Only the user's "Save & enable" click in `WorkflowProposalCard`
//! creates the flow, via the `openhuman.flows_create` RPC directly from the
//! client. `permission_level() == PermissionLevel::None` and
//! `external_effect() == false` reflect that this call has no side effect —
//! it is pure validation.

use crate::tools::schema_cache::static_schema;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;
use crate::flows::ops::build_builder_proposal;
use tinyflows::authoring::validate_and_migrate_graph;
use tinytools::{PermissionLevel, Tool, ToolResult};

pub struct ProposeWorkflowTool {
    config: Arc<Config>,
}

impl ProposeWorkflowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ProposeWorkflowTool {
    fn name(&self) -> &str {
        "propose_workflow"
    }

    fn description(&self) -> &str {
        // Generated once, not hand-written.
        //
        // This description used to carry a 5,841-byte copy of the node-kind
        // reference: every kind, its required *and* optional config, and a
        // paragraph of gotchas each. That is the third copy of the same
        // material — `get_node_kind_contract` serves it authoritatively and
        // `workflow_builder`'s prompt carries an index — and it shipped on
        // every request of every agent holding this tool.
        //
        // `tinyflows::catalog::render_node_kinds_required` reduces it to the one thing a caller
        // cannot recover from a failed call: which kinds exist and what config
        // each cannot be built without (401 bytes). Everything else is one
        // `get_node_kind_contract { kind }` away.
        //
        // Generating it also retires a drift class rather than testing for it.
        // `propose_workflow_description_matches_typed_node_contracts` existed
        // because a hand-written copy could fall behind `node_contracts.rs`;
        // it now passes by construction, and stays as the regression guard for
        // anyone tempted to hand-write this again.
        static DESCRIPTION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        DESCRIPTION
            .get_or_init(|| {
                format!(
                    "Propose a candidate automation workflow for the user to review and save. \
                 This tool ONLY VALIDATES the graph and returns a summary — it NEVER creates \
                 or enables the flow; the user must click \"Save & enable\" in the UI before \
                 anything is persisted or can run. If validation fails, fix the graph and call \
                 this tool again.\n\
                 \n\
                 Build a tinyflows WorkflowGraph: nodes[] ({{id, kind, name, config}}) + \
                 edges[] ({{from_node, to_node, from_port?, to_port?}}; ports default \
                 \"main\"). Exactly ONE trigger node is required.\n\
                 \n\
                 Branching (condition/switch): the branch label goes on from_port, NEVER on \
                 to_port (which stays \"main\"). Routing is keyed exclusively on from_port, so \
                 a label on to_port silently turns the branch into an unconditional fan-out \
                 and is a hard reject.\n\
                 \n\
                 A memory node may only target config.scope \"flow\" for remember/forget; \
                 \"user\" is READ-ONLY and a write to it is a hard reject.\n\
                 \n\
                 Node kinds and their required config: {kinds}.\n\
                 Call `get_node_kind_contract {{ kind }}` for a kind's optional fields, ports, \
                 a worked example, and its gotchas — it is generated from the catalog the \
                 validator enforces, so it is always current.",
                    kinds = tinyflows::catalog::render_node_kinds_required(
                        &crate::flows::all_node_kind_contracts()
                    )
                )
            })
            .as_str()
    }

    fn parameters_schema(&self) -> Value {
        static_schema!(include_str!("parameters/propose_workflow.json"))
    }

    fn permission_level(&self) -> PermissionLevel {
        // Pure validation with no side effect — see module doc.
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        // Never persists or executes anything; only `flows_create` (invoked
        // from the client by the user's own "Save & enable" click) does.
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        tool_context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(tool_context);
        let name = match args.get("name").and_then(Value::as_str).map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
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

        tracing::debug!(
            target: "flows",
            %name,
            require_approval,
            workspace = %self.config.workspace_dir.display(),
            "[flows] propose_workflow: validating candidate graph"
        );

        let graph = match validate_and_migrate_graph(graph_json) {
            Ok(graph) => graph,
            Err(e) => {
                tracing::debug!(
                    target: "flows",
                    %name,
                    error = %e,
                    "[flows] propose_workflow: validation failed"
                );
                return Ok(ToolResult::error(format!(
                    "Workflow graph is invalid: {e}. Fix the graph and call propose_workflow \
                     again."
                )));
            }
        };

        // Route every first proposal through the same canonical hard-gate and
        // payload builder as revise/edit/save. In particular, this includes
        // compatibility checks for literal workflow_id children, which cannot
        // be detected by graph-only validation because they require the store.
        match build_builder_proposal(
            &ctx,
            &self.config,
            "propose_workflow",
            &name,
            &graph,
            require_approval,
            false,
            None,
            None,
            None,
        )
        .await
        {
            Ok(payload) => Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?)),
            Err(error) => {
                tracing::debug!(
                    target: "flows",
                    %name,
                    %error,
                    "[flows] propose_workflow: builder gate rejected the graph"
                );
                Ok(ToolResult::error(error))
            }
        }
    }
}

/// Runs a **saved** workflow by id so the `workflow-builder` agent can *test*
/// it end-to-end. Unlike `tinyflows_copilot::tools::DryRunWorkflowTool`
/// (a MOCK sandbox), this is a **real** run — so it is `PermissionLevel::Write`
/// with `external_effect() == true`. Two safety layers remain: the flow's own
/// `require_approval` gate still pauses outbound-action nodes mid-run, and the
/// agent prompt requires it to ASK THE USER for confirmation before ever
/// calling this. It only runs an already-persisted flow (no `flow_id`, no run).
pub struct RunFlowTool {
    config: Arc<Config>,
}

impl RunFlowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for RunFlowTool {
    fn name(&self) -> &str {
        // NOTE: deliberately `run_flow`, not `run_workflow` — the latter
        // name is already taken by the unrelated legacy skills-workflow
        // runner (`crate::agent::tools::run_workflow`,
        // `RUN_WORKFLOW_TOOL_NAME`), which is registered in the same
        // default tool registry (`tools::ops::all_tools_with_runtime`).
        // Both tools were previously named identically, which
        // `all_tools_default_registry_has_no_duplicate_tool_names` caught
        // as a duplicate-tool-name registry bug.
        "run_flow"
    }

    fn description(&self) -> &str {
        "Run a SAVED workflow by id to TEST it end-to-end. This is a REAL run, not a \
         simulation — real effects can fire (use dry_run_workflow for a safe MOCK run \
         instead). It only works on a flow the user has already saved; pass its `flow_id`. \
         You MUST ask the user to confirm and wait for an explicit 'yes' before calling this \
         — never run a workflow unprompted. The flow's own approval gate still pauses \
         outbound-action nodes. If the flow declares workflow inputs (read `graph.inputs` \
         via get_flow), pass their values in `inputs` — ask the user for any required one \
         rather than inventing it; a missing or wrongly-typed value is rejected and nothing \
         runs. Params: { flow_id (required), input?, inputs? }. Returns the run's status + \
         any nodes paused for approval."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": {
                    "type": "string",
                    "description": "Id of the SAVED flow to run (the user must have saved it first)."
                },
                "input": {
                    "description": "Optional trigger input passed to the run (defaults to {})."
                },
                "inputs": {
                    "type": "object",
                    "description": "Values for the flow's DECLARED workflow inputs, keyed by \
                                    name. Read the declarations from the flow's graph.inputs \
                                    first; ask the user for any required value instead of \
                                    guessing. Distinct from 'input', the free-form trigger \
                                    payload."
                }
            },
            "required": ["flow_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // A real run with real effects — gated like a Write-class action.
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        tool_context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(tool_context);
        let flow_id = match args.get("flow_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing 'flow_id' — run_flow only works on a SAVED flow. Ask the user \
                     to Save the workflow first, then run it by id."
                        .to_string(),
                ))
            }
        };
        let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
        // A non-object `inputs` is the model mis-shaping the call; say so
        // plainly rather than silently running with none, which would produce a
        // confusing "required input missing" for a value it thinks it sent.
        let inputs = match args.get("inputs") {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(map)) => map.clone(),
            Some(_) => {
                return Ok(ToolResult::error(
                    "'inputs' must be an object keyed by the flow's declared input names, \
                     e.g. {\"repo\": \"acme/api\"}. Read the declarations from the flow's \
                     graph.inputs."
                        .to_string(),
                ))
            }
        };

        tracing::info!(
            target: "flows",
            %flow_id,
            "[flows] run_flow: agent-initiated test run starting (detached)"
        );

        // Detach (bug B41): a flow whose first real node is a live-research
        // agent node inherently runs longer than the tinyagents harness's 120s
        // per-tool-call cap, so a blocking `run_flow` could never succeed —
        // it died at 120s, orphaning the run row (bug B42). `flows_run_detached`
        // validates + compile-checks synchronously (so a broken flow still
        // returns an immediate, actionable error), fires the run on a background
        // task, and returns `{ run_id, status: "running" }` in well under 120s.
        // The copilot then polls `get_flow_run(run_id)` to observe completion.
        match crate::flows::ops::flows_run_detached(
            &ctx,
            &self.config,
            &flow_id,
            input,
            inputs,
            tinyflows_catalog::types::FlowRunTrigger::Rpc,
        )
        .await
        {
            Ok(outcome) => {
                let run_id = outcome
                    .value
                    .get("run_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "type": "workflow_run_started",
                    "flow_id": flow_id,
                    "run_id": run_id,
                    "status": "running",
                    "detached": true,
                    "note": "The run started in the background and is now 'running'. Poll \
                             get_flow_run with this run_id to see it settle to a terminal \
                             status (completed / failed / interrupted / pending_approval); \
                             do not assume success from this response alone.",
                    "result": outcome.value,
                }))?))
            }
            Err(e) => {
                tracing::debug!(target: "flows", %flow_id, error = %e, "[flows] run_flow: failed to start");
                Ok(ToolResult::error(format!(
                    "Could not start flow '{flow_id}': {e}"
                )))
            }
        }
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools_schema_tests.rs"]
mod schema_tests;
