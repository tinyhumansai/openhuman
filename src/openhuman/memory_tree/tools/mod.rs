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
mod ingest_document;
mod query_global;
mod query_source;
mod query_topic;
mod search_entities;
pub mod walk;

// Re-export individual tool types for callers that need them directly
// (e.g. tool registration in ops.rs).
pub use drill_down::MemoryTreeDrillDownTool;
pub use fetch_leaves::MemoryTreeFetchLeavesTool;
pub use ingest_document::MemoryTreeIngestDocumentTool;
pub use query_global::MemoryTreeQueryGlobalTool;
pub use query_source::MemoryTreeQuerySourceTool;
pub use query_topic::MemoryTreeQueryTopicTool;
pub use search_entities::MemoryTreeSearchEntitiesTool;
pub use walk::{run_walk, MemoryTreeWalkTool, WalkOptions, WalkOutcome, WalkStep, WalkStopReason};

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
         `fetch_leaves` (pull raw chunks for citation), `ingest_document` (write a document into the tree for future retrieval), \
         `walk` (agentic multi-turn walk — LLM navigates summaries and returns a synthesized answer for a natural-language query)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["search_entities", "query_topic", "query_source",
                             "query_global", "drill_down", "fetch_leaves", "ingest_document", "walk"],
                    "description": "Which operation to run (retrieval or write)."
                },
                // search_entities params
                "query": {
                    "type": "string",
                    "description": "search_entities: substring to match. query_topic/query_source: semantic rerank query (optional). walk: natural-language question to answer by walking the memory tree."
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
                    "description": "query_source/query_topic: look-back window in days. query_global also accepts this as a compatibility alias."
                },
                "window_days": {
                    "type": "integer",
                    "description": "query_global: look-back window in days."
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
                // ingest_document params
                "title": {
                    "type": "string",
                    "description": "ingest_document: document title."
                },
                "body": {
                    "type": "string",
                    "description": "ingest_document: document body (markdown or plain text)."
                },
                "source_id": {
                    "type": "string",
                    "description": "ingest_document / query_source: stable source identifier. For ingest, re-ingesting same id replaces old chunks."
                },
                "provider": {
                    "type": "string",
                    "description": "ingest_document: source provider (e.g. github, web, root_docs). Defaults to agent."
                },
                "source_ref": {
                    "type": "string",
                    "description": "ingest_document: optional URL back to original source."
                },
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
            "query_global" => {
                MemoryTreeQueryGlobalTool
                    .execute(translate_query_global_args(args))
                    .await
            }
            "drill_down" => MemoryTreeDrillDownTool.execute(args).await,
            "fetch_leaves" => MemoryTreeFetchLeavesTool.execute(args).await,
            "ingest_document" => MemoryTreeIngestDocumentTool.execute(args).await,
            "walk" => MemoryTreeWalkTool.execute(args).await,
            other => {
                log::debug!("[tool][memory_tree] unknown_mode mode={other}");
                Err(anyhow::anyhow!(
                    "memory_tree: unknown mode `{other}`. Valid: search_entities, query_topic, query_source, query_global, drill_down, fetch_leaves, ingest_document, walk"
                ))
            }
        }
    }
}

/// Rename the consolidated tool's `time_window_days` field to `window_days`
/// before dispatching to [`MemoryTreeQueryGlobalTool`].
///
/// The consolidated `parameters_schema()` exposes one shared
/// `time_window_days` field for both `query_source` and `query_global` (the
/// `query_source` backend uses that exact name). The `query_global` backend
/// — `QueryGlobalRequest { window_days: u32 }` in `memory/tree/retrieval/rpc.rs`
/// — was never aligned with that schema, so any call following the
/// consolidated contract failed with `missing field 'window_days'`.
///
/// Translating in the dispatch keeps the LLM-facing schema stable and
/// leaves the standalone [`MemoryTreeQueryGlobalTool`] (which advertises
/// `window_days` natively) untouched. An explicit `window_days` always
/// wins so callers can opt into the underlying contract if they ever
/// want to.
fn translate_query_global_args(mut args: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = args.as_object_mut() {
        if !obj.contains_key("window_days") {
            if let Some(value) = obj.remove("time_window_days") {
                obj.insert("window_days".to_string(), value);
            }
        }
    }
    args
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
    fn memory_tree_schema_mode_enum_has_all_modes() {
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
        assert!(modes.contains(&"ingest_document"));
        assert!(modes.contains(&"walk"));
    }

    #[test]
    fn memory_tree_schema_exposes_global_window_days() {
        let schema = MemoryTreeTool.parameters_schema();
        let properties = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap();
        assert!(properties.contains_key("window_days"));
        assert!(properties.contains_key("time_window_days"));
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

    #[tokio::test]
    async fn memory_tree_query_global_mode_dispatches_successfully() {
        let result = MemoryTreeTool
            .execute(json!({
                "mode": "query_global",
                "time_window_days": 7
            }))
            .await
            .expect("query_global mode should dispatch successfully");
        assert!(!result.is_error);
        let parsed: serde_json::Value =
            serde_json::from_str(&result.text()).expect("result should be valid json");
        assert!(parsed.get("hits").is_some());
        assert!(parsed.get("total").is_some());
    }

    #[tokio::test]
    async fn memory_tree_fetch_leaves_mode_dispatches_successfully() {
        let result = MemoryTreeTool
            .execute(json!({
                "mode": "fetch_leaves",
                "chunk_ids": ["chunk-does-not-exist"]
            }))
            .await
            .expect("fetch_leaves mode should dispatch successfully");
        assert!(!result.is_error);
        let parsed: serde_json::Value =
            serde_json::from_str(&result.text()).expect("result should be valid json");
        assert!(parsed.is_array());
    }

    #[test]
    fn translate_query_global_args_renames_time_window_days_to_window_days() {
        // Per-issue #2252: the consolidated schema advertises `time_window_days`
        // but `QueryGlobalRequest` deserializes from `window_days`. The
        // translation closes that gap inside the dispatch.
        let translated = translate_query_global_args(json!({
            "mode": "query_global",
            "time_window_days": 7,
        }));
        assert_eq!(
            translated,
            json!({
                "mode": "query_global",
                "window_days": 7,
            }),
        );
    }

    #[test]
    fn translate_query_global_args_passes_through_window_days_when_already_set() {
        // The standalone `MemoryTreeQueryGlobalTool` schema advertises
        // `window_days` natively — callers using that path should reach the
        // backend unchanged.
        let translated = translate_query_global_args(json!({
            "mode": "query_global",
            "window_days": 30,
        }));
        assert_eq!(translated["window_days"], 30);
        assert!(translated.get("time_window_days").is_none());
    }

    #[test]
    fn translate_query_global_args_prefers_explicit_window_days_over_time_window_days() {
        // If a caller somehow supplies both, the underlying contract wins
        // (`window_days` is what the deserializer reads). Without this,
        // a future caller migrating to `window_days` while leaving a stale
        // `time_window_days` in the payload would silently lose their
        // explicit choice to the legacy alias.
        let translated = translate_query_global_args(json!({
            "mode": "query_global",
            "window_days": 30,
            "time_window_days": 7,
        }));
        assert_eq!(translated["window_days"], 30);
    }

    #[test]
    fn translate_query_global_args_leaves_payload_untouched_when_neither_field_present() {
        // The underlying tool surfaces its own missing-field error in this
        // case; the translator should not invent a value.
        let translated = translate_query_global_args(json!({
            "mode": "query_global",
        }));
        assert_eq!(
            translated,
            json!({
                "mode": "query_global",
            }),
        );
    }
}
