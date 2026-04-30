use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::retrieval;
use crate::openhuman::memory::tree::retrieval::rpc::QuerySourceRequest;
use crate::openhuman::memory::tree::types::SourceKind;
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
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_query_source: load config failed: {e}"))?;
        let source_kind = match req.source_kind.as_deref() {
            Some(s) => Some(
                SourceKind::parse(s)
                    .map_err(|e| anyhow::anyhow!("memory_tree_query_source: {e}"))?,
            ),
            None => None,
        };
        let resp = retrieval::query_source(
            &cfg,
            req.source_id.as_deref(),
            source_kind,
            req.time_window_days,
            req.query.as_deref(),
            req.limit.unwrap_or(10),
        )
        .await?;
        log::debug!(
            "[tool][memory_tree] query_source returning hits={} total={}",
            resp.hits.len(),
            resp.total
        );
        let json = serde_json::to_string(&resp)?;
        Ok(ToolResult::success(json))
    }
}
