use super::*;

#[test]
fn name_and_permission() {
    let tool = UpdateCheckTool::new();
    assert_eq!(tool.name(), "update_check");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn schema_is_closed_object_with_no_properties() {
    let schema = UpdateCheckTool::new().parameters_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    let props = schema["properties"].as_object().expect("properties object");
    assert!(
        props.is_empty(),
        "update_check takes no arguments — schema must not advertise any"
    );
}

#[test]
fn description_mentions_safety_and_companion_tool() {
    let tool = UpdateCheckTool::new();
    let desc = tool.description();
    assert!(desc.contains("Read-only"));
    assert!(desc.contains("update_apply"));
    assert!(desc.contains("Does NOT download"));
}

#[test]
fn default_constructs_same_as_new() {
    let a = UpdateCheckTool::default();
    let b = UpdateCheckTool::new();
    assert_eq!(a.name(), b.name());
}
