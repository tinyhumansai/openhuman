use serde_json::{Map, Value};

use crate::core::all;
use crate::openhuman::tools::SEARXNG_MAX_RESULTS;

use super::types::{
    McpToolSpec, ToolCallError, DEFAULT_LIMIT, MAX_LIMIT, MEMORY_NOTE_ARGUMENTS,
    MEMORY_STORE_ARGUMENTS, QUERY_ARGUMENTS, SEARXNG_SEARCH_ARGUMENTS, SUBAGENT_RUN_ARGUMENTS,
    TREE_BROWSE_ARGUMENTS, TREE_LIST_SOURCES_ARGUMENTS, TREE_READ_CHUNK_ARGUMENTS,
    TREE_TAG_ARGUMENTS, TREE_TAG_MAX_TAGS, TREE_TAG_MAX_TAG_LENGTH, TREE_TOP_ENTITIES_ARGUMENTS,
};

pub fn build_rpc_params(
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
        "searxng_search" => {
            reject_unexpected_arguments(&args, SEARXNG_SEARCH_ARGUMENTS)?;
            let query = required_non_empty_string(&args, "query")?;
            let mut params = Map::new();
            params.insert("query".to_string(), Value::String(query));
            if let Some(categories) = optional_string_array(&args, "categories")? {
                crate::openhuman::tools::normalize_categories(categories.clone())
                    .map_err(|err| ToolCallError::InvalidParams(err.to_string()))?;
                params.insert("categories".to_string(), Value::from(categories));
            }
            if let Some(language) = optional_non_empty_string(&args, "language")? {
                params.insert("language".to_string(), Value::String(language));
            }
            if let Some(max_results) = optional_max_results(&args, "max_results")? {
                params.insert("max_results".to_string(), Value::from(max_results));
            }
            Ok(params)
        }
        "tree.read_chunk" => {
            reject_unexpected_arguments(&args, TREE_READ_CHUNK_ARGUMENTS)?;
            let chunk_id = required_non_empty_string(&args, "chunk_id")?;
            Ok(Map::from_iter([(
                "id".to_string(),
                Value::String(chunk_id),
            )]))
        }
        "tree.browse" => {
            reject_unexpected_arguments(&args, TREE_BROWSE_ARGUMENTS)?;
            let mut params = Map::new();
            // MCP-side `k` maps to the controller's `limit` and is capped at
            // MAX_LIMIT for parity with the search / recall tools. The
            // controller itself accepts up to 1000, but the MCP layer keeps
            // the surface narrow so the LLM doesn't waste tokens pulling a
            // huge page.
            params.insert("limit".to_string(), Value::from(optional_limit(&args)?));
            if let Some(values) = optional_string_array(&args, "source_kinds")? {
                params.insert("source_kinds".to_string(), Value::from(values));
            }
            if let Some(values) = optional_string_array(&args, "source_ids")? {
                params.insert("source_ids".to_string(), Value::from(values));
            }
            if let Some(values) = optional_string_array(&args, "entity_ids")? {
                params.insert("entity_ids".to_string(), Value::from(values));
            }
            if let Some(value) = optional_i64(&args, "since_ms")? {
                params.insert("since_ms".to_string(), Value::from(value));
            }
            if let Some(value) = optional_i64(&args, "until_ms")? {
                params.insert("until_ms".to_string(), Value::from(value));
            }
            if let Some(value) = optional_non_empty_string(&args, "query")? {
                params.insert("query".to_string(), Value::String(value));
            }
            if let Some(value) = optional_u64(&args, "offset")? {
                params.insert("offset".to_string(), Value::from(value));
            }
            Ok(params)
        }
        "tree.top_entities" => {
            reject_unexpected_arguments(&args, TREE_TOP_ENTITIES_ARGUMENTS)?;
            // The controller's `limit` is required; default + cap at the MCP
            // layer so the LLM doesn't have to know the underlying contract.
            let mut params = Map::new();
            params.insert("limit".to_string(), Value::from(optional_limit(&args)?));
            if let Some(value) = optional_non_empty_string(&args, "kind")? {
                params.insert("kind".to_string(), Value::String(value));
            }
            Ok(params)
        }
        "tree.list_sources" => {
            reject_unexpected_arguments(&args, TREE_LIST_SOURCES_ARGUMENTS)?;
            let mut params = Map::new();
            if let Some(value) = optional_non_empty_string(&args, "user_email_hint")? {
                params.insert("user_email_hint".to_string(), Value::String(value));
            }
            Ok(params)
        }
        "memory.store" => {
            reject_unexpected_arguments(&args, MEMORY_STORE_ARGUMENTS)?;
            let title = required_non_empty_string(&args, "title")?;
            let content = required_non_empty_string(&args, "content")?;
            let namespace =
                optional_non_empty_string(&args, "namespace")?.unwrap_or_else(|| "mcp".to_string());
            // Generate a deterministic key from the title for upsert dedup.
            let key = format!("mcp-store-{}", slug_from(&title));
            let mut params = Map::new();
            params.insert("namespace".to_string(), Value::String(namespace));
            params.insert("key".to_string(), Value::String(key));
            params.insert("title".to_string(), Value::String(title));
            params.insert("content".to_string(), Value::String(content));
            params.insert("source_type".to_string(), Value::String("mcp".to_string()));
            if let Some(tags) = optional_string_array(&args, "tags")? {
                params.insert(
                    "tags".to_string(),
                    Value::Array(tags.into_iter().map(Value::String).collect()),
                );
            }
            Ok(params)
        }
        "memory.note" => {
            reject_unexpected_arguments(&args, MEMORY_NOTE_ARGUMENTS)?;
            let chunk_id = required_non_empty_string(&args, "chunk_id")?;
            let note_text = required_non_empty_string(&args, "note_text")?;
            let key = format!("mcp-note-{chunk_id}");
            let title = format!("Note on chunk {chunk_id}");
            let content = format!("[annotation for chunk_id={chunk_id}]\n\n{note_text}");
            let mut metadata = Map::new();
            metadata.insert("annotates_chunk_id".to_string(), Value::String(chunk_id));
            let mut params = Map::new();
            params.insert("namespace".to_string(), Value::String("mcp".to_string()));
            params.insert("key".to_string(), Value::String(key));
            params.insert("title".to_string(), Value::String(title));
            params.insert("content".to_string(), Value::String(content));
            params.insert("source_type".to_string(), Value::String("mcp".to_string()));
            params.insert("metadata".to_string(), Value::Object(metadata));
            Ok(params)
        }
        "tree.tag" => {
            reject_unexpected_arguments(&args, TREE_TAG_ARGUMENTS)?;
            let chunk_id = required_non_empty_string(&args, "chunk_id")?;
            // `required_non_empty_string_array` checks both presence and
            // that the resulting list isn't empty after trimming — keeps
            // the LLM honest about supplying at least one label per call.
            let tags = required_non_empty_string_array(&args, "tags")?;
            // Cap the tag set to keep the tag-record document bounded:
            //   * `TREE_TAG_MAX_TAGS` rejects pathological cases where a
            //     misbehaving client floods one chunk with hundreds of
            //     labels (would also bloat the document tags index).
            //   * `TREE_TAG_MAX_TAG_LENGTH` rejects oversize labels that
            //     are almost certainly free-form text (which belongs in
            //     `memory.note`, not the categorical tag surface).
            // Both reject up-front rather than silently truncating — same
            // "explicit rejection" pattern as `required_non_empty_string_array`.
            if tags.len() > TREE_TAG_MAX_TAGS {
                return Err(ToolCallError::InvalidParams(format!(
                    "argument `tags` accepts at most {TREE_TAG_MAX_TAGS} entries (got {})",
                    tags.len()
                )));
            }
            if let Some(oversize) = tags.iter().find(|t| t.len() > TREE_TAG_MAX_TAG_LENGTH) {
                return Err(ToolCallError::InvalidParams(format!(
                    "argument `tags` entry exceeds {TREE_TAG_MAX_TAG_LENGTH} bytes (got {} bytes)",
                    oversize.len()
                )));
            }
            // Deterministic key keyed on `chunk_id` (not on tag content)
            // so re-tagging the same chunk upserts the prior tag-record
            // document rather than accumulating duplicate annotations.
            // This is the structural difference from `memory.note`
            // (which keys on chunk_id too but is content-additive in
            // intent; the LLM is expected to call note again to append).
            let key = format!("mcp-tag-{chunk_id}");
            let title = format!("Tags for chunk {chunk_id}");
            let content = format!(
                "[tag record for chunk_id={chunk_id}]\n\nApplied tags: {}",
                tags.join(", ")
            );
            // Build the tag list as a JSON array once, then share it
            // between metadata.applied_tags and the top-level `tags`
            // field. `tags_array.clone()` on the cached Value is the
            // cheapest path — it clones each tag String once total,
            // matching what an in-place double-collect would do.
            let tags_array = Value::Array(tags.into_iter().map(Value::String).collect());
            let mut metadata = Map::new();
            metadata.insert("tags_for_chunk_id".to_string(), Value::String(chunk_id));
            // `applied_tags` mirrors `tags` for callers that consume the
            // metadata view; the top-level `tags` field below feeds the
            // document tags index (queryable through `doc_list` etc.).
            metadata.insert("applied_tags".to_string(), tags_array.clone());
            let mut params = Map::new();
            params.insert("namespace".to_string(), Value::String("mcp".to_string()));
            params.insert("key".to_string(), Value::String(key));
            params.insert("title".to_string(), Value::String(title));
            params.insert("content".to_string(), Value::String(content));
            params.insert("source_type".to_string(), Value::String("mcp".to_string()));
            params.insert("tags".to_string(), tags_array);
            params.insert("metadata".to_string(), Value::Object(metadata));
            Ok(params)
        }
        _ => Err(ToolCallError::InvalidParams(format!(
            "unknown MCP tool `{tool_name}`"
        ))),
    }
}

