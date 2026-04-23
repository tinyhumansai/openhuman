use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::XmlToolDispatcher;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::context::prompt::SystemPromptBuilder;
use openhuman_core::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use openhuman_core::openhuman::providers::{ChatRequest, ChatResponse, Provider};
use openhuman_core::openhuman::tools::{Tool, ToolResult};
use std::collections::HashSet;
use std::sync::Arc;

struct StubProvider;

#[async_trait]
impl Provider for StubProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("ok".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some("ok".into()),
            tool_calls: Vec::new(),
            usage: None,
        })
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

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: openhuman_core::openhuman::memory::RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(
        &self,
    ) -> Result<Vec<openhuman_core::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "stub"
    }
}

fn base_builder() -> openhuman_core::openhuman::agent::AgentBuilder {
    Agent::builder()
        .provider(Box::new(StubProvider))
        .tools(vec![
            Box::new(StubTool("alpha")),
            Box::new(StubTool("beta")),
        ])
        .memory(Arc::new(StubMemory))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
}

#[test]
fn builder_validates_required_fields() {
    let err = Agent::builder()
        .build()
        .err()
        .expect("missing tools should error");
    assert!(err.to_string().contains("tools are required"));

    let err = Agent::builder()
        .tools(vec![Box::new(StubTool("alpha"))])
        .build()
        .err()
        .expect("missing provider should error");
    assert!(err.to_string().contains("provider is required"));

    let err = Agent::builder()
        .provider(Box::new(StubProvider))
        .tools(vec![Box::new(StubTool("alpha"))])
        .build()
        .err()
        .expect("missing memory should error");
    assert!(err.to_string().contains("memory is required"));

    let err = Agent::builder()
        .provider(Box::new(StubProvider))
        .tools(vec![Box::new(StubTool("alpha"))])
        .memory(Arc::new(StubMemory))
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
    assert_eq!(
        agent.model_name(),
        openhuman_core::openhuman::config::DEFAULT_MODEL
    );
    assert_eq!(agent.temperature(), 0.7);
    assert_eq!(agent.workspace_dir(), std::path::Path::new("."));
    assert!(agent.skills().is_empty());
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
