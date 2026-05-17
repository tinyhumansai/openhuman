use serde_json::{json, Map, Value};

use crate::core::all;
use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::providers::traits::build_tool_instructions_text;
use crate::openhuman::security::{SecurityPolicy, ToolOperation};

const DEFAULT_LIMIT: u64 = 10;
const MAX_LIMIT: u64 = 50;
const QUERY_ARGUMENTS: &[&str] = &["query", "k"];
const TREE_READ_CHUNK_ARGUMENTS: &[&str] = &["chunk_id"];
const SUBAGENT_RUN_ARGUMENTS: &[&str] = &["agent_id", "prompt"];

#[derive(Debug, Clone)]
pub struct McpToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub rpc_method: Option<&'static str>,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallError {
    /// Client-side problem: malformed arguments, unknown tool, validation
    /// failure. Maps to JSON-RPC `-32602 Invalid params`.
    InvalidParams(String),
    /// Server-side problem outside the caller's control: config load failure,
    /// missing platform resources. Maps to JSON-RPC `-32603 Internal error`.
    /// Kept distinct from `InvalidParams` so MCP clients don't display
    /// internal failures as if the user supplied bad arguments.
    Internal(String),
}

impl ToolCallError {
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidParams(message) | Self::Internal(message) => message,
        }
    }

    /// JSON-RPC error code corresponding to this variant.
    pub fn code(&self) -> i64 {
        match self {
            Self::InvalidParams(_) => -32602,
            Self::Internal(_) => -32603,
        }
    }

    /// JSON-RPC error `message` field (short, spec-canonical phrase). The
    /// human-readable detail belongs in the response's `data` field.
    pub fn jsonrpc_message(&self) -> &'static str {
        match self {
            Self::InvalidParams(_) => "Invalid params",
            Self::Internal(_) => "Internal error",
        }
    }
}

pub fn tool_specs() -> Vec<McpToolSpec> {
    vec![
        McpToolSpec {
            name: "core.list_tools",
            title: "List Core Tools",
            description: "List the live core agent tool catalog that OpenHuman exposes to its orchestrator session.",
            rpc_method: None,
            input_schema: no_args_schema(),
        },
        McpToolSpec {
            name: "core.tool_instructions",
            title: "Get Tool Instructions",
            description: "Emit the markdown tool-use instructions block that OpenHuman injects into prompt-guided agents.",
            rpc_method: None,
            input_schema: no_args_schema(),
        },
        McpToolSpec {
            name: "agent.list_subagents",
            title: "List Subagents",
            description: "List registered sub-agent definitions that the core can dispatch for specialized work.",
            rpc_method: None,
            input_schema: no_args_schema(),
        },
        McpToolSpec {
            name: "agent.run_subagent",
            title: "Run Subagent",
            description: "Run a registered OpenHuman sub-agent directly from the core and return its final response.",
            rpc_method: None,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Registered sub-agent id (for example `researcher`, `planner`, `code_executor`)."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Task prompt for the sub-agent. Include the context it needs because this is a fresh session."
                    }
                },
                "required": ["agent_id", "prompt"],
                "additionalProperties": false
            }),
        },
        McpToolSpec {
            name: "memory.search",
            title: "Search Memory",
            description: "Keyword-search OpenHuman's local memory tree and return matching chunks ordered by recency.",
            rpc_method: Some("openhuman.memory_tree_search"),
            input_schema: query_schema("Substring to match against stored memory chunks."),
        },
        McpToolSpec {
            name: "memory.recall",
            title: "Recall Memory",
            description: "Semantically recall local memory-tree chunks relevant to a natural-language query.",
            rpc_method: Some("openhuman.memory_tree_recall"),
            input_schema: query_schema("Natural-language query to embed and rerank against memory summaries."),
        },
        McpToolSpec {
            name: "tree.read_chunk",
            title: "Read Memory Chunk",
            description: "Read one memory-tree chunk by id. Use this to inspect the source text behind search or recall results.",
            rpc_method: Some("openhuman.memory_tree_get_chunk"),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "chunk_id": {
                        "type": "string",
                        "description": "Chunk id returned by memory.search or memory.recall."
                    }
                },
                "required": ["chunk_id"],
                "additionalProperties": false
            }),
        },
    ]
}

pub fn list_tools_result() -> Value {
    let tools = tool_specs()
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "title": tool.title,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    json!({ "tools": tools })
}

