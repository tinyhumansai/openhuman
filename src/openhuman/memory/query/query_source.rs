use crate::openhuman::memory::api::chunks::SourceKind;
use crate::openhuman::memory::query::backend;
use crate::openhuman::memory::tree::retrieval::rpc::QuerySourceRequest;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct MemoryTreeQuerySourceTool;

#[async_trait]
impl Tool for MemoryTreeQuerySourceTool {
    fn name(&self) -> &str {
        "memory_tree_query_source"
    }

    fn description(&self) -> &str {
        "Return summaries from per-source memory trees, optionally filtered \
         by `source_id` (exact), `source_kind` (chat/email/document) and/or \
         `time_window_days`. Use this for intents like \"in my email last \
         week...\" or \"summarise our slack #eng activity\". Newest-first \
         by default; pass `query` for semantic rerank."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "source_id": {
                    "type": "string",
                    "description": "Exact source id (e.g. `slack:#eng`, `gmail:abc`)."
                },
                "source_kind": {
                    "type": "string",
                    "enum": ["chat", "email", "document"],
                    "description": "Source kind filter when no exact id is known."
                },
                "time_window_days": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Only return summaries whose time range overlaps the last N days."
                },
                "query": {
                    "type": "string",
                    "description": "Optional natural-language query for cosine-similarity rerank."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Max hits to return (default 10)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] query_source invoked");
        let req: QuerySourceRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_query_source: {e}"))?;
        // Validate arguments before touching config/disk — `SourceKind::parse`
        // is pure, so a bad `source_kind` must fail with the parse error
        // regardless of workspace state.
        let source_kind = match req.source_kind.as_deref() {
            Some(s) => Some(
                SourceKind::parse(s)
                    .map_err(|e| anyhow::anyhow!("memory_tree_query_source: {e}"))?,
            ),
            None => None,
        };
        let resp = match req.source_id.as_deref() {
            Some(source_id) => {
                backend::query_source_scope(
                    Some(source_id),
                    req.time_window_days,
                    req.query.as_deref(),
                    req.limit.unwrap_or(10),
                )
                .await?
            }
            None => {
                backend::query_source_kind(
                    source_kind,
                    req.time_window_days,
                    req.query.as_deref(),
                    req.limit.unwrap_or(10),
                )
                .await?
            }
        };
        log::debug!(
            "[tool][memory_tree] query_source returning hits={} total={}",
            resp.hits.len(),
            resp.total
        );
        let json = serde_json::to_string(&resp)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
#[path = "query_source_tests.rs"]
mod tests;
