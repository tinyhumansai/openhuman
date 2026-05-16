use serde_json::{json, Map, Value};

use crate::core::all;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::security::{SecurityPolicy, ToolOperation};

const DEFAULT_LIMIT: u64 = 10;
const MAX_LIMIT: u64 = 50;
const QUERY_ARGUMENTS: &[&str] = &["query", "k"];
const TREE_READ_CHUNK_ARGUMENTS: &[&str] = &["chunk_id"];

#[derive(Debug, Clone)]
pub struct McpToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub rpc_method: &'static str,
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
            name: "memory.search",
            title: "Search Memory",
            description: "Keyword-search OpenHuman's local memory tree and return matching chunks ordered by recency.",
            rpc_method: "openhuman.memory_tree_search",
            input_schema: query_schema("Substring to match against stored memory chunks."),
        },
        McpToolSpec {
            name: "memory.recall",
            title: "Recall Memory",
            description: "Semantically recall local memory-tree chunks relevant to a natural-language query.",
            rpc_method: "openhuman.memory_tree_recall",
            input_schema: query_schema("Natural-language query to embed and rerank against memory summaries."),
        },
        McpToolSpec {
            name: "tree.read_chunk",
            title: "Read Memory Chunk",
            description: "Read one memory-tree chunk by id. Use this to inspect the source text behind search or recall results.",
            rpc_method: "openhuman.memory_tree_get_chunk",
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
    validate_controller_params(&spec, &params)?;
    enforce_read_policy(spec.name).await?;

    log::debug!(
        "[mcp_server] tools/call dispatch tool={} rpc_method={} arg_keys={:?}",
        spec.name,
        spec.rpc_method,
        params.keys().collect::<Vec<_>>()
    );

    match all::try_invoke_registered_rpc(spec.rpc_method, params).await {
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
                spec.rpc_method
            );
            Ok(tool_error(format!(
                "{} is unavailable: mapped RPC method `{}` is not registered",
                spec.name, spec.rpc_method
            )))
        }
    }
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
    let schema = all::schema_for_rpc_method(spec.rpc_method).ok_or_else(|| {
        ToolCallError::InvalidParams(format!(
            "mapped RPC method `{}` is not registered",
            spec.rpc_method
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

fn tool_success(value: Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
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
    fn list_tools_exposes_curated_read_only_surface() {
        let result = list_tools_result();
        let names = result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["memory.search", "memory.recall", "tree.read_chunk"]
        );
    }

    #[test]
    fn mapped_rpc_methods_are_registered() {
        for spec in tool_specs() {
            assert!(
                all::schema_for_rpc_method(spec.rpc_method).is_some(),
                "missing registered RPC method for {} -> {}",
                spec.name,
                spec.rpc_method
            );
        }
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
