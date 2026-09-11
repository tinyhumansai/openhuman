use super::*;
use serde_json::json;

#[test]
fn internal_controller_registers_expected_rpc_name() {
    let controllers = all_internal_controllers();
    assert_eq!(controllers.len(), 1);
    assert_eq!(controllers[0].schema.namespace, "mcp_audit");
    assert_eq!(controllers[0].schema.function, "list");
    assert_eq!(controllers[0].rpc_method_name(), "openhuman.mcp_audit_list");
}

#[test]
fn domain_schema_exports_match_internal_controller() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();

    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].namespace, "mcp_audit");
    assert_eq!(controllers.len(), 1);
    assert_eq!(controllers[0].schema.function, schemas[0].function);
}

#[tokio::test]
async fn handle_list_returns_persisted_audit_records() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let config = config_rpc::load_config_with_timeout()
        .await
        .expect("config");
    crate::openhuman::mcp::audit::record_write(
        &config,
        crate::openhuman::mcp::audit::NewMcpWriteRecord {
            timestamp_ms: 10,
            client_info: "mcp:test".into(),
            tool_name: "memory.store".into(),
            args_summary: json!({ "title": "safe" }),
            resulting_chunk_id: Some("chunk-1".into()),
            success: true,
            error_message: None,
        },
    )
    .expect("record write");

    let value = handle_list(Map::new()).await.expect("handle list");
    let records = value["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["tool_name"], "memory.store");
    assert_eq!(records[0]["client_info"], "mcp:test");

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}
