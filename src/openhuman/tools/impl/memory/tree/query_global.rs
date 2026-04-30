use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::retrieval;
use crate::openhuman::memory::tree::retrieval::rpc::QueryGlobalRequest;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct MemoryTreeQueryGlobalTool;

#[async_trait]
impl Tool for MemoryTreeQueryGlobalTool {
    fn name(&self) -> &str {
        "memory_tree_query_global"
    }

    fn description(&self) -> &str {
        "Return the cross-source global digest for the last `window_days`. \
         The 7-day digest is also pre-loaded into the session context at \
         start, so only call this for a different window (e.g. 30 days, \
         1 day) or to refresh after new ingest."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "window_days": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Lookback window in days (e.g. 7 for weekly recap)."
                }
            },
            "required": ["window_days"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] query_global invoked");
        let req: QueryGlobalRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_query_global: {e}"))?;
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_query_global: load config failed: {e}"))?;
        let resp = retrieval::query_global(&cfg, req.window_days).await?;
        log::debug!(
            "[tool][memory_tree] query_global returning hits={} total={}",
            resp.hits.len(),
            resp.total
        );
        let json = serde_json::to_string(&resp)?;
        Ok(ToolResult::success(json))
    }
}
