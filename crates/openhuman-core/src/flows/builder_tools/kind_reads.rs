//! Read-only DSL/registry tools: agent definitions, node kinds, and node-kind contracts (F2).

use async_trait::async_trait;
use serde_json::{json, Value};

use tinytools::{PermissionLevel, Tool, ToolResult};

// ─────────────────────────────────────────────────────────────────────────────
// list_agent_definitions — read-only: selectable definitions for an `agent` node
// ─────────────────────────────────────────────────────────────────────────────

/// `list_agent_definitions`: read-only listing of the agent **definitions** an `agent`
/// node can select via `agent_ref` (agent_memory, vision_agent, planner, …).
///
/// Grounds the builder's `agent_ref` choice in real registry ids — the agent
/// analogue of `search_tool_catalog` for `tool_call` slugs — so it never
/// hallucinates an agent kind. Returns `{ id, name, description, model, tools,
/// tags }` for every enabled registered agent.
pub struct ListAgentDefinitionsTool;

impl ListAgentDefinitionsTool {
    /// Builds the tool (no configuration — reads the process-global registry).
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListAgentDefinitionsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListAgentDefinitionsTool {
    fn name(&self) -> &str {
        "list_agent_definitions"
    }

    fn description(&self) -> &str {
        "List the agent KINDS an `agent` node can run via its `agent_ref` config \
         field (e.g. agent_memory, vision_agent, presentation_agent). Read-only. Returns \
         a JSON array of { id, name, description, model, tools, tags }. Use this to \
         pick a real agent_ref — a memory-lookup step should reference agent_memory, \
         an image-reading step vision_agent — instead of guessing an id. Note: setting \
         agent_ref runs the step as a REAL agent turn (its own `run_single`), with \
         the selected specialist's full persona, model, tool loop, and iteration \
         cap — not just a persona-flavored completion. A plain `agent` node with \
         no agent_ref only gets the default LLM plus its own inline `tools` list; \
         it cannot run code, search the web, or use any specialist's tools."
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
        tracing::debug!(target: "flows", "[flows] list_agent_definitions: listing registered definitions (read-only)");
        match crate::agent::registry::list_agents(false).await {
            Ok(agents) => {
                let definitions: Vec<Value> = agents
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
                    &json!({ "agent_definitions": definitions }),
                )?))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to list agent definitions: {e}"
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_node_kinds / get_node_kind_contract — queryable DSL schema (F2)
// ─────────────────────────────────────────────────────────────────────────────

/// `list_node_kinds`: enumerate the 14 tinyflows node kinds with a one-line
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
        let kinds: Vec<Value> = crate::flows::all_node_kind_contracts()
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
                        tinyflows::catalog::NODE_KINDS.len()
                    ),
                    "enum": tinyflows::catalog::NODE_KINDS,
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
        match crate::flows::node_kind_contract(&kind) {
            Some(contract) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &contract,
            )?)),
            None => Ok(ToolResult::error(format!(
                "'{kind}' is not a tinyflows node kind — call list_node_kinds for the {} valid \
                 kinds.",
                tinyflows::catalog::NODE_KINDS.len()
            ))),
        }
    }
}
