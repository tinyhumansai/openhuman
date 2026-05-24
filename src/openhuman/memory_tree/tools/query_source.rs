use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::retrieval;
use crate::openhuman::memory::retrieval::rpc::QuerySourceRequest;
use crate::openhuman::memory_store::chunks::types::SourceKind;
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
    fn parameters_schema_exposes_supported_source_filters() {
        let tool = MemoryTreeQuerySourceTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(
            schema["properties"]["source_kind"]["enum"],
            json!(["chat", "email", "document"])
        );
        assert_eq!(schema["properties"]["time_window_days"]["minimum"], 0);
    }

    #[tokio::test]
    async fn execute_rejects_invalid_source_kind() {
        let tool = MemoryTreeQuerySourceTool;
        let err = tool
            .execute(json!({
                "source_kind": "not-real"
            }))
            .await
            .expect_err("invalid source kind should fail");
        assert!(err.to_string().contains("memory_tree_query_source:"));
    }

    #[tokio::test]
    async fn execute_rejects_wrong_type_for_limit() {
        let tool = MemoryTreeQuerySourceTool;
        let err = tool
            .execute(json!({
                "limit": "five"
            }))
            .await
            .expect_err("wrong limit type should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tree_query_source"));
    }

    #[tokio::test]
    async fn execute_success_path_returns_empty_payload_for_isolated_workspace() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeQuerySourceTool;
        let result = tool
            .execute(json!({
                "source_kind": "document",
                "limit": 2
            }))
            .await
            .expect("valid query_source should succeed in isolated workspace");
        assert!(!result.is_error);
        let payload = result.text();
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("result should be valid json");
        assert!(parsed.get("hits").is_some(), "payload should include hits");
        assert!(
            parsed.get("total").is_some(),
            "payload should include total"
        );
        assert_eq!(parsed["hits"], json!([]));
        assert_eq!(parsed["total"], json!(0));

        let direct = retrieval::query_source(&cfg, None, Some(SourceKind::Document), None, None, 2)
            .await
            .expect("direct query_source on empty workspace");
        assert!(direct.hits.is_empty());
        assert_eq!(direct.total, 0);
    }

    #[tokio::test]
    async fn execute_accepts_exact_source_id_without_source_kind() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeQuerySourceTool;
        let result = tool
            .execute(json!({
                "source_id": "slack:#eng",
                "limit": 1
            }))
            .await
            .expect("source_id-only query should succeed");
        assert!(!result.is_error);
    }
}
