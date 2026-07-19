//! Agent tool belt for the `workflow-builder` specialist (Phase 5b).
//!
//! These tools give the `workflow-builder` agent (see
//! `agent_registry/agents/workflow_builder/`) a **deliberately narrow**,
//! propose-or-read surface for authoring tinyflows [`WorkflowGraph`]s in chat:
//!
//! | Tool                    | Permission              | Effect                                    |
//! | ----------------------- | ----------------------- | ----------------------------------------- |
//! | [`ReviseWorkflowTool`]  | `None`                  | validate a revised draft → proposal       |
//! | [`ListFlowsTool`]       | `None`                  | read: list saved flows                    |
//! | [`GetFlowTool`]         | `None`                  | read: fetch a saved flow's graph          |
//! | [`GetFlowRunTool`]      | `None`                  | read: fetch a run's steps                 |
//! | [`ListFlowConnectionsTool`] | `None`              | read: connection refs (ids/names only)    |
//! | [`SearchToolCatalogTool`]   | `None`              | read: real Composio tool slugs (live catalog) |
//! | [`GetToolContractTool`]     | `None`              | read: one action's FULL live contract     |
//! | [`GetToolOutputSampleTool`] | `ReadOnly`          | ONE bounded real Composio call (Read-scope only, connected toolkit only) |
//! | [`ListAgentProfilesTool`]   | `None`              | read: selectable agent kinds (`agent_ref`)|
//! | [`DryRunWorkflowTool`]  | `Execute` (tier-gated)  | run a *draft* against MOCK capabilities   |
//! | [`SaveWorkflowTool`]    | `Write`                 | persist a graph onto an EXISTING flow     |
//!
//! **Human-in-the-loop invariant (shared with [`super::tools::ProposeWorkflowTool`]),
//! with one deliberate carve-out:** `revise_workflow` only validates and
//! returns a proposal payload (identical contract to `propose_workflow`); the
//! read tools are pure reads; `dry_run_workflow` executes against `tinyflows`'
//! deterministic **mock** capabilities so no real LLM / tool / HTTP / code side
//! effect can fire. The carve-out is [`SaveWorkflowTool`]: it persists a graph
//! onto a flow that ALREADY exists (the Flows prompt bar's instant-create path
//! makes the flow first and hands the agent its id) — but the agent still
//! cannot *create* a flow, and never touches `enabled`/`require_approval`.
//!
//! The agent's full tool scope (see `agent_registry/agents/workflow_builder/
//! agent.toml`) also grants the Composio **discovery/connect** tools —
//! `composio_list_toolkits`, `composio_list_connections`, `composio_connect`
//! (defined in `composio/tools.rs`) — so the builder can link an app the
//! workflow needs before proposing. Those stay within the invariant: connect
//! is an approval-gated OAuth hand-off, and `composio_execute` (running an
//! arbitrary real action, any scope) remains deliberately OUT of scope.
//!
//! **One narrow, deliberate carve-out (B12):** [`GetToolOutputSampleTool`]
//! (`get_tool_output_sample`) DOES perform a real Composio call — but only
//! ever a `Read`-scope one (hard-refused otherwise, regardless of the user's
//! per-toolkit scope preference), and only against a toolkit the user has
//! ALREADY connected. It exists because some actions' live listings publish
//! no output schema at all (verified for every GitHub action), leaving
//! `get_tool_contract` with no ground truth for a downstream `split_out.path`
//! — this makes exactly one bounded real read to observe the actual shape
//! instead. It can never send/create/update/delete anything.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::model::WorkflowGraph;

use crate::openhuman::config::Config;
use crate::openhuman::flows::ops;
use crate::openhuman::flows::ops::validate_and_migrate_graph;
use crate::openhuman::flows::tools;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Wall-clock bound on a single `dry_run_workflow` mock execution. A malformed
/// or pathological draft graph must never hang the agent tool-loop; the mock
/// capabilities are non-blocking echoes, so this is a generous safety net.
const DRY_RUN_TIMEOUT_SECS: u64 = 30;

/// Comma list of the valid `op` tag values, for the missing-/unknown-`op`
/// parse errors surfaced by [`EditWorkflowTool`].
const VALID_OP_TYPES: &str = "add_node, update_node_config, set_node_name, rename_node, \
     remove_node, add_edge, remove_edge, set_node_position";