pub fn reject_unexpected_arguments(
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

pub fn object_arguments(arguments: Value) -> Result<Map<String, Value>, ToolCallError> {
    match arguments {
        Value::Null => Ok(Map::new()),
        Value::Object(map) => Ok(map),
        other => Err(ToolCallError::InvalidParams(format!(
            "tools/call arguments must be an object, got {}",
            json_type_name(&other)
        ))),
    }
}

pub fn required_non_empty_string(
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

pub fn optional_non_empty_string(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, ToolCallError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(raw) = value.as_str() else {
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must be a string"
        )));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        // Distinguish "absent" (Ok(None)) from "present but blank" — the
        // latter is a client bug worth surfacing so the LLM can drop the
        // field entirely on the next call instead of resending whitespace.
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must not be empty when provided"
        )));
    }
    Ok(Some(trimmed.to_string()))
}

pub fn optional_string_array(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, ToolCallError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(items) = value.as_array() else {
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must be an array of strings, got {}",
            json_type_name(value)
        )));
    };
    let mut out = Vec::with_capacity(items.len());
    let mut dropped_blank = 0usize;
    for item in items {
        let Some(s) = item.as_str() else {
            return Err(ToolCallError::InvalidParams(format!(
                "argument `{key}` must contain only strings, got {} entry",
                json_type_name(item)
            )));
        };
        let trimmed = s.trim();
        if trimmed.is_empty() {
            dropped_blank += 1;
            continue;
        }
        out.push(trimmed.to_string());
    }
    if dropped_blank > 0 {
        // Visibility for the silent-drop behaviour: callers don't see how many
        // entries were skipped, and a downstream "the filter didn't match"
        // bug is much faster to triage when this trace is in the log.
        log::trace!(
            "[mcp_server] optional_string_array key={key} dropped_blank_entries={dropped_blank}"
        );
    }
    Ok(Some(out))
}

