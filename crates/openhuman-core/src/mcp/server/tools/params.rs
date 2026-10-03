use serde_json::{Map, Value};

use tinymcp::server::args::{
    object_arguments, optional_non_empty_string, optional_positive_u64,
    reject_unexpected_arguments, required_non_empty_string, required_non_empty_string_array,
};
use tinymcp::ToolCallError;

use crate::core::all;

use super::types::{
    McpToolSpec, FETCH_MODES, FILTER_FIELDS, FILTER_STRING_FIELDS, ITEM_KINDS, LEARNING_KINDS,
    MEMORY_FETCH_ARGUMENTS, MEMORY_FORGET_ARGUMENTS, MEMORY_FORGET_MAX_IDS, MEMORY_LEARN_ARGUMENTS,
    MEMORY_LIST_ARGUMENTS, MEMORY_MAX_LIMIT, MEMORY_RECALL_ARGUMENTS, SEARCH_MAX_RESULTS,
    SEARXNG_SEARCH_ARGUMENTS, SOURCE_KINDS, SUBAGENT_RUN_ARGUMENTS, WEB_ANSWER_ARGUMENTS,
    WEB_SEARCH_ARGUMENTS,
};

pub fn build_rpc_params(
    tool_name: &str,
    arguments: Value,
) -> Result<Map<String, Value>, ToolCallError> {
    let args = object_arguments(arguments)?;
    let name = tool_name;
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
        "memory.recall" => {
            reject_unexpected_arguments(&args, MEMORY_RECALL_ARGUMENTS)?;
            let mut params = Map::new();
            params.insert(
                "question".to_string(),
                Value::String(required_non_empty_string(&args, "question")?),
            );
            insert_filter_and_limit(&args, &mut params)?;
            Ok(params)
        }
        "memory.fetch" => {
            reject_unexpected_arguments(&args, MEMORY_FETCH_ARGUMENTS)?;
            let mut params = Map::new();
            params.insert(
                "query".to_string(),
                Value::String(required_non_empty_string(&args, "query")?),
            );
            if let Some(mode) = optional_non_empty_string(&args, "mode")? {
                require_one_of("mode", &mode, FETCH_MODES)?;
                params.insert("mode".to_string(), Value::String(mode));
            }
            insert_filter_and_limit(&args, &mut params)?;
            insert_cursor(&args, &mut params)?;
            Ok(params)
        }
        "memory.list" => {
            reject_unexpected_arguments(&args, MEMORY_LIST_ARGUMENTS)?;
            let mut params = Map::new();
            insert_filter_and_limit(&args, &mut params)?;
            insert_cursor(&args, &mut params)?;
            Ok(params)
        }
        "memory.learn" => {
            reject_unexpected_arguments(&args, MEMORY_LEARN_ARGUMENTS)?;
            let mut params = Map::new();
            params.insert(
                "text".to_string(),
                Value::String(required_non_empty_string(&args, "text")?),
            );
            if let Some(kind) = optional_non_empty_string(&args, "kind")? {
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
            reject_unexpected_arguments(&args, MEMORY_FORGET_ARGUMENTS)?;
            let ids = required_non_empty_string_array(&args, "ids")?;
            if ids.len() > MEMORY_FORGET_MAX_IDS {
                return Err(ToolCallError::InvalidParams(format!(
                    "argument `ids` accepts at most {MEMORY_FORGET_MAX_IDS} entries (got {})",
                    ids.len()
                )));
            }
            Ok(Map::from_iter([("ids".to_string(), Value::from(ids))]))
        }
        "searxng_search" | "web_search" => {
            let allowed = if name == "web_search" {
                WEB_SEARCH_ARGUMENTS
            } else {
                SEARXNG_SEARCH_ARGUMENTS
            };
            reject_unexpected_arguments(&args, allowed)?;
            let query = required_non_empty_string(&args, "query")?;
            let mut params = Map::new();
            params.insert("query".to_string(), Value::String(query));
            if let Some(max_results) =
                optional_positive_u64(&args, "max_results", SEARCH_MAX_RESULTS as u64)?
            {
                params.insert("max_results".to_string(), Value::from(max_results));
            }
            if let Some(provider) = optional_non_empty_string(&args, "provider")? {
                params.insert("provider".to_string(), Value::String(provider));
            }
            Ok(params)
        }
        "web_answer" => {
            reject_unexpected_arguments(&args, WEB_ANSWER_ARGUMENTS)?;
            let query = required_non_empty_string(&args, "query")?;
            let mut params = Map::new();
            params.insert("query".to_string(), Value::String(query));
            if let Some(depth) = optional_non_empty_string(&args, "depth")? {
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
    if let Some(limit) = optional_positive_u64(args, "limit", MEMORY_MAX_LIMIT)? {
        params.insert("limit".to_string(), Value::from(limit));
    }
    Ok(())
}

fn insert_cursor(
    args: &Map<String, Value>,
    params: &mut Map<String, Value>,
) -> Result<(), ToolCallError> {
    if let Some(cursor) = optional_non_empty_string(args, "cursor")? {
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