/// The expected field shape for a given `op` tag, used in `edit_workflow`'s
/// per-op parse diagnostics so a failing op tells the agent exactly what that
/// op type wants. Returns `None` for an unrecognized tag.
fn edit_op_shape(op: &str) -> Option<&'static str> {
    Some(match op {
        "add_node" => "{ op, node: { id, kind, name, config? } }",
        "update_node_config" => {
            "{ op, id, config } (id also accepts alias `node_id`; config is a JSON merge-patch)"
        }
        "set_node_name" => "{ op, id, name } (id also accepts alias `node_id`)",
        "rename_node" => "{ op, id, new_id } (also accept aliases `node_id` / `new_node_id`)",
        "remove_node" => "{ op, id } (id also accepts alias `node_id`)",
        "add_edge" => "{ op, edge: { from_node, to_node, from_port?, to_port? } }",
        "remove_edge" => "{ op, from_node, to_node, from_port?, to_port? }",
        "set_node_position" => "{ op, id, position: { x, y } } (id also accepts alias `node_id`)",
        _ => return None,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// revise_workflow — iterative refine of an existing draft (proposal only)
// ─────────────────────────────────────────────────────────────────────────────

/// `revise_workflow`: validate a **revised** draft graph and return the same
/// `workflow_proposal` payload as `propose_workflow`.
///
/// Framed for iterative refinement: the agent supplies the updated `graph` (its
/// revision of a prior draft) plus the `instruction` that motivated the change;
/// the tool validates via the exact same [`validate_and_migrate_graph`] path
/// `flows_create` uses and echoes an optional `revision` note. It NEVER
/// persists — identical human-in-the-loop invariant to
/// [`super::tools::ProposeWorkflowTool`].
pub struct ReviseWorkflowTool {
    config: Arc<Config>,
}

impl ReviseWorkflowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ReviseWorkflowTool {
    fn name(&self) -> &str {
        "revise_workflow"
    }

    fn description(&self) -> &str {
        "Refine an EXISTING workflow draft: supply the full updated tinyflows \
         WorkflowGraph (your revision applied to the prior draft — NOT a \
         regeneration from scratch) plus the `instruction` that motivated the \
         change. Like propose_workflow, this ONLY VALIDATES the revised graph \
         and returns a proposal summary for the user to review — it NEVER \
         creates, updates, or enables the flow. Same graph shape and node kinds \
         as propose_workflow. If validation fails, fix the graph and call again."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for the (revised) proposed flow."
                },
                "graph": {
                    "type": "object",
                    "description": "The full REVISED tinyflows WorkflowGraph: { name?, nodes: [...], edges: [...] }. Apply your changes to the prior draft and pass the whole graph — see propose_workflow for node kinds and config shapes.",
                    "properties": {
                        "nodes": { "type": "array" },
                        "edges": { "type": "array" }
                    },
                    "required": ["nodes", "edges"]
                },
                "instruction": {
                    "type": "string",
                    "description": "The revision instruction that motivated this change (e.g. 'add a Slack step after the summary'). Echoed back for the review card; does not affect validation."
                },
                "require_approval": {
                    "type": "boolean",
                    "description": "Force a human-approval gate on every outbound action once saved. Defaults to true for agent-proposed flows."
                }
            },
            "required": ["name", "graph"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Pure validation, no side effect — mirrors propose_workflow.
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
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
        let instruction = args
            .get("instruction")
            .and_then(Value::as_str)
            .map(str::to_string);
        let require_approval = args
            .get("require_approval")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        tracing::debug!(
            target: "flows",
            %name,
            require_approval,
            has_instruction = instruction.is_some(),
            workspace = %self.config.workspace_dir.display(),
            "[flows] revise_workflow: validating revised candidate graph"
        );

        let graph = match validate_and_migrate_graph(graph_json) {
            Ok(graph) => graph,
            Err(e) => {
                tracing::debug!(target: "flows", %name, error = %e, "[flows] revise_workflow: validation failed");
                return Ok(ToolResult::error(format!(
                    "Revised workflow graph is invalid: {e}. Fix the graph and call \
                     revise_workflow again."
                )));
            }
        };

        // Full builder hard-gate stack (binding-resolvability → tool-contract →
        // required-arg resolvability) + summary/warning assembly, shared with
        // edit_workflow so the two proposal paths can't drift.
        match ops::build_builder_proposal(
            &self.config,
            "revise_workflow",
            &name,
            &graph,
            require_approval,
            true,
            instruction,
            // revise_workflow takes only an inline graph — no draft/flow handle
            // to echo. The payload still carries persisted:false unconditionally.
            None,
            None,
        )
        .await
        {
            Ok(payload) => Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?)),
            Err(message) => {
                tracing::debug!(target: "flows", %name, "[flows] revise_workflow: a hard gate rejected the revised graph");
                Ok(ToolResult::error(message))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// edit_workflow — structured incremental edits (proposal only) — F1
// ─────────────────────────────────────────────────────────────────────────────

/// `edit_workflow`: apply a small list of structured graph ops to a base graph
/// (a saved flow by `flow_id`, or an inline `graph`) instead of re-emitting the
/// whole graph. Applies the ops, runs the full validate + hard-gate stack, and
/// returns the same `workflow_proposal` payload as `revise_workflow`.
///
/// This is the cheap, low-regression iteration path (audit F1): a one-field
/// tweak on a 20-node flow is one `update_node_config` op, not a full re-emit.
/// Still proposal-only — never persists or enables.
pub struct EditWorkflowTool {
    config: Arc<Config>,
}

impl EditWorkflowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for EditWorkflowTool {
    fn name(&self) -> &str {
        "edit_workflow"
    }

    fn description(&self) -> &str {
        "Iterate on a workflow with STRUCTURED EDITS instead of re-emitting the whole graph — the \
         cheap, low-regression path for changing a draft, saved, or inline flow. Provide the base \
         (draft_id for a working draft — the applied edit is written back to it; flow_id for a \
         saved flow; or an inline graph) plus ops[]: a list of edits applied in \
         order. Op shapes (each is { \"op\": <type>, ... }): add_node {node}, update_node_config \
         {id, config} (JSON merge-patch — a null value deletes that config key), set_node_name \
         {id, name}, rename_node {id, new_id} (rewires edges), remove_node {id} (drops its edges), \
         add_edge {edge}, remove_edge {from_node, to_node, from_port?, to_port?}, set_node_position \
         {id, position}. PERSISTENCE: the applied edit is written to a DRAFT, never onto the saved \
         flow — this tool NEVER saves. Editing a flow_id SEEDS A NEW DRAFT from that flow's graph \
         and returns its `draft_id`; editing a draft_id writes back to that same draft. The result \
         carries `draft_id`, `flow_id` (if any), `persisted: false`, and a `next` hint. To keep \
         iterating pass that `draft_id` (to edit_workflow / dry_run_workflow); to persist, call \
         save_workflow { flow_id, draft_id } when the user asks. If an op fails or the resulting \
         graph is invalid, the error names the failing op / node; fix it and call edit_workflow \
         again."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "draft_id": {
                    "type": "string",
                    "description": "A working draft to edit as the base; the applied edit is written back to it. Provide one of draft_id / flow_id / graph."
                },
                "flow_id": {
                    "type": "string",
                    "description": "The saved flow to edit as the base graph. Provide one of draft_id / flow_id / graph."
                },
                "graph": {
                    "type": "object",
                    "description": "An inline base tinyflows WorkflowGraph to edit. Provide one of draft_id / flow_id / graph.",
                    "properties": {
                        "nodes": { "type": "array" },
                        "edges": { "type": "array" }
                    }
                },
                "ops": {
                    "type": "array",
                    "description": "The structured edits, applied in order. Each item is { op, ... } — see the tool description for op shapes.",
                    "items": { "type": "object", "properties": { "op": { "type": "string" } }, "required": ["op"] },
                    "minItems": 1
                },
                "name": {
                    "type": "string",
                    "description": "Name for the resulting proposed flow. Defaults to the base flow's name."
                },
                "instruction": {
                    "type": "string",
                    "description": "The change that motivated these ops (echoed back on the review card)."
                },
                "require_approval": {
                    "type": "boolean",
                    "description": "Force a human-approval gate on every outbound action once saved. Defaults to true."
                }
            },
            "required": ["ops"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Pure validation, no side effect — mirrors propose/revise_workflow.
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Resolve the base graph + a default name from exactly one of: a draft
        // (the shared working copy — edits are written back to it), a saved
        // flow, or an inline graph.
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

        // The applied edit is always written back to a durable DRAFT (the shared
        // working copy across turns/reloads). `write_back_draft` is the draft id
        // it lands on; `edited_from_flow` is the saved flow this edit derives
        // from / would persist onto, if any. The core WS2 fix: editing a bare
        // `flow_id` used to persist NOTHING and return NO handle — the edit was
        // unreachable and read as "written onto the flow". Now a `flow_id` base
        // seeds a NEW draft, so the edit is durable, addressable, and clearly
        // NOT the saved flow.
        let mut write_back_draft: Option<String> = None;
        let mut edited_from_flow: Option<String> = None;

        let (base_graph, default_name) = match (draft_id, flow_id, inline_graph) {
            (Some(id), _, _) => match ops::flows_draft_get(&self.config, id) {
                Ok(outcome) => {
                    let draft = outcome.value;
                    match ops::migrate_and_deserialize_graph(draft.graph.clone()) {
                        Ok(graph) => {
                            write_back_draft = Some(draft.id.clone());
                            // A draft may already be linked to a saved flow —
                            // carry that through so the proposal echoes it.
                            edited_from_flow = draft.flow_id.clone();
                            (graph, draft.name)
                        }
                        Err(e) => {
                            return Ok(ToolResult::error(format!(
                                "Draft '{id}' holds a graph that could not be parsed: {e}."
                            )));
                        }
                    }
                }
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Could not load draft '{id}' to edit: {e}"
                    )));
                }
            },
            (None, Some(id), _) => match ops::flows_get(&self.config, id).await {
                Ok(outcome) => {
                    let flow = outcome.value;
                    // Seed a NEW draft from the saved flow's graph so the edit is
                    // durable and reachable (the RPC/canvas path uses the same
                    // `flows_draft_create` op). Linking the draft to `flow.id`
                    // means a later save_workflow { flow_id, draft_id } knows its
                    // target.
                    let graph_json = match serde_json::to_value(&flow.graph) {
                        Ok(v) => v,
                        Err(e) => {
                            return Ok(ToolResult::error(format!(
                                "Could not serialize flow '{id}' to seed a draft: {e}"
                            )));
                        }
                    };
                    match ops::flows_draft_create(
                        &self.config,
                        Some(flow.id.clone()),
                        flow.name.clone(),
                        graph_json,
                        crate::openhuman::flows::DraftOrigin::Chat,
                    ) {
                        Ok(created) => {
                            let new_draft_id = created.value.id.clone();
                            tracing::debug!(
                                target: "flows",
                                draft_id = %new_draft_id,
                                flow_id = %flow.id,
                                "[flows] edit_workflow: seeded a new draft from saved flow (edits live on the draft, NOT the flow)"
                            );
                            write_back_draft = Some(new_draft_id);
                            edited_from_flow = Some(flow.id.clone());
                            (flow.graph, flow.name)
                        }
                        Err(e) => {
                            return Ok(ToolResult::error(format!(
                                "Could not create a draft to edit flow '{id}': {e}"
                            )));
                        }
                    }
                }
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Could not load flow '{id}' to edit: {e}"
                    )));
                }
            },
            (None, None, Some(graph_json)) => {
                match ops::migrate_and_deserialize_graph(graph_json.clone()) {
                    Ok(graph) => {
                        let name = graph.name.clone();
                        (graph, name)
                    }
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "The inline base `graph` could not be parsed: {e}."
                        )));
                    }
                }
            }
            (None, None, None) => {
                return Ok(ToolResult::error(
                    "Provide one of `draft_id` (a working draft), `flow_id` (a saved flow), or \
                     `graph` (an inline base graph) to edit."
                        .to_string(),
                ));
            }
        };

        // Parse the ops list element-by-element so a bad op reports its index,
        // its `op` tag, the serde error, AND the expected field shape for THAT
        // op type — instead of a bare aggregate "missing field `id`" that names
        // neither the failing op nor what it wanted (audit WS4).
        let ops_array = match args.get("ops") {
            Some(Value::Array(items)) => items.clone(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing 'ops' parameter (a non-empty array of structured edits).".to_string(),
                ));
            }
        };
        if ops_array.is_empty() {
            return Ok(ToolResult::error(
                "`ops` is empty — provide at least one edit.".to_string(),
            ));
        }
        let mut graph_ops: Vec<tinyflows::graph_ops::GraphOp> = Vec::with_capacity(ops_array.len());
        for (index, item) in ops_array.into_iter().enumerate() {
            let op_tag = item.get("op").and_then(Value::as_str).map(str::to_string);
            match serde_json::from_value::<tinyflows::graph_ops::GraphOp>(item) {
                Ok(op) => graph_ops.push(op),
                Err(e) => {
                    let shape = match op_tag.as_deref() {
                        Some(tag) => match edit_op_shape(tag) {
                            Some(shape) => format!("op `{tag}` expects {shape}"),
                            None => {
                                format!("unknown op type `{tag}` — valid types: {VALID_OP_TYPES}")
                            }
                        },
                        None => format!("missing `op` field — valid types: {VALID_OP_TYPES}"),
                    };
                    tracing::debug!(target: "flows", index, ?op_tag, error = %e, "[flows] edit_workflow: op failed to parse");
                    return Ok(ToolResult::error(format!(
                        "Could not parse op {index}: {e}. Expected {shape}. Each op is \
                         {{ \"op\": <type>, ... }}. Fix the ops and call edit_workflow again."
                    )));
                }
            }
        }

        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or(default_name);
        let name = if name.is_empty() {
            "Untitled workflow".to_string()
        } else {
            name
        };
        let instruction = args
            .get("instruction")
            .and_then(Value::as_str)
            .map(str::to_string);
        let require_approval = args
            .get("require_approval")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        tracing::debug!(
            target: "flows",
            %name,
            op_count = graph_ops.len(),
            from_flow = flow_id.is_some(),
            "[flows] edit_workflow: applying structured ops to base graph"
        );

        // Apply the ops (structural mutation, precise per-op errors).
        let edited = match tinyflows::graph_ops::apply_ops(&base_graph, &graph_ops) {
            Ok(graph) => graph,
            Err(e) => {
                tracing::debug!(target: "flows", %name, error = %e, "[flows] edit_workflow: an op failed to apply");
                // Ops apply strictly in array order, so an add_node for an id
                // that already exists is almost always an ordering mistake
                // (adding before removing the old node). Point at the fix — this
                // is the exact 2nd wasted call the WS4 audit caught.
                let hint = match (e.op, &e.kind) {
                    ("add_node", tinyflows::graph_ops::GraphOpErrorKind::NodeIdExists(id)) => {
                        format!(
                            "\n\nOps apply strictly in array order. To replace node `{id}`, put a \
                             remove_node op for it BEFORE the add_node, or use update_node_config \
                             to patch it in place."
                        )
                    }
                    _ => String::new(),
                };
                return Ok(ToolResult::error(format!(
                    "{e}{hint}\n\nFix the ops and call edit_workflow again."
                )));
            }
        };

        // Write the applied edit back to the draft (the durable working copy),
        // so it survives across turns/reloads even if validation/gates below
        // still flag something to fix next.
        if let Some(ref draft_id) = write_back_draft {
            let edited_json = serde_json::to_value(&edited)?;
            if let Err(e) = ops::flows_draft_update(
                &self.config,
                draft_id,
                Some(name.clone()),
                Some(edited_json),
                None,
            ) {
                tracing::warn!(target: "flows", %draft_id, error = %e, "[flows] edit_workflow: could not write edit back to draft");
            }
        }

        // Structural validation of the RESULT — surface every problem at once.
        let structural = tinyflows::validate::validate_all(&edited);
        if !structural.is_empty() {
            let messages: Vec<String> = structural.iter().map(ToString::to_string).collect();
            tracing::debug!(
                target: "flows",
                %name,
                error_count = messages.len(),
                "[flows] edit_workflow: the edited graph is structurally invalid"
            );
            return Ok(ToolResult::error(format!(
                "The edited graph is invalid:\n\n{}\n\nFix the ops and call edit_workflow again.",
                messages.join("\n")
            )));
        }

        // Full builder hard-gate stack + proposal payload (shared with revise).
        // Thread the persistence-state handles so the payload carries draft_id /
        // flow_id / persisted:false and can't be misread as a save.
        match ops::build_builder_proposal(
            &self.config,
            "edit_workflow",
            &name,
            &edited,
            require_approval,
            true,
            instruction,
            write_back_draft.clone(),
            edited_from_flow.clone(),
        )
        .await
        {
            Ok(mut payload) => {
                // A prominent, one-line pointer at where the edit actually lives
                // (the draft) vs. where it does NOT (the saved flow) — the exact
                // confusion the WS2 audit caught. Only meaningful when the edit
                // landed on a draft (inline-graph edits have no durable handle).
                if let Some(draft_id) = write_back_draft.as_deref() {
                    let next = match edited_from_flow.as_deref() {
                        Some(flow_id) => format!(
                            "Edits live on draft {draft_id}, NOT on flow {flow_id}. Iterate with \
                             edit_workflow/dry_run_workflow {{ draft_id: \"{draft_id}\" }}, then \
                             persist with save_workflow {{ flow_id: \"{flow_id}\", draft_id: \
                             \"{draft_id}\" }} when the user asks."
                        ),
                        None => format!(
                            "Edits live on draft {draft_id} (not yet linked to a saved flow). \
                             Iterate with edit_workflow/dry_run_workflow {{ draft_id: \
                             \"{draft_id}\" }}, then persist with create_workflow, or save_workflow \
                             {{ flow_id, draft_id: \"{draft_id}\" }} once a flow exists."
                        ),
                    };
                    payload["next"] = json!(next);
                }
                Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?))
            }
            Err(message) => {
                tracing::debug!(target: "flows", %name, "[flows] edit_workflow: a hard gate rejected the edited graph");
                Ok(ToolResult::error(message))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// validate_workflow — standalone check without proposing (F3)
// ─────────────────────────────────────────────────────────────────────────────

/// `validate_workflow`: run the SAME structural validation + hard-gate stack
/// the propose/revise/edit/save tools use, but WITHOUT emitting a proposal —
/// a pure check so the agent can verify a draft (or a saved flow) mid-build.
///
/// Returns a structured report `{ ok, structurally_valid, errors[],
/// error_details[], gate_errors[], warnings[] }`, so a failing check is
/// fix-and-retry rather than a proposal the user has to reject.
pub struct ValidateWorkflowTool {
    config: Arc<Config>,
}

impl ValidateWorkflowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

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
        let gate_errors = if validation.valid {
            match ops::migrate_and_deserialize_graph(graph_json) {
                Ok(graph) => ops::run_builder_gates(&self.config, &graph).await,
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        let ok = validation.valid && gate_errors.is_empty();
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
         Stops a runaway or stuck run; fires no new outbound effect. Params: { run_id }."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "The run (thread) id to cancel." }
            },
            "required": ["run_id"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let run_id = match args.get("run_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'run_id' parameter".to_string())),
        };
        tracing::debug!(target: "flows", %run_id, "[flows] cancel_flow_run: cancelling run");
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
        // graph that flows_create would otherwise create enabled.
        if flow.enabled {
            if let Err(e) = ops::flows_set_enabled(&self.config, &flow.id, false).await {
                tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] create_workflow: could not force-disable the new flow");
            }
        }

        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "type": "workflow_created",
            "flow_id": flow.id,
            "name": flow.name,
            "enabled": false,
            "require_approval": flow.require_approval,
            "note": "Flow created DISABLED. The user must enable it explicitly before it can run.",
        }))?))
    }
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

