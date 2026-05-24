//! Unit tests for the MCP audit log insert + readback round-trip.

use std::sync::Mutex;
use tempfile::TempDir;

use super::store::{recent, record_write};
use super::types::McpWriteRecord;
use crate::openhuman::config::Config;

// `tree::store::with_connection` keys its connection cache by DB path,
// so concurrent tests with different `tempdir` paths run independently
// — but within a single test we still want one workspace for clarity.
// The mutex is a defensive guard against cargo running tests in parallel
// against the same Config workspace_dir if a test forgets to use TempDir.
static TEST_GUARD: Mutex<()> = Mutex::new(());

fn test_config() -> (Config, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    (config, tmp)
}

fn sample_record(client: &str, tool: &str, success: bool) -> McpWriteRecord {
    McpWriteRecord {
        timestamp_ms: 1_700_000_000_000,
        client_source_type: client.to_string(),
        tool_name: tool.to_string(),
        args_summary: Some(format!(r#"{{"sample":"{tool}"}}"#)),
        resulting_chunk_id: success.then(|| "doc-abc-123".to_string()),
        success,
        error_message: if success {
            None
        } else {
            Some("doc_put validation failed: missing 'title'".to_string())
        },
    }
}

#[tokio::test]
async fn record_then_recent_round_trips_a_success_row() {
    let _g = TEST_GUARD.lock().unwrap();
    let (config, _tmp) = test_config();

    let record = sample_record("mcp:claude-desktop", "memory.store", true);
    record_write(&config, record.clone())
        .await
        .expect("insert ok");

    let rows = recent(&config, 10).await.expect("query ok");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], record);
}

#[tokio::test]
async fn record_preserves_failure_rows_with_error_message() {
    let _g = TEST_GUARD.lock().unwrap();
    let (config, _tmp) = test_config();

    let record = sample_record("mcp:cursor", "tree.tag", false);
    record_write(&config, record.clone())
        .await
        .expect("insert ok");

    let rows = recent(&config, 10).await.expect("query ok");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert!(rows[0]
        .error_message
        .as_deref()
        .expect("failure row has error_message")
        .contains("doc_put validation failed"));
    assert!(rows[0].resulting_chunk_id.is_none());
}

#[tokio::test]
async fn recent_orders_by_timestamp_descending() {
    let _g = TEST_GUARD.lock().unwrap();
    let (config, _tmp) = test_config();

    let mut newer = sample_record("mcp", "memory.store", true);
    newer.timestamp_ms = 2_000_000_000_000;
    let mut older = sample_record("mcp", "memory.note", true);
    older.timestamp_ms = 1_000_000_000_000;

    // Insert older first so any natural insertion order can't masquerade
    // as correct sorting.
    record_write(&config, older.clone()).await.expect("ok");
    record_write(&config, newer.clone()).await.expect("ok");

    let rows = recent(&config, 10).await.expect("query ok");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].timestamp_ms, 2_000_000_000_000);
    assert_eq!(rows[1].timestamp_ms, 1_000_000_000_000);
}

#[tokio::test]
async fn recent_honours_limit() {
    let _g = TEST_GUARD.lock().unwrap();
    let (config, _tmp) = test_config();

    for i in 0..5 {
        let mut r = sample_record("mcp", "memory.store", true);
        r.timestamp_ms = 1_000_000_000_000 + i;
        record_write(&config, r).await.expect("ok");
    }

    let rows = recent(&config, 3).await.expect("query ok");
    assert_eq!(rows.len(), 3);
    // Most recent three only — timestamps 4, 3, 2.
    assert_eq!(rows[0].timestamp_ms, 1_000_000_000_004);
    assert_eq!(rows[2].timestamp_ms, 1_000_000_000_002);
}

#[tokio::test]
async fn record_truncates_oversize_error_message_at_char_boundary() {
    let _g = TEST_GUARD.lock().unwrap();
    let (config, _tmp) = test_config();

    // 2 KiB error message — must be capped at 1 KiB on insert.
    let oversize_error = "a".repeat(2048);
    let mut record = sample_record("mcp", "memory.note", false);
    record.error_message = Some(oversize_error);

    record_write(&config, record).await.expect("insert ok");
    let rows = recent(&config, 1).await.expect("query ok");
    assert_eq!(rows.len(), 1);
    let stored = rows[0]
        .error_message
        .as_ref()
        .expect("error_message stored");
    assert!(stored.len() <= 1024, "got {} bytes", stored.len());
}

#[tokio::test]
async fn record_handles_multibyte_truncation_safely() {
    let _g = TEST_GUARD.lock().unwrap();
    let (config, _tmp) = test_config();

    // A 4-byte UTF-8 char repeated past the 1024-byte cap. Naive byte
    // truncation would slice mid-codepoint and produce invalid UTF-8;
    // our `is_char_boundary` walk-back guards against that.
    let multibyte_char = "🦀"; // 4 bytes
    let oversize_error = multibyte_char.repeat(300); // 1200 bytes, well past cap
    let mut record = sample_record("mcp", "memory.note", false);
    record.error_message = Some(oversize_error);

    record_write(&config, record).await.expect("insert ok");
    let rows = recent(&config, 1).await.expect("query ok");
    let stored = rows[0]
        .error_message
        .as_ref()
        .expect("error_message stored");
    // Must round-trip as valid UTF-8 — implicit because `String` was
    // built from a `str` slice on a char boundary.
    assert!(stored.is_char_boundary(stored.len()));
    assert!(stored.len() <= 1024);
}
