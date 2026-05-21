use serde_json::{json, Map, Value};

use crate::core::all;
use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::inference::provider::traits::build_tool_instructions_text;
use crate::openhuman::integrations::searxng::MAX_RESULTS as SEARXNG_MAX_RESULTS;
use crate::openhuman::security::{SecurityPolicy, ToolOperation};

const DEFAULT_LIMIT: u64 = 10;
const MAX_LIMIT: u64 = 50;
const QUERY_ARGUMENTS: &[&str] = &["query", "k"];
const SEARXNG_SEARCH_ARGUMENTS: &[&str] = &["query", "categories", "language", "max_results"];
const TREE_READ_CHUNK_ARGUMENTS: &[&str] = &["chunk_id"];
const SUBAGENT_RUN_ARGUMENTS: &[&str] = &["agent_id", "prompt"];
const TREE_BROWSE_ARGUMENTS: &[&str] = &[
    "source_kinds",
    "source_ids",
    "entity_ids",
    "since_ms",
    "until_ms",
    "query",
    "k",
    "offset",
];
const TREE_TOP_ENTITIES_ARGUMENTS: &[&str] = &["kind", "k"];
const TREE_LIST_SOURCES_ARGUMENTS: &[&str] = &["user_email_hint"];

#[derive(Debug, Clone)]
pub struct McpToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub rpc_method: Option<&'static str>,
    pub input_schema: Value,
    /// MCP `ToolAnnotations` per the 2025-03-26+ spec — `readOnlyHint`,
    /// `destructiveHint`, `idempotentHint`, `openWorldHint`. Hints, not
    /// guarantees; clients use them to surface accurate safety affordances
    /// (e.g. Claude Desktop's "this tool can take destructive actions"
    /// confirmation gate). Per spec, destructive/idempotent are meaningful
    /// only when `readOnlyHint == false`, so read-only tools omit them.
    pub annotations: Value,
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
    let mut specs = base_tool_specs();
    specs.push(searxng_tool_spec());
    specs
}

fn base_tool_specs() -> Vec<McpToolSpec> {
    vec![
        McpToolSpec {
            name: "core.list_tools",
            title: "List Core Tools",
            description: "List the live core agent tool catalog that OpenHuman exposes to its orchestrator session.",
            rpc_method: None,
            input_schema: no_args_schema(),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "core.tool_instructions",
            title: "Get Tool Instructions",
            description: "Emit the markdown tool-use instructions block that OpenHuman injects into prompt-guided agents.",
            rpc_method: None,
            input_schema: no_args_schema(),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "agent.list_subagents",
            title: "List Subagents",
            description: "List registered sub-agent definitions that the core can dispatch for specialized work.",
            rpc_method: None,
            input_schema: no_args_schema(),
            annotations: read_only_local_annotations(),
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
            // Sub-agent execution is the one Act-policy surface on the MCP
            // server today (see `enforce_act_policy` dispatch in `call_tool`).
            // Sub-agents can call further tools, so destructive/openWorld are
            // both true; running the same agent twice is not a no-op so
            // idempotent is false.
            annotations: json!({
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": true
            }),
        },
        McpToolSpec {
            name: "memory.search",
            title: "Search Memory",
            description: "Keyword-search OpenHuman's local memory tree and return matching chunks ordered by recency.",
            rpc_method: Some("openhuman.memory_tree_search"),
            input_schema: query_schema("Substring to match against stored memory chunks."),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "memory.recall",
            title: "Recall Memory",
            description: "Semantically recall local memory-tree chunks relevant to a natural-language query.",
            rpc_method: Some("openhuman.memory_tree_recall"),
            input_schema: query_schema("Natural-language query to embed and rerank against memory summaries."),
            annotations: read_only_local_annotations(),
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
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "tree.browse",
            title: "Browse Memory",
            description: "Paginated listing of memory-tree chunks in reverse-chronological order, \
                          with optional filters by source kind, source id, entity id, time window, \
                          and substring keyword. Use this when the user wants to enumerate (\"what's \
                          recent in my Gmail\", \"show me everything from last week about Alice\") \
                          rather than search by query. Returns chunks plus a total match count for \
                          pagination.",
            rpc_method: Some("openhuman.memory_tree_list_chunks"),
            input_schema: tree_browse_schema(),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "tree.top_entities",
            title: "Top Memory Entities",
            description: "List the most-referenced canonical entities (people, organizations, \
                          topics, emails) across the local memory tree. Call this for entity \
                          discovery before drilling in with `tree.browse` (passing `entity_ids`) \
                          or `memory.search`. Returns entities ordered by reference count.",
            rpc_method: Some("openhuman.memory_tree_top_entities"),
            input_schema: tree_top_entities_schema(),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "tree.list_sources",
            title: "List Memory Sources",
            description: "List every distinct ingest source (Gmail account, Slack channel, Notion \
                          workspace, email thread, …) that has data in the memory tree, with \
                          chunk counts and last-activity timestamps. Use this when the user asks \
                          \"what data sources do I have\" or to discover source ids to pass into \
                          `tree.browse`.",
            rpc_method: Some("openhuman.memory_tree_list_sources"),
            input_schema: tree_list_sources_schema(),
            annotations: read_only_local_annotations(),
        },
    ]
}