#[async_trait]
impl Tool for DuplicateFlowTool {
    fn name(&self) -> &str {
        "duplicate_flow"
    }

    fn description(&self) -> &str {
        "Duplicate a saved flow: create an independent, DISABLED copy of its graph under a new id \
         (name suffixed \" (copy)\"). The copy never fires until the user enables it. Use this for \
         the clone-then-edit pattern (edit_workflow the copy). Params: { flow_id }."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "flow_id": { "type": "string", "description": "The saved flow to duplicate." } },
            "required": ["flow_id"],
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
        tracing::info!(target: "flows", %flow_id, "[flows] duplicate_flow: agent-initiated duplicate");
        match ops::flows_duplicate(&self.config, &flow_id).await {
            Ok(outcome) => {
                let flow = outcome.value;
                Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "type": "workflow_duplicated",
                    "flow_id": flow.id,
                    "name": flow.name,
                    "enabled": flow.enabled,
                }))?))
            }
            Err(e) => Ok(ToolResult::error(format!("Could not duplicate flow: {e}"))),
        }
    }
}

/// `list_connectable_toolkits`: read-only list of the Composio toolkits the
/// builder can wire, each tagged connected/unconnected — so the agent can steer
/// toolkit choice toward what's already connected (audit Phase 5, item 19).
pub struct ListConnectableToolkitsTool {
    config: Arc<Config>,
}

impl ListConnectableToolkitsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ListConnectableToolkitsTool {
    fn name(&self) -> &str {
        "list_connectable_toolkits"
    }

    fn description(&self) -> &str {
        "List the Composio toolkits available to wire into a tool_call/app_event, each flagged \
         `connected: true/false`. Read-only. Use it to prefer an ALREADY-connected toolkit when \
         several would work, and to tell the user which toolkits a proposed flow still needs \
         connecting. Returns a JSON array of { toolkit, connected }."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        use crate::openhuman::memory_sync::composio::providers::agent_ready_toolkits;
        tracing::debug!(target: "flows", "[flows] list_connectable_toolkits: listing toolkits + connected state (read-only)");
        let connected = ops::connected_toolkits(&self.config).await;
        let toolkits: Vec<Value> = agent_ready_toolkits()
            .into_iter()
            .map(|tk| {
                let tk_lc = tk.to_ascii_lowercase();
                json!({ "toolkit": tk_lc, "connected": connected.contains(&tk_lc) })
            })
            .collect();
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &json!({ "toolkits": toolkits }),
        )?))
    }
}

