
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
        "List the 14 tinyflows node kinds you can put in a WorkflowGraph, each with a one-line \
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
                    "description": format!(
                        "One of the {} node kinds, e.g. 'tool_call' (from list_node_kinds).",
                        crate::openhuman::flows::NODE_KINDS.len()
                    ),
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
                "'{kind}' is not a tinyflows node kind — call list_node_kinds for the {} valid \
                 kinds.",
                super::node_contracts::NODE_KINDS.len()
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// dry_run_workflow — execute a DRAFT against MOCK capabilities (ungated, F7)
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
/// **Not autonomy-tier gated (F7):** `permission_level()` returns
/// [`PermissionLevel::None`], so this tool runs on EVERY tier, read-only
/// included — a read-only agent must be able to self-verify its own proposal.
/// This is intentional, not an oversight: the mock capabilities never touch a
/// real integration, so there is nothing for a tier gate to protect. See
/// `dry_run_allowed_under_readonly_tier` in `builder_tools_tests.rs` for the
/// pinned regression (an earlier draft of this tool *was* tier-gated via an
/// unused `SecurityPolicy` field; the field was dead code by the time it
/// shipped and was removed rather than wired up, since side-effect-free
/// simulation has no tier to gate against).
///
/// **Wiring preflight:** the mock tool invoker is wrapped in the host's
/// [`PreflightToolInvoker`](crate::openhuman::flows::tinyflows::caps::PreflightToolInvoker),
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
/// the output of an upstream Composio-or-native `tool_call` node
/// ([`ops::mock_opaque_tool_call_upstream_ref`]), the entry is instead marked
/// `unverifiable: true` and carries an honest `suggestion`: the echo sandbox
/// can NEVER produce a tool's real output fields, so this particular null is
/// expected here and does NOT prove the binding wrong (WS6 — the transcript
/// audit where the agent re-wired an already-correct binding three times
/// chasing this exact false negative). The suggestion adapts to the upstream
/// kind: a Composio upstream points at `get_tool_contract` /
/// `get_tool_output_sample` and the `.item.json.data.` nesting; a native `oh:`
/// upstream points at the flat `.item.json.<field>` shape instead.
fn build_null_resolution_entry(
    node_id: &str,
    diag: &tinyflows::expr::NullResolution,
    graph: &WorkflowGraph,
) -> Value {
    if let Some(upstream) =
        tinyflows::preflight::mock_opaque_tool_call_upstream_ref(&diag.expression, graph, node_id)
    {
        let field = diag.location.strip_prefix("args.").unwrap_or("args");
        // The disambiguation advice differs by upstream kind: a native `oh:`
        // tool's output binds FLAT (`.item.json.<field>`) after
        // `native_tool_payload`'s unwrap — it has no `.data.` wrapper and no
        // Composio `get_tool_contract` — whereas a Composio action nests under
        // `.item.json.data.`. Emitting the Composio advice for a native
        // upstream would send the agent chasing a `.data.` path that will
        // never exist.
        let upstream_is_native = graph
            .nodes
            .iter()
            .find(|n| n.id == upstream)
            .and_then(|n| n.config.get("slug").and_then(Value::as_str))
            .is_some_and(|s| s.starts_with("oh:"));
        let suggestion = if upstream_is_native {
            format!(
                "required arg `{field}` binds to the output of native tool_call node \
                 `{upstream}` — the SANDBOX only echoes tool calls and can never produce \
                 their real output fields, so this binding is UNVERIFIABLE here (not \
                 necessarily wrong). A native `oh:` tool's real output binds FLAT at \
                 `=nodes.{upstream}.item.json.<field>` (no `.data.` wrapper). Confirm the \
                 field name against that tool's own output shape. It is a real bug only if \
                 the path doesn't match the tool's actual output."
            )
        } else {
            format!(
                "required arg `{field}` binds to the output of Composio tool_call node \
                 `{upstream}` — the SANDBOX only echoes tool calls and can never produce \
                 their real output fields, so this binding is UNVERIFIABLE here (not \
                 necessarily wrong). Confirm the path against get_tool_contract {{ slug }}'s \
                 output_fields / primary_array_path (remember Composio results nest under \
                 `.item.json.data.`), or get_tool_output_sample {{ slug, args }} for the \
                 real shape. It is a real bug only if the path doesn't match the action's \
                 actual output."
            )
        };
        return json!({
            "node_id": node_id,
            "location": diag.location,
            "expression": diag.expression,
            "unverifiable": true,
            "upstream_tool_call": upstream,
            "suggestion": suggestion,
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
/// `unverifiable` Composio-or-native-upstream variant). Shared by the settled-run path
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
    config: Arc<Config>,
}

impl DryRunWorkflowTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
