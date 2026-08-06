use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use tinyagents::harness::message::{AssistantMessage, Message};
use tinyagents::harness::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyagents::harness::tool::ToolCall;

struct MockCalendarModel {
    captured_messages: Arc<Mutex<Vec<Message>>>,
    iter_count: Arc<Mutex<usize>>,
    profile: ModelProfile,
}

#[async_trait]
impl ChatModel<()> for MockCalendarModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let mut count = self.iter_count.lock();
        *count += 1;

        let mut captured = self.captured_messages.lock();
        for msg in request.messages {
            captured.push(msg.clone());
        }

        if *count == 1 {
            // Return a tool call to GOOGLECALENDAR_EVENTS_LIST
            Ok(ModelResponse {
                message: AssistantMessage {
                    id: None,
                    content: Vec::new(),
                    tool_calls: vec![ToolCall::new(
                        "call_1",
                        "GOOGLECALENDAR_EVENTS_LIST",
                        json!({
                        "timeMin": "2026-04-27T00:00:00Z",
                        "timeMax": "2026-05-04T00:00:00Z"
                        }),
                    )],
                    usage: None,
                },
                usage: None,
                finish_reason: Some("tool_calls".into()),
                raw: None,
                resolved_model: None,
                continue_turn: None,
            })
        } else {
            // End the loop
            Ok(ModelResponse::assistant("You have no events this week."))
        }
    }
}

fn calendar_model(captured_messages: Arc<Mutex<Vec<Message>>>) -> Arc<MockCalendarModel> {
    let mut profile = ModelProfile::default();
    profile.tool_calling = true;
    Arc::new(MockCalendarModel {
        captured_messages,
        iter_count: Arc::new(Mutex::new(0)),
        profile,
    })
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
    let model = calendar_model(captured_messages.clone());

    let mut agent = Agent::builder()
        .chat_model(model)
        .tools(vec![Box::new(MockCalendarTool)])
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .memory(Arc::new(StubMemory))
        .workspace_dir(std::env::temp_dir())
        .build()?;

    // Trigger a turn
    let _ = agent.turn("what is on my calendar this week?").await?;

    let messages = captured_messages.lock();
    // The system prompt carries the static grounding *rule* (#3602) — the
    // concrete clock no longer lives here, it rides the per-turn user message
    // so a long-lived session can't go stale.
    messages
        .iter()
        .find(|m| matches!(m, Message::System(_)) && m.text().contains("## Current Date & Time"))
        .expect("System prompt should carry the Current Date & Time grounding rule");

    // The live date/time is injected on the user message every turn. Assert it
    // carries the stamp and a concrete year token.
    let user_msg = messages
        .iter()
        .find(|m| matches!(m, Message::User(_)) && m.text().contains("Current Date & Time:"))
        .expect("User message should carry the per-turn Current Date & Time stamp");
    // Assert a concrete `YYYY-MM-DD HH:MM:SS` shape rather than a decade token
    // (which would rot as years advance).
    let user_text = user_msg.text();
    let after = user_text
        .split("Current Date & Time: ")
        .nth(1)
        .expect("stamp must follow the canonical prefix");
    let dt = after
        .get(0..19)
        .expect("stamp must include YYYY-MM-DD HH:MM:SS");
    chrono::NaiveDateTime::parse_from_str(dt, "%Y-%m-%d %H:%M:%S")
        .expect("user message stamp must include a parseable YYYY-MM-DD HH:MM:SS");

    Ok(())
}

#[tokio::test]
async fn test_integrations_agent_has_current_date_context() -> Result<()> {
    let captured_messages = Arc::new(Mutex::new(Vec::new()));
    let model = calendar_model(captured_messages.clone());

    let _ = openhuman_core::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();

    let parent = openhuman_core::openhuman::agent::harness::ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: ["integrations_agent".to_string()].into_iter().collect(),
        turn_model_source:
            openhuman_core::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(vec![Box::new(MockCalendarTool)]),
        all_tool_specs: Arc::new(vec![MockCalendarTool.spec()]),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.4,
        workspace_dir: std::env::temp_dir(),
        workspace_descriptor: None,
        memory: Arc::new(StubMemory),
        agent_config: openhuman_core::openhuman::config::AgentConfig::default(),
        workflows: Arc::new(vec![]),
        memory_context: Arc::new(None),
        session_id: "test-session".into(),
        channel: "test".into(),
        connected_integrations: vec![],
        tool_call_format:
            openhuman_core::openhuman::agent::context::prompt::ToolCallFormat::PFormat,
        session_key: "0_test".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    };

    let mut def =
        openhuman_core::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
            .unwrap()
            .get("integrations_agent")
            .unwrap()
            .clone();
    // `integrations_agent` ships with `[model] hint = "agentic"`. After
    // #1710, a Hint sub-agent builds a fresh provider via the workload
    // factory instead of inheriting `parent.provider` — which here would
    // resolve to the OpenHuman backend and fail with "No backend session"
    // before the MockCalendarModel ever sees a request. This test only
    // asserts prompt construction (the "Current Date & Time" context), so
    // override the model spec to Inherit to keep the real integrations_agent
    // definition (prompt, tools, scope) while routing through the captured
    // mock provider. Provider *routing* for Hint sub-agents is covered by
    // `subagent_runner::ops::tests::resolve_subagent_provider_*`.
    def.model = openhuman_core::openhuman::agent::harness::definition::ModelSpec::Inherit;

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
        if matches!(m, Message::User(_)) && m.text().contains("Current Date & Time:") {
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
