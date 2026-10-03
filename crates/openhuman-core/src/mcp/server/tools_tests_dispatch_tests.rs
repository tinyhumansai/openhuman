use super::*;

#[tokio::test]
async fn call_tool_records_write_argument_rejection() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let config = config_rpc::load_config_with_timeout()
        .await
        .expect("config");

    let err = call_tool("memory.learn", json!({ "kind": "fact" }), "mcp:test")
        .await
        .expect_err("missing text should reject");
    assert!(
        err.message().contains("missing required argument `text`"),
        "got: {}",
        err.message()
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
    assert!(rows[0]
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("missing required argument `text`"));
    assert!(rows[0].args_summary.get("text").is_none());

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn call_tool_rejects_v1_memory_tools_as_unknown() {
    for name in ["memory.search", "memory.store", "memory.note", "tree.tag"] {
        let err = call_tool(name, json!({}), "mcp:test")
            .await
            .expect_err("removed tool");
        assert!(err.message().contains("unknown MCP tool"), "{name}");
    }
}

#[tokio::test]
async fn call_tool_validates_memory_arguments_before_dispatch() {
    let err = call_tool("memory.forget", json!({ "ids": [] }), "mcp:test")
        .await
        .expect_err("empty ids rejected before any RPC");
    assert!(err.message().contains("`ids`"), "got: {}", err.message());
    let err = call_tool(
        "memory.recall",
        json!({ "question": "q", "filter": { "x": 1 } }),
        "mcp:test",
    )
    .await
    .expect_err("unknown filter field");
    assert!(err.message().contains("unexpected filter field"));
}