pub async fn call_tool(name: &str, arguments: Value) -> Result<Value, ToolCallError> {
    let spec = tool_specs()
        .into_iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| ToolCallError::InvalidParams(format!("unknown MCP tool `{name}`")))?;

    let params = build_rpc_params(spec.name, arguments)?;
    match spec.name {
        "core.list_tools" => {
            reject_unexpected_arguments(&params, &[])?;
            enforce_read_policy(spec.name).await?;
            return list_core_tools().await;
        }
        "core.tool_instructions" => {
            reject_unexpected_arguments(&params, &[])?;
            enforce_read_policy(spec.name).await?;
            return core_tool_instructions().await;
        }
        "agent.list_subagents" => {
            reject_unexpected_arguments(&params, &[])?;
            enforce_read_policy(spec.name).await?;
            return list_subagents().await;
        }
        "agent.run_subagent" => {
            enforce_act_policy(spec.name).await?;
            return run_subagent_tool(&params).await;
        }
        _ => {}
    }

    validate_controller_params(&spec, &params)?;
    enforce_read_policy(spec.name).await?;

    let rpc_method = spec.rpc_method.ok_or_else(|| {
        ToolCallError::Internal(format!(
            "MCP tool `{}` is missing its RPC mapping",
            spec.name
        ))
    })?;

    log::debug!(
        "[mcp_server] tools/call dispatch tool={} rpc_method={} arg_keys={:?}",
        spec.name,
        rpc_method,
        params.keys().collect::<Vec<_>>()
    );

    match all::try_invoke_registered_rpc(rpc_method, params).await {
        Some(Ok(value)) => {
            log::debug!("[mcp_server] tools/call success tool={}", spec.name);
            Ok(tool_success(value))
        }
        Some(Err(message)) => {
            log::warn!(
                "[mcp_server] tools/call handler error tool={} error={}",
                spec.name,
                message
            );
            Ok(tool_error(format!("{} failed: {message}", spec.name)))
        }
        None => {
            log::error!(
                "[mcp_server] tools/call mapping missing registered RPC method tool={} rpc_method={}",
                spec.name,
                rpc_method
            );
            Ok(tool_error(format!(
                "{} is unavailable: mapped RPC method `{}` is not registered",
                spec.name, rpc_method
            )))
        }
    }
}

fn no_args_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn query_schema(query_description: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": query_description,
                "minLength": 1
            },
            "k": {
                "type": "integer",
                "description": format!("Maximum chunks to return. Defaults to {DEFAULT_LIMIT}; capped at {MAX_LIMIT}."),
                "minimum": 1,
                "maximum": MAX_LIMIT
            }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

fn build_rpc_params(
    tool_name: &str,
    arguments: Value,
) -> Result<Map<String, Value>, ToolCallError> {
    let args = object_arguments(arguments)?;
    match tool_name {
        "core.list_tools" | "core.tool_instructions" | "agent.list_subagents" => {
            reject_unexpected_arguments(&args, &[])?;
            Ok(Map::new())
        }
        "agent.run_subagent" => {
            reject_unexpected_arguments(&args, SUBAGENT_RUN_ARGUMENTS)?;
            let agent_id = required_non_empty_string(&args, "agent_id")?;
            let prompt = required_non_empty_string(&args, "prompt")?;
            Ok(Map::from_iter([
                ("agent_id".to_string(), Value::String(agent_id)),
                ("prompt".to_string(), Value::String(prompt)),
            ]))
        }
        "memory.search" | "memory.recall" => {
            reject_unexpected_arguments(&args, QUERY_ARGUMENTS)?;
            let query = required_non_empty_string(&args, "query")?;
            let limit = optional_limit(&args)?;
            Ok(Map::from_iter([
                ("query".to_string(), Value::String(query)),
                ("k".to_string(), Value::from(limit)),
            ]))
        }
        "tree.read_chunk" => {
            reject_unexpected_arguments(&args, TREE_READ_CHUNK_ARGUMENTS)?;
            let chunk_id = required_non_empty_string(&args, "chunk_id")?;
            Ok(Map::from_iter([(
                "id".to_string(),
                Value::String(chunk_id),
            )]))
        }
        _ => Err(ToolCallError::InvalidParams(format!(
            "unknown MCP tool `{tool_name}`"
        ))),
    }
}

fn reject_unexpected_arguments(
    args: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), ToolCallError> {
    let mut unexpected = args
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if unexpected.is_empty() {
        return Ok(());
    }
    unexpected.sort();
    Err(ToolCallError::InvalidParams(format!(
        "unexpected argument `{}`",
        unexpected.join("`, `")
    )))
}