/// Annotation preset for the read-only, closed-world tools that just read
/// OpenHuman's local memory tree or agent registry. The MCP spec defaults are
/// `readOnlyHint: false` / `openWorldHint: true`, so both fields must be set
/// explicitly to communicate the actual shape to clients. Destructive and
/// idempotent hints are deliberately omitted — per the spec they are
/// meaningful only when `readOnlyHint == false`.
fn read_only_local_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "openWorldHint": false
    })
}

fn searxng_tool_spec() -> McpToolSpec {
    McpToolSpec {
        name: "searxng_search",
        title: "SearXNG Search",
        description: "Search the configured self-hosted SearXNG instance and return normalized title, URL, snippet, and source results. Requires searxng.enabled=true in OpenHuman config.",
        rpc_method: Some("openhuman.tools_searxng_search"),
        input_schema: searxng_search_schema(),
        // SearXNG queries an external (self-hosted but network-reachable)
        // search engine: read-only (no state mutation), open-world (results
        // come from outside OpenHuman). Per spec, destructive/idempotent
        // hints are meaningful only when readOnlyHint=false, so omit them.
        annotations: json!({
            "readOnlyHint": true,
            "openWorldHint": true
        }),
    }
}

fn tree_browse_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source_kinds": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Restrict to one or more source kinds (e.g. `email`, `chat`, `document`). Omit to include all kinds."
            },
            "source_ids": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Restrict to specific logical source ids (e.g. a Slack channel id). Use `tree.list_sources` to discover these."
            },
            "entity_ids": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Restrict to chunks referencing any of these canonical entity ids (e.g. `person:Alice`, `email:alice@example.com`). Use `tree.top_entities` to discover these."
            },
            "since_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Inclusive lower bound on chunk timestamp, in milliseconds since Unix epoch."
            },
            "until_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Inclusive upper bound on chunk timestamp, in milliseconds since Unix epoch."
            },
            "query": {
                "type": "string",
                "minLength": 1,
                "description": "Substring keyword filter over the chunk preview text."
            },
            "k": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LIMIT,
                "description": format!("Maximum chunks per page. Defaults to {DEFAULT_LIMIT}; capped at {MAX_LIMIT}.")
            },
            "offset": {
                "type": "integer",
                "minimum": 0,
                "description": "Pagination offset (number of rows to skip). Defaults to 0."
            }
        },
        "required": [],
        "additionalProperties": false
    })
}

fn tree_top_entities_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {
                "type": "string",
                "minLength": 1,
                "description": "Restrict to a single entity kind (`person`, `email`, `topic`, `org`, …). Omit to span all kinds."
            },
            "k": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LIMIT,
                "description": format!("Maximum entities to return. Defaults to {DEFAULT_LIMIT}; capped at {MAX_LIMIT}.")
            }
        },
        "required": [],
        "additionalProperties": false
    })
}

fn tree_list_sources_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "user_email_hint": {
                "type": "string",
                "minLength": 1,
                "description": "When provided, the user's own email is stripped from email-thread display names so the other party shows up instead. Optional."
            }
        },
        "required": [],
        "additionalProperties": false
    })
}

