use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use tinymcp_bus::ToolCallError;

use crate::core::all;

use super::types::{
    McpToolSpec, FETCH_MODES, FILTER_FIELDS, FILTER_STRING_FIELDS, ITEM_KINDS, LEARNING_KINDS,
    MEMORY_FETCH_ARGUMENTS, MEMORY_FORGET_ARGUMENTS, MEMORY_FORGET_MAX_IDS, MEMORY_LEARN_ARGUMENTS,
    MEMORY_LIST_ARGUMENTS, MEMORY_MAX_LIMIT, MEMORY_RECALL_ARGUMENTS, SEARCH_MAX_RESULTS,
    SEARXNG_SEARCH_ARGUMENTS, SOURCE_KINDS, SUBAGENT_RUN_ARGUMENTS, WEB_ANSWER_ARGUMENTS,
    WEB_SEARCH_ARGUMENTS,
};

pub async fn build_rpc_params(
    ctx: &DispatchCtx,
    tool_name: &str,
    arguments: Value,
) -> Result<Map<String, Value>, ToolCallError> {
    let args = validate_tool_arguments(ctx, tool_name, arguments).await?;
    build_rpc_params_from_validated(tool_name, &args)
}

fn build_rpc_params_from_validated(
    tool_name: &str,
    args: &Map<String, Value>,
) -> Result<Map<String, Value>, ToolCallError> {
    match tool_name {
        "core.list_tools" | "core.tool_instructions" | "agent.list_subagents" => Ok(Map::new()),
        "agent.run_subagent" => {
            let agent_id = validated_string(args, "agent_id")?;
            let prompt = validated_string(args, "prompt")?;
            Ok(Map::from_iter([
                ("agent_id".to_string(), Value::String(agent_id)),
                ("prompt".to_string(), Value::String(prompt)),
            ]))
        }
        "memory.recall" => {
            let mut params = Map::new();
            params.insert(
                "question".to_string(),
                Value::String(validated_string(args, "question")?),
            );
            insert_filter_and_limit(args, &mut params)?;
            Ok(params)
        }
        "memory.fetch" => {
            let mut params = Map::new();
            params.insert(
                "query".to_string(),
                Value::String(validated_string(args, "query")?),
            );
            if let Some(mode) = optional_string(args, "mode") {
                require_one_of("mode", &mode, FETCH_MODES)?;
                params.insert("mode".to_string(), Value::String(mode));
            }
            insert_filter_and_limit(args, &mut params)?;
            insert_cursor(args, &mut params)?;
            Ok(params)
        }
        "memory.list" => {
            let mut params = Map::new();
            insert_filter_and_limit(args, &mut params)?;
            insert_cursor(args, &mut params)?;
            Ok(params)
        }
        "memory.learn" => {
            let mut params = Map::new();
            params.insert(
                "text".to_string(),
                Value::String(validated_string(args, "text")?),
            );
            if let Some(kind) = optional_string(args, "kind") {
                require_one_of("kind", &kind, LEARNING_KINDS)?;
                params.insert("kind".to_string(), Value::String(kind));
            }
            if let Some(value) = args.get("confidence").filter(|v| !v.is_null()) {
                let confidence = value.as_f64().ok_or_else(|| {
                    ToolCallError::InvalidParams(
                        "argument `confidence` must be a number between 0 and 1".to_string(),
                    )
                })?;
                if !(0.0..=1.0).contains(&confidence) {
                    return Err(ToolCallError::InvalidParams(format!(
                        "argument `confidence` must be between 0 and 1 (got {confidence})"
                    )));
                }
                params.insert("confidence".to_string(), Value::from(confidence));
            }
            Ok(params)
        }
        "memory.forget" => {
            let ids = validated_string_array(args, "ids")?;
            if ids.len() > MEMORY_FORGET_MAX_IDS {
                return Err(ToolCallError::InvalidParams(format!(
                    "argument `ids` accepts at most {MEMORY_FORGET_MAX_IDS} entries (got {})",
                    ids.len()
                )));
            }
            Ok(Map::from_iter([("ids".to_string(), Value::from(ids))]))
        }
        "searxng_search" | "web_search" => {
            let query = validated_string(args, "query")?;
            let mut params = Map::new();
            params.insert("query".to_string(), Value::String(query));
            if let Some(max_results) = optional_u64(args, "max_results") {
                params.insert("max_results".to_string(), Value::from(max_results));
            }
            if let Some(provider) = optional_string(args, "provider") {
                params.insert("provider".to_string(), Value::String(provider));
            }
            Ok(params)
        }
        "web_answer" => {
            let query = validated_string(args, "query")?;
            let mut params = Map::new();
            params.insert("query".to_string(), Value::String(query));
            if let Some(depth) = optional_string(args, "depth") {
                if depth != "quick" && depth != "deep" {
                    return Err(ToolCallError::InvalidParams(
                        "argument `depth` must be quick or deep".to_string(),
                    ));
                }
                params.insert("depth".to_string(), Value::String(depth));
            }
            Ok(params)
        }
        _ => Err(ToolCallError::InvalidParams(format!(
            "unknown MCP tool `{tool_name}`"
        ))),
    }
}