fn object_arguments(arguments: Value) -> Result<Map<String, Value>, ToolCallError> {
    match arguments {
        Value::Null => Ok(Map::new()),
        Value::Object(map) => Ok(map),
        other => Err(ToolCallError::InvalidParams(format!(
            "tools/call arguments must be an object, got {}",
            json_type_name(&other)
        ))),
    }
}

fn required_non_empty_string(
    args: &Map<String, Value>,
    key: &str,
) -> Result<String, ToolCallError> {
    let raw = args.get(key).and_then(Value::as_str).ok_or_else(|| {
        ToolCallError::InvalidParams(format!("missing required argument `{key}`"))
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must not be empty"
        )));
    }
    Ok(trimmed.to_string())
}

fn optional_limit(args: &Map<String, Value>) -> Result<u64, ToolCallError> {
    let Some(value) = args.get("k") else {
        return Ok(DEFAULT_LIMIT);
    };
    let Some(limit) = value.as_u64() else {
        return Err(ToolCallError::InvalidParams(
            "argument `k` must be a positive integer".to_string(),
        ));
    };
    if limit == 0 {
        return Err(ToolCallError::InvalidParams(
            "argument `k` must be greater than zero".to_string(),
        ));
    }
    if limit > MAX_LIMIT {
        // Reject explicitly instead of silently clamping. The schema advertises
        // `maximum: MAX_LIMIT`, so a higher value is a client bug; surfacing it
        // lets the LLM self-correct on the next call instead of believing it
        // received the page size it asked for.
        return Err(ToolCallError::InvalidParams(format!(
            "argument `k` must not exceed {MAX_LIMIT} (got {limit})"
        )));
    }
    Ok(limit)
}

fn validate_controller_params(
    spec: &McpToolSpec,
    params: &Map<String, Value>,
) -> Result<(), ToolCallError> {
    let rpc_method = spec.rpc_method.ok_or_else(|| {
        ToolCallError::Internal(format!(
            "MCP tool `{}` does not dispatch through RPC validation",
            spec.name
        ))
    })?;
    let schema = all::schema_for_rpc_method(rpc_method).ok_or_else(|| {
        ToolCallError::InvalidParams(format!(
            "mapped RPC method `{}` is not registered",
            rpc_method
        ))
    })?;
    all::validate_params(&schema, params).map_err(ToolCallError::InvalidParams)
}

async fn enforce_read_policy(tool_name: &str) -> Result<(), ToolCallError> {
    // Config-load failure is an internal/server issue (disk error, corrupt
    // config), not bad client input — report it as `-32603 Internal error`
    // rather than `-32602 Invalid params`.
    let config = match config_rpc::load_config_with_timeout().await {
        Ok(config) => config,
        Err(err) => {
            log::warn!(
                "[mcp_server] enforce_read_policy config load failed tool={tool_name} error={err}"
            );
            return Err(ToolCallError::Internal(format!(
                "failed to load config: {err}"
            )));
        }
    };
    let policy = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
    // A policy denial *is* something the caller can act on (toggle autonomy,
    // approve the tool) — keep that as `InvalidParams` so clients surface the
    // reason text instead of a generic internal-error banner.
    policy
        .enforce_tool_operation(ToolOperation::Read, tool_name)
        .map_err(ToolCallError::InvalidParams)
}

async fn enforce_act_policy(tool_name: &str) -> Result<(), ToolCallError> {
    let config = match config_rpc::load_config_with_timeout().await {
        Ok(config) => config,
        Err(err) => {
            log::warn!(
                "[mcp_server] enforce_act_policy config load failed tool={tool_name} error={err}"
            );
            return Err(ToolCallError::Internal(format!(
                "failed to load config: {err}"
            )));
        }
    };
    let policy = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
    policy
        .enforce_tool_operation(ToolOperation::Act, tool_name)
        .map_err(ToolCallError::InvalidParams)
}

async fn load_config_and_init_registry() -> Result<crate::openhuman::config::Config, ToolCallError>
{
    let config = config_rpc::load_config_with_timeout()
        .await
        .map_err(|err| ToolCallError::Internal(format!("failed to load config: {err}")))?;
    AgentDefinitionRegistry::init_global(&config.workspace_dir).map_err(|err| {
        ToolCallError::Internal(format!(
            "failed to initialise AgentDefinitionRegistry: {err}"
        ))
    })?;
    Ok(config)
}

