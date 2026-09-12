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
    let msg = err.to_string();
    assert!(
        msg.contains("memory_tree_query_source:") && !msg.contains("load config failed"),
        "expected a source-kind parse error, got: {msg}"
    );
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
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads the summary tree through the bound driver, not the in-process engine"]
async fn execute_success_path_returns_empty_payload_for_isolated_workspace() {
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
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
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads the summary tree through the bound driver, not the in-process engine"]
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
