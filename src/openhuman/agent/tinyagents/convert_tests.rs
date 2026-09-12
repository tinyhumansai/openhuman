use super::*;

#[test]
fn converts_tool_schema() {
    let spec = ToolSpec {
        name: "echo".into(),
        description: "echoes".into(),
        parameters: serde_json::json!({"type": "object"}),
    };
    let schema = spec_to_schema(&spec);
    assert_eq!(schema.name, "echo");
    assert_eq!(schema.parameters, serde_json::json!({"type": "object"}));
}
