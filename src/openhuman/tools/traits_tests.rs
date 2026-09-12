use super::*;
use async_trait::async_trait;

struct DummyTool;

#[async_trait]
impl Tool for DummyTool {
    fn name(&self) -> &str {
        "dummy_tool"
    }

    fn description(&self) -> &str {
        "A deterministic test tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "value": { "type": "string" } }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let text = args
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(ToolResult::success(text))
    }
}

#[tokio::test]
async fn a_tool_written_against_this_path_satisfies_the_shared_trait() {
    // The point of the re-export: `dyn Tool` here is `dyn tinytools::Tool`,
    // which is what the harness accepts. If these ever became two traits,
    // this coercion is what would stop compiling.
    let erased: &dyn tinytools::Tool = &DummyTool;
    let result = erased
        .execute(serde_json::json!({ "value": "hello-tool" }))
        .await
        .expect("the tool runs");
    assert_eq!(result.output(), "hello-tool");
    assert_eq!(erased.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(erased.scope(), ToolScope::All);
    assert_eq!(erased.category(), ToolCategory::System);
}

#[test]
fn a_tool_carrying_no_host_extension_yields_none() {
    let tool = DummyTool;
    assert!(pack_registry_handle(&tool).is_none());
    assert!(generated_runtime_context(&tool, &serde_json::Value::Null).is_none());
}

#[test]
fn spec_uses_tool_metadata_and_schema() {
    let spec = DummyTool.spec();
    assert_eq!(spec.name, "dummy_tool");
    assert_eq!(spec.description, "A deterministic test tool");
    assert_eq!(spec.parameters["type"], "object");
}
