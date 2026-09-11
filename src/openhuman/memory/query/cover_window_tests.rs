use super::*;
use crate::openhuman::tools::traits::Tool;
use serde_json::json;

#[test]
fn parameters_schema_requires_window_bounds() {
    let schema = MemoryTreeCoverWindowTool.parameters_schema();
    let required = schema.get("required").and_then(|r| r.as_array()).unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("since_ms")));
    assert!(required.iter().any(|v| v.as_str() == Some("until_ms")));
}

#[tokio::test]
async fn execute_rejects_missing_window_bounds() {
    let err = MemoryTreeCoverWindowTool
        .execute(json!({ "source_kind": "chat" }))
        .await
        .expect_err("missing since_ms/until_ms should fail");
    assert!(err
        .to_string()
        .contains("invalid arguments for memory_tree_cover_window"));
}

#[tokio::test]
async fn execute_rejects_invalid_source_kind() {
    let err = MemoryTreeCoverWindowTool
        .execute(json!({ "since_ms": 0, "until_ms": 1, "source_kind": "not-real" }))
        .await
        .expect_err("invalid source kind should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("memory_tree_cover_window:") && !msg.contains("load config failed"),
        "expected a source-kind parse error, got: {msg}"
    );
}
