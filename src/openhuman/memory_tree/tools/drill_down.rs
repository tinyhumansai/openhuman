use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::retrieval;
use crate::openhuman::memory::retrieval::rpc::DrillDownRequest;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct MemoryTreeDrillDownTool;

#[async_trait]
impl Tool for MemoryTreeDrillDownTool {
    fn name(&self) -> &str {
        "memory_tree_drill_down"
    }

    fn description(&self) -> &str {
        "Walk a summary node's children one step (or more if `max_depth > \
         1`). Returns leaf chunks for an L1 summary, or lower-level \
         summaries for L2+. Use this when a `query_*` summary is too coarse \
         and you want to expand it. Pass `query` to rerank children by \
         cosine similarity."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "Id of the summary (or leaf) to expand."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "How many levels down to walk (default 1)."
                },
                "query": {
                    "type": "string",
                    "description": "Optional natural-language query — when set, children are reranked by cosine similarity."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional cap on returned hits, applied after rerank."
                }
            },
            "required": ["node_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] drill_down invoked");
        let req: DrillDownRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_drill_down: {e}"))?;
        if matches!(req.max_depth, Some(0)) {
            return Err(anyhow::anyhow!(
                "memory_tree_drill_down: max_depth must be >= 1"
            ));
        }
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_drill_down: load config failed: {e}"))?;
        let hits = retrieval::drill_down(
            &cfg,
            &req.node_id,
            req.max_depth.unwrap_or(1),
            req.query.as_deref(),
            req.limit,
        )
        .await?;
        log::debug!(
            "[tool][memory_tree] drill_down returning hits={}",
            hits.len()
        );
        let json = serde_json::to_string(&hits)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    use tempfile::TempDir;

    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    use crate::openhuman::tools::traits::Tool;
    use serde_json::json;

    struct WorkspaceEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<OsString>,
    }

    impl WorkspaceEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for WorkspaceEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("OPENHUMAN_WORKSPACE", previous);
            } else {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            }
        }
    }

    async fn isolated_config(tmp: &TempDir) -> (WorkspaceEnvGuard, Config) {
        let guard = WorkspaceEnvGuard::set(tmp.path());
        let config = Config::load_or_init().await.expect("load config");
        (guard, config)
    }

    #[test]
    fn parameters_schema_requires_node_id() {
        let tool = MemoryTreeDrillDownTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], json!(["node_id"]));
        assert_eq!(schema["properties"]["max_depth"]["minimum"], 1);
    }

    #[test]
    fn drill_down_request_deserializes_optional_fields() {
        let req: DrillDownRequest = serde_json::from_value(json!({
            "node_id": "summary-1",
            "max_depth": 2,
            "query": "deployment blockers",
            "limit": 7
        }))
        .unwrap();
        assert_eq!(req.node_id, "summary-1");
        assert_eq!(req.max_depth, Some(2));
        assert_eq!(req.query.as_deref(), Some("deployment blockers"));
        assert_eq!(req.limit, Some(7));
    }

    #[tokio::test]
    async fn execute_rejects_missing_node_id() {
        let tool = MemoryTreeDrillDownTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("missing node_id should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tree_drill_down"));
    }

    #[tokio::test]
    async fn execute_rejects_zero_max_depth() {
        let tool = MemoryTreeDrillDownTool;
        let err = tool
            .execute(json!({
                "node_id": "summary-1",
                "max_depth": 0
            }))
            .await
            .expect_err("max_depth=0 should fail at tool boundary");
        assert!(err.to_string().contains("max_depth must be >= 1"));
    }

    #[tokio::test]
    async fn execute_success_path_returns_empty_json_array_for_isolated_workspace() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeDrillDownTool;
        let result = tool
            .execute(json!({
                "node_id": "summary-does-not-exist",
                "max_depth": 1
            }))
            .await
            .expect("valid drill_down request should succeed in isolated workspace");
        assert!(!result.is_error);
        let payload = result.text();
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("result should be valid json");
        assert!(
            parsed.is_array(),
            "drill_down should serialize a JSON array"
        );
        assert_eq!(parsed, json!([]));

        let direct = retrieval::drill_down(&cfg, "summary-does-not-exist", 1, None, None)
            .await
            .expect("direct drill_down on empty workspace");
        assert!(direct.is_empty());
    }

    #[tokio::test]
    async fn execute_accepts_query_and_limit_together() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeDrillDownTool;
        let result = tool
            .execute(json!({
                "node_id": "summary-does-not-exist",
                "max_depth": 2,
                "query": "deployment blockers",
                "limit": 5
            }))
            .await
            .expect("query+limit drill_down should succeed");
        assert!(!result.is_error);
    }
}
