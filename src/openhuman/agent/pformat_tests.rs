use super::*;
use crate::openhuman::tools::ToolResult;
use async_trait::async_trait;

struct StubTool(&'static str);

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "stub tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" },
                "count": { "type": "integer" }
            }
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

#[test]
fn build_registry_keys_on_the_tools_own_names() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(StubTool("echo")), Box::new(StubTool("shell"))];
    let reg = build_registry(&tools);
    assert!(reg.contains_key("echo"));
    assert!(reg.contains_key("shell"));
    assert_eq!(reg.len(), 2);
}

#[test]
fn a_tool_absent_from_the_registry_cannot_be_called() {
    // The safety boundary: the parser must not invent argument names for a
    // tool it does not know, or a model could tunnel arbitrary JSON through
    // by guessing a name. This adapter is what decides what is "known".
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(StubTool("echo"))];
    let reg = build_registry(&tools);
    assert!(parse_call("shell[rm -rf /]", &reg).is_none());
}

#[test]
fn a_registered_tool_parses_positionally_through_the_adapter() {
    // End-to-end through the adapter: schema comes from the Tool impl, and
    // arguments come back named and coerced.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(StubTool("echo"))];
    let reg = build_registry(&tools);
    let (name, args) = parse_call("echo[3|hi]", &reg).expect("known tool parses");
    assert_eq!(name, "echo");
    // Schema properties are ordered alphabetically: count, value.
    assert_eq!(args["count"], 3);
    assert_eq!(args["value"], "hi");
}

#[test]
fn signature_rendering_agrees_between_the_tool_and_schema_forms() {
    // The signature goes into the system prompt; if these two disagreed the
    // catalogue would advertise a different argument order than the parser
    // reconstructs.
    let tool = StubTool("echo");
    let from_tool = render_signature_from_tool(&tool);
    let from_schema = render_signature_from_schema("echo", &tool.parameters_schema());
    assert_eq!(from_tool, from_schema);
    assert_eq!(from_tool, "echo[count|value]");
}
