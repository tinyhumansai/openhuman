//! Agent-facing tool for the `flows::` domain (issue B4 — agent-first
//! Workflow authoring): [`ProposeWorkflowTool`] ("propose_workflow").
//!
//! The user asks the assistant in chat to build an automation; the agent
//! calls this tool with a candidate `tinyflows::model::WorkflowGraph`. The
//! tool runs the graph through the exact same
//! [`crate::openhuman::flows::ops::validate_and_migrate_graph`] path
//! `flows_create` uses, and returns a `workflow_proposal` summary for the
//! chat UI's `WorkflowProposalCard` — it never persists anything itself.
//!
//! **Human-in-the-loop invariant:** this tool must NEVER call
//! [`crate::openhuman::flows::ops::flows_create`] (or any other persistence
//! path). Only the user's "Save & enable" click in `WorkflowProposalCard`
//! creates the flow, via the `openhuman.flows_create` RPC directly from the
//! client. `permission_level() == PermissionLevel::None` and
//! `external_effect() == false` reflect that this call has no side effect —
//! it is pure validation.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::model::{Node, NodeKind, WorkflowGraph};

use crate::openhuman::config::Config;
use crate::openhuman::flows::ops::{
    validate_and_migrate_graph, validate_binding_resolvability, validate_tool_contracts,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Max characters kept for a `config_hint` before truncation, so a long
/// prompt/expression doesn't blow up the proposal summary sent to the LLM
/// and rendered in the chat card.
const MAX_CONFIG_HINT_CHARS: usize = 80;

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
        "Propose a candidate automation workflow for the user to review and save. This tool \
         ONLY VALIDATES the graph and returns a summary — it NEVER creates or enables the flow; \
         the user must click \"Save & enable\" in the UI before anything is persisted or can \
         run. Build a tinyflows WorkflowGraph: nodes[] ({id, kind, name, config}) + edges[] \
         ({from_node, to_node, from_port?, to_port?}; ports default \"main\"). Exactly ONE \
         trigger node is required. The 12 node kinds: trigger (config.trigger_kind: manual | \
         schedule | webhook | app_event | form | chat_message | evaluation | system | \
         execute_by_workflow; schedule needs config.schedule = {kind:\"cron\",expr,tz?} | \
         {kind:\"at\",at} | {kind:\"every\",every_ms}; app_event needs config.toolkit + \
         config.trigger_slug), agent (config.prompt), tool_call (config.slug REQUIRED + \
         config.args), http_request (config.method/url, optional headers/body), code \
         (config.language: \"javascript\"|\"python\" + config.source), condition (config.field; \
         routes ports \"true\"/\"false\"), switch (config.expression or config.field; routes to \
         the matching case port, or \"default\"), transform (config.set: {key: \"=expr\"} \
         merged onto each item), split_out (config.path to an array field; fans out one item per \
         element), merge (fan-in passthrough, no config), output_parser (passthrough today; no \
         config required), sub_workflow (config.workflow: an embedded child WorkflowGraph). If \
         validation fails, fix the graph and call this tool again."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for the proposed flow."
                },
                "graph": {
                    "type": "object",
                    "description": "A tinyflows WorkflowGraph: { name?, nodes: [...], edges: [...] }. See the tool description for node kinds and their config shapes.",
                    "properties": {
                        "nodes": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string", "description": "Unique id within the graph." },
                                    "kind": {
                                        "type": "string",
                                        "enum": [
                                            "trigger", "agent", "tool_call", "http_request",
                                            "code", "condition", "switch", "merge", "split_out",
                                            "transform", "output_parser", "sub_workflow"
                                        ]
                                    },
                                    "name": { "type": "string", "description": "Human-readable node name." },
                                    "config": { "description": "Kind-specific configuration; see tool description." }
                                },
                                "required": ["id", "kind", "name"]
                            }
                        },
                        "edges": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "from_node": { "type": "string" },
                                    "to_node": { "type": "string" },
                                    "from_port": { "type": "string", "description": "Defaults to \"main\"." },
                                    "to_port": { "type": "string", "description": "Defaults to \"main\"." }
                                },
                                "required": ["from_node", "to_node"]
                            }
                        }
                    },
                    "required": ["nodes", "edges"]
                },
                "require_approval": {
                    "type": "boolean",
                    "description": "Force a human-approval gate on every outbound tool/HTTP action this flow takes once saved. Defaults to true for agent-proposed flows."
                }
            },
            "required": ["name", "graph"]
        })
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

        // Enforcing binding-resolvability gate (see
        // `ops::validate_binding_resolvability`): reject outright — rather
        // than merely warn — a `tool_call` binding that is guaranteed to
        // resolve null (or the wrong value) at runtime, so the builder must
        // fix the graph before it can even be proposed to the user.
        let binding_errors = validate_binding_resolvability(&graph);
        if !binding_errors.is_empty() {
            tracing::debug!(
                target: "flows",
                %name,
                error_count = binding_errors.len(),
                "[flows] propose_workflow: binding-resolvability check rejected the graph"
            );
            return Ok(ToolResult::error(format!(
                "{}\n\nFix these bindings and call propose_workflow again.",
                binding_errors.join("\n\n")
            )));
        }

        // Tool-contract enforcement gate (systemic tool-contract fix, Part 2):
        // reject a `tool_call` node whose slug isn't a REAL action in the
        // live Composio catalog, or whose real required args aren't all
        // wired — before the user ever reviews the proposal.
        let contract_errors = validate_tool_contracts(&self.config, &graph).await;
        if !contract_errors.is_empty() {
            tracing::debug!(
                target: "flows",
                %name,
                error_count = contract_errors.len(),
                "[flows] propose_workflow: tool-contract check rejected the graph"
            );
            return Ok(ToolResult::error(format!(
                "{}\n\nFix these tool_call nodes and call propose_workflow again.",
                contract_errors.join("\n\n")
            )));
        }

        let summary = build_summary(&graph);
        // Author-time warnings: unfired trigger kinds + unwired REQUIRED
        // Composio args (see `ops::graph_wiring_warnings`) — surfaced on the
        // proposal so the builder fixes wiring before the user saves.
        let mut warnings = crate::openhuman::flows::ops::graph_trigger_warnings(&graph);
        warnings.extend(
            crate::openhuman::flows::ops::graph_wiring_warnings(&self.config, &graph).await,
        );
        let graph_value = serde_json::to_value(&graph)?;

        tracing::info!(
            target: "flows",
            %name,
            node_count = graph.nodes.len(),
            require_approval,
            warning_count = warnings.len(),
            "[flows] propose_workflow: proposal ready for user review"
        );

        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "type": "workflow_proposal",
            "name": name,
            "graph": graph_value,
            "require_approval": require_approval,
            "summary": summary,
            "warnings": warnings,
        }))?))
    }
}

