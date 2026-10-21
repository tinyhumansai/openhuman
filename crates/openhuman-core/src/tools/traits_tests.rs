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
fn a_tool_carrying_no_host_extension_has_no_delegation_target() {
    let tool = DummyTool;
    assert!(delegation_target(&tool).is_none());
}

#[test]
fn spec_uses_tool_metadata_and_schema() {
    let spec = DummyTool.spec();
    assert_eq!(spec.name, "dummy_tool");
    assert_eq!(spec.description, "A deterministic test tool");
    assert_eq!(spec.parameters["type"], "object");
}

#[test]
fn tool_call_id_reads_the_harness_execution_context() {
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::ids::CallId;
    use tinyagents_harness::tool::ToolExecutionContext;

    let run_ctx: RunContext = RunContext::new(RunConfig::new("test-run"), ());
    let exec_ctx = ToolExecutionContext::from_run_context(&run_ctx, CallId::new("call-42"));

    let erased: &dyn tinytools::ToolRunContext = &exec_ctx;
    assert_eq!(tool_call_id(Some(erased)), Some("call-42".to_string()));
}

#[test]
fn tool_call_id_is_none_without_a_context() {
    assert_eq!(tool_call_id(None), None);
}
