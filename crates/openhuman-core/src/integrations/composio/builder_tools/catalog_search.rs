//! `search_tool_catalog`: live Composio catalog search with per-keyword fallback ranking.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolResult};

mod ranking;
pub(crate) use ranking::{search_catalog, search_live_catalog};

// ─────────────────────────────────────────────────────────────────────────────
// search_tool_catalog — read-only: real Composio tool slugs from the FULL
// LIVE catalog (systemic tool-contract fix, Part 1)
// ─────────────────────────────────────────────────────────────────────────────

/// `search_tool_catalog`: search the FULL LIVE Composio catalog — every real
/// action for a named app, connected or not, curated or not — so `tool_call`
/// nodes are grounded in slugs that actually exist (rather than a hallucinated
/// slug that fails the save-time flow contract validation
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

#[async_trait]
impl Tool for SearchToolCatalogTool {
    fn name(&self) -> &str {
        "search_tool_catalog"
    }

    fn description(&self) -> &str {
        "Search the FULL LIVE Composio catalog for REAL action slugs to use on `tool_call` \
         nodes — every action for a named app, whether or not the user has connected it yet \
         and whether or not it's one of OpenHuman's hand-curated actions. Read-only. Query by \
         keyword (e.g. 'send email', 'slack message'); optionally scope to one `toolkit` (e.g. \
         'gmail', or any Composio app name) to search that app specifically. Returns matching \
         { slug, toolkit, description, required_args, output_fields, primary_array_path, \
         featured } entries, curated (`featured: true`) matches ranked first. ALWAYS ground a \
         tool_call node's `slug` in a real result here — never invent one. Before wiring a \
         match's args or a downstream binding, call get_tool_contract { slug } for the FULL \
         contract (exact required_args, full input/output JSON Schema) — this search result is \
         enough to FIND the right slug, get_tool_contract is what grounds the WIRING. If the \
         app isn't connected yet, you can still build the node and use composio_connect (or \
         tell the user) — the flow will prompt for the connection at run time."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to match against tool slugs/descriptions (case-insensitive). All terms must match for an exact hit; a multi-word query with no exact match falls back to the nearest per-keyword matches. For the widest listing, prefer ONE keyword (e.g. 'gmail' or 'tweets')."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Optional toolkit/app slug to scope the search (e.g. 'gmail', 'slack', or any named Composio app — connected or not)."
                }
            },
            "required": ["query"],
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
        let query = match args.get("query").and_then(Value::as_str).map(str::trim) {
            Some(q) if !q.is_empty() => q.to_string(),
            _ => return Ok(ToolResult::error("Missing 'query' parameter".to_string())),
        };
        let toolkit = args.get("toolkit").and_then(Value::as_str);
        tracing::debug!(
            target: "flows",
            %query,
            toolkit = toolkit.unwrap_or("(any)"),
            "[flows] search_tool_catalog: searching the FULL LIVE Composio catalog (read-only)"
        );
        let outcome =
            search_catalog(&self.config, &query, toolkit, ranking::MAX_CATALOG_RESULTS).await;
        // Build with `note` first so an agent reading top-down sees the
        // near-miss / keyword-based advisory before the (possibly zero) rows.
        // `count` is always the number of returned rows, never a stand-in for
        // "no such action" — a fallback carries a non-zero count.
        let mut obj = serde_json::Map::new();
        if let Some(note) = outcome.note {
            obj.insert("note".to_string(), Value::String(note));
        }
        obj.insert("query".to_string(), Value::String(query));
        obj.insert(
            "count".to_string(),
            Value::Number(outcome.results.len().into()),
        );
        obj.insert("results".to_string(), Value::Array(outcome.results));
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &Value::Object(obj),
        )?))
    }
}
