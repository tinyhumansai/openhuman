use serde_json::{json, Value};
use tinymcp::ServerToolSpec;

use super::types::{
    McpToolSpec, FETCH_MODES, ITEM_KINDS, LEARNING_KINDS, MEMORY_FORGET_MAX_IDS, MEMORY_MAX_LIMIT,
    SEARCH_MAX_RESULTS, SOURCE_KINDS,
};

pub fn tool_specs() -> Vec<McpToolSpec> {
    let mut specs = base_tool_specs();
    specs.extend(search_tool_specs());
    specs
}

/// Every search tool MCP can expose; `list_tools_result_for_config` keeps
/// only those whose role has a usable provider.
pub fn search_tool_specs() -> Vec<McpToolSpec> {
    vec![
        web_search_tool_spec(),
        web_answer_tool_spec(),
        searxng_tool_spec(),
    ]
}

pub fn web_search_tool_spec() -> McpToolSpec {
    McpToolSpec {
        name: "web_search",
        title: "Web Search",
        description: "Search the web through the providers configured in OpenHuman search settings (first usable provider, with fallbacks). Returns ranked results with title, URL and snippet, plus the provider that answered.",
        rpc_method: Some("openhuman.tools_web_search"),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "description": "Search query string."},
                "max_results": {
                    "type": "integer", "minimum": 1, "maximum": SEARCH_MAX_RESULTS,
                    "description": format!("Maximum results to return (capped at {SEARCH_MAX_RESULTS}).")
                },
                "provider": {"type": "string", "minLength": 1, "description": "Pin one configured provider, e.g. `exa` or `brave`; disables fallback."}
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        annotations: json!({"readOnlyHint": true, "openWorldHint": true}),
    }
}

pub fn web_answer_tool_spec() -> McpToolSpec {
    McpToolSpec {
        name: "web_answer",
        title: "Grounded Web Answer",
        description: "Answer a question from live web sources (Gemini with Google Search grounding by default) and return the answer with its citations. `depth: deep` runs deep research when a Gemini key is configured.",
        rpc_method: Some("openhuman.tools_web_answer"),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "description": "The question to answer."},
                "depth": {"type": "string", "enum": ["quick", "deep"], "description": "quick (default) or deep."}
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        annotations: json!({"readOnlyHint": true, "openWorldHint": true}),
    }
}

pub fn base_tool_specs() -> Vec<McpToolSpec> {
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
                        "description": "Registered sub-agent id (for example `planner`, `code_executor`, `critic`)."
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
            name: "memory.recall",
            title: "Recall Memory",
            description: "Ask OpenHuman's memory a natural-language question and get a synthesised \
                          answer with citations to the stored items it rests on. Read-only. Fails \
                          with MEMORY_OFF when no memory engine is usable.",
            rpc_method: Some("openhuman.memory_recall"),
            input_schema: memory_recall_schema(),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "memory.fetch",
            title: "Fetch Memory",
            description: "Raw retrieval over stored memory items: returns matching hits (text, \
                          metadata, score) best first, optionally narrowed by a metadata filter. \
                          `mode` must be one the active engine supports; both launch engines \
                          (`tinyhumans`, `cortexdb`) support only `hybrid`, so omit `mode` unless \
                          you know otherwise. Pass the returned `next_cursor` as `cursor` for the \
                          next page. Read-only.",
            rpc_method: Some("openhuman.memory_fetch"),
            input_schema: memory_fetch_schema(),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "memory.list",
            title: "List Memory Items",
            description: "Page through stored memory items, newest first, optionally narrowed by a \
                          metadata filter. Use this to enumerate (\"what did I store last week\") \
                          rather than search by query. Pass the returned `next_cursor` as `cursor` \
                          for the next page. Read-only.",
            rpc_method: Some("openhuman.memory_items_list"),
            input_schema: memory_list_schema(),
            annotations: read_only_local_annotations(),
        },
        McpToolSpec {
            name: "memory.learn",
            title: "Learn Memory",
            description: "Store one explicit learning (a preference, fact, procedure or correction) \
                          in OpenHuman's memory. Returns the new item id. Adds an item; never \
                          modifies or removes existing ones.",
            rpc_method: Some("openhuman.memory_learn"),
            input_schema: memory_learn_schema(),
            annotations: learn_annotations(),
        },
        McpToolSpec {
            name: "memory.forget",
            title: "Forget Memory",
            description: "Permanently remove memory items by id (ids come from memory.fetch, \
                          memory.list or the citations of memory.recall). Returns how many were \
                          forgotten. This cannot be undone.",
            rpc_method: Some("openhuman.memory_forget"),
            input_schema: memory_forget_schema(),
            annotations: forget_annotations(),
        },
    ]
}

/// Annotation preset for the read-only, closed-world tools that just read
/// OpenHuman's local memory or agent registry. The MCP spec defaults are
/// `readOnlyHint: false` / `openWorldHint: true`, so both fields must be set
/// explicitly to communicate the actual shape to clients. Destructive and
/// idempotent hints are deliberately omitted — per the spec they are
/// meaningful only when `readOnlyHint == false`.
pub fn read_only_local_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "openWorldHint": false
    })
}

/// Annotation for `memory.learn`: writes a new item into the local memory
/// engine, never overwrites or removes (`destructiveHint: false`). Each call
/// stores another item, so it is not idempotent. Closed-world.
pub fn learn_annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": false,
        "openWorldHint": false
    })
}

/// Annotation for `memory.forget`: permanently deletes items (destructive);
/// repeating the same call leaves the same state (idempotent). Closed-world.
pub fn forget_annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": true,
        "idempotentHint": true,
        "openWorldHint": false
    })
}