/// Runs a **saved** workflow by id so the `workflow-builder` agent can *test*
/// it end-to-end. Unlike [`crate::openhuman::flows::builder_tools::DryRunWorkflowTool`]
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
        // runner (`crate::openhuman::agent::tools::run_workflow`,
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
         outbound-action nodes. Params: { flow_id (required), input? }. Returns the run's \
         status + any nodes paused for approval."
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

        tracing::info!(
            target: "flows",
            %flow_id,
            "[flows] run_flow: agent-initiated test run starting"
        );

        match crate::openhuman::flows::ops::flows_run(
            &self.config,
            &flow_id,
            input,
            crate::openhuman::flows::types::FlowRunTrigger::Rpc,
        )
        .await
        {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                "type": "workflow_run_result",
                "flow_id": flow_id,
                "result": outcome.value,
            }))?)),
            Err(e) => {
                tracing::debug!(target: "flows", %flow_id, error = %e, "[flows] run_flow: failed");
                Ok(ToolResult::error(format!(
                    "Could not run flow '{flow_id}': {e}"
                )))
            }
        }
    }
}

/// Builds the `{ trigger, steps }` summary surfaced to both the LLM (in the
/// tool result) and the chat UI's `WorkflowProposalCard`.
///
/// `pub(crate)` so the `workflow-builder` tool belt's
/// [`crate::openhuman::flows::builder_tools::ReviseWorkflowTool`] reuses the
/// identical summary shape rather than duplicating it.
pub(crate) fn build_summary(graph: &WorkflowGraph) -> Value {
    let trigger = graph
        .trigger()
        .map(describe_trigger)
        .unwrap_or_else(|| "no trigger".to_string());

    let steps: Vec<Value> = graph
        .nodes
        .iter()
        .filter(|n| n.kind != NodeKind::Trigger)
        .map(|n| {
            let mut step = json!({
                "kind": node_kind_str(&n.kind),
                "name": n.name,
            });
            if let Some(hint) = config_hint(n) {
                step["config_hint"] = json!(hint);
            }
            step
        })
        .collect();

    json!({ "trigger": trigger, "steps": steps })
}

