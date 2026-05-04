use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::retrieval;
use crate::openhuman::memory::tree::retrieval::rpc::QueryTopicRequest;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct MemoryTreeQueryTopicTool;

#[async_trait]
impl Tool for MemoryTreeQueryTopicTool {
    fn name(&self) -> &str {
        "memory_tree_query_topic"
    }

    fn description(&self) -> &str {
        "Return summaries / chunks linked to a canonical entity id (e.g. \
         `email:alice@example.com`, `topic:phoenix`) across every memory \
         tree. Sorted by score then recency, or by cosine similarity if \
         `query` is provided. Use this after `memory_tree_search_entities` \
         resolves a name to a canonical id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "Canonical entity id (e.g. `email:alice@example.com`, `topic:phoenix`)."
                },
                "time_window_days": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Only return hits whose time range overlaps the last N days."
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
            },
            "required": ["entity_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] query_topic invoked");
        let req: QueryTopicRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_query_topic: {e}"))?;
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_query_topic: load config failed: {e}"))?;
        let resp = retrieval::query_topic(
            &cfg,
            &req.entity_id,
            req.time_window_days,
            req.query.as_deref(),
            req.limit.unwrap_or(10),
        )
        .await?;
        log::debug!(
            "[tool][memory_tree] query_topic returning hits={} total={}",
            resp.hits.len(),
            resp.total
        );
        let json = serde_json::to_string(&resp)?;
        Ok(ToolResult::success(json))
    }
}
