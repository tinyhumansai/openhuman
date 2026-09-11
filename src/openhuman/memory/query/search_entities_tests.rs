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
fn parameters_schema_requires_query() {
    let tool = MemoryTreeSearchEntitiesTool;
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["query"]));
    assert_eq!(
        schema["properties"]["limit"]["description"].is_string(),
        true
    );
}

#[test]
fn kind_enum_contains_expected_memory_entity_kinds() {
    let tool = MemoryTreeSearchEntitiesTool;
    let schema = tool.parameters_schema();
    let kinds = schema["properties"]["kinds"]["items"]["enum"]
        .as_array()
        .unwrap();
    for required in ["email", "person", "organization", "topic"] {
        assert!(
            kinds.iter().any(|v| v == required),
            "missing kind {required}"
        );
    }
}

#[tokio::test]
async fn execute_rejects_missing_query() {
    let tool = MemoryTreeSearchEntitiesTool;
    let err = tool
        .execute(json!({}))
        .await
        .expect_err("missing query should fail");
    assert!(err
        .to_string()
        .contains("invalid arguments for memory_tree_search_entities"));
}

/// An unknown `kinds` value is refused — by the **driver**, not the host.
///
/// This used to assert that validation happened before any workspace was
/// touched, because the host owned a closed copy of the engine's
/// `EntityKind`. It no longer does: the vocabulary is open on the wire and
/// the driver is its authority. With no module artifact bound, the failure
/// now surfaces as the driver being unable to serve the family, which is
/// still a refusal of the same request — but it is a weaker guarantee than
/// the pure-function check it replaced, so it is called out rather than
/// quietly relaxed.
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
kind validation moved into the driver with the open entity-kind vocabulary"]
async fn execute_rejects_invalid_kind_after_validation() {
    let tool = MemoryTreeSearchEntitiesTool;
    let err = tool
        .execute(json!({
            "query": "alice",
            "kinds": ["not-a-real-kind"]
        }))
        .await
        .expect_err("invalid kind should fail");
    assert!(err
        .to_string()
        .contains("memory_tree_search_entities: invalid kind:"));
}

/// The parity half of this test is gone with the split brain.
///
/// It used to run the tool and then call `retrieval::search_entities`
/// directly on the same workspace, asserting both saw an empty store. That
/// second call is exactly the in-process engine access this port removes —
/// there is no longer a second reader to agree with.
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads entities through the bound driver, not the in-process engine"]
async fn execute_success_path_returns_empty_json_array_for_isolated_workspace() {
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
    let tool = MemoryTreeSearchEntitiesTool;
    let result = tool
        .execute(json!({
            "query": "alice",
            "limit": 3
        }))
        .await
        .expect("valid search_entities request should succeed in isolated workspace");
    assert!(!result.is_error);
    let payload = result.text();
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).expect("result should be valid json");
    assert!(
        parsed.is_array(),
        "search_entities should serialize a JSON array"
    );
    assert_eq!(parsed, json!([]));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads entities through the bound driver, not the in-process engine"]
async fn execute_accepts_kind_filter_and_clamps_large_limit() {
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
    let tool = MemoryTreeSearchEntitiesTool;
    let result = tool
        .execute(json!({
            "query": "alice",
            "kinds": ["email", "person"],
            "limit": 999
        }))
        .await
        .expect("filtered search_entities request should succeed");
    assert!(!result.is_error);
}
