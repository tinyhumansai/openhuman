//! Consolidated memory-tree tool — dispatches to the correct retrieval
//! primitive based on the `mode` argument. Reduces the orchestrator's
//! tool surface from 6 entries to 1.
//!
//! The individual per-mode structs are still re-exported for callers that
//! need them directly (e.g. tool registration in ops.rs for agents that
//! prefer the individual tools). The consolidated [`MemoryTreeTool`] is
//! the recommended single entry point for the orchestrator.

mod drill_down;
mod fetch_leaves;
mod query_global;
mod query_source;
mod query_topic;
mod search_entities;

// Re-export individual tool types for callers that need them directly
// (e.g. tool registration in ops.rs).
pub use drill_down::MemoryTreeDrillDownTool;
pub use fetch_leaves::MemoryTreeFetchLeavesTool;
pub use query_global::MemoryTreeQueryGlobalTool;
pub use query_source::MemoryTreeQuerySourceTool;
pub use query_topic::MemoryTreeQueryTopicTool;
pub use search_entities::MemoryTreeSearchEntitiesTool;

use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Single multi-mode tool that consolidates all six memory-tree retrieval
/// primitives behind one LLM-facing entry. The `mode` field routes to the
/// appropriate underlying implementation.
pub struct MemoryTreeTool;

#[async_trait]
impl Tool for MemoryTreeTool {
    fn name(&self) -> &str {
        "memory_tree"
    }

    fn description(&self) -> &str {
        "Query the user's ingested email/chat/document memory tree. \
         Set `mode` to one of: `search_entities` (resolve a name to a \
         canonical id — call first when the user mentions someone by name), \
         `query_topic` (all cross-source mentions of an entity), \
         `query_source` (filter by source type + time window), \
         `query_global` (cross-source daily digest), \
         `drill_down` (expand a coarse summary one level), \
         `fetch_leaves` (pull raw chunks for citation)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["search_entities", "query_topic", "query_source",
                             "query_global", "drill_down", "fetch_leaves"],
                    "description": "Which retrieval operation to run."
                },
                // search_entities params
                "query": {
                    "type": "string",
                    "description": "search_entities: substring to match. query_topic/query_source: semantic rerank query (optional)."
                },
                "kinds": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "search_entities: optional entity kind filter (email, url, handle, person, ...)."
                },
                // query_topic params
                "entity_id": {
                    "type": "string",
                    "description": "query_topic: canonical entity id returned by search_entities."
                },
                // query_source params
                "source_kind": {
                    "type": "string",
                    "description": "query_source: source type to filter (chat, email, document, ...)."
                },
                "time_window_days": {
                    "type": "integer",
                    "description": "query_source/query_global: look-back window in days."
                },
                // drill_down params
                "node_id": {
                    "type": "string",
                    "description": "drill_down: id of the summary node to expand."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "drill_down: how many levels to expand (default 1, max 3)."
                },
                // fetch_leaves params
                "chunk_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "fetch_leaves: list of chunk ids to pull."
                },
                // shared
                "limit": {
                    "type": "integer",
                    "description": "Max results (default varies by mode)."
                }
            },
            "required": ["mode"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("memory_tree: `mode` is required"))?;
        log::debug!("[tool][memory_tree] mode={mode}");
        match mode {
            "search_entities" => MemoryTreeSearchEntitiesTool.execute(args).await,
            "query_topic" => MemoryTreeQueryTopicTool.execute(args).await,
            "query_source" => MemoryTreeQuerySourceTool.execute(args).await,
            "query_global" => MemoryTreeQueryGlobalTool.execute(args).await,
            "drill_down" => MemoryTreeDrillDownTool.execute(args).await,
            "fetch_leaves" => MemoryTreeFetchLeavesTool.execute(args).await,
            other => Err(anyhow::anyhow!(
                "memory_tree: unknown mode `{other}`. Valid: search_entities, query_topic, query_source, query_global, drill_down, fetch_leaves"
            )),
        }
    }
}

#[cfg(test)]
mod memory_tree_dispatcher_tests {
    use super::*;
    use crate::openhuman::tools::traits::Tool;
    use serde_json::json;

    #[test]
    fn memory_tree_tool_name_is_correct() {
        assert_eq!(MemoryTreeTool.name(), "memory_tree");
    }

    #[test]
    fn memory_tree_schema_requires_mode() {
        let schema = MemoryTreeTool.parameters_schema();
        let required = schema.get("required").and_then(|r| r.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("mode")));
    }

    #[test]
    fn memory_tree_schema_mode_enum_has_all_six_modes() {
        let schema = MemoryTreeTool.parameters_schema();
        let modes: Vec<&str> = schema
            .get("properties")
            .unwrap()
            .get("mode")
            .unwrap()
            .get("enum")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(modes.contains(&"search_entities"));
        assert!(modes.contains(&"query_topic"));
        assert!(modes.contains(&"query_source"));
        assert!(modes.contains(&"query_global"));
        assert!(modes.contains(&"drill_down"));
        assert!(modes.contains(&"fetch_leaves"));
    }

    #[tokio::test]
    async fn memory_tree_unknown_mode_returns_error() {
        let result = MemoryTreeTool
            .execute(json!({"mode": "invalid_mode"}))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unknown mode"),
            "Expected 'unknown mode' in: {msg}"
        );
    }

    #[tokio::test]
    async fn memory_tree_missing_mode_returns_error() {
        let result = MemoryTreeTool.execute(json!({})).await;
        assert!(result.is_err());
    }
}
