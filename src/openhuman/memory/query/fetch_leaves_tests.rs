use super::*;
use std::ffi::OsString;

use tempfile::TempDir;

use crate::openhuman::config::Config;
use crate::openhuman::config::TEST_ENV_LOCK;
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
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads the summary tree through the bound driver, not the in-process engine"]
async fn execute_success_path_returns_empty_json_array_for_isolated_workspace() {
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
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
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads the summary tree through the bound driver, not the in-process engine"]
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
