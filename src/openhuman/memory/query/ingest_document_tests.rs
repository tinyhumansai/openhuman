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
    // `list_chunks_rpc` reads through the bound driver now, and a unit-test
    // workspace binds the null one — which serves no Chunks family and
    // therefore answers empty however many rows the ingest wrote. Bind the
    // in-process driver over this same workspace so the read sees the
    // write; it is the driver the loadable module wraps, which is as close
    // to production as a test process can get (a dlopen'ed module is a
    // process singleton a unit test cannot load).
    crate::openhuman::memory::test_support::install_memory_driver_for_test(&config);
    (guard, config)
}

#[test]
fn parameters_schema_requires_title_body_and_source_id() {
    let tool = MemoryTreeIngestDocumentTool;
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["title", "body", "source_id"]));
    assert_eq!(schema["properties"]["provider"]["type"], "string");
}

#[test]
fn missing_required_fields_produce_none_via_json_accessors() {
    let value = json!({
        "title": "Doc title",
        "body": "Body"
    });
    assert_eq!(value.get("source_id").and_then(|v| v.as_str()), None);
}

#[test]
fn source_kind_document_string_is_expected() {
    assert_eq!(SourceKind::Document.as_str(), "document");
}

#[tokio::test]
async fn execute_rejects_missing_title_before_config_load() {
    let tool = MemoryTreeIngestDocumentTool;
    let err = tool
        .execute(json!({
            "body": "Body text",
            "source_id": "doc-1"
        }))
        .await
        .expect_err("missing title should fail");
    assert!(err
        .to_string()
        .contains("ingest_document: missing required field `title`"));
}

#[tokio::test]
async fn execute_rejects_missing_body_before_config_load() {
    let tool = MemoryTreeIngestDocumentTool;
    let err = tool
        .execute(json!({
            "title": "Doc title",
            "source_id": "doc-1"
        }))
        .await
        .expect_err("missing body should fail");
    assert!(err
        .to_string()
        .contains("ingest_document: missing required field `body`"));
}

#[tokio::test]
async fn execute_rejects_missing_source_id_before_config_load() {
    let tool = MemoryTreeIngestDocumentTool;
    let err = tool
        .execute(json!({
            "title": "Doc title",
            "body": "Body text"
        }))
        .await
        .expect_err("missing source_id should fail");
    assert!(err
        .to_string()
        .contains("ingest_document: missing required field `source_id`"));
}

#[tokio::test]
async fn execute_rejects_blank_required_fields() {
    let tool = MemoryTreeIngestDocumentTool;
    let result = tool
        .execute(json!({
            "title": "   ",
            "body": "Body text",
            "source_id": "doc-1"
        }))
        .await
        .expect("blank title should return ToolResult error, not anyhow failure");
    assert!(result.is_error);
    assert_eq!(
        result.text(),
        "ingest_document: title, body, and source_id must be non-empty"
    );

    let result = tool
        .execute(json!({
            "title": "Doc title",
            "body": "   ",
            "source_id": "doc-1"
        }))
        .await
        .expect("blank body should return ToolResult error");
    assert!(result.is_error);

    let result = tool
        .execute(json!({
            "title": "Doc title",
            "body": "Body text",
            "source_id": "   "
        }))
        .await
        .expect("blank source_id should return ToolResult error");
    assert!(result.is_error);
}
