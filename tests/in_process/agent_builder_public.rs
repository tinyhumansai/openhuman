use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::agent::prompts::SystemPromptBuilder;
use openhuman_core::agent::OpenHumanSessionHost;
use tinytools::{Tool, ToolResult};
use tinytools_agent::dialect::XmlDialect;

use std::collections::HashSet;
use std::sync::Arc;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

struct StubModel;

#[async_trait]
impl ChatModel<()> for StubModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse::assistant("ok"))
    }
}

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
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success(args.to_string()))
    }
}

fn base_builder() -> openhuman_core::agent::SessionHostBuilder {
    OpenHumanSessionHost::builder()
        .chat_model(Arc::new(StubModel))
        .tools(vec![
            Box::new(StubTool("alpha")),
            Box::new(StubTool("beta")),
        ])
        .tool_dispatcher(Box::new(XmlDialect))
}

#[test]
fn builder_validates_required_fields() {
    let err = OpenHumanSessionHost::builder()
        .build()
        .err()
        .expect("missing tools should error");
    assert!(err.to_string().contains("tools are required"));

    let err = OpenHumanSessionHost::builder()
        .tools(vec![Box::new(StubTool("alpha"))])
        .build()
        .err()
        .expect("missing provider should error");
    assert!(err.to_string().contains("provider is required"));

    let err = OpenHumanSessionHost::builder()
        .chat_model(Arc::new(StubModel))
        .tools(vec![Box::new(StubTool("alpha"))])
        .build()
        .err()
        .expect("missing dispatcher should error");
    assert!(err.to_string().contains("tool_dispatcher is required"));
}

#[test]
fn builder_applies_defaults_and_exposes_public_accessors() {
    let agent = base_builder()
        .build()
        .expect("minimal builder should succeed");

    assert_eq!(agent.tools().len(), 2);
    assert_eq!(agent.tool_specs().len(), 2);
    assert_eq!(agent.model_name(), openhuman_core::config::DEFAULT_MODEL);
    assert_eq!(agent.temperature(), 0.7);
    assert_eq!(agent.workspace_dir(), std::path::Path::new("."));
    assert!(agent.workflows().is_empty());
    assert!(agent.history().is_empty());
    assert_eq!(agent.agent_config().max_tool_iterations, 10);
}

#[test]
fn builder_filters_visible_tools_and_keeps_full_registry() {
    let agent = base_builder()
        .visible_tool_names(HashSet::from_iter(["beta".to_string()]))
        .model_name("model-x".into())
        .temperature(0.4)
        .workspace_dir(std::path::PathBuf::from("/tmp/agent-builder-visible"))
        .prompt_builder(SystemPromptBuilder::with_defaults())
        .event_context("session-9", "cli")
        .agent_definition_name("orchestrator")
        .build()
        .expect("builder should succeed");

    assert_eq!(agent.tools().len(), 2);
    assert_eq!(agent.tool_specs().len(), 2);
    assert_eq!(agent.model_name(), "model-x");
    assert_eq!(agent.temperature(), 0.4);
    assert_eq!(
        agent.workspace_dir(),
        std::path::Path::new("/tmp/agent-builder-visible")
    );
}
