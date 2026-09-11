use super::*;

#[test]
fn summarize_write_args_omits_memory_store_content() {
    let summary = summarize_write_args(
        "memory.store",
        &json!({
            "title": "A".repeat(140),
            "content": "private body",
            "namespace": "work",
            "tags": ["project", "planning"]
        }),
    );
    assert_eq!(summary["title"].as_str().unwrap().chars().count(), 128);
    assert_eq!(summary["namespace"], "work");
    assert_eq!(summary["tag_count"], 2);
    assert!(summary.get("content").is_none());
}

#[test]
fn summarize_write_args_omits_memory_note_text() {
    let summary = summarize_write_args(
        "memory.note",
        &json!({ "chunk_id": "chunk-42", "note_text": "Important context" }),
    );
    assert_eq!(summary["chunk_id"], "chunk-42");
    assert_eq!(
        summary["note_text_length"].as_u64(),
        Some("Important context".chars().count() as u64)
    );
    assert!(summary.get("note_text").is_none());
}

#[test]
fn summarize_write_args_keeps_tree_tag_labels() {
    let summary = summarize_write_args(
        "tree.tag",
        &json!({ "chunk_id": "chunk-42", "tags": ["todo", "q3"] }),
    );
    assert_eq!(summary["chunk_id"], "chunk-42");
    assert_eq!(summary["tags"], json!(["todo", "q3"]));
}

#[test]
fn summarize_rejected_write_args_includes_param_keys_only() {
    let mut params = Map::new();
    params.insert("content".into(), Value::String("private body".into()));
    params.insert("source_type".into(), Value::String("mcp:test".into()));
    params.insert("title".into(), Value::String("T".into()));

    let summary = summarize_rejected_write_args(
        "memory.store",
        &json!({ "title": "T", "content": "private body" }),
        Some(&params),
    );

    assert_eq!(
        summary["param_keys"],
        json!(["content", "source_type", "title"])
    );
    assert!(summary.get("content").is_none());
}

#[test]
fn write_policy_logs_and_returns_denial() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.autonomy.level = crate::openhuman::security::AutonomyLevel::ReadOnly;

    let err = enforce_write_policy_for_config("memory.store", &config)
        .expect_err("read-only mode should deny writes");
    assert!(err.message().contains("read-only mode"));
}

#[tokio::test]
async fn audit_write_rejection_records_failure_row() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    let err = ToolCallError::InvalidParams("bad write request".into());
    audit_write_rejection(
        &config,
        "memory.store",
        &json!({ "title": "T", "content": "private body" }),
        None,
        "mcp:test",
        &err,
    );

    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::openhuman::mcp::audit::list_writes(
            &config,
            &crate::openhuman::mcp::audit::McpWriteListQuery::default(),
        )
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.store");
    assert_eq!(rows[0].client_info, "mcp:test");
    assert_eq!(rows[0].error_message.as_deref(), Some("bad write request"));
    assert!(rows[0].args_summary.get("content").is_none());
}

#[test]
fn extract_document_id_reads_rpc_outcome_envelope() {
    assert_eq!(
        extract_document_id(&json!({"result": {"document_id": "doc-123"}, "logs": []})).as_deref(),
        Some("doc-123")
    );
    assert_eq!(
        extract_document_id(&json!({"document_id": "doc-456"})).as_deref(),
        Some("doc-456")
    );
}

#[test]
fn test_is_write_tool_recognizes_write_tools() {
    assert!(is_write_tool("memory.store"));
    assert!(is_write_tool("memory.note"));
    assert!(is_write_tool("tree.tag"));
    assert!(!is_write_tool("memory.search"));
    assert!(!is_write_tool("core.list_tools"));
    assert!(!is_write_tool("unknown"));
}

#[test]
fn test_now_ms_returns_recent_timestamp() {
    let t = now_ms();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert!((now - t).abs() < 5_000);
}

#[test]
fn test_first_chars_truncates_at_boundary() {
    assert_eq!(first_chars("hello world", 5), "hello");
    assert_eq!(first_chars("hi", 10), "hi");
    assert_eq!(first_chars("", 5), "");
}

#[test]
fn test_summarize_write_args_unknown_tool_returns_empty_object() {
    let result = summarize_write_args("unknown.tool", &json!({"foo": "bar"}));
    assert_eq!(result, json!({}));
}

#[test]
fn test_summarize_write_args_non_object_returns_empty_object() {
    let result = summarize_write_args("memory.store", &json!("not an object"));
    assert_eq!(result, json!({}));
}

#[test]
fn test_extract_document_id_returns_none_for_missing() {
    assert!(extract_document_id(&json!({"other": "field"})).is_none());
    assert!(extract_document_id(&json!({})).is_none());
}

#[test]
fn test_summarize_rejected_write_args_without_params() {
    let result = summarize_rejected_write_args("memory.store", &json!({"title": "T"}), None);
    // When params is None, no "param_keys" field should appear
    assert!(result.get("param_keys").is_none());
}

#[tokio::test]
async fn test_dispatch_write_tool_rpc_error_records_audit_and_returns_tool_error() {
    // When the underlying RPC handler returns an error (Some(Err)), dispatch_write_tool
    // should record a failure audit row and return Ok(tool_error_value) — not propagate Err.
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    // Intentionally omit required fields so the handler returns Some(Err(...))
    let params = serde_json::Map::new();

    let result =
        dispatch_write_tool("memory.store", &params, &json!({}), "mcp:test", &config).await;

    // Returns Ok(tool_error_value) — not an Err
    assert!(result.is_ok());
    let val = result.unwrap();
    // A tool_error response has isError:true and a content array
    assert_eq!(val.get("isError"), Some(&json!(true)));

    // Confirm the failure was audited with success=false
    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::openhuman::mcp::audit::list_writes(
            &config,
            &crate::openhuman::mcp::audit::McpWriteListQuery::default(),
        )
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.store");
    assert!(rows[0].error_message.is_some());
}

#[tokio::test]
async fn test_audit_write_success_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    audit_write(
        &config,
        NewMcpWriteRecord {
            timestamp_ms: now_ms(),
            client_info: "mcp:test".to_string(),
            tool_name: "memory.store".to_string(),
            args_summary: json!({"title": "Test", "namespace": "mcp", "tag_count": 0}),
            resulting_chunk_id: Some("doc-success-1".to_string()),
            success: true,
            error_message: None,
        },
    );

    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::openhuman::mcp::audit::list_writes(
            &config,
            &crate::openhuman::mcp::audit::McpWriteListQuery::default(),
        )
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(rows.len(), 1);
    assert!(rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.store");
    assert_eq!(rows[0].client_info, "mcp:test");
    assert!(rows[0].error_message.is_none());
    assert_eq!(rows[0].resulting_chunk_id.as_deref(), Some("doc-success-1"));
}
