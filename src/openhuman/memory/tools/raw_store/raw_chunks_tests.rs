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
fn args_deserialize_optional_filters() {
    let args: Args = serde_json::from_value(json!({
        "source_kind": "chat",
        "source_id": "slack:#eng",
        "owner": "alice",
        "since_ms": 10,
        "until_ms": 20,
        "tags_all_of": ["person:alice"],
        "limit": 25
    }))
    .unwrap();

    assert_eq!(args.source_kind.as_deref(), Some("chat"));
    assert_eq!(args.source_id.as_deref(), Some("slack:#eng"));
    assert_eq!(args.owner.as_deref(), Some("alice"));
    assert_eq!(args.since_ms, Some(10));
    assert_eq!(args.until_ms, Some(20));
    assert_eq!(args.tags_all_of, Some(vec!["person:alice".to_string()]));
    assert_eq!(args.limit, Some(25));
}

#[test]
fn parameters_schema_exposes_supported_source_kinds() {
    let tool = MemoryStoreRawChunksTool;
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["properties"]["source_kind"]["enum"],
        json!(["chat", "email", "document"])
    );
    assert_eq!(schema["properties"]["limit"]["maximum"], 1000);
}

#[tokio::test]
async fn execute_rejects_invalid_source_kind() {
    let tool = MemoryStoreRawChunksTool;
    let err = tool
        .execute(json!({
            "source_kind": "not-real"
        }))
        .await
        .expect_err("invalid source kind should fail");
    assert!(err.to_string().contains("memory_store_raw_chunks:"));
}

#[tokio::test]
async fn execute_rejects_wrong_type_for_limit() {
    let tool = MemoryStoreRawChunksTool;
    let err = tool
        .execute(json!({
            "limit": "ten"
        }))
        .await
        .expect_err("wrong limit type should fail");
    assert!(err
        .to_string()
        .contains("invalid arguments for memory_store_raw_chunks"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the module bus belongs to the runtime that creates it, so run this test alone"]
// Was a pure-SQLite test: it opened the workspace store in-process and read
// an empty table. That is the split brain this port removes — the tool now
// reads chunks through the bound driver, so the success path needs a driver
// that advertises the chunk family. With no module artifact the binding
// falls back to the null driver and the tool refuses, which is the correct
// answer rather than a regression.
async fn execute_success_path_returns_json_array() {
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _config) = isolated_config(&tmp).await;
    let tool = MemoryStoreRawChunksTool;
    let result = tool
        .execute(json!({
            "source_kind": "document",
            "limit": 2
        }))
        .await
        .expect("valid raw_chunks request should succeed");
    assert!(!result.is_error);
    let parsed: serde_json::Value =
        serde_json::from_str(&result.text()).expect("tool result should be json");
    assert!(
        parsed.is_array(),
        "raw_chunks should serialize a JSON array"
    );
}
