use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::model::WorkflowGraph;

use crate::openhuman::config::Config;
use crate::openhuman::flows::ops;
use crate::openhuman::flows::ops::validate_and_migrate_graph;
use crate::openhuman::flows::tools;
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
         {id, name}, rename_node {id, new_id} (rewires EDGES onto the new id, but does NOT rewrite \
         `=nodes.<old_id>...` binding expressions inside OTHER nodes' config — re-point those \
         yourself, or validate_workflow will catch the dangling reference), remove_node {id} \
         (drops its edges), \
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

        // T-m6: returns `Err` (rather than only `warn!`-logging) when the
        // draft write-back itself fails, so callers can surface the failure
        // instead of telling the agent "Edits live on draft {id}" when the
        // draft still holds the PREVIOUS graph.
        let write_edit_to_draft = || -> Result<(), String> {
            if let Some(ref draft_id) = write_back_draft {
                let edited_json = serde_json::to_value(&edited).map_err(|e| e.to_string())?;
                if let Err(e) = ops::flows_draft_update(
                    &self.config,
                    draft_id,
                    Some(name.clone()),
                    Some(edited_json),
                    None,
                ) {
                    tracing::warn!(target: "flows", %draft_id, error = %e, "[flows] edit_workflow: could not write edit back to draft");
                    return Err(e);
                }
            }
            Ok(())
        };

        // Structural validation of the RESULT — surface every problem at once.
        let structural = tinyflows::validate::validate_all(&edited);
        if !structural.is_empty() {
            // Preserve the longstanding working-copy contract: an applied edit
            // survives for the next repair turn even when structurally invalid.
            // T-m6: surface (not just log) a write-back failure here too, so the
            // agent knows the draft may still hold the PREVIOUS graph rather than
            // this attempted (invalid) edit.
            let write_back_note = match write_edit_to_draft() {
                Ok(()) => String::new(),
                Err(e) => format!(
                    "\n\nNote: the edit could also NOT be written back to the draft ({e}) — the \
                     draft still holds the PREVIOUS graph, not this attempted edit."
                ),
            };
            let messages: Vec<String> = structural.iter().map(ToString::to_string).collect();
            tracing::debug!(
                target: "flows",
                %name,
                error_count = messages.len(),
                "[flows] edit_workflow: the edited graph is structurally invalid"
            );
            return Ok(ToolResult::error(format!(
                "The edited graph is invalid:\n\n{}\n\nFix the ops and call edit_workflow again.{write_back_note}",
                messages.join("\n")
            )));
        }

        // Engine-incompatible topologies are different from ordinary builder
        // follow-up errors: persisting one would leave a draft that no current
        // save/run path can accept. Reject it before advancing the durable
        // working copy, while preserving the established write-back behavior
        // for later binding/connection/contract gates.
        let compatibility = ops::config_aware_engine_compatibility_errors(&self.config, &edited);
        if !compatibility.is_empty() {
            tracing::debug!(
                target: "flows",
                %name,
                error_count = compatibility.len(),
                "[flows] edit_workflow: the edited graph is engine-incompatible"
            );
            return Ok(ToolResult::error(format!(
                "The edited graph is incompatible with the current engine:\n\n{}\n\nFix the ops and call edit_workflow again.",
                compatibility.join("\n\n")
            )));
        }

        // Write the accepted structural edit back to the draft (the durable
        // working copy), so it survives across turns/reloads even if a later
        // binding/connection/contract gate flags something to fix next.
        //
        // T-m6: a failure here MUST short-circuit rather than fall through to
        // the proposal payload below — that payload's `next` text tells the
        // agent "Edits live on draft {id}", which would be false if the write
        // never landed, leaving the next turn silently iterating on a stale
        // draft.
        if let Some(draft_id) = write_back_draft.as_deref() {
            if let Err(e) = write_edit_to_draft() {
                tracing::warn!(
                    target: "flows",
                    %name,
                    %draft_id,
                    error = %e,
                    "[flows] edit_workflow: draft write-back failed after validation passed"
                );
                return Ok(ToolResult::error(format!(
                    "The edit passed validation, but could NOT be written back to draft \
                     {draft_id}: {e}\n\nThe draft still holds the PREVIOUS graph, not this edit. \
                     Retry edit_workflow."
                )));
            }
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