fn searxng_search_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "minLength": 1,
                "description": "Search query string."
            },
            "categories": {
                "type": "array",
                "items": {
                    "type": "string",
                    "enum": ["web", "general", "news", "images"]
                },
                "description": "Optional SearXNG categories. `web` maps to SearXNG `general`."
            },
            "language": {
                "type": "string",
                "minLength": 1,
                "description": "Optional language code, e.g. `en`, `zh-CN`, or `fr`."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": SEARXNG_MAX_RESULTS,
                "description": format!("Maximum results to return. Defaults to searxng.max_results; capped at {SEARXNG_MAX_RESULTS}.")
            }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

pub async fn list_tools_result() -> Value {
    match config_rpc::load_config_with_timeout().await {
        Ok(config) => list_tools_result_for_config(&config),
        Err(err) => {
            log::warn!(
                "[mcp_server] tools/list config load failed; omitting config-gated tools: {err}"
            );
            list_tools_result_from_specs(base_tool_specs())
        }
    }
}

fn list_tools_result_for_config(config: &crate::openhuman::config::Config) -> Value {
    let mut specs = base_tool_specs();
    if config.searxng.enabled {
        specs.push(searxng_tool_spec());
    }
    list_tools_result_from_specs(specs)
}

fn list_tools_result_from_specs(specs: Vec<McpToolSpec>) -> Value {
    let tools = specs
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "title": tool.title,
                "description": tool.description,
                "inputSchema": tool.input_schema,
                "annotations": tool.annotations,
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
        "searxng_search" => {
            reject_unexpected_arguments(&args, SEARXNG_SEARCH_ARGUMENTS)?;
            let query = required_non_empty_string(&args, "query")?;
            let mut params = Map::new();
            params.insert("query".to_string(), Value::String(query));
            if let Some(categories) = optional_string_array(&args, "categories")? {
                crate::openhuman::integrations::searxng::normalize_categories(categories.clone())
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

fn optional_non_empty_string(
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

fn optional_string_array(
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

fn optional_i64(args: &Map<String, Value>, key: &str) -> Result<Option<i64>, ToolCallError> {
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

fn optional_u64(args: &Map<String, Value>, key: &str) -> Result<Option<u64>, ToolCallError> {
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

fn optional_max_results(
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
    fn list_tools_exposes_base_mcp_surface_when_searxng_disabled() {
        let config = crate::openhuman::config::Config::default();
        let result = list_tools_result_for_config(&config);
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
                "tree.read_chunk",
                "tree.browse",
                "tree.top_entities",
                "tree.list_sources",
            ]
        );
    }

    #[test]
    fn list_tools_emits_annotations_for_every_tool() {
        // Exercise the searxng-enabled config so the annotation contract covers
        // every shipping tool, not just the base set.
        let mut config = crate::openhuman::config::Config::default();
        config.searxng.enabled = true;
        let result = list_tools_result_for_config(&config);
        let tools = result["tools"].as_array().expect("tools array");
        for tool in tools {
            let name = tool["name"].as_str().expect("tool name");
            assert!(
                tool.get("annotations")
                    .map(Value::is_object)
                    .unwrap_or(false),
                "tool `{name}` is missing a serialized `annotations` object",
            );
        }
    }

    #[test]
    fn read_only_tools_are_marked_read_only_and_closed_world() {
        // Every tool except the act-capable ones reads local OpenHuman state
        // (memory tree / agent registry) or queries an external read-only
        // search engine. Per MCP spec defaults these would be
        // `readOnlyHint: false` and `openWorldHint: true`, so we MUST set
        // `readOnlyHint` explicitly to communicate accurate safety affordances
        // to clients. (`searxng_search` is read-only but openWorld, so it
        // verifies the read-only axis here and is exempt from the
        // openWorld=false check below.)
        let act_tool_names = ["agent.run_subagent"];
        let open_world_read_only = ["searxng_search"];
        for spec in tool_specs() {
            if act_tool_names.contains(&spec.name) {
                continue;
            }
            let annotations = &spec.annotations;
            assert_eq!(
                annotations.get("readOnlyHint").and_then(Value::as_bool),
                Some(true),
                "expected `{}` to advertise readOnlyHint=true",
                spec.name
            );
            let expected_open_world = open_world_read_only.contains(&spec.name);
            assert_eq!(
                annotations.get("openWorldHint").and_then(Value::as_bool),
                Some(expected_open_world),
                "expected `{}` to advertise openWorldHint={}",
                spec.name,
                expected_open_world
            );
            // Per spec these are meaningful only when readOnlyHint == false.
            // Emitting them on a read-only tool would be misleading.
            assert!(
                annotations.get("destructiveHint").is_none(),
                "read-only tool `{}` should not emit destructiveHint",
                spec.name
            );
            assert!(
                annotations.get("idempotentHint").is_none(),
                "read-only tool `{}` should not emit idempotentHint",
                spec.name
            );
        }
    }

    #[test]
    fn run_subagent_annotations_signal_act_semantics() {
        let spec = tool_specs()
            .into_iter()
            .find(|spec| spec.name == "agent.run_subagent")
            .expect("agent.run_subagent must be registered");
        assert_eq!(
            spec.annotations
                .get("readOnlyHint")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            spec.annotations
                .get("destructiveHint")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            spec.annotations
                .get("idempotentHint")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            spec.annotations
                .get("openWorldHint")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn list_tools_includes_searxng_when_enabled() {
        let mut config = crate::openhuman::config::Config::default();
        config.searxng.enabled = true;
        let result = list_tools_result_for_config(&config);
        let names = result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();

        assert!(names.contains(&"searxng_search"));
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
    fn searxng_search_params_accept_optional_fields() {
        let params = build_rpc_params(
            "searxng_search",
            json!({
                "query": " rust async ",
                "categories": ["web", "news"],
                "language": " en ",
                "max_results": 12
            }),
        )
        .expect("params");

        assert_eq!(params["query"], "rust async");
        assert_eq!(params["categories"], json!(["web", "news"]));
        assert_eq!(params["language"], "en");
        assert_eq!(params["max_results"], 12);
    }

    #[test]
    fn searxng_search_rejects_unknown_category() {
        let err = build_rpc_params(
            "searxng_search",
            json!({
                "query": "rust",
                "categories": ["videos"]
            }),
        )
        .expect_err("must reject");

        assert!(err.message().contains("unsupported SearXNG category"));
    }

    #[test]
    fn searxng_search_rejects_max_results_above_max() {
        let err = build_rpc_params(
            "searxng_search",
            json!({
                "query": "rust",
                "max_results": SEARXNG_MAX_RESULTS + 1
            }),
        )
        .expect_err("must reject");

        assert!(err.message().contains("must not exceed"));
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

    // ── tree.browse ────────────────────────────────────────────────────

    #[test]
    fn tree_browse_no_args_sends_default_limit_only() {
        // Empty filter is a valid request — the controller treats unset filters
        // as "no constraint" — and the MCP layer still applies its own DEFAULT_LIMIT
        // so the LLM doesn't accidentally pull the controller's 50-row default
        // when it asked for nothing.
        let params = build_rpc_params("tree.browse", json!({})).expect("empty args are valid");
        assert_eq!(params.len(), 1);
        assert_eq!(params["limit"], DEFAULT_LIMIT);
    }

    #[test]
    fn tree_browse_passes_through_filters_and_renames_k_to_limit() {
        let params = build_rpc_params(
            "tree.browse",
            json!({
                "source_kinds": ["email", "chat"],
                "source_ids": ["acme-thread-1"],
                "entity_ids": ["person:Alice"],
                "since_ms": 1_700_000_000_000_i64,
                "until_ms": 1_710_000_000_000_i64,
                "query": "Q3 plan",
                "k": 20,
                "offset": 10
            }),
        )
        .expect("params");

        assert_eq!(params["limit"], 20);
        assert!(!params.contains_key("k"));
        assert_eq!(params["source_kinds"], json!(["email", "chat"]));
        assert_eq!(params["source_ids"], json!(["acme-thread-1"]));
        assert_eq!(params["entity_ids"], json!(["person:Alice"]));
        assert_eq!(params["since_ms"], 1_700_000_000_000_i64);
        assert_eq!(params["until_ms"], 1_710_000_000_000_i64);
        assert_eq!(params["query"], "Q3 plan");
        assert_eq!(params["offset"], 10);
    }

    #[test]
    fn tree_browse_rejects_k_above_max() {
        // Same reject-don't-clamp policy as memory.search / memory.recall so the
        // LLM gets corrective feedback instead of silently receiving fewer rows
        // than it asked for.
        let err = build_rpc_params("tree.browse", json!({ "k": MAX_LIMIT + 1 }))
            .expect_err("must reject k > MAX_LIMIT");
        assert!(err.message().contains("must not exceed"));
    }

    #[test]
    fn tree_browse_rejects_unknown_argument() {
        let err = build_rpc_params("tree.browse", json!({ "limit": 10 }))
            .expect_err("must reject the controller's `limit` alias");
        assert!(err.message().contains("unexpected argument `limit`"));
    }

    #[test]
    fn tree_browse_rejects_non_array_source_kinds() {
        let err = build_rpc_params("tree.browse", json!({ "source_kinds": "email" }))
            .expect_err("must reject scalar where array is required");
        assert!(err.message().contains("must be an array of strings"));
    }

    #[test]
    fn tree_browse_rejects_non_integer_since_ms() {
        let err = build_rpc_params("tree.browse", json!({ "since_ms": "yesterday" }))
            .expect_err("must reject ISO-style date for ms field");
        assert!(err.message().contains("must be an integer"));
    }

    #[test]
    fn tree_browse_drops_blank_array_entries_silently() {
        // Empty / whitespace strings inside an array are tolerated — clients
        // sometimes send `["", "email"]` after a partial UI selection and the
        // intent ("filter to email") is unambiguous. A fully-blank array is OK
        // too and produces an empty filter (same as omitting the field).
        let params = build_rpc_params(
            "tree.browse",
            json!({ "source_kinds": ["", "email", "  "] }),
        )
        .expect("blank entries don't fail the whole call");
        assert_eq!(params["source_kinds"], json!(["email"]));
    }

    // ── tree.top_entities ──────────────────────────────────────────────

    #[test]
    fn tree_top_entities_defaults_limit_and_omits_kind() {
        let params =
            build_rpc_params("tree.top_entities", json!({})).expect("empty args are valid");
        assert_eq!(params["limit"], DEFAULT_LIMIT);
        assert!(!params.contains_key("kind"));
    }

    #[test]
    fn tree_top_entities_passes_kind_through_and_caps_limit_at_max() {
        let params = build_rpc_params(
            "tree.top_entities",
            json!({ "kind": "person", "k": MAX_LIMIT }),
        )
        .expect("k = MAX_LIMIT is the boundary, inclusive");
        assert_eq!(params["kind"], "person");
        assert_eq!(params["limit"], MAX_LIMIT);
    }

    #[test]
    fn tree_top_entities_rejects_empty_kind() {
        // Blank kind is a client bug — the controller would happily run it as
        // "no filter" but that's exactly what *omitting* the field already
        // means. Rejecting nudges the LLM to drop the field instead.
        let err = build_rpc_params("tree.top_entities", json!({ "kind": "   " }))
            .expect_err("must reject blank-only kind");
        assert!(err.message().contains("must not be empty"));
    }

    // ── tree.list_sources ──────────────────────────────────────────────

    #[test]
    fn tree_list_sources_accepts_empty_args() {
        let params =
            build_rpc_params("tree.list_sources", json!({})).expect("no args is the common case");
        assert!(params.is_empty());
    }

    #[test]
    fn tree_list_sources_passes_user_email_hint() {
        let params = build_rpc_params(
            "tree.list_sources",
            json!({ "user_email_hint": "me@example.com" }),
        )
        .expect("params");
        assert_eq!(params["user_email_hint"], "me@example.com");
    }

    #[test]
    fn tree_list_sources_rejects_unknown_argument() {
        let err = build_rpc_params("tree.list_sources", json!({ "limit": 5 }))
            .expect_err("list_sources takes no pagination");
        assert!(err.message().contains("unexpected argument `limit`"));
    }
}