/// The `snake_case` wire string for a [`NodeKind`] (its `Serialize` impl),
/// for the summary/step JSON. Falls back to `"unknown"` only if serializing
/// ever somehow fails — `NodeKind`'s derive is infallible in practice.
fn node_kind_str(kind: &NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// One-line human description of a trigger node, for the summary's
/// `"trigger"` field — e.g. `"schedule: 0 9 * * *"`, `"app event:
/// gmail/GMAIL_NEW_GMAIL_MESSAGE"`, `"manual"`.
fn describe_trigger(node: &Node) -> String {
    let trigger_kind = node
        .config
        .get("trigger_kind")
        .and_then(Value::as_str)
        .unwrap_or("manual");

    match trigger_kind {
        "schedule" => {
            let schedule = node.config.get("schedule");
            if let Some(expr) = schedule.and_then(|s| s.get("expr")).and_then(Value::as_str) {
                format!("schedule: {expr}")
            } else if let Some(ms) = schedule
                .and_then(|s| s.get("every_ms"))
                .and_then(Value::as_u64)
            {
                format!("schedule: every {ms}ms")
            } else if let Some(at) = schedule.and_then(|s| s.get("at")).and_then(Value::as_str) {
                format!("schedule: once at {at}")
            } else {
                "schedule (unspecified)".to_string()
            }
        }
        "app_event" => {
            let toolkit = node
                .config
                .get("toolkit")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let slug = node
                .config
                .get("trigger_slug")
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("app event: {toolkit}/{slug}")
        }
        other => other.to_string(),
    }
}

/// Short, human-readable hint for a non-trigger node's config, for the
/// step's optional `"config_hint"` field. `None` when the kind has nothing
/// worth surfacing (e.g. `merge`, `output_parser`).
fn config_hint(node: &Node) -> Option<String> {
    let cfg = &node.config;
    match &node.kind {
        NodeKind::Agent => cfg.get("prompt").and_then(Value::as_str).map(truncate_hint),
        NodeKind::ToolCall => cfg.get("slug").and_then(Value::as_str).map(str::to_string),
        NodeKind::HttpRequest => {
            let method = cfg.get("method").and_then(Value::as_str).unwrap_or("GET");
            let url = cfg.get("url").and_then(Value::as_str).unwrap_or("?");
            Some(truncate_hint(&format!("{method} {url}")))
        }
        NodeKind::Code => cfg
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some("javascript".to_string())),
        NodeKind::Condition => cfg
            .get("field")
            .and_then(Value::as_str)
            .map(|f| format!("field: {f}")),
        NodeKind::Switch => cfg
            .get("expression")
            .and_then(Value::as_str)
            .or_else(|| cfg.get("field").and_then(Value::as_str))
            .map(truncate_hint),
        NodeKind::Transform => cfg.get("set").and_then(Value::as_object).map(|set| {
            let keys: Vec<&str> = set.keys().map(String::as_str).collect();
            truncate_hint(&format!("sets: {}", keys.join(", ")))
        }),
        NodeKind::SplitOut => cfg
            .get("path")
            .and_then(Value::as_str)
            .map(|p| format!("path: {p}")),
        NodeKind::SubWorkflow => Some("embedded sub-workflow".to_string()),
        NodeKind::Merge | NodeKind::OutputParser | NodeKind::Trigger => None,
    }
}

/// Truncates a hint string to [`MAX_CONFIG_HINT_CHARS`], appending an
/// ellipsis when it was cut — mirrors
/// `crate::openhuman::tools::traits::render_context_value`'s truncation
/// behavior for tool-call timeline details.
fn truncate_hint(s: &str) -> String {
    if s.chars().count() <= MAX_CONFIG_HINT_CHARS {
        return s.to_string();
    }
    let truncated: String = s
        .chars()
        .take(MAX_CONFIG_HINT_CHARS.saturating_sub(1))
        .collect();
    format!("{truncated}…")
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