/// Extracts a string array from `args[key]`, ignoring non-strings; empty when
/// absent. Shared by the resume tool's approve/reject lists.
fn string_array(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// list_flows — read-only: saved flow summaries
// ─────────────────────────────────────────────────────────────────────────────

/// `list_flows`: read-only listing of saved flows (id / name / enabled /
/// last_status) so the builder can reference, clone, or avoid duplicating an
/// existing automation.
pub struct ListFlowsTool {
    config: Arc<Config>,
}

impl ListFlowsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ListFlowsTool {
    fn name(&self) -> &str {
        "list_flows"
    }

    fn description(&self) -> &str {
        "List the user's saved automation flows (tinyflows workflows). Read-only. \
         Returns a JSON array of { id, name, enabled, last_status, last_run_at } so \
         you can reference an existing flow, clone its structure (fetch the full \
         graph with get_flow), or avoid proposing a duplicate."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(target: "flows", "[flows] list_flows: listing saved flows (read-only)");
        match ops::flows_list(&self.config).await {
            Ok(outcome) => {
                let flows: Vec<Value> = outcome
                    .value
                    .iter()
                    .map(|f| {
                        json!({
                            "id": f.id,
                            "name": f.name,
                            "enabled": f.enabled,
                            "last_status": f.last_status,
                            "last_run_at": f.last_run_at,
                        })
                    })
                    .collect();
                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &json!({ "flows": flows }),
                )?))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to list flows: {e}"))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_flow — read-only: a saved flow's graph
// ─────────────────────────────────────────────────────────────────────────────

/// `get_flow`: read-only fetch of a saved flow's full [`WorkflowGraph`] by id,
/// so the builder can clone or extend an existing automation.
pub struct GetFlowTool {
    config: Arc<Config>,
}

impl GetFlowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for GetFlowTool {
    fn name(&self) -> &str {
        "get_flow"
    }

    fn description(&self) -> &str {
        "Fetch a saved flow's full tinyflows WorkflowGraph (nodes + edges) plus \
         its metadata by id. Read-only. Use it to clone or extend an existing \
         automation — pass the returned graph (possibly modified) to \
         revise_workflow or dry_run_workflow."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The saved flow's id (from list_flows)." }
            },
            "required": ["id"],
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
        let id = match args.get("id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'id' parameter".to_string())),
        };
        tracing::debug!(target: "flows", flow_id = %id, "[flows] get_flow: fetching saved flow (read-only)");
        match ops::flows_get(&self.config, &id).await {
            Ok(outcome) => {
                let f = outcome.value;
                let graph = serde_json::to_value(&f.graph)?;
                Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "id": f.id,
                    "name": f.name,
                    "enabled": f.enabled,
                    "require_approval": f.require_approval,
                    "last_status": f.last_status,
                    "graph": graph,
                }))?))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to get flow '{id}': {e}"))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_flow_run — read-only: a run's steps (for repair/debugging)
// ─────────────────────────────────────────────────────────────────────────────

/// `get_flow_run`: read-only fetch of a single flow run's step records, so the
/// builder can diagnose a failure and propose a repair.
pub struct GetFlowRunTool {
    config: Arc<Config>,
}

impl GetFlowRunTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for GetFlowRunTool {
    fn name(&self) -> &str {
        "get_flow_run"
    }

    fn description(&self) -> &str {
        "Fetch a single flow run's record by run id: status, per-node step \
         results, any pending approvals, and the error (if it failed). Read-only. \
         Use it to debug a failing flow from an error report and propose a repair."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "The run id (also the run's thread_id)." }
            },
            "required": ["run_id"],
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
        let run_id = match args.get("run_id").and_then(Value::as_str).map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(ToolResult::error("Missing 'run_id' parameter".to_string())),
        };
        tracing::debug!(target: "flows", %run_id, "[flows] get_flow_run: fetching run record (read-only)");
        match ops::flows_get_run(&self.config, &run_id).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &outcome.value,
            )?)),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to get flow run '{run_id}': {e}"
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_flow_connections — read-only: connection refs (ids/names only)
// ─────────────────────────────────────────────────────────────────────────────

/// `list_flow_connections`: read-only enumeration of the connection sources a
/// node's `connection_ref` can attach to (Composio connected accounts +
/// named HTTP credentials) — non-secret metadata only (ids / display labels
/// / kind / toolkit / scheme / platform_user_id), never secrets.
pub struct ListFlowConnectionsTool {
    config: Arc<Config>,
}

impl ListFlowConnectionsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ListFlowConnectionsTool {
    fn name(&self) -> &str {
        "list_flow_connections"
    }

    fn description(&self) -> &str {
        "List the connection sources a flow node's `connection_ref` can attach to: \
         Composio connected accounts and named HTTP credentials. Read-only; \
         returns only non-secret metadata — ids, display labels, kind, and \
         `toolkit`/`scheme` (never any secret). Each \
         Composio entry also carries `platform_user_id` — the connected \
         account's own member id (e.g. Slack `U123ABC`) — use it to wire a \
         self-targeted action like 'DM me' to that account instead of a \
         public channel. Use the `connection_ref` values verbatim on \
         tool_call / http_request nodes so the generated flow carries valid \
         connections."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(target: "flows", "[flows] list_flow_connections: enumerating connection refs (read-only)");
        match ops::flows_list_connections(&self.config).await {
            Ok(outcome) => {
                let conns: Vec<Value> = outcome.value.iter().map(flow_connection_to_json).collect();
                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &json!({ "connections": conns }),
                )?))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to list flow connections: {e}"
            ))),
        }
    }
}

