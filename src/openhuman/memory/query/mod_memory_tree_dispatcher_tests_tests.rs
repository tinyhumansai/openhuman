use super::*;
use crate::openhuman::memory::query::test_workspace::isolated_config;
use crate::openhuman::tools::traits::Tool;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn memory_tree_tool_name_is_correct() {
    assert_eq!(MemoryTreeTool.name(), "memory_tree");
}

#[test]
fn memory_tree_schema_requires_mode() {
    let schema = MemoryTreeTool.parameters_schema();
    let required = schema.get("required").and_then(|r| r.as_array()).unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("mode")));
}

#[test]
fn memory_tree_schema_mode_enum_has_all_modes() {
    let schema = MemoryTreeTool.parameters_schema();
    let modes: Vec<&str> = schema
        .get("properties")
        .unwrap()
        .get("mode")
        .unwrap()
        .get("enum")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(modes.contains(&"search_entities"));
    assert!(modes.contains(&"query_source"));
    assert!(modes.contains(&"drill_down"));
    assert!(modes.contains(&"cover_window"));
    assert!(modes.contains(&"fetch_leaves"));
    assert!(modes.contains(&"ingest_document"));
    assert!(modes.contains(&"walk"));
    assert!(modes.contains(&"smart_walk"));
    // Removed with the global/topic trees.
    assert!(!modes.contains(&"query_topic"));
    assert!(!modes.contains(&"query_global"));
}

#[test]
fn memory_tree_schema_exposes_source_window_days() {
    let schema = MemoryTreeTool.parameters_schema();
    let properties = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .unwrap();
    assert!(properties.contains_key("time_window_days"));
}

#[tokio::test]
async fn memory_tree_unknown_mode_returns_error() {
    let result = MemoryTreeTool
        .execute(json!({"mode": "invalid_mode"}))
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("unknown mode"),
        "Expected 'unknown mode' in: {msg}"
    );
}

#[tokio::test]
async fn memory_tree_missing_mode_returns_error() {
    let result = MemoryTreeTool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool now reads the summary tree through the bound driver, not the in-process engine"]
async fn memory_tree_fetch_leaves_mode_dispatches_successfully() {
    // `fetch_leaves` loads config from `OPENHUMAN_WORKSPACE`. Without an
    // isolated workspace this races sibling tests whose `TempDir` is
    // deleted mid-call ("Failed to create temporary config file ... No
    // such file or directory").
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
    let result = MemoryTreeTool
        .execute(json!({
            "mode": "fetch_leaves",
            "chunk_ids": ["chunk-does-not-exist"]
        }))
        .await
        .expect("fetch_leaves mode should dispatch successfully");
    assert!(!result.is_error);
    let parsed: serde_json::Value =
        serde_json::from_str(&result.text()).expect("result should be valid json");
    assert!(parsed.is_array());
}