async fn build_orchestrator_agent() -> Result<Agent, ToolCallError> {
    let config = load_config_and_init_registry().await?;
    let mut agent = Agent::from_config_for_agent(&config, "orchestrator").map_err(|err| {
        ToolCallError::Internal(format!("failed to build orchestrator agent: {err}"))
    })?;
    agent.fetch_connected_integrations().await;
    let _ = agent.refresh_delegation_tools();
    Ok(agent)
}

async fn list_core_tools() -> Result<Value, ToolCallError> {
    let agent = build_orchestrator_agent().await?;
    let tools = agent
        .tool_specs()
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "parameters": spec.parameters,
            })
        })
        .collect::<Vec<_>>();
    Ok(tool_success(json!({ "tools": tools })))
}

async fn core_tool_instructions() -> Result<Value, ToolCallError> {
    let agent = build_orchestrator_agent().await?;
    Ok(tool_text_success(build_tool_instructions_text(
        agent.tool_specs(),
    )))
}

async fn list_subagents() -> Result<Value, ToolCallError> {
    let config = load_config_and_init_registry().await?;
    let registry = AgentDefinitionRegistry::global().ok_or_else(|| {
        ToolCallError::Internal("AgentDefinitionRegistry missing after init".to_string())
    })?;

    let definitions = registry
        .list()
        .into_iter()
        .map(|def| {
            json!({
                "id": def.id,
                "display_name": def.display_name(),
                "when_to_use": def.when_to_use,
                "temperature": def.temperature,
                "max_iterations": def.max_iterations,
                "sandbox_mode": def.sandbox_mode,
                "tool_scope": def.tools,
                "subagents": def.subagents,
                "source": def.source,
            })
        })
        .collect::<Vec<_>>();

    let summary = format!(
        "# OpenHuman Subagents\n\nWorkspace: `{}`\n\n{}",
        config.workspace_dir.display(),
        definitions
            .iter()
            .map(|def| {
                let id = def.get("id").and_then(Value::as_str).unwrap_or("<unknown>");
                let when = def.get("when_to_use").and_then(Value::as_str).unwrap_or("");
                format!("- **{id}**: {when}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    );

    Ok(json!({
        "content": [{
            "type": "text",
            "text": summary,
        }],
        "structuredContent": {
            "definitions": definitions,
        }
    }))
}

async fn run_subagent_tool(params: &Map<String, Value>) -> Result<Value, ToolCallError> {
    let agent_id = required_non_empty_string(params, "agent_id")?;
    let prompt = required_non_empty_string(params, "prompt")?;
    if agent_id == "integrations_agent" {
        return Err(ToolCallError::InvalidParams(
            "agent.run_subagent does not yet support `integrations_agent`; first-level MCP support is currently limited to standalone agents that do not require toolkit binding".to_string(),
        ));
    }

    let config = load_config_and_init_registry().await?;
    let mut agent = Agent::from_config_for_agent(&config, &agent_id).map_err(|err| {
        ToolCallError::InvalidParams(format!("failed to build agent `{agent_id}`: {err}"))
    })?;
    agent.set_event_context(
        format!("mcp:{}:{}", agent_id, uuid::Uuid::new_v4()),
        "mcp_server",
    );
    agent.fetch_connected_integrations().await;
    let _ = agent.refresh_delegation_tools();

    let response = agent
        .run_single(&prompt)
        .await
        .map_err(|err| ToolCallError::Internal(format!("subagent `{agent_id}` failed: {err}")))?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": response,
        }],
        "structuredContent": {
            "agent_id": agent_id,
            "response": response,
        }
    }))
}

fn tool_success(value: Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        }]
    })
}

fn tool_text_success(text: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text,
        }]
    })
}

