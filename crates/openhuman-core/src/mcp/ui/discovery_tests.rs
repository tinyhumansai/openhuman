use super::*;
use serde_json::json;

#[test]
fn meta_keys_include_ui_members() {
    let keys = meta_keys(&json!({"ui": {"resourceUri": "ui://a", "visibility": ["app"]}, "x": 1}));
    assert!(keys.contains(&"ui".to_string()));
    assert!(keys.contains(&"ui.resourceUri".to_string()));
    assert!(keys.contains(&"x".to_string()));
}

fn remote_tool(value: serde_json::Value) -> tinymcp_bus::McpRemoteTool {
    serde_json::from_value(value).expect("remote tool")
}

#[test]
fn meta_keys_of_a_non_object_are_empty() {
    assert!(meta_keys(&json!("text")).is_empty());
    assert!(meta_keys(&json!(null)).is_empty());
}

#[test]
fn logging_a_tool_does_not_expose_values() {
    log_tool(
        "shop",
        "order",
        &json!({"ui": {"resourceUri": "ui://shop/cart"}}),
    );
}

#[test]
fn configured_server_logs_every_tool_with_meta() {
    let tools = vec![
        remote_tool(json!({
            "name": "order",
            "inputSchema": {"type": "object"},
            "_meta": {"ui": {"resourceUri": "ui://shop/cart"}}
        })),
        remote_tool(json!({"name": "plain", "inputSchema": {"type": "object"}})),
    ];
    log_configured_server("shop", &tools);
}