pub fn searxng_tool_spec() -> McpToolSpec {
    McpToolSpec {
        name: "searxng_search",
        title: "SearXNG Search",
        description: "Search the configured self-hosted SearXNG instance and return title, URL and snippet results. Present when SearXNG is enabled in OpenHuman search settings.",
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

/// Every tool this config can serve: the base set plus the search tools with
/// a usable provider.
pub fn tool_specs_for_loaded_config(config: &crate::config::Config) -> Vec<McpToolSpec> {
    tool_specs_for_config(
        config,
        crate::search::providers::backend_credential_available(config),
    )
}

/// Base tools plus the search tools this config can serve.
pub fn tool_specs_for_config(
    config: &crate::config::Config,
    managed_available: bool,
) -> Vec<McpToolSpec> {
    use crate::search::providers::{effective_role_providers, resolve_with};
    use tinysearch_bus::Role;

    let mut specs = base_tool_specs();
    let resolved = resolve_with(config, managed_available);
    if !effective_role_providers(&resolved, config, Role::Search).is_empty() {
        specs.push(web_search_tool_spec());
    }
    if !effective_role_providers(&resolved, config, Role::Answer).is_empty() {
        specs.push(web_answer_tool_spec());
    }
    if resolved.iter().any(|p| p.id == "searxng" && p.usable) {
        specs.push(searxng_tool_spec());
    }
    specs
}

/// A catalog entry as `tinymcp` advertises it in `tools/list`.
pub fn server_tool_spec(spec: &McpToolSpec) -> ServerToolSpec {
    ServerToolSpec::new(spec.name, spec.description, spec.input_schema.clone())
        .with_title(spec.title)
        .with_annotations(spec.annotations.clone())
}

// ── Schema builder helpers ────────────────────────────────────────────────────

pub fn no_args_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

/// JSON Schema for the `MetaFilter` object of the memory-v2 spec.
pub fn meta_filter_schema() -> Value {
    let text =
        |description: &str| json!({"type": "string", "minLength": 1, "description": description});
    json!({
        "type": "object",
        "description": "Metadata filter; every given field must match. An empty filter matches everything.",
        "properties": {
            "workspace": text("Exact workspace (absolute path or logical id)."),
            "folder": text("Folder, exact or as a path prefix."),
            "file_path": text("File path, exact or as a path prefix."),
            "language": text("Exact language (`rust`, `python`, `en`, ...)."),
            "repo": text("Exact repository, `owner/name` or a remote URL."),
            "commit": text("Exact commit."),
            "url": text("Exact URL the item was read from."),
            "thread_id": text("Exact conversation thread id."),
            "agent_id": text("Exact id of the agent that produced the item."),
            "kinds": {
                "type": "array",
                "items": {"type": "string", "enum": ITEM_KINDS},
                "description": "Item kinds to include; omit for all."
            },
            "sources": {
                "type": "array",
                "items": {"type": "string", "enum": SOURCE_KINDS},
                "description": "Source kinds to include; omit for all."
            },
            "tags_any": {
                "type": "array",
                "items": {"type": "string", "minLength": 1},
                "description": "Match items carrying any one of these tags."
            },
            "observed_after": {
                "type": "string", "format": "date-time",
                "description": "Inclusive lower bound on when the fact was observed (RFC 3339)."
            },
            "observed_before": {
                "type": "string", "format": "date-time",
                "description": "Exclusive upper bound on when the fact was observed (RFC 3339)."
            }
        },
        "additionalProperties": false
    })
}

fn limit_schema(what: &str) -> Value {
    json!({
        "type": "integer",
        "minimum": 1,
        "maximum": MEMORY_MAX_LIMIT,
        "description": format!("Maximum {what} to return. Defaults to 10; capped at {MEMORY_MAX_LIMIT}.")
    })
}

fn cursor_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "`next_cursor` from the previous page."
    })
}

fn memory_recall_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "question": {
                "type": "string",
                "minLength": 1,
                "description": "The question to answer from memory, in natural language."
            },
            "filter": meta_filter_schema(),
            "limit": limit_schema("citations")
        },
        "required": ["question"],
        "additionalProperties": false
    })
}

fn memory_fetch_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "minLength": 1,
                "description": "What to search for."
            },
            "mode": {
                "type": "string",
                "enum": FETCH_MODES,
                "description": "Retrieval mode. Must be one the active engine supports; both launch engines support only `hybrid`. Omit to use the engine's default."
            },
            "filter": meta_filter_schema(),
            "limit": limit_schema("hits"),
            "cursor": cursor_schema()
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

fn memory_list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "filter": meta_filter_schema(),
            "limit": limit_schema("items"),
            "cursor": cursor_schema()
        },
        "required": [],
        "additionalProperties": false
    })
}

fn memory_learn_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "text": {
                "type": "string",
                "minLength": 1,
                "description": "What was learned, as a self-contained statement."
            },
            "kind": {
                "type": "string",
                "enum": LEARNING_KINDS,
                "description": "What kind of learning this is. Defaults to `fact`."
            },
            "confidence": {
                "type": "number",
                "minimum": 0,
                "maximum": 1,
                "description": "Confidence in the learning, 0 to 1. Defaults to 0.8."
            }
        },
        "required": ["text"],
        "additionalProperties": false
    })
}

fn memory_forget_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "ids": {
                "type": "array",
                "items": {"type": "string", "minLength": 1},
                "minItems": 1,
                "maxItems": MEMORY_FORGET_MAX_IDS,
                "description": "Ids of the memory items to remove permanently."
            }
        },
        "required": ["ids"],
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
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": SEARCH_MAX_RESULTS,
                "description": format!("Maximum results to return (capped at {SEARCH_MAX_RESULTS}).")
            }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}