/// Variant of [`optional_string_array`] that errors when the field is
/// absent, null, or resolves to an empty list after blank-trim.
///
/// Used by tools where supplying an empty `tags: []` is a no-op the
/// caller almost certainly didn't mean (e.g. `tree.tag`). The MCP layer
/// rejects it up-front instead of letting it through to the document
/// RPC where the failure mode is silent.
pub fn required_non_empty_string_array(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, ToolCallError> {
    let trimmed = optional_string_array(args, key)?.ok_or_else(|| {
        ToolCallError::InvalidParams(format!("missing required argument `{key}`"))
    })?;
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must contain at least one non-empty string"
        )));
    }
    Ok(trimmed)
}

pub fn optional_i64(args: &Map<String, Value>, key: &str) -> Result<Option<i64>, ToolCallError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value.as_i64().map(Some).ok_or_else(|| {
        ToolCallError::InvalidParams(format!(
            "argument `{key}` must be an integer in the i64 range"
        ))
    })
}

pub fn optional_u64(args: &Map<String, Value>, key: &str) -> Result<Option<u64>, ToolCallError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value.as_u64().map(Some).ok_or_else(|| {
        ToolCallError::InvalidParams(format!("argument `{key}` must be a non-negative integer"))
    })
}

pub fn optional_limit(args: &Map<String, Value>) -> Result<u64, ToolCallError> {
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

pub fn optional_max_results(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<u64>, ToolCallError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(limit) = value.as_u64() else {
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must be a positive integer"
        )));
    };
    if limit == 0 {
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must be greater than zero"
        )));
    }
    if limit > SEARXNG_MAX_RESULTS as u64 {
        return Err(ToolCallError::InvalidParams(format!(
            "argument `{key}` must not exceed {SEARXNG_MAX_RESULTS} (got {limit})"
        )));
    }
    Ok(Some(limit))
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

/// Produce a URL-safe slug from a title for use as a document key.
/// Lowercases, replaces non-alphanumeric runs with a single hyphen, and
/// truncates at 64 characters.
pub fn slug_from(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // Collapse runs of hyphens, trim leading/trailing.
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = true; // treat start as hyphen to trim leading
    for ch in slug.chars() {
        if ch == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(ch);
            prev_hyphen = false;
        }
    }
    // Trim trailing hyphen
    while result.ends_with('-') {
        result.pop();
    }
    if result.len() > 64 {
        result.truncate(64);
        while result.ends_with('-') {
            result.pop();
        }
    }
    if result.is_empty() {
        // Fallback for titles with no ASCII-alphanumeric characters (e.g.
        // Unicode-only titles like "会议记录" or "Протокол"). Use a short
        // stable hash of the original title to ensure distinct slugs.
        use sha2::{Digest, Sha256};
        let hash = hex::encode(&Sha256::digest(title.as_bytes())[..8]);
        return format!("untitled-{hash}");
    }
    result
}

pub fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
