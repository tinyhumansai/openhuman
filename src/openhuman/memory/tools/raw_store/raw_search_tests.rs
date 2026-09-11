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
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("OPENHUMAN_WORKSPACE", previous);
            } else {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            }
        }
    }
}

async fn isolated_config(tmp: &TempDir) -> (WorkspaceEnvGuard, Config) {
    let guard = WorkspaceEnvGuard::set(tmp.path());
    let config = Config::load_or_init().await.expect("load config");
    (guard, config)
}

#[test]
fn default_limit_is_five() {
    assert_eq!(default_limit(), 5);
}

#[test]
fn args_deserialize_with_default_limit() {
    let args: Args = serde_json::from_value(json!({ "query": "alice" })).unwrap();
    assert_eq!(args.query, "alice");
    assert_eq!(args.limit, 5);
    assert!(args.kinds.is_none());
}

#[test]
fn parameters_schema_describes_required_query() {
    let tool = MemoryStoreRawSearchTool;
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["query"]));
    assert_eq!(schema["properties"]["limit"]["maximum"], 100);
}

#[tokio::test]
async fn execute_rejects_missing_query() {
    let tool = MemoryStoreRawSearchTool;
    let err = tool
        .execute(json!({}))
        .await
        .expect_err("missing query should fail");
    assert!(err
        .to_string()
        .contains("invalid arguments for memory_store_raw_search"));
}

#[tokio::test]
async fn execute_rejects_invalid_kind() {
    let tool = MemoryStoreRawSearchTool;
    let err = tool
        .execute(json!({
            "query": "alice",
            "kinds": ["not-a-kind"]
        }))
        .await
        .expect_err("invalid kind should fail");
    assert!(err.to_string().contains("memory_store_raw_search:"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads the entity index through the bound driver, not the in-process engine"]
async fn execute_success_path_returns_json_array() {
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _config) = isolated_config(&tmp).await;
    let tool = MemoryStoreRawSearchTool;
    let result = tool
        .execute(json!({
            "query": "alice",
            "limit": 3
        }))
        .await
        .expect("valid raw_search request should succeed");
    assert!(!result.is_error);
    let parsed: serde_json::Value =
        serde_json::from_str(&result.text()).expect("tool result should be json");
    assert!(
        parsed.is_array(),
        "raw_search should serialize a JSON array"
    );
}