#[cfg(test)]
pub(crate) fn build_rpc_params_for_test(
    tool_name: &str,
    arguments: Value,
) -> Result<Map<String, Value>, ToolCallError> {
    let args = arguments.as_object().cloned().unwrap_or_default();
    build_rpc_params_from_validated(tool_name, &args)
}

async fn validate_tool_arguments(
    ctx: &DispatchCtx,
    tool_name: &str,
    arguments: Value,
) -> Result<Map<String, Value>, ToolCallError> {
    use tinymcp_bus::{
        ServerArgumentRule as Rule, ServerArgumentsValidation, ValidateServerArgumentsRequest,
    };

    let required = |key: &str| Rule::RequiredNonEmptyString {
        key: key.to_owned(),
    };
    let optional = |key: &str| Rule::OptionalNonEmptyString {
        key: key.to_owned(),
    };
    let positive = |key: &str, max| Rule::OptionalPositiveU64 {
        key: key.to_owned(),
        max,
    };
    let array = |key: &str| Rule::RequiredNonEmptyStringArray {
        key: key.to_owned(),
    };
    let (allowed, rules): (&[&str], Vec<Rule>) = match tool_name {
        "core.list_tools" | "core.tool_instructions" | "agent.list_subagents" => (&[], Vec::new()),
        "agent.run_subagent" => (
            SUBAGENT_RUN_ARGUMENTS,
            vec![required("agent_id"), required("prompt")],
        ),
        "memory.recall" => (
            MEMORY_RECALL_ARGUMENTS,
            vec![required("question"), positive("limit", MEMORY_MAX_LIMIT)],
        ),
        "memory.fetch" => (
            MEMORY_FETCH_ARGUMENTS,
            vec![
                required("query"),
                optional("mode"),
                positive("limit", MEMORY_MAX_LIMIT),
                optional("cursor"),
            ],
        ),
        "memory.list" => (
            MEMORY_LIST_ARGUMENTS,
            vec![positive("limit", MEMORY_MAX_LIMIT), optional("cursor")],
        ),
        "memory.learn" => (
            MEMORY_LEARN_ARGUMENTS,
            vec![required("text"), optional("kind")],
        ),
        "memory.forget" => (MEMORY_FORGET_ARGUMENTS, vec![array("ids")]),
        "searxng_search" => (
            SEARXNG_SEARCH_ARGUMENTS,
            vec![
                required("query"),
                positive("max_results", SEARCH_MAX_RESULTS as u64),
                optional("provider"),
            ],
        ),
        "web_search" => (
            WEB_SEARCH_ARGUMENTS,
            vec![
                required("query"),
                positive("max_results", SEARCH_MAX_RESULTS as u64),
                optional("provider"),
            ],
        ),
        "web_answer" => (
            WEB_ANSWER_ARGUMENTS,
            vec![required("query"), optional("depth")],
        ),
        _ => {
            return Err(ToolCallError::InvalidParams(format!(
                "unknown MCP tool `{tool_name}`"
            )))
        }
    };

    let config = crate::config::ops::load_current_or_init(ctx)
        .await
        .map_err(|_| ToolCallError::Internal("MCP argument validation is unavailable".into()))?;
    let client = crate::mcp::module_client::McpConfiguredClient::for_config(&config)
        .map_err(|_| ToolCallError::Internal("MCP argument validation is unavailable".into()))?;
    let result = client
        .validate_server_arguments(ValidateServerArgumentsRequest {
            arguments,
            allowed: allowed.iter().map(|key| (*key).to_owned()).collect(),
            rules,
        })
        .await
        .map_err(|_| ToolCallError::Internal("MCP argument validation is unavailable".into()))?;
    match result {
        ServerArgumentsValidation::Valid(arguments) => Ok(arguments),
        ServerArgumentsValidation::Invalid(error) => Err(error),
    }
}

fn validated_string(args: &Map<String, Value>, key: &str) -> Result<String, ToolCallError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ToolCallError::Internal("MCP module returned invalid arguments".into()))
}