/// Render one [`crate::openhuman::flows::types::FlowConnection`] as the
/// picker JSON shape the agent reads — ids/display/kind/toolkit/scheme plus
/// `platform_user_id` (the connected account's own member id, e.g. Slack
/// `U123ABC`, or `null` when no identity has synced yet). Never secret
/// material. A free function (rather than inline in `execute`) so the
/// mapping is unit-testable without a live Composio backend.
fn flow_connection_to_json(c: &crate::openhuman::flows::types::FlowConnection) -> Value {
    json!({
        "connection_ref": c.connection_ref,
        "kind": c.kind,
        "display": c.display,
        "toolkit": c.toolkit,
        "scheme": c.scheme,
        "platform_user_id": c.platform_user_id,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// search_tool_catalog — read-only: real Composio tool slugs from the FULL
// LIVE catalog (systemic tool-contract fix, Part 1)
// ─────────────────────────────────────────────────────────────────────────────

/// `search_tool_catalog`: search the FULL LIVE Composio catalog — every real
/// action for a named app, connected or not, curated or not — so `tool_call`
/// nodes are grounded in slugs that actually exist (rather than a hallucinated
/// slug that fails the save-time [`crate::openhuman::flows::ops::validate_tool_contracts`]
/// gate).
///
/// Also grounds the OUTPUT side: each result carries the action's real
/// `output_fields` (top-level response field names) and — when known — a
/// `primary_array_path`, so a downstream binding
/// (`=nodes.<id>.item.json.<field>`) or a `split_out.path` can be wired to a
/// real field/path instead of a guessed one. Call
/// [`GetToolContractTool`]/`get_tool_contract` for the FULL contract (schemas
/// included) before wiring a match's args.
pub struct SearchToolCatalogTool {
    config: Arc<Config>,
}

impl SearchToolCatalogTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

/// Cap on returned matches so a broad query can't flood the agent's context.
const MAX_CATALOG_RESULTS: usize = 40;

/// Search the FULL LIVE Composio catalog (via
/// [`crate::openhuman::tinyflows::caps::fetch_live_toolkit_catalog`]) for
/// actions whose slug or description matches every whitespace-separated term
/// in `query` (case-insensitive AND). When `toolkit` is set, only that
/// toolkit is scanned — this is how the builder can search ANY named app
/// (connected or not) rather than only the toolkits already
/// [`agent_ready_toolkits`](crate::openhuman::memory_sync::composio::providers::agent_ready_toolkits);
/// with no `toolkit` filter, the search is scoped to that agent-ready set (a
/// bare keyword query with no app named would otherwise have to fan out to
/// every toolkit Composio knows about).
///
/// Curated matches (`is_curated`) are ranked first (a stable sort, so ties
/// preserve fetch order) — never filtered out; a real, uncurated action is
/// just as valid a result, only ranked after the curated ones. A toolkit
/// whose live-catalog fetch fails (no backend session, network error)
/// contributes zero results rather than erroring the whole search.
pub(crate) async fn search_live_catalog(
    config: &Config,
    query: &str,
    toolkit_filter: Option<&str>,
    limit: usize,
) -> Vec<Value> {
    search_catalog(config, query, toolkit_filter, limit)
        .await
        .results
}

/// Cap on fallback (per-keyword) matches — a near-miss query must not flood the
/// agent's context with the whole toolkit, so the OR-scored fallback returns at
/// most this many rows regardless of the primary `limit`.
const MAX_FALLBACK_RESULTS: usize = 10;

/// Outcome of a catalog search: the shaped rows, whether the per-keyword
/// fallback pass fired, and an optional advisory `note` the tool surfaces so an
/// agent never misreads a keyword miss as "the action doesn't exist".
pub(crate) struct CatalogSearchOutcome {
    pub results: Vec<Value>,
    /// True when the per-token OR fallback pass ran (primary AND match was
    /// empty for a multi-word query).
    pub fallback: bool,
    /// Advisory note explaining a near-miss / keyword-based search, if any.
    pub note: Option<String>,
}

/// Shape one live-catalog [`ToolContract`](crate::openhuman::tinyflows::caps::ToolContract)
/// into a search-result row. The SINGLE row-construction site shared by both
/// the primary AND-match path and the per-keyword fallback path, so every row
/// carries the same fields — including WS3's `runtime_gated: true` on an
/// uncurated action of a toolkit that ships a curated-only allowlist.
fn shape_catalog_row(
    tool: &crate::openhuman::tinyflows::caps::ToolContract,
    toolkit: &str,
    toolkit_curated: bool,
) -> Value {
    let mut row = json!({
        "slug": tool.slug,
        "toolkit": toolkit,
        "description": tool.description,
        "required_args": tool.required_args,
        "output_fields": tool.output_fields,
        "primary_array_path": tool.primary_array_path,
        "featured": tool.is_curated,
    });
    // Compact: only present when true.
    if !tool.is_curated && toolkit_curated {
        if let Some(obj) = row.as_object_mut() {
            obj.insert("runtime_gated".to_string(), Value::Bool(true));
        }
    }
    row
}

/// Search the FULL LIVE Composio catalog and return a [`CatalogSearchOutcome`].
///
/// Primary pass: case-insensitive AND — an action matches only if EVERY
/// whitespace-separated term substring-matches its slug, toolkit name, or
/// description (curated matches ranked first, stable sort preserves fetch
/// order). When that yields zero rows for a MULTI-WORD query, a per-keyword OR
/// fallback runs: each action is scored by how many query tokens match its
/// slug/toolkit/description, and the top [`MAX_FALLBACK_RESULTS`] (ranked by
/// hit-count desc, then curated first) are returned with an advisory `note`.
/// This is what keeps a natural-language query like "twitter tweet replies
/// lookup" from returning a bare `count: 0` even though `TWITTER_*` actions
/// exist — the agent gets the nearest keyword matches instead of falsely
/// concluding the action is missing.
pub(crate) async fn search_catalog(
    config: &Config,
    query: &str,
    toolkit_filter: Option<&str>,
    limit: usize,
) -> CatalogSearchOutcome {
    use crate::openhuman::memory_sync::composio::providers::agent_ready_toolkits;
    use crate::openhuman::tinyflows::caps::fetch_live_toolkit_catalog;

    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();

    let toolkits: Vec<String> = match toolkit_filter {
        Some(tk) if !tk.trim().is_empty() => vec![tk.trim().to_ascii_lowercase()],
        _ => agent_ready_toolkits()
            .into_iter()
            .map(str::to_string)
            .collect(),
    };

    // Fetch every candidate toolkit's live catalog concurrently — a bare
    // keyword query (no `toolkit` filter) fans out across every agent-ready
    // toolkit, and fetching them one at a time would pay for each one's
    // round trip back-to-back (the per-toolkit cache only helps repeats).
    let fetched: Vec<(
        String,
        Option<Vec<crate::openhuman::tinyflows::caps::ToolContract>>,
    )> = futures::future::join_all(toolkits.into_iter().map(|toolkit| async move {
        let catalog = fetch_live_toolkit_catalog(config, &toolkit).await;
        (toolkit, catalog)
    }))
    .await;

    // Drop toolkits whose fetch failed (no backend session / network error) —
    // they contribute zero results rather than erroring the whole search.
    let fetched: Vec<(String, Vec<crate::openhuman::tinyflows::caps::ToolContract>)> = fetched
        .into_iter()
        .filter_map(|(tk, catalog)| catalog.map(|c| (tk, c)))
        .collect();

    // Does the scanned scope hold ANY actions at all? Distinguishes "keyword
    // miss" (has actions, none matched) from "nothing to search" (empty scope).
    let any_actions = fetched.iter().any(|(_, catalog)| !catalog.is_empty());

    // ── Primary pass: case-insensitive AND across every term ──
    let mut matches: Vec<(bool, Value)> = Vec::new();
    for (toolkit, catalog) in &fetched {
        // WS3 — a toolkit that ships a curated catalog is a hard curated-only
        // allowlist at RUNTIME, so any `featured: false` action of it is
        // rejected on every real run. Compute once per toolkit and flag those
        // rows so the blocker is visible at search time (transcript failure #2).
        let toolkit_curated = ops::toolkit_has_curated_catalog(toolkit);
        for tool in catalog {
            let slug_lc = tool.slug.to_ascii_lowercase();
            let desc_lc = tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_match = terms.iter().all(|term| {
                slug_lc.contains(term) || toolkit.contains(term) || desc_lc.contains(term)
            });
            if !is_match {
                continue;
            }
            matches.push((
                tool.is_curated,
                shape_catalog_row(tool, toolkit, toolkit_curated),
            ));
        }
    }

    // Curated (`featured`) results first; stable sort preserves fetch order
    // within each group.
    matches.sort_by_key(|(is_curated, _)| std::cmp::Reverse(*is_curated));
    matches.truncate(limit);
    let primary: Vec<Value> = matches.into_iter().map(|(_, v)| v).collect();

    if !primary.is_empty() {
        return CatalogSearchOutcome {
            results: primary,
            fallback: false,
            note: None,
        };
    }

    // ── Zero primary hits ──
    // Single-token queries keep today's behavior exactly; only attach a light
    // advisory note so a lone keyword miss still explains the search is
    // keyword-based (task WS5.4, optional).
    if terms.len() <= 1 {
        let note = if any_actions {
            Some(format!(
                "No actions matched '{query}'. This search is keyword-based (matches action \
                 slug/name/description) — try a different single keyword (e.g. 'gmail' or \
                 'tweets')."
            ))
        } else {
            None
        };
        return CatalogSearchOutcome {
            results: Vec::new(),
            fallback: false,
            note,
        };
    }

    // ── Fallback pass (multi-word, zero primary hits): per-token OR scoring ──
    // Score each action by how many DISTINCT query tokens match its
    // slug/toolkit/description; keep the primary path's curated boost as the
    // tiebreak. Rows go through the SAME `shape_catalog_row` path as primary.
    let mut scored: Vec<(usize, bool, Value)> = Vec::new();
    for (toolkit, catalog) in &fetched {
        let toolkit_curated = ops::toolkit_has_curated_catalog(toolkit);
        for tool in catalog {
            let slug_lc = tool.slug.to_ascii_lowercase();
            let desc_lc = tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let hits = terms
                .iter()
                .filter(|term| {
                    slug_lc.contains(*term) || toolkit.contains(*term) || desc_lc.contains(*term)
                })
                .count();
            if hits == 0 {
                continue;
            }
            scored.push((
                hits,
                tool.is_curated,
                shape_catalog_row(tool, toolkit, toolkit_curated),
            ));
        }
    }

    // Most keyword hits first, then curated first; stable sort preserves fetch
    // order within a (hits, curated) group.
    scored.sort_by_key(|(hits, is_curated, _)| std::cmp::Reverse((*hits, *is_curated)));
    scored.truncate(limit.min(MAX_FALLBACK_RESULTS));
    let results: Vec<Value> = scored.into_iter().map(|(_, _, v)| v).collect();

    tracing::debug!(
        target: "flows",
        query,
        fallback = true,
        hits = results.len(),
        "[flows] search_tool_catalog: primary AND-match empty for a multi-word query — ran per-keyword OR fallback"
    );

    if results.is_empty() {
        // Literally zero tokens matched anything: no rows, but a note so the
        // agent doesn't read `count: 0` as "action doesn't exist" (task WS5.3).
        return CatalogSearchOutcome {
            results,
            fallback: true,
            note: Some(format!(
                "No actions matched any keyword in '{query}'. This search is keyword-based \
                 (matches action slug/name/description) — retry with a single keyword (e.g. one \
                 word like 'gmail' or 'tweets') for a full listing."
            )),
        };
    }

    CatalogSearchOutcome {
        results,
        fallback: true,
        note: Some(format!(
            "No exact match for '{query}'. Showing the nearest per-keyword matches — retry with a \
             single keyword (e.g. one word like 'gmail' or 'tweets') for a full listing."
        )),
    }
}

#[async_trait]
impl Tool for SearchToolCatalogTool {
    fn name(&self) -> &str {
        "search_tool_catalog"
    }

    fn description(&self) -> &str {
        "Search the FULL LIVE Composio catalog for REAL action slugs to use on `tool_call` \
         nodes — every action for a named app, whether or not the user has connected it yet \
         and whether or not it's one of OpenHuman's hand-curated actions. Read-only. Query by \
         keyword (e.g. 'send email', 'slack message'); optionally scope to one `toolkit` (e.g. \
         'gmail', or any Composio app name) to search that app specifically. Returns matching \
         { slug, toolkit, description, required_args, output_fields, primary_array_path, \
         featured } entries, curated (`featured: true`) matches ranked first. ALWAYS ground a \
         tool_call node's `slug` in a real result here — never invent one. Before wiring a \
         match's args or a downstream binding, call get_tool_contract { slug } for the FULL \
         contract (exact required_args, full input/output JSON Schema) — this search result is \
         enough to FIND the right slug, get_tool_contract is what grounds the WIRING. If the \
         app isn't connected yet, you can still build the node and use composio_connect (or \
         tell the user) — the flow will prompt for the connection at run time."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to match against tool slugs/descriptions (case-insensitive). All terms must match for an exact hit; a multi-word query with no exact match falls back to the nearest per-keyword matches. For the widest listing, prefer ONE keyword (e.g. 'gmail' or 'tweets')."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Optional toolkit/app slug to scope the search (e.g. 'gmail', 'slack', or any named Composio app — connected or not)."
                }
            },
            "required": ["query"],
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
        let query = match args.get("query").and_then(Value::as_str).map(str::trim) {
            Some(q) if !q.is_empty() => q.to_string(),
            _ => return Ok(ToolResult::error("Missing 'query' parameter".to_string())),
        };
        let toolkit = args.get("toolkit").and_then(Value::as_str);
        tracing::debug!(
            target: "flows",
            %query,
            toolkit = toolkit.unwrap_or("(any)"),
            "[flows] search_tool_catalog: searching the FULL LIVE Composio catalog (read-only)"
        );
        let outcome = search_catalog(&self.config, &query, toolkit, MAX_CATALOG_RESULTS).await;
        // Build with `note` first so an agent reading top-down sees the
        // near-miss / keyword-based advisory before the (possibly zero) rows.
        // `count` is always the number of returned rows, never a stand-in for
        // "no such action" — a fallback carries a non-zero count.
        let mut obj = serde_json::Map::new();
        if let Some(note) = outcome.note {
            obj.insert("note".to_string(), Value::String(note));
        }
        obj.insert("query".to_string(), Value::String(query));
        obj.insert(
            "count".to_string(),
            Value::Number(outcome.results.len().into()),
        );
        obj.insert("results".to_string(), Value::Array(outcome.results));
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &Value::Object(obj),
        )?))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_tool_contract — read-only: the FULL live contract for one action slug
