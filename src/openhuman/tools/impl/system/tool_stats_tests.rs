//! The tool resolves the ambient guarded driver per call, so these bind the
//! shared test workspace and write through that same guard rather than
//! handing the tool a mock. Serialised on the global memory lock because
//! the binding is process-wide.

use super::*;
use crate::openhuman::agent::learning::tool_tracker::ToolStats;
use crate::openhuman::memory::ops::{shared_memory_test_workspace, GLOBAL_MEMORY_TEST_LOCK};
use serde_json::json;

fn make_tool() -> ToolStatsTool {
    ToolStatsTool::new()
}

/// Writes one `ToolStats` row through the guard the tool will read.
async fn record(tool_key: &str, stats: &ToolStats) {
    let guard = active_memory_guard().await.expect("guard resolves");
    guard
        .store(
            "tool_effectiveness",
            tool_key,
            &serde_json::to_string(stats).unwrap(),
            MemoryCategory::Custom("tool_effectiveness".into()),
            None,
            tinymemory_api::types::MemoryTaint::Internal,
        )
        .await
        .unwrap();
}

#[test]
fn name_is_correct() {
    assert_eq!(make_tool().name(), "tool_stats");
}

#[test]
fn description_is_non_empty() {
    assert!(!make_tool().description().is_empty());
}

#[test]
fn schema_is_object_type() {
    let schema = make_tool().parameters_schema();
    assert_eq!(schema["type"], "object");
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn returns_stats_for_a_recorded_tool() {
    let _serial = GLOBAL_MEMORY_TEST_LOCK.lock().await;
    shared_memory_test_workspace();

    record(
        "tool/shell",
        &ToolStats {
            total_calls: 5,
            successes: 4,
            failures: 1,
            avg_duration_ms: 120.0,
            common_error_patterns: vec![],
        },
    )
    .await;

    let result = make_tool().execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    let out = result.output();
    assert!(out.contains("shell"), "got: {out}");
    assert!(out.contains("Calls: 5"), "got: {out}");
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn filter_by_tool_name_reports_no_data_for_an_unrecorded_tool() {
    let _serial = GLOBAL_MEMORY_TEST_LOCK.lock().await;
    shared_memory_test_workspace();

    record(
        "tool/shell",
        &ToolStats {
            total_calls: 1,
            successes: 1,
            failures: 0,
            avg_duration_ms: 50.0,
            common_error_patterns: vec![],
        },
    )
    .await;

    let result = make_tool()
        .execute(json!({"tool_name": "file_read"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result
        .output()
        .contains("No effectiveness data recorded for tool 'file_read'"));
}
