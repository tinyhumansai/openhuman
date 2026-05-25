use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::query::backend;
use crate::openhuman::memory_tree::retrieval::rpc::QueryGlobalRequest;
use crate::openhuman::memory_tree::tree::TreeProfile;
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
        "Return the cross-source global digest for the last `time_window_days`. \
         The 7-day digest is also pre-loaded into the session context at \
         start, so only call this for a different window (e.g. 30 days, \
         1 day) or to refresh after new ingest."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "time_window_days": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Lookback window in days (e.g. 7 for weekly recap)."
                }
            },
            "required": ["time_window_days"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] query_global invoked");
        let req: QueryGlobalRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_query_global: {e}"))?;
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_query_global: load config failed: {e}"))?;
        let resp = backend::query_profile(
            &cfg,
            TreeProfile::Global,
            None,
            Some(req.time_window_days),
            None,
            10,
        )
        .await?;
        log::debug!(
            "[tool][memory_tree] query_global returning hits={} total={}",
            resp.hits.len(),
            resp.total
        );
        let json = serde_json::to_string(&resp)?;
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
    fn parameters_schema_requires_time_window_days() {
        let tool = MemoryTreeQueryGlobalTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], json!(["time_window_days"]));
        assert_eq!(schema["properties"]["time_window_days"]["minimum"], 1);
    }

    #[tokio::test]
    async fn execute_rejects_missing_time_window_days() {
        let tool = MemoryTreeQueryGlobalTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("missing time_window_days should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tree_query_global"));
    }

    #[tokio::test]
    async fn execute_rejects_wrong_type_for_time_window_days() {
        let tool = MemoryTreeQueryGlobalTool;
        let err = tool
            .execute(json!({"time_window_days": "seven"}))
            .await
            .expect_err("wrong type should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tree_query_global"));
    }

    #[tokio::test]
    async fn execute_accepts_window_days_alias() {
        let tool = MemoryTreeQueryGlobalTool;
        let req: QueryGlobalRequest =
            serde_json::from_value(json!({"window_days": 7})).expect("alias should deserialize");
        assert_eq!(req.time_window_days, 7);

        let result = tool
            .execute(json!({"window_days": 7}))
            .await
            .expect("window_days alias should succeed");
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn execute_success_path_returns_empty_payload_for_isolated_workspace() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeQueryGlobalTool;
        let result = tool
            .execute(json!({"time_window_days": 7}))
            .await
            .expect("valid query_global should succeed in isolated workspace");
        assert!(!result.is_error);
        let payload = result.text();
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("result should be valid json");
        assert!(
            parsed.get("hits").is_some(),
            "payload should include hits array"
        );
        assert!(
            parsed.get("total").is_some(),
            "payload should include total count"
        );
        assert_eq!(parsed["total"], json!(0));
        assert_eq!(parsed["hits"], json!([]));

        let direct = crate::openhuman::memory_tree::retrieval::global::query_global(&cfg, 7)
            .await
            .expect("direct query_global on empty workspace");
        assert_eq!(direct.total, 0);
        assert!(direct.hits.is_empty());
    }
}
