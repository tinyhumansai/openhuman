use super::*;

#[test]
fn every_registered_controller_is_in_the_mcp_ui_namespace() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), all_controller_schemas().len());
    for controller in controllers {
        assert_eq!(controller.schema.namespace, "mcp_ui");
        assert_ne!(controller.schema.function, "unknown");
    }
}

#[test]
fn tool_call_requires_server_and_tool() {
    let schema = schemas("tool_call");
    let required: Vec<_> = schema
        .inputs
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    assert_eq!(required, vec!["server_id", "tool_name"]);
}