fn tool_error(message: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": message,
        }],
        "isError": true
    })
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_tools_exposes_first_level_mcp_surface() {
        let result = list_tools_result();
        let names = result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "core.list_tools",
                "core.tool_instructions",
                "agent.list_subagents",
                "agent.run_subagent",
                "memory.search",
                "memory.recall",
                "tree.read_chunk"
            ]
        );
    }

    #[test]
    fn mapped_rpc_methods_are_registered() {
        for spec in tool_specs() {
            if let Some(rpc_method) = spec.rpc_method {
                assert!(
                    all::schema_for_rpc_method(rpc_method).is_some(),
                    "missing registered RPC method for {} -> {}",
                    spec.name,
                    rpc_method
                );
            }
        }
    }

    #[test]
    fn build_rpc_params_parses_run_subagent_arguments() {
        let params = build_rpc_params(
            "agent.run_subagent",
            json!({
                "agent_id": "researcher",
                "prompt": "Find the root cause."
            }),
        )
        .expect("params should parse");

        assert_eq!(
            params.get("agent_id").and_then(Value::as_str),
            Some("researcher")
        );
        assert_eq!(
            params.get("prompt").and_then(Value::as_str),
            Some("Find the root cause.")
        );
    }

    #[test]
    fn build_rpc_params_rejects_extra_run_subagent_fields() {
        let err = build_rpc_params(
            "agent.run_subagent",
            json!({
                "agent_id": "researcher",
                "prompt": "Find the root cause.",
                "toolkit": "gmail"
            }),
        )
        .expect_err("unexpected field should be rejected");

        assert!(
            matches!(err, ToolCallError::InvalidParams(message) if message.contains("unexpected argument"))
        );
    }

    #[test]
    fn memory_search_params_trim_query_and_use_default_k() {
        let params = build_rpc_params(
            "memory.search",
            json!({
                "query": " phoenix migration ",
            }),
        )
        .expect("params");

        assert_eq!(params["query"], "phoenix migration");
        assert_eq!(params["k"], DEFAULT_LIMIT);
    }

    #[test]
    fn memory_search_rejects_k_above_max() {
        // Reject (don't silent-clamp) so the LLM can self-correct on the next
        // call. Silent clamping makes the model believe it got the page size
        // it asked for and prevents the corrective feedback loop.
        let err = build_rpc_params(
            "memory.search",
            json!({
                "query": "phoenix",
                "k": MAX_LIMIT + 1
            }),
        )
        .expect_err("must reject k > MAX_LIMIT");

        let message = err.message();
        assert!(
            message.contains("must not exceed"),
            "error should mention the cap, got: {message}"
        );
        assert!(
            message.contains(&MAX_LIMIT.to_string()),
            "error should mention the limit value, got: {message}"
        );
    }

    #[test]
    fn memory_search_accepts_k_at_max() {
        let params = build_rpc_params(
            "memory.search",
            json!({ "query": "phoenix", "k": MAX_LIMIT }),
        )
        .expect("k = MAX_LIMIT must be accepted (boundary inclusive)");
        assert_eq!(params["k"], MAX_LIMIT);
    }

    #[test]
    fn tool_call_error_invalid_params_maps_to_jsonrpc_invalid_params() {
        let err = ToolCallError::InvalidParams("missing query".to_string());
        assert_eq!(err.code(), -32602);
        assert_eq!(err.jsonrpc_message(), "Invalid params");
        assert_eq!(err.message(), "missing query");
    }

    #[test]
    fn tool_call_error_internal_maps_to_jsonrpc_internal_error() {
        // Server-side failures (config load, missing resources) must surface
        // as `-32603 Internal error`, not `-32602 Invalid params`, so the MCP
        // client doesn't mislead the user / LLM into retrying with different
        // arguments.
        let err = ToolCallError::Internal("disk read failed".to_string());
        assert_eq!(err.code(), -32603);
        assert_eq!(err.jsonrpc_message(), "Internal error");
        assert_eq!(err.message(), "disk read failed");
    }

    #[test]
    fn memory_recall_requires_query() {
        let err = build_rpc_params("memory.recall", json!({})).expect_err("must reject");
        assert!(err.message().contains("missing required argument `query`"));
    }

    #[test]
    fn memory_search_rejects_undocumented_limit_alias() {
        let err = build_rpc_params(
            "memory.search",
            json!({
                "query": "phoenix",
                "limit": 5
            }),
        )
        .expect_err("must reject");

        assert!(err.message().contains("unexpected argument `limit`"));
    }

    #[test]
    fn tree_read_chunk_maps_chunk_id_to_controller_id() {
        let params =
            build_rpc_params("tree.read_chunk", json!({"chunk_id": "abc"})).expect("params");
        assert_eq!(params["id"], "abc");
        assert!(!params.contains_key("chunk_id"));
    }

    #[test]
    fn tree_read_chunk_rejects_unknown_arguments() {
        let err = build_rpc_params(
            "tree.read_chunk",
            json!({
                "chunk_id": "abc",
                "unused": true
            }),
        )
        .expect_err("must reject");

        assert!(err.message().contains("unexpected argument `unused`"));
    }

    #[test]
    fn non_object_arguments_are_invalid() {
        let err = build_rpc_params("memory.search", json!("query")).expect_err("must reject");
        assert!(err.message().contains("arguments must be an object"));
    }
}