// ─────────────────────────────────────────────────────────────────────────────

/// `get_tool_contract`: fetch the FULL live [`ToolContract`](crate::openhuman::tinyflows::caps::ToolContract)
/// for one Composio action slug — the grounding step the builder MUST take
/// before wiring a `search_tool_catalog` match's args or a downstream
/// binding/`split_out.path` off it. Where `search_tool_catalog` is for
/// FINDING a real slug, this is for WIRING it correctly: exact
/// `required_args` (wire every one), the full `input_schema`/`output_schema`,
/// and `primary_array_path` (prefixed `json.` for a `split_out.path`).
pub struct GetToolContractTool {
    config: Arc<Config>,
}

impl GetToolContractTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for GetToolContractTool {
    fn name(&self) -> &str {
        "get_tool_contract"
    }

    fn description(&self) -> &str {
        "Fetch the FULL live contract for one Composio action slug (found via \
         search_tool_catalog) before wiring it into a tool_call node. Read-only. Returns { \
         slug, toolkit, description, required_args, input_schema, output_fields, \
         output_schema, primary_array_path, is_curated }. Use `required_args` for EVERY arg \
         you must wire in config.args; use `output_fields` for a downstream \
         `=nodes.<id>.item.json.data.<field>` binding — note the `data.` segment: a Composio \
         tool_call's real runtime output wraps its payload in `data` \
         (`ComposioExecuteResponse`), so `output_fields` names fields INSIDE that wrapper, not \
         top-level envelope keys — never guess a field name, and never drop the `data.` \
         segment (`.item.json.<field>` with no `data.` resolves null even when `<field>` is a \
         real output field). Use `primary_array_path` (prefixed with `json.`, e.g. \
         \"json.data.messages\" — the `data.` segment is already baked into the value) verbatim \
         as a downstream split_out.path when you need to fan out over this action's result \
         list. Call this for every real slug right before you wire its args — \
         search_tool_catalog's summary is enough to find the slug, this is what grounds the \
         wiring."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "The exact Composio action slug, e.g. 'GMAIL_SEND_EMAIL' (from search_tool_catalog)."
                }
            },
            "required": ["slug"],
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
        let slug = match args.get("slug").and_then(Value::as_str).map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return Ok(ToolResult::error("Missing 'slug' parameter".to_string())),
        };
        let Some(toolkit) =
            crate::openhuman::memory_sync::composio::providers::toolkit_from_slug(&slug)
        else {
            return Ok(ToolResult::error(format!(
                "Could not extract a toolkit from slug '{slug}' — it must look like \
                 '<TOOLKIT>_<ACTION>' (e.g. 'GMAIL_SEND_EMAIL')."
            )));
        };

        tracing::debug!(
            target: "flows",
            %slug,
            %toolkit,
            "[flows] get_tool_contract: fetching the live contract (read-only)"
        );

        let Some(catalog) =
            crate::openhuman::tinyflows::caps::fetch_live_toolkit_catalog(&self.config, &toolkit)
                .await
        else {
            return Ok(ToolResult::error(format!(
                "Could not fetch the live Composio catalog for toolkit '{toolkit}' (no backend \
                 session, or a transient failure) — try again, or use search_tool_catalog to \
                 confirm the toolkit is reachable."
            )));
        };

        match catalog.iter().find(|c| c.slug.eq_ignore_ascii_case(&slug)) {
            Some(contract) => {
                // B12: a prior real-output probe (get_tool_output_sample) for
                // this exact slug is ACTUAL observed data and always wins
                // over the schema-derived hint — most relevant for an action
                // whose live listing publishes no output schema at all (e.g.
                // every GitHub action verified live as of this fix), where
                // `contract.primary_array_path` would otherwise be
                // permanently `None`.
                let contract =
                    crate::openhuman::tinyflows::caps::apply_probe_override(contract.clone());

                // WS3 — EARLY runtime-gate warning (transcript failure #2): a
                // real-but-uncurated action of a toolkit that ships a curated
                // catalog is a hard curated-only allowlist at RUNTIME, so it is
                // REJECTED on every real run. The late `validate_workflow` gate
                // catches it, but only ~15 tool calls after the agent has built
                // and wired the node. Surface the blocker HERE, at contract-fetch
                // time (and first in the payload), so the agent never wires it.
                if !contract.is_curated && ops::toolkit_has_curated_catalog(&toolkit) {
                    tracing::debug!(
                        target: "flows",
                        %slug,
                        %toolkit,
                        "[flows] get_tool_contract: uncurated action of a curated toolkit — attaching runtime_gate warning"
                    );
                    #[derive(serde::Serialize)]
                    struct ContractWithRuntimeGate {
                        runtime_gate: &'static str,
                        #[serde(flatten)]
                        contract: crate::openhuman::tinyflows::caps::ToolContract,
                    }
                    let payload = ContractWithRuntimeGate {
                        runtime_gate: "This action will be REJECTED on every real run — the \
                                       runtime tool gate only allows curated actions for this \
                                       toolkit. Pick a `featured: true` result from \
                                       search_tool_catalog instead.",
                        contract,
                    };
                    return Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?));
                }

                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &contract,
                )?))
            }
            None => Ok(ToolResult::error(format!(
                "'{slug}' is not a real action in the '{toolkit}' toolkit's live catalog — use \
                 search_tool_catalog to find a real slug."
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_tool_output_sample — READ-ONLY real Composio call: the B12 output probe
// ─────────────────────────────────────────────────────────────────────────────

/// `get_tool_output_sample`: make ONE bounded, READ-ONLY, REAL Composio call
/// for `slug` and derive its `primary_array_path`/`output_fields` from the
/// ACTUAL response, overriding `get_tool_contract`'s schema-derived hint for
/// this slug from then on (see
/// [`crate::openhuman::tinyflows::caps::apply_probe_override`]).
///
/// **Exists because a schema-derived hint sometimes doesn't exist at all**:
/// Composio's live listing genuinely omits `output_parameters` for some
/// actions — verified live for every GitHub action, including the curated
/// `GITHUB_LIST_REPOSITORY_ISSUES` — leaving `get_tool_contract`'s
/// `primary_array_path` permanently `null`. Without ground truth the builder
/// has been observed guessing the whole-payload `"json.data"` as a
/// `split_out.path` (live flow "funny reminders v2": one item — the
/// `{issues:[...]}` container itself — instead of the real per-item list),
/// silently degrading a fan-out to a single item.
///
/// **This is a deliberate, narrow carve-out of the workflow-builder agent's
/// "propose/read only, no composio_execute" invariant** (see this module's
/// top doc): unlike `composio_execute`, this tool can ONLY ever perform a
/// `Read`-scope action (gated by
/// [`crate::openhuman::tinyflows::caps::probe_tool_output_sample`]'s scope
/// check, which ignores the user's per-toolkit scope preference — a probe
/// must never perform a real mutation no matter what the user has toggled
/// on) against a toolkit the user has ALREADY connected. No message is sent,
/// no record created/updated/deleted, ever.
///
/// Pass the SAME `args` you intend to wire into the real `tool_call` node —
/// this samples THAT call, not a generic fixture. Omit `args` (or pass `{}`)
/// for a zero-required-arg action.
pub struct GetToolOutputSampleTool {
    config: Arc<Config>,
}

impl GetToolOutputSampleTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for GetToolOutputSampleTool {
    fn name(&self) -> &str {
        "get_tool_output_sample"
    }

    fn description(&self) -> &str {
        "Make ONE bounded, READ-ONLY, REAL call to a Composio action and derive its real \
         `primary_array_path`/`output_fields` from the ACTUAL response — use this when \
         get_tool_contract returns `output_schema: null` / `primary_array_path: null` for a \
         source tool you plan to `split_out` (e.g. every GitHub action, verified live), so a \
         downstream split_out.path never fans out over the whole-payload container by mistake. \
         Only ever performs a Read action (refuses Write/Admin actions unconditionally, \
         regardless of the user's scope preference) against an ALREADY-CONNECTED toolkit — never \
         sends, creates, updates, or deletes anything. Pass the SAME args you intend to wire into \
         the real tool_call node — this samples THAT exact call. Call get_tool_contract again \
         afterward (or trust this tool's own `primary_array_path`/`output_fields`) to see the \
         override applied. Real actions only, not `oh:` or `=`-derived slugs."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "The exact Composio action slug, e.g. 'GITHUB_LIST_REPOSITORY_ISSUES'."
                },
                "args": {
                    "type": "object",
                    "description": "Arguments for the real call — the SAME ones you intend to wire into the tool_call node (e.g. {\"owner\": \"acme\", \"repo\": \"widgets\"}). Omit for a zero-required-arg action."
                }
            },
            "required": ["slug"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let slug = match args.get("slug").and_then(Value::as_str).map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return Ok(ToolResult::error("Missing 'slug' parameter".to_string())),
        };
        let call_args = args.get("args").cloned().unwrap_or(json!({}));

        tracing::debug!(
            target: "flows",
            %slug,
            "[flows] get_tool_output_sample: tool invoked"
        );

        match crate::openhuman::tinyflows::caps::probe_tool_output_sample(
            &self.config,
            &slug,
            call_args,
        )
        .await
        {
            Ok(sample) => {
                let primary_array_path_for_split_out = sample
                    .primary_array_path
                    .as_ref()
                    .map(|p| format!("json.{p}"));
                Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "slug": slug,
                    "primary_array_path": sample.primary_array_path,
                    "split_out_path": primary_array_path_for_split_out,
                    "output_fields": sample.output_fields,
                }))?))
            }
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_agent_profiles — read-only: selectable agent kinds for an `agent` node
// ─────────────────────────────────────────────────────────────────────────────

