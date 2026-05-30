use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::query::backend;
use crate::openhuman::memory_tree::retrieval::rpc::FetchLeavesRequest;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Hard cap on `chunk_ids` enforced at the tool boundary so the tool's
/// behaviour matches the schema description. The retrieval RPC also
/// truncates internally; we mirror that here so excess ids are dropped
/// rather than silently passed through.
const MAX_CHUNK_IDS_PER_CALL: usize = 20;

pub struct MemoryTreeFetchLeavesTool;

#[async_trait]
impl Tool for MemoryTreeFetchLeavesTool {
    fn name(&self) -> &str {
        "memory_tree_fetch_leaves"
    }

    fn description(&self) -> &str {
        "Batch-fetch raw chunk rows by id (max 20 per call). Use this when \
         you need verbatim content for a citation — the `content` and \
         `source_ref` fields on each hit are the authoritative quote source."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chunk_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Chunk ids to hydrate. Capped at 20 per call."
                }
            },
            "required": ["chunk_ids"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let req: FetchLeavesRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_fetch_leaves: {e}"))?;
        log::debug!(
            "[rpc][memory_tree] fetch_leaves invoked requested_ids={}",
            req.chunk_ids.len()
        );
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_fetch_leaves: load config failed: {e}"))?;
        let take = req.chunk_ids.len().min(MAX_CHUNK_IDS_PER_CALL);
        if req.chunk_ids.len() > MAX_CHUNK_IDS_PER_CALL {
            log::debug!(
                "[rpc][memory_tree] fetch_leaves truncating requested_ids={} truncated_to={}",
                req.chunk_ids.len(),
                MAX_CHUNK_IDS_PER_CALL
            );
        }
        let hits = backend::fetch_leaves(&cfg, &req.chunk_ids[..take]).await?;
        log::debug!(
            "[rpc][memory_tree] fetch_leaves completed hits={}",
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
    fn parameters_schema_requires_chunk_ids() {
        let tool = MemoryTreeFetchLeavesTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], json!(["chunk_ids"]));
        assert_eq!(schema["properties"]["chunk_ids"]["type"], "array");
    }

    #[test]
    fn max_chunk_ids_per_call_matches_description() {
        assert_eq!(MAX_CHUNK_IDS_PER_CALL, 20);
    }

    #[test]
    fn request_slice_is_truncated_to_cap() {
        let ids: Vec<String> = (0..25).map(|i| format!("chunk-{i}")).collect();
        let take = ids.len().min(MAX_CHUNK_IDS_PER_CALL);
        assert_eq!(take, 20);
        assert_eq!(ids[..take].len(), 20);
        assert_eq!(ids[..take].first().map(String::as_str), Some("chunk-0"));
        assert_eq!(ids[..take].last().map(String::as_str), Some("chunk-19"));
    }

    #[tokio::test]
    async fn execute_rejects_missing_chunk_ids() {
        let tool = MemoryTreeFetchLeavesTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("missing chunk_ids should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tree_fetch_leaves"));
    }

    #[tokio::test]
    async fn execute_rejects_wrong_type_for_chunk_ids() {
        let tool = MemoryTreeFetchLeavesTool;
        let err = tool
            .execute(json!({"chunk_ids": "not-an-array"}))
            .await
            .expect_err("wrong chunk_ids type should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tree_fetch_leaves"));
    }

    #[tokio::test]
    async fn execute_success_path_returns_empty_json_array_for_isolated_workspace() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeFetchLeavesTool;
        let result = tool
            .execute(json!({
                "chunk_ids": ["chunk-does-not-exist-1", "chunk-does-not-exist-2"]
            }))
            .await
            .expect("valid fetch_leaves request should succeed in isolated workspace");
        assert!(!result.is_error);
        let payload = result.text();
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("result should be valid json");
        assert!(
            parsed.is_array(),
            "fetch_leaves should serialize a JSON array"
        );
        assert_eq!(parsed, json!([]));

        let direct = crate::openhuman::memory_tree::retrieval::fetch::fetch_leaves(
            &cfg,
            &[
                "chunk-does-not-exist-1".to_string(),
                "chunk-does-not-exist-2".to_string(),
            ],
        )
        .await
        .expect("direct fetch_leaves on empty workspace");
        assert!(direct.is_empty());
    }

    #[tokio::test]
    async fn execute_truncates_requests_to_twenty_ids() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeFetchLeavesTool;
        let ids: Vec<String> = (0..25).map(|i| format!("chunk-{i}")).collect();
        let result = tool
            .execute(json!({ "chunk_ids": ids }))
            .await
            .expect("over-cap request should still succeed");
        assert!(!result.is_error);
        let parsed: serde_json::Value =
            serde_json::from_str(&result.text()).expect("result should be valid json");
        assert_eq!(parsed, json!([]));
    }
}
