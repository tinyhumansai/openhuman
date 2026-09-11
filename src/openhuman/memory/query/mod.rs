//! Consolidated memory query tool — dispatches to the correct memory-tree
//! retrieval primitive based on the `mode` argument.
//!
//! The individual per-mode structs are still re-exported for callers that
//! need them directly (e.g. tool registration in ops.rs for agents that
//! prefer the individual tools). The consolidated [`MemoryQueryTool`] is
//! the recommended single entry point for the `memory` orchestration layer.

mod backend;
mod cover_window;
mod drill_down;
mod fast_walk;
mod fetch_leaves;
mod ingest_document;
mod query_source;
mod search_entities;
#[cfg(test)]
mod test_workspace;

// Re-export individual tool types for callers that need them directly
// (e.g. tool registration in ops.rs).
pub use cover_window::MemoryTreeCoverWindowTool;
pub use drill_down::MemoryTreeDrillDownTool;
pub use fetch_leaves::MemoryTreeFetchLeavesTool;
pub use ingest_document::MemoryTreeIngestDocumentTool;
pub use query_source::MemoryTreeQuerySourceTool;
pub use search_entities::MemoryTreeSearchEntitiesTool;
pub use MemoryTreeTool as MemoryQueryTool;

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
         `query_source` (filter by source type + time window), \
         `drill_down` (expand a coarse summary one level), \
         `cover_window` (minimum node set covering a time window [since_ms, until_ms] — use for last-24h / time-bounded recaps), \
         `fetch_leaves` (pull raw chunks for citation), `ingest_document` (write a document into the tree for future retrieval), \
         `walk` / `smart_walk` (deterministic E2GraphRAG retrieval — extracts query entities, routes between \
         entity-graph (local) and dense-summary (global) search with no LLM, and returns ranked evidence \
         hits for a natural-language query)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["search_entities", "query_source",
                             "drill_down", "cover_window", "fetch_leaves", "ingest_document", "walk",
                             "smart_walk"],
                    "description": "Which operation to run (retrieval or write)."
                },
                // cover_window params (epoch-milliseconds)
                "since_ms": {
                    "type": "integer",
                    "description": "cover_window: inclusive window start, epoch-milliseconds."
                },
                "until_ms": {
                    "type": "integer",
                    "description": "cover_window: inclusive window end, epoch-milliseconds."
                },
                // search_entities params
                "query": {
                    "type": "string",
                    "description": "search_entities: substring to match. query_source: semantic rerank query (optional). walk: natural-language question to answer by walking the memory tree."
                },
                "kinds": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "search_entities: optional entity kind filter (email, url, handle, person, ...)."
                },
                // query_source params
                "source_kind": {
                    "type": "string",
                    "description": "query_source: source type to filter (chat, email, document, ...)."
                },
                "time_window_days": {
                    "type": "integer",
                    "description": "query_source / walk / smart_walk: look-back window in days (applied to the dense/global branch for walk)."
                },
                // walk / smart_walk params
                "max_hops": {
                    "type": "integer",
                    "description": "walk / smart_walk: entity-graph relatedness hop threshold for E2GraphRAG routing (default 2, capped at 4)."
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
            "query_source" => MemoryTreeQuerySourceTool.execute(args).await,
            "drill_down" => MemoryTreeDrillDownTool.execute(args).await,
            "cover_window" => MemoryTreeCoverWindowTool.execute(args).await,
            "fetch_leaves" => MemoryTreeFetchLeavesTool.execute(args).await,
            "ingest_document" => MemoryTreeIngestDocumentTool.execute(args).await,
            "walk" | "smart_walk" => fast_walk::run_fast_walk(args).await,
            other => {
                log::debug!("[tool][memory_tree] unknown_mode mode={other}");
                Err(anyhow::anyhow!(
                    "memory_tree: unknown mode `{other}`. Valid: search_entities, query_source, drill_down, cover_window, fetch_leaves, ingest_document, walk, smart_walk"
                ))
            }
        }
    }
}

#[cfg(test)]
#[path = "mod_memory_tree_dispatcher_tests_tests.rs"]
mod memory_tree_dispatcher_tests;
