use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::providers::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ToolCall,
};
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;

struct MockCalendarProvider {
    captured_messages: Arc<Mutex<Vec<ChatMessage>>>,
    iter_count: Arc<Mutex<usize>>,
}

#[async_trait]
impl Provider for MockCalendarProvider {
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
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        let mut count = self.iter_count.lock();
        *count += 1;

        let mut captured = self.captured_messages.lock();
        for msg in request.messages {
            captured.push(msg.clone());
        }

        if *count == 1 {
            // Return a tool call to GOOGLECALENDAR_EVENTS_LIST
            Ok(ChatResponse {
                text: Some("Checking your calendar for this week...".into()),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "GOOGLECALENDAR_EVENTS_LIST".into(),
                    arguments: json!({
                        "timeMin": "2026-04-27T00:00:00Z",
                        "timeMax": "2026-05-04T00:00:00Z"
                    })
                    .to_string(),
                }],
                usage: None,
            })
        } else {
            // End the loop
            Ok(ChatResponse {
                text: Some("You have no events this week.".into()),
                tool_calls: vec![],
                usage: None,
            })
        }
    }

    fn supports_native_tools(&self) -> bool {
        true
    }
}

struct MockCalendarTool;

#[async_trait]
impl Tool for MockCalendarTool {
    fn name(&self) -> &str {
        "GOOGLECALENDAR_EVENTS_LIST"
    }
    fn description(&self) -> &str {
        "List calendar events"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "timeMin": { "type": "string" },
                "timeMax": { "type": "string" }
            }
        })
    }
    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("[]"))
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

#[tokio::test]
async fn test_orchestrator_has_current_date_context() -> Result<()> {
    let captured_messages = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(MockCalendarProvider {
        captured_messages: captured_messages.clone(),
        iter_count: Arc::new(Mutex::new(0)),
    });

    let mut agent = Agent::builder()
        .provider_arc(provider)
        .tools(vec![Box::new(MockCalendarTool)])
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .memory(Arc::new(StubMemory))
        .workspace_dir(std::env::temp_dir())
        .build()?;

    // Trigger a turn
    let _ = agent.turn("what is on my calendar this week?").await?;

    let messages = captured_messages.lock();
    let system_prompt = messages
        .iter()
        .find(|m| m.role == "system" && m.content.contains("## Current Date & Time"))
        .expect("System prompt should contain Current Date & Time");

    assert!(system_prompt.content.contains("202"));

    Ok(())
}

#[tokio::test]
async fn test_integrations_agent_has_current_date_context() -> Result<()> {
    let captured_messages = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(MockCalendarProvider {
        captured_messages: captured_messages.clone(),
        iter_count: Arc::new(Mutex::new(0)),
    });

    let _ = openhuman_core::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();

    let parent = openhuman_core::openhuman::agent::harness::ParentExecutionContext {
        provider: provider.clone(),
        all_tools: Arc::new(vec![Box::new(MockCalendarTool)]),
        all_tool_specs: Arc::new(vec![MockCalendarTool.spec()]),
        model_name: "test-model".into(),
        temperature: 0.4,
        workspace_dir: std::env::temp_dir(),
        memory: Arc::new(StubMemory),
        agent_config: openhuman_core::openhuman::config::AgentConfig::default(),
        skills: Arc::new(vec![]),
        memory_context: None,
        session_id: "test-session".into(),
        channel: "test".into(),
        connected_integrations: vec![],
        composio_client: None,
        tool_call_format: openhuman_core::openhuman::context::prompt::ToolCallFormat::PFormat,
        session_key: "0_test".into(),
        session_parent_prefix: None,
    };

    let def =
        openhuman_core::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
            .unwrap()
            .get("integrations_agent")
            .unwrap()
            .clone();

    let _ = openhuman_core::openhuman::agent::harness::with_parent_context(parent, async {
        openhuman_core::openhuman::agent::harness::run_subagent(
            &def,
            "list my calendar events for today",
            openhuman_core::openhuman::agent::harness::SubagentRunOptions::default(),
        )
        .await
    })
    .await?;

    let messages = captured_messages.lock();
    // Use substring search on all user messages
    let mut found = false;
    for m in messages.iter() {
        if m.role == "user" && m.content.contains("Current Date & Time:") {
            found = true;
            break;
        }
    }

    assert!(
        found,
        "User message should contain Current Date & Time context"
    );

    Ok(())
}

struct StubMemory;

#[async_trait]
impl openhuman_core::openhuman::memory::Memory for StubMemory {
    async fn store(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: openhuman_core::openhuman::memory::MemoryCategory,
        _: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }
    async fn recall(
        &self,
        _: &str,
        _: usize,
        _: openhuman_core::openhuman::memory::RecallOpts<'_>,
    ) -> Result<Vec<openhuman_core::openhuman::memory::MemoryEntry>> {
        Ok(vec![])
    }
    async fn get(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<openhuman_core::openhuman::memory::MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _: Option<&str>,
        _: Option<&openhuman_core::openhuman::memory::MemoryCategory>,
        _: Option<&str>,
    ) -> Result<Vec<openhuman_core::openhuman::memory::MemoryEntry>> {
        Ok(vec![])
    }
    async fn forget(&self, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn namespace_summaries(
        &self,
    ) -> Result<Vec<openhuman_core::openhuman::memory::NamespaceSummary>> {
        Ok(vec![])
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
