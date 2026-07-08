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
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Wall-clock bound on a single `dry_run_workflow` mock execution. A malformed
/// or pathological draft graph must never hang the agent tool-loop; the mock
/// capabilities are non-blocking echoes, so this is a generous safety net.
const DRY_RUN_TIMEOUT_SECS: u64 = 30;

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

        // Enforcing binding-resolvability gate (see
        // `ops::validate_binding_resolvability`): reject outright — rather
        // than merely warn — a `tool_call` binding that is guaranteed to
        // resolve null (or the wrong value) at runtime, so the builder must
        // fix the graph before the revision can even be proposed.
        let binding_errors = ops::validate_binding_resolvability(&graph);
        if !binding_errors.is_empty() {
            tracing::debug!(
                target: "flows",
                %name,
                error_count = binding_errors.len(),
                "[flows] revise_workflow: binding-resolvability check rejected the revised graph"
            );
            return Ok(ToolResult::error(format!(
                "{}\n\nFix these bindings and call revise_workflow again.",
                binding_errors.join("\n\n")
            )));
        }

        // Tool-contract enforcement gate (systemic tool-contract fix, Part 2):
        // reject a `tool_call` node whose slug isn't a REAL action in the
        // live Composio catalog, or whose real required args aren't all wired.
        let contract_errors = ops::validate_tool_contracts(&self.config, &graph).await;
        if !contract_errors.is_empty() {
            tracing::debug!(
                target: "flows",
                %name,
                error_count = contract_errors.len(),
                "[flows] revise_workflow: tool-contract check rejected the revised graph"
            );
            return Ok(ToolResult::error(format!(
                "{}\n\nFix these tool_call nodes and call revise_workflow again.",
                contract_errors.join("\n\n")
            )));
        }

        let summary = super::tools::build_summary(&graph);
        let mut warnings = ops::graph_trigger_warnings(&graph);
        // Author-time wiring check: unwired REQUIRED Composio args come back
        // as warnings naming the field, before the user ever saves.
        warnings.extend(ops::graph_wiring_warnings(&self.config, &graph).await);
        let graph_value = serde_json::to_value(&graph)?;

        tracing::info!(
            target: "flows",
            %name,
            node_count = graph.nodes.len(),
            require_approval,
            warning_count = warnings.len(),
            "[flows] revise_workflow: revised proposal ready for user review"
        );

        let mut payload = json!({
            "type": "workflow_proposal",
            "revision": true,
            "name": name,
            "graph": graph_value,
            "require_approval": require_approval,
            "summary": summary,
            "warnings": warnings,
        });
        if let Some(instruction) = instruction {
            payload["instruction"] = json!(instruction);
        }

        Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?))
    }
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
/// named HTTP credentials) — ids / display labels / kind only, never secrets.
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
         returns ids + display labels + kind ONLY (never any secret). Use the \
         `connection_ref` values verbatim on tool_call / http_request nodes so the \
         generated flow carries valid connections."
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
                let conns: Vec<Value> = outcome
                    .value
                    .iter()
                    .map(|c| {
                        json!({
                            "connection_ref": c.connection_ref,
                            "kind": c.kind,
                            "display": c.display,
                            "toolkit": c.toolkit,
                            "scheme": c.scheme,
                        })
                    })
                    .collect();
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

    let mut matches: Vec<(bool, Value)> = Vec::new();
    for (toolkit, catalog) in fetched {
        let Some(catalog) = catalog else { continue };
        for tool in &catalog {
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
                json!({
                    "slug": tool.slug,
                    "toolkit": toolkit,
                    "description": tool.description,
                    "required_args": tool.required_args,
                    "output_fields": tool.output_fields,
                    "primary_array_path": tool.primary_array_path,
                    "featured": tool.is_curated,
                }),
            ));
        }
    }

    // Curated (`featured`) results first; stable sort preserves fetch order
    // within each group.
    matches.sort_by_key(|(is_curated, _)| std::cmp::Reverse(*is_curated));
    matches.truncate(limit);
    matches.into_iter().map(|(_, v)| v).collect()
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
                    "description": "Keywords to match against tool slugs/descriptions (case-insensitive; all terms must match)."
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
        let results = search_live_catalog(&self.config, &query, toolkit, MAX_CATALOG_RESULTS).await;
        Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
            "query": query,
            "count": results.len(),
            "results": results,
        }))?))
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
        "Dry-run a DRAFT workflow graph in a SANDBOX to self-verify it before \
         proposing. Compiles the graph and executes it against MOCK capabilities \
         — every LLM / tool_call / http_request / code node returns a deterministic \
         echo, so NOTHING real happens (no messages sent, no code run). Returns the \
         simulated per-node output labeled as sandbox output. Use it to catch \
         wiring/routing mistakes; it does NOT prove real integrations work. Pass \
         the same graph shape as propose_workflow, plus an optional `input`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "graph": {
                    "type": "object",
                    "description": "The DRAFT tinyflows WorkflowGraph to simulate: { nodes: [...], edges: [...] }.",
                    "properties": {
                        "nodes": { "type": "array" },
                        "edges": { "type": "array" }
                    },
                    "required": ["nodes", "edges"]
                },
                "input": {
                    "description": "Optional trigger input passed to the run (defaults to {})."
                }
            },
            "required": ["graph"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Represents executable capability (a full sandbox could run code/http),
        // so it is gated like an execute-class tool even though the mock backend
        // means no real side effect can fire.
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        // Mock capabilities only — no real outbound effect. The `Execute`
        // permission above plus the read-only tier refusal below carry the gate.
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Autonomy-tier gate: a read-only session cannot dry-run (executable
        // capability, even simulated). Supervised / Full may.
        if self.security.autonomy == AutonomyLevel::ReadOnly {
            tracing::debug!(
                target: "flows",
                "[flows] dry_run_workflow: refused — autonomy tier is read-only"
            );
            return Ok(ToolResult::error(
                "dry_run_workflow requires at least 'supervised' autonomy — the current \
                 tier is read-only. Propose the workflow instead (propose_workflow), or \
                 raise autonomy in Settings → Agent access."
                    .to_string(),
            ));
        }

        let graph_json = match args.get("graph") {
            Some(v) if !v.is_null() => v.clone(),
            _ => return Ok(ToolResult::error("Missing 'graph' parameter".to_string())),
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
        let null_resolutions: Vec<Value> = observer
            .steps()
            .iter()
            .filter(|step| tool_call_node_ids.contains(step.node_id.as_str()))
            .flat_map(|step| {
                step.diagnostics.iter().filter_map(|diag| {
                    (diag.location == "args" || diag.location.starts_with("args.")).then(|| {
                        json!({
                            "node_id": step.node_id,
                            "location": diag.location,
                            "expression": diag.expression,
                        })
                    })
                })
            })
            .collect();

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
                step.diagnostics.iter().filter_map(|diag| {
                    (diag.location == "prompt").then(|| {
                        json!({
                            "node_id": step.node_id,
                            "location": diag.location,
                            "expression": diag.expression,
                            "suggestion": "Feed upstream data via input_context:\"=item\" and \
                                make the prompt a plain instruction.",
                        })
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
                step.diagnostics.iter().filter_map(|diag| {
                    (diag.location == "input_context").then(|| {
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
#[derive(Default)]
struct CapturingObserver {
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
    fn steps(&self) -> Vec<tinyflows::observability::ExecutionStep> {
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
         Use this after the user asked you to build/update a workflow and you have \
         dry-run-verified the graph: it validates and writes the graph (and optional new \
         `name`) to that flow. It can NOT create a new flow, and it never changes the \
         flow's enabled state or its approval gate. NOTE: if the flow is enabled and the \
         graph has a schedule/app_event trigger, saving arms it — it will start firing on \
         its own. Always tell the user what you saved. Params: { flow_id, graph, name? }."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": {
                    "type": "string",
                    "description": "Id of the EXISTING saved flow to write the graph to."
                },
                "graph": {
                    "type": "object",
                    "description": "The full tinyflows WorkflowGraph to persist: { name?, nodes: [...], edges: [...] }. Same shape as propose_workflow.",
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
            "required": ["flow_id", "graph"],
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
        let graph_json = match args.get("graph") {
            Some(v) if !v.is_null() => v.clone(),
            _ => return Ok(ToolResult::error("Missing 'graph' parameter".to_string())),
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
        let binding_errors = ops::validate_binding_resolvability(&graph);
        if !binding_errors.is_empty() {
            tracing::debug!(
                target: "flows",
                %flow_id,
                error_count = binding_errors.len(),
                "[flows] save_workflow: binding-resolvability check rejected the graph"
            );
            return Ok(ToolResult::error(format!(
                "{}\n\nFix these bindings and call save_workflow again.",
                binding_errors.join("\n\n")
            )));
        }
        // Tool-contract enforcement gate (systemic tool-contract fix, Part 2):
        // reject a `tool_call` node whose slug isn't a REAL action in the
        // live Composio catalog, or whose real required args aren't all
        // wired — before the graph is ever persisted.
        let contract_errors = ops::validate_tool_contracts(&self.config, &graph).await;
        if !contract_errors.is_empty() {
            tracing::debug!(
                target: "flows",
                %flow_id,
                error_count = contract_errors.len(),
                "[flows] save_workflow: tool-contract check rejected the graph"
            );
            return Ok(ToolResult::error(format!(
                "{}\n\nFix these tool_call nodes and call save_workflow again.",
                contract_errors.join("\n\n")
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

        match ops::flows_update(&self.config, &flow_id, name, Some(graph_json), None).await {
            Ok(outcome) => {
                let flow = outcome.value;
                tracing::info!(
                    target: "flows",
                    %flow_id,
                    node_count = flow.graph.nodes.len(),
                    enabled = flow.enabled,
                    "[flows] save_workflow: persisted"
                );
                Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "type": "workflow_saved",
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
