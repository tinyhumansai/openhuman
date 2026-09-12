
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
        // The contract crate, not `memory::sync::composio::providers` (#5560).
        // That host shim is `pub use tinymemory_core::sync::composio::providers::*`
        // and the engine's `providers` module in turn re-exports this function
        // verbatim from `tinymemory_api::composio::scopes` — so the two paths
        // name the SAME item and this is a path change with no behaviour delta.
        // Naming the contract directly is what lets the shim's caller list
        // shrink to the sites that genuinely need the engine's registry and
        // curated catalogs.
        use tinymemory_api::composio::agent_ready_toolkits;
        tracing::debug!(target: "flows", "[flows] list_connectable_toolkits: listing toolkits + connected state (read-only) via the memory contract");
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
/// [`crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog`]) for
/// actions whose slug or description matches every whitespace-separated term
/// in `query` (case-insensitive AND). When `toolkit` is set, only that
/// toolkit is scanned — this is how the builder can search ANY named app
/// (connected or not) rather than only the toolkits already
/// `tinymemory_api::composio::agent_ready_toolkits`;
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

/// Shape one live-catalog [`ToolContract`](crate::openhuman::flows::tinyflows::caps::ToolContract)
/// into a search-result row. The SINGLE row-construction site shared by both
/// the primary AND-match path and the per-keyword fallback path, so every row
/// carries the same fields — including WS3's `runtime_gated: true` on an
/// uncurated action of a toolkit that ships a curated-only allowlist.
fn shape_catalog_row(
    tool: &crate::openhuman::flows::tinyflows::caps::ToolContract,
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