/// `list_agent_profiles`: read-only listing of the agent **kinds** an `agent`
/// node can select via `agent_ref` (researcher, code_executor, crypto_agent, …).
///
/// Grounds the builder's `agent_ref` choice in real registry ids — the agent
/// analogue of `search_tool_catalog` for `tool_call` slugs — so it never
/// hallucinates an agent kind. Returns `{ id, name, description, model, tools,
/// tags }` for every enabled registered agent.
pub struct ListAgentProfilesTool;

impl ListAgentProfilesTool {
    /// Builds the tool (no configuration — reads the process-global registry).
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListAgentProfilesTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListAgentProfilesTool {
    fn name(&self) -> &str {
        "list_agent_profiles"
    }

    fn description(&self) -> &str {
        "List the agent KINDS an `agent` node can run via its `agent_ref` config \
         field (e.g. researcher, code_executor, crypto_agent). Read-only. Returns \
         a JSON array of { id, name, description, model, tools, tags }. Use this to \
         pick a real agent_ref — a coding step should reference the coding agent, a \
         research step the researcher — instead of guessing an id. Note: an \
         agent_ref applies that agent's persona/model to the step; its private \
         tool loop is a follow-up, so a step still gets tools from the node's own \
         inline `tools` list for now."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(target: "flows", "[flows] list_agent_profiles: listing registered agent kinds (read-only)");
        match crate::openhuman::agent_registry::list_agents(false).await {
            Ok(agents) => {
                let profiles: Vec<Value> = agents
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id,
                            "name": a.name,
                            "description": a.description,
                            "model": a.model,
                            "tools": a.tool_allowlist,
                            "tags": a.tags,
                        })
                    })
                    .collect();
                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &json!({ "agent_profiles": profiles }),
                )?))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to list agent profiles: {e}"
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_node_kinds / get_node_kind_contract — queryable DSL schema (F2)
// ─────────────────────────────────────────────────────────────────────────────

/// `list_node_kinds`: enumerate the 12 tinyflows node kinds with a one-line
/// summary each. The DSL counterpart of `search_tool_catalog` for Composio
/// actions — a cheap first call to orient before fetching a full contract.
pub struct ListNodeKindsTool;

impl ListNodeKindsTool {
    /// Builds the tool (no configuration — the contracts are static).
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListNodeKindsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListNodeKindsTool {
    fn name(&self) -> &str {
        "list_node_kinds"
    }

    fn description(&self) -> &str {
        "List the 12 tinyflows node kinds you can put in a WorkflowGraph, each with a one-line \
         summary and its config field names. Read-only, no args. Returns a JSON array of { kind, \
         summary, required_config, optional_config }. Call get_node_kind_contract { kind } for the \
         full config-field shapes, ports, an example node, and authoring gotchas of any one kind — \
         this is the machine-readable DSL schema, so you don't have to rely on prose or memory."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(target: "flows", "[flows] list_node_kinds: enumerating node kinds (read-only)");
        let kinds: Vec<Value> = crate::openhuman::flows::all_node_kind_contracts()
            .iter()
            .map(|c| {
                let required: Vec<&str> = c
                    .config_fields
                    .iter()
                    .filter(|f| f.required)
                    .map(|f| f.name.as_str())
                    .collect();
                let optional: Vec<&str> = c
                    .config_fields
                    .iter()
                    .filter(|f| !f.required)
                    .map(|f| f.name.as_str())
                    .collect();
                json!({
                    "kind": c.kind,
                    "summary": c.summary,
                    "required_config": required,
                    "optional_config": optional,
                })
            })
            .collect();
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &json!({ "node_kinds": kinds }),
        )?))
    }
}

/// `get_node_kind_contract`: the FULL machine-readable contract for one node
/// kind — config fields (name/required/type/description/enum), ports, a valid
/// example node, and the authoring gotchas. Mirrors `get_tool_contract` for
/// Composio actions but for the DSL itself.
pub struct GetNodeKindContractTool;

