use super::*;

#[test]
fn summarize_write_args_omits_memory_learn_text() {
    let summary = summarize_write_args(
        "memory.learn",
        &json!({ "text": "private preference", "kind": "preference", "confidence": 0.9 }),
    );
    assert_eq!(
        summary["text_length"].as_u64(),
        Some("private preference".chars().count() as u64)
    );
    assert_eq!(summary["kind"], "preference");
    assert!(summary.get("text").is_none());
    let default_kind = summarize_write_args("memory.learn", &json!({ "text": "t" }));
    assert_eq!(default_kind["kind"], "fact");
}

#[test]
fn summarize_write_args_counts_forget_ids_without_listing_them() {
    let summary = summarize_write_args("memory.forget", &json!({ "ids": ["a", "b", "c"] }));
    assert_eq!(summary["id_count"], 3);
    assert!(summary.get("ids").is_none());
}

#[test]
fn summarize_rejected_write_args_includes_param_keys_only() {
    let mut params = Map::new();
    params.insert("text".into(), Value::String("private body".into()));
    params.insert("kind".into(), Value::String("fact".into()));

    let summary = summarize_rejected_write_args(
        "memory.learn",
        &json!({ "text": "private body", "kind": "fact" }),
        Some(&params),
    );

    assert_eq!(summary["param_keys"], json!(["kind", "text"]));
    assert!(summary.get("text").is_none());
}

#[test]
fn write_policy_logs_and_returns_denial() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    // The tier only binds with the policy on; the shipped config default is off.
    config.autonomy.enabled = true;
    config.autonomy.level = crate::security::AutonomyLevel::ReadOnly;

    let err = enforce_write_policy_for_config("memory.forget", &config)
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
        "memory.learn",
        &json!({ "text": "private body" }),
        None,
        "mcp:test",
        &err,
    );

    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::mcp::audit::list_writes(
            &config,
            &crate::mcp::audit::McpWriteListQuery::default(),
        )
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.learn");
    assert_eq!(rows[0].client_info, "mcp:test");
    assert_eq!(rows[0].error_message.as_deref(), Some("bad write request"));
    assert!(rows[0].args_summary.get("text").is_none());
}

#[test]
fn extract_item_id_reads_rpc_outcome_envelope() {
    assert_eq!(
        extract_item_id(&json!({"result": {"id": "item-123"}, "logs": []})).as_deref(),
        Some("item-123")
    );
    assert_eq!(
        extract_item_id(&json!({"id": "item-456"})).as_deref(),
        Some("item-456")
    );
}

#[test]
fn test_is_write_tool_recognizes_write_tools() {
    assert!(is_write_tool("memory.learn"));
    assert!(is_write_tool("memory.forget"));
    assert!(!is_write_tool("memory.recall"));
    assert!(!is_write_tool("memory.fetch"));
    assert!(!is_write_tool("memory.list"));
    assert!(!is_write_tool("memory.store"));
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
fn test_summarize_write_args_unknown_tool_returns_empty_object() {
    let result = summarize_write_args("unknown.tool", &json!({"foo": "bar"}));
    assert_eq!(result, json!({}));
}

#[test]
fn test_summarize_write_args_non_object_returns_empty_object() {
    let result = summarize_write_args("memory.learn", &json!("not an object"));
    assert_eq!(result, json!({}));
}

#[test]
fn test_extract_item_id_returns_none_for_missing() {
    assert!(extract_item_id(&json!({"forgotten": 2})).is_none());
    assert!(extract_item_id(&json!({})).is_none());
}

#[test]
fn test_summarize_rejected_write_args_without_params() {
    let result = summarize_rejected_write_args("memory.learn", &json!({"text": "T"}), None);
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

    // Omit the required `text` so the handler (or the missing registration)
    // yields a failure instead of a stored item.
    let params = serde_json::Map::new();

    let result = dispatch_write_tool(
        "memory.learn",
        "openhuman.memory_learn",
        &params,
        &json!({}),
        "mcp:test",
        &config,
    )
    .await;

    // Returns Ok(tool_error_value) — not an Err
    assert!(result.is_ok());
    let val = result.unwrap();
    // A tool_error response has isError:true and a content array
    assert_eq!(val.get("isError"), Some(&json!(true)));

    // Confirm the failure was audited with success=false
    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::mcp::audit::list_writes(
            &config,
            &crate::mcp::audit::McpWriteListQuery::default(),
        )
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.learn");
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
            tool_name: "memory.learn".to_string(),
            args_summary: json!({"text_length": 4, "kind": "fact"}),
            resulting_chunk_id: Some("doc-success-1".to_string()),
            success: true,
            error_message: None,
        },
    );

    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::mcp::audit::list_writes(
            &config,
            &crate::mcp::audit::McpWriteListQuery::default(),
        )
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(rows.len(), 1);
    assert!(rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.learn");
    assert_eq!(rows[0].client_info, "mcp:test");
    assert!(rows[0].error_message.is_none());
    assert_eq!(rows[0].resulting_chunk_id.as_deref(), Some("doc-success-1"));
}

#[tokio::test]
async fn test_dispatch_write_tool_unregistered_rpc_is_a_tool_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    let val = dispatch_write_tool(
        "memory.forget",
        "openhuman.memory_no_such_method",
        &Map::new(),
        &json!({ "ids": ["a"] }),
        "mcp:test",
        &config,
    )
    .await
    .expect("tool error, not a protocol error");
    assert_eq!(val.get("isError"), Some(&json!(true)));
    assert!(val["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("is not registered"));
}