fn validated_string_array(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, ToolCallError> {
    args.get(key)
        .and_then(Value::as_array)
        .and_then(|values| values.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
        .map(|values| values.into_iter().map(str::to_owned).collect())
        .ok_or_else(|| ToolCallError::Internal("MCP module returned invalid arguments".into()))
}

fn optional_string(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn optional_u64(args: &Map<String, Value>, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

pub fn validate_controller_params(
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

/// Copies the optional `filter` and `limit` arguments into the RPC params.
fn insert_filter_and_limit(
    args: &Map<String, Value>,
    params: &mut Map<String, Value>,
) -> Result<(), ToolCallError> {
    if let Some(filter) = args.get("filter").filter(|v| !v.is_null()) {
        params.insert("filter".to_string(), validated_filter(filter)?);
    }
    if let Some(limit) = optional_u64(args, "limit") {
        params.insert("limit".to_string(), Value::from(limit));
    }
    Ok(())
}

fn insert_cursor(
    args: &Map<String, Value>,
    params: &mut Map<String, Value>,
) -> Result<(), ToolCallError> {
    if let Some(cursor) = optional_string(args, "cursor") {
        params.insert("cursor".to_string(), Value::String(cursor));
    }
    Ok(())
}

fn require_one_of(name: &str, value: &str, allowed: &[&str]) -> Result<(), ToolCallError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(ToolCallError::InvalidParams(format!(
            "argument `{name}` must be one of {} (got `{value}`)",
            allowed.join(", ")
        )))
    }
}

fn invalid(message: String) -> ToolCallError {
    ToolCallError::InvalidParams(message)
}

/// Validates a `MetaFilter` object (memory-v2 spec) and returns it with blank
/// entries removed. Unknown fields and wrong types are rejected rather than
/// dropped so the model can correct itself.
fn validated_filter(value: &Value) -> Result<Value, ToolCallError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("argument `filter` must be an object".to_string()))?;
    let mut out = Map::new();
    for (key, field) in object {
        if field.is_null() {
            continue;
        }
        let name = key.as_str();
        if !FILTER_FIELDS.contains(&name) {
            return Err(invalid(format!("unexpected filter field `{name}`")));
        }
        if FILTER_STRING_FIELDS.contains(&name) {
            let text = field.as_str().map(str::trim).filter(|s| !s.is_empty());
            let text = text.ok_or_else(|| {
                invalid(format!("filter field `{name}` must be a non-empty string"))
            })?;
            out.insert(key.clone(), Value::String(text.to_string()));
            continue;
        }
        match name {
            "kinds" => {
                out.insert(key.clone(), enum_array(name, field, ITEM_KINDS)?);
            }
            "sources" => {
                out.insert(key.clone(), enum_array(name, field, SOURCE_KINDS)?);
            }
            "tags_any" => {
                let items = field
                    .as_array()
                    .ok_or_else(|| invalid("filter field `tags_any` must be an array".into()))?;
                let mut tags = Vec::with_capacity(items.len());
                for item in items {
                    let tag = item.as_str().map(str::trim).filter(|s| !s.is_empty());
                    tags.push(Value::String(
                        tag.ok_or_else(|| {
                            invalid(
                                "filter field `tags_any` entries must be non-empty strings".into(),
                            )
                        })?
                        .to_string(),
                    ));
                }
                out.insert(key.clone(), Value::Array(tags));
            }
            // observed_after / observed_before: RFC 3339 timestamps.
            _ => {
                let text = field.as_str().ok_or_else(|| {
                    invalid(format!(
                        "filter field `{name}` must be an RFC 3339 timestamp string"
                    ))
                })?;
                chrono::DateTime::parse_from_rfc3339(text).map_err(|_| {
                    invalid(format!(
                        "filter field `{name}` must be an RFC 3339 timestamp (got `{text}`)"
                    ))
                })?;
                out.insert(key.clone(), Value::String(text.to_string()));
            }
        }
    }
    Ok(Value::Object(out))
}

fn enum_array(name: &str, value: &Value, allowed: &[&str]) -> Result<Value, ToolCallError> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid(format!("filter field `{name}` must be an array")))?;
    for item in items {
        let text = item
            .as_str()
            .ok_or_else(|| invalid(format!("filter field `{name}` entries must be strings")))?;
        if !allowed.contains(&text) {
            return Err(invalid(format!(
                "filter field `{name}` entries must be one of {} (got `{text}`)",
                allowed.join(", ")
            )));
        }
    }
    Ok(value.clone())
}