impl GetNodeKindContractTool {
    /// Builds the tool (no configuration — the contracts are static).
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for GetNodeKindContractTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GetNodeKindContractTool {
    fn name(&self) -> &str {
        "get_node_kind_contract"
    }

    fn description(&self) -> &str {
        "Fetch the FULL contract for ONE tinyflows node kind before you author a node of that \
         kind. Read-only. Returns { kind, summary, description, config_fields:[{name, required, \
         value_type, description, enum_values?}], ports:{inputs, outputs}, example, notes }. Use \
         config_fields for exactly what to put in config, ports for how to wire branch edges (the \
         branch label goes on the edge's from_port), and notes for the envelope/gotcha rules that \
         otherwise silently resolve to null. Find the kind names via list_node_kinds."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "description": "One of the 12 node kinds, e.g. 'tool_call' (from list_node_kinds).",
                    "enum": crate::openhuman::flows::NODE_KINDS,
                }
            },
            "required": ["kind"],
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
        let kind = match args.get("kind").and_then(Value::as_str).map(str::trim) {
            Some(k) if !k.is_empty() => k.to_string(),
            _ => return Ok(ToolResult::error("Missing 'kind' parameter".to_string())),
        };
        tracing::debug!(target: "flows", %kind, "[flows] get_node_kind_contract: fetching contract (read-only)");
        match crate::openhuman::flows::node_kind_contract(&kind) {
            Some(contract) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &contract,
            )?)),
            None => Ok(ToolResult::error(format!(
                "'{kind}' is not a tinyflows node kind — call list_node_kinds for the 12 valid \
                 kinds."
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// dry_run_workflow — execute a DRAFT against MOCK capabilities (tier-gated)
// ─────────────────────────────────────────────────────────────────────────────

/// `dry_run_workflow`: compile a **draft** graph and run it against tinyflows'
/// deterministic **mock** capabilities, returning the merged node-state output
/// so the builder can self-verify a proposal before presenting it.
///
/// **No real side effects:** the run is wired to
/// [`tinyflows::caps::mock::mock_capabilities`] — the LLM / tool / HTTP / code
/// capabilities are echo stubs, so nothing external ever fires regardless of
/// the graph. The output is explicitly labeled `sandbox: true`.
///
/// Autonomy-tier gated (issue: Phase 2 node gating): read-only tier refuses,
/// mirroring the `SecurityPolicy` contract that a read-only session cannot
/// exercise executable capability even in simulation.
///
/// **Wiring preflight:** the mock tool invoker is wrapped in the host's
/// [`PreflightToolInvoker`](crate::openhuman::tinyflows::caps::PreflightToolInvoker),
/// so a Composio `tool_call` whose required arg is missing or `=`-resolved to
/// null fails the dry run with the same actionable, field-naming error a real
/// run would produce — the echo mocks alone would happily accept a null `to`.
///
/// **Null-resolution check (the "produces functionally-broken workflows" fix):**
/// a required arg can be present *and non-Composio* (a native `oh:` tool, or a
/// Composio arg the catalog has no cached schema for) and still be wired to a
/// `=`-expression that silently resolves to `null` — the preflight above only
/// catches a *missing/null Composio-required* arg, so a graph like that used to
/// dry-run green and then do nothing at runtime. The run is driven through
/// [`tinyflows::engine::run_with_observer`] with a [`CapturingObserver`] that
/// records every node's [`ExecutionStep::diagnostics`](tinyflows::observability::ExecutionStep)
/// — the `=`-expressions the vendored engine itself traced as null-resolved
/// (see `tinyflows::expr::resolve_traced`). After the run settles, every
/// diagnostic on a **`tool_call` node's `args.*` location** is collected; any
/// hit fails the dry run with `ok: false` and the offending
/// `{ node_id, location, expression }` list, rather than reporting `ok: true`
/// for a graph that would silently no-op. Diagnostics on any OTHER
/// `agent`-node config subfield are NOT fatal here — a null there degrades
/// output quality but doesn't break execution the way a null tool arg does.
///
/// **Agent-prompt null check:** the ONE `agent`-node diagnostic that IS fatal
/// is a null-resolved **`prompt` itself** (`location == "prompt"`) — `prompt`
/// is the node's only input channel to the completion, so a `null` there
/// means the agent runs with a completely EMPTY prompt (the root-cause bug
/// `config.input_context` and `ops::validate_binding_resolvability`'s static
/// gate both exist to prevent). Collected separately into
/// `agent_prompt_nulls` (`{ node_id, location, expression, suggestion }`) and
/// added to the same `ok: false` condition as `null_resolutions`.
///
/// **Agent-`input_context` null check:** the SAME treatment applies to a
/// null-resolved **`input_context`** (`location == "input_context"`) — since
/// #4590 this is the agent's primary upstream-data channel (the very field
/// `prompt`-embedded jq expressions were supposed to stop needing), so a
/// `null` here is just as execution-breaking as a null `prompt`: the agent
/// runs with no upstream data at all. Collected separately into
/// `agent_input_context_nulls` (`{ node_id, location, expression, suggestion }`,
/// mirroring `agent_prompt_nulls` exactly) and added to the same `ok: false`
/// condition as `null_resolutions`/`agent_prompt_nulls`.
///
/// **`on_error: continue`/`route` does not mask a `tool_call` failure either.**
/// Those policies convert an executor error (e.g. the required-arg preflight
/// rejecting a null arg) into a routed error ITEM so the *run* still completes
/// (`Ok(outcome)`) — the failing node's `ExecutionStep` carries an EMPTY
/// `diagnostics` (the null check above would miss it) but its `status` is
/// [`StepStatus::Error`](tinyflows::observability::StepStatus::Error). Every
/// such `tool_call` step is collected into `node_errors`
/// (`{ node_id, error }`, the error text read back out of the run's `output`
/// state — see [`tool_call_error_message`]) and fails the dry run the same as
/// a null resolution.
///
/// **Routing-divergence warning (B15's dry-run blind spot):** none of the
/// checks above see a node that never ran at all. An `agent`/`tool_call` node
/// downstream of a `condition` can be silently unexercised because the
/// sandbox's mock trigger payload has a different *shape* than a real
/// trigger's (e.g. a webhook's real JSON body vs. the dry run's `{}`
/// default), so the condition takes a different branch under mock data than
/// it would at runtime — a graph can dry-run `ok: true` while its most
/// data-dependent node was never actually checked. After the run settles,
/// every `agent`/`tool_call` node with no [`ExecutionStep`] in the
/// [`CapturingObserver`] is collected into `routing_divergence_warnings`
/// (`{ node_id, condition_node_id, message }`, `condition_node_id` naming the
/// nearest upstream `condition` node found by walking predecessors — see
/// [`find_upstream_condition`] — or `null` if none is found). This is a
/// **warning, not a hard reject**: it never flips `ok` to `false` by itself
/// (an unexercised branch can be entirely intentional), and is surfaced on
/// both the `ok: true` and `ok: false` result shapes so the caller can
/// double-check that node's wiring by hand.
/// Builds one `null_resolutions` diagnostic entry for a `tool_call` node's
/// null-resolved `args.*` config expression.
///
/// The common case reports `{ node_id, location, expression }` — a wiring
/// mistake the agent should fix. But when the null-resolved expression binds to
/// the output of an upstream Composio `tool_call` node
/// ([`ops::composio_tool_call_upstream_ref`]), the entry is instead marked
/// `unverifiable: true` and carries an honest `suggestion`: the echo sandbox
/// can NEVER produce a Composio tool's real output fields, so this particular
/// null is expected here and does NOT prove the binding wrong (WS6 — the
/// transcript audit where the agent re-wired an already-correct binding three
/// times chasing this exact false negative). The message points at
/// `get_tool_contract` / `get_tool_output_sample` as the real disambiguators.
fn build_null_resolution_entry(
    node_id: &str,
    diag: &tinyflows::expr::NullResolution,
    graph: &WorkflowGraph,
) -> Value {
    if let Some(upstream) = crate::openhuman::flows::ops::composio_tool_call_upstream_ref(
        &diag.expression,
        graph,
        node_id,
    ) {
        let field = diag.location.strip_prefix("args.").unwrap_or("args");
        return json!({
            "node_id": node_id,
            "location": diag.location,
            "expression": diag.expression,
            "unverifiable": true,
            "upstream_tool_call": upstream,
            "suggestion": format!(
                "required arg `{field}` binds to the output of Composio tool_call node \
                 `{upstream}` — the SANDBOX only echoes tool calls and can never produce \
                 their real output fields, so this binding is UNVERIFIABLE here (not \
                 necessarily wrong). Confirm the path against get_tool_contract {{ slug }}'s \
                 output_fields / primary_array_path (remember Composio results nest under \
                 `.item.json.data.`), or get_tool_output_sample {{ slug, args }} for the \
                 real shape. It is a real bug only if the path doesn't match the action's \
                 actual output."
            ),
        });
    }
    json!({
        "node_id": node_id,
        "location": diag.location,
        "expression": diag.expression,
    })
}

/// Every null-resolved `args.*` config expression that landed on a `tool_call`
/// node, as `null_resolutions` diagnostic entries (see
/// [`build_null_resolution_entry`] for the shape, including the WS6
/// `unverifiable` Composio-upstream variant). Shared by the settled-run path
/// (which fails the dry run on these) and the errored-run path (which surfaces
/// only the `unverifiable` ones so a stop-policy preflight abort explains
/// itself honestly instead of via the generic required-arg text).
fn tool_call_arg_null_entries(
    steps: &[tinyflows::observability::ExecutionStep],
    graph: &WorkflowGraph,
    tool_call_node_ids: &std::collections::HashSet<&str>,
) -> Vec<Value> {
    steps
        .iter()
        .filter(|step| tool_call_node_ids.contains(step.node_id.as_str()))
        .flat_map(|step| {
            step.diagnostics
                .iter()
                .filter(|&diag| diag.location == "args" || diag.location.starts_with("args."))
                .map(|diag| build_null_resolution_entry(&step.node_id, diag, graph))
        })
        .collect()
}

pub struct DryRunWorkflowTool {
    security: Arc<SecurityPolicy>,
    config: Arc<Config>,
}

impl DryRunWorkflowTool {
    pub fn new(security: Arc<SecurityPolicy>, config: Arc<Config>) -> Self {
        Self { security, config }
    }
}

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
            crate::openhuman::tinyflows::caps::SchemaAwareMockAgentRunner,
        );
        // Plain agent nodes (no `agent_ref`) never reach the runner above —
        // the vendored `agent` node routes them to the `llm` slot instead (see
        // `SchemaAwareMockLlm`'s doc). Swap the vendored `MockLlm` echo for the
        // schema-aware mock so their `output_parser.schema` is honored too,
        // instead of the echo shape failing the sub-port's validation.
        caps.llm = std::sync::Arc::new(crate::openhuman::tinyflows::caps::SchemaAwareMockLlm);
        // Wiring preflight over the echo mocks (see the struct doc): required
        // Composio args must be present and non-null even in the sandbox.
        caps.tools = std::sync::Arc::new(crate::openhuman::tinyflows::caps::PreflightToolInvoker {
            config: self.config.clone(),
            inner: caps.tools.clone(),
        });

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

        tracing::info!(
            target: "flows",
            node_count = graph.nodes.len(),
            pending_approvals = outcome.pending_approvals.len(),
            null_resolution_count = null_resolutions.len(),
            agent_prompt_null_count = agent_prompt_nulls.len(),
            agent_input_context_null_count = agent_input_context_nulls.len(),
            node_error_count = node_errors.len(),
            routing_divergence_warning_count = routing_divergence_warnings.len(),
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

/// A [`tinyflows::observability::RunObserver`] that captures every finished
/// node's [`ExecutionStep`](tinyflows::observability::ExecutionStep) — in
/// particular its `diagnostics` (null-resolved `=`-expressions the engine
/// traced during that node's config resolution) — so [`DryRunWorkflowTool`]
/// can inspect them once the sandbox run settles. See the struct's "Null-
/// resolution check" doc for why this exists.
/// `pub(crate)` (not private) so [`crate::openhuman::flows::ops::validate_required_arg_resolvability`]
/// (issue B18 — escalating a null-resolved REQUIRED outbound arg to a hard
/// authoring-time reject) can run the identical sandbox-capture shape without
/// duplicating this struct.
#[derive(Default)]
pub(crate) struct CapturingObserver {
    steps: std::sync::Mutex<Vec<tinyflows::observability::ExecutionStep>>,
}

impl tinyflows::observability::RunObserver for CapturingObserver {
    fn on_step_finish(&self, step: &tinyflows::observability::ExecutionStep) {
        self.steps
            .lock()
            .expect("CapturingObserver steps mutex poisoned")
            .push(step.clone());
    }
}

impl CapturingObserver {
    /// A snapshot of every step recorded so far (steps are pushed
    /// synchronously from `on_step_finish`, so once the run's future resolves
    /// every step it will ever record is already present).
    pub(crate) fn steps(&self) -> Vec<tinyflows::observability::ExecutionStep> {
        self.steps
            .lock()
            .expect("CapturingObserver steps mutex poisoned")
            .clone()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// save_workflow — persist a built graph onto an EXISTING saved flow
// ─────────────────────────────────────────────────────────────────────────────

/// `save_workflow`: persist a validated graph (and optionally a new name) onto
/// an **existing, already-saved** flow via [`ops::flows_update`] — the same
/// validate-and-migrate path the UI's Save uses.
///
/// This is the deliberate, narrow exception to the belt's original
/// "propose, never persist" invariant (added for the Flows prompt bar's
/// instant-create path, where the host creates the flow *before* delegating and
/// hands the agent its `flow_id`). The boundaries that remain:
///
/// - **Update-only.** It requires an existing `flow_id`; there is still no tool
///   to *create* a flow, so the agent can only write where the host (or user)
///   already made a flow.
/// - **Never touches enablement or the approval gate.** `enabled` and
///   `require_approval` are not parameters; whatever the user set stays.
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

#[cfg(test)]
#[path = "builder_tools_tests.rs"]
mod tests;
