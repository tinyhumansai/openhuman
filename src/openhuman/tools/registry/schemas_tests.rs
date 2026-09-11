use super::*;
use serde_json::json;

#[test]
fn schemas_cover_registered_controllers() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();

    assert_eq!(schemas.len(), 3);
    assert_eq!(controllers.len(), 3);
    assert_eq!(schemas[0].function, controllers[0].schema.function);
    assert_eq!(schemas[1].function, controllers[1].schema.function);
    assert_eq!(schemas[2].function, controllers[2].schema.function);
}

#[test]
fn list_schema_has_no_inputs() {
    let schema = schemas("list");
    assert_eq!(schema.namespace, "tool_registry");
    assert_eq!(schema.function, "list");
    assert!(schema.inputs.is_empty());
    assert_eq!(schema.outputs[0].name, "tools");
}

#[test]
fn get_schema_requires_tool_id() {
    let schema = schemas("get");
    assert_eq!(schema.inputs[0].name, "tool_id");
    assert!(schema.inputs[0].required);
}

#[test]
fn diagnostics_schema_has_no_inputs() {
    let schema = schemas("diagnostics");
    assert_eq!(schema.namespace, "tool_registry");
    assert_eq!(schema.function, "diagnostics");
    assert!(schema.inputs.is_empty());
    assert_eq!(schema.outputs[0].name, "diagnostics");
}

#[test]
fn required_tool_id_rejects_wrong_type() {
    let mut params = Map::new();
    params.insert("tool_id".to_string(), json!(10));

    let err = required_tool_id(&params).expect_err("numeric id should fail");
    assert!(err.contains("non-empty string"));
}

#[tokio::test]
async fn handle_list_returns_registry_object() {
    let value = handle_list(Map::new()).await.expect("list json");
    let tools = value
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools array");

    // `memory.search` is an MCP-transport entry, absent when the `mcp`
    // feature is compiled out. The behaviour under test is that `list`
    // returns a populated registry object, so assert against an entry that
    // exists in the build at hand rather than gating the test away.
    #[cfg(feature = "mcp")]
    let expected = "memory.search";
    #[cfg(not(feature = "mcp"))]
    let expected = "tools.web_search";

    assert!(tools
        .iter()
        .any(|tool| { tool.get("tool_id").and_then(Value::as_str) == Some(expected) }));
}

#[tokio::test]
async fn handle_get_returns_one_registry_entry() {
    let mut params = Map::new();
    params.insert("tool_id".to_string(), json!("tools.web_search"));

    let value = handle_get(params).await.expect("get json");
    assert_eq!(
        value.get("tool_id").and_then(Value::as_str),
        Some("tools.web_search")
    );
}

#[tokio::test]
async fn handle_diagnostics_returns_counts() {
    let value = handle_diagnostics(Map::new())
        .await
        .expect("diagnostics json");
    let diagnostics = value.get("diagnostics").unwrap_or(&value);
    assert!(diagnostics
        .get("total_tools")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
    assert!(diagnostics
        .get("policy_surfaces")
        .and_then(Value::as_array)
        .is_some());

    // Expanded diagnostics surface for #2136.
    let posture = diagnostics
        .get("posture")
        .and_then(Value::as_object)
        .expect("posture");
    assert!(posture
        .get("autonomy_level")
        .and_then(Value::as_str)
        .is_some());
    assert!(posture
        .get("workspace_only")
        .and_then(Value::as_bool)
        .is_some());

    let mcp_allowlists = diagnostics
        .get("mcp_allowlists")
        .and_then(Value::as_object)
        .expect("mcp_allowlists");
    assert!(mcp_allowlists
        .get("server_count")
        .and_then(Value::as_u64)
        .is_some());

    let mcp_write_audit = diagnostics
        .get("mcp_write_audit")
        .and_then(Value::as_object)
        .expect("mcp_write_audit");
    assert!(mcp_write_audit
        .get("enabled")
        .and_then(Value::as_bool)
        .is_some());

    assert!(diagnostics
        .get("recent_denials")
        .and_then(Value::as_array)
        .is_some());
}
