use super::*;
use crate::openhuman::agent::harness::definition::{ModelSpec, ToolScope};

fn make_def_named_tools(names: &[&str]) -> AgentDefinition {
    AgentDefinition {
        id: "test".into(),
        when_to_use: "t".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("system".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools: ToolScope::Named(names.iter().map(|s| s.to_string()).collect()),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 5,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: crate::openhuman::agent::harness::definition::SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
        source: crate::openhuman::agent::harness::definition::DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

/// Local tool used to populate `parent_tools` in tests.
struct StubTool {
    name: &'static str,
}

use crate::openhuman::tools::{PermissionLevel, ToolResult};
use async_trait::async_trait;

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }
}

fn stub(name: &'static str) -> Box<dyn Tool> {
    Box::new(StubTool { name })
}

// ── End-to-end runner tests with mock provider ────────────────────────

use crate::openhuman::agent::harness::fork_context::with_parent_context;
use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};
use parking_lot::Mutex;
use std::sync::Arc;
use tinyinference::message::{AssistantMessage, ContentBlock, Message, MessageDelta};
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyinference::tool::ToolCall;

/// Mock provider whose response queue can be inspected by the test
/// to verify the bytes that arrive at the model.
#[derive(Clone)]
struct CapturedRequest {
    messages: Vec<CapturedMessage>,
    tool_count: usize,
    model: String,
}

#[derive(Clone)]
struct CapturedMessage {
    role: &'static str,
    content: String,
}

struct ScriptedProvider {
    responses: Mutex<Vec<ModelResponse>>,
    captured: Mutex<Vec<CapturedRequest>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ModelResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            captured: Mutex::new(Vec::new()),
        })
    }

    fn take_response(&self, request: ModelRequest) -> ModelResponse {
        self.captured.lock().push(CapturedRequest {
            messages: request
                .messages
                .iter()
                .map(|message| CapturedMessage {
                    role: match message {
                        Message::System(_) => "system",
                        Message::User(_) => "user",
                        Message::Assistant(_) => "assistant",
                        Message::Tool(_) => "tool",
                    },
                    content: message.text(),
                })
                .collect(),
            tool_count: request.tools.len(),
            model: request.model.unwrap_or_default(),
        });
        let mut responses = self.responses.lock();
        if responses.is_empty() {
            ModelResponse::assistant("")
        } else {
            responses.remove(0)
        }
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(|| ModelProfile {
                provider: Some("subagent-runner-test".to_string()),
                tool_calling: true,
                parallel_tool_calls: true,
                streaming: true,
                ..ModelProfile::default()
            });
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Ok(self.take_response(request))
    }

    async fn stream(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let response = self.take_response(request);
        let reasoning = response
            .message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Thinking { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        let text = response.text();
        let mut items = vec![ModelStreamItem::Started];
        if !reasoning.is_empty() {
            items.push(ModelStreamItem::MessageDelta(MessageDelta::reasoning(
                reasoning,
            )));
        }
        if !text.is_empty() {
            items.push(ModelStreamItem::MessageDelta(MessageDelta::text(text)));
        }
        items.push(ModelStreamItem::Completed(response));
        Ok(Box::pin(futures::stream::iter(items)))
    }
}

fn text_response(text: &str) -> ModelResponse {
    ModelResponse::assistant(text)
}

fn text_response_with_reasoning(text: &str, reasoning: &str) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: vec![
                ContentBlock::Thinking {
                    text: reasoning.to_string(),
                    signature: None,
                },
                ContentBlock::Text(text.to_string()),
            ],
            tool_calls: Vec::new(),
            usage: None,
        },
        usage: None,
        finish_reason: None,
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    }
}

fn tool_response(name: &str, args: &str) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(
                "call-1",
                name,
                serde_json::from_str(args).expect("valid scripted tool arguments"),
            )],
            usage: None,
        },
        usage: None,
        finish_reason: Some("tool_calls".to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    }
}

/// Build a minimal `ParentExecutionContext` suitable for runner tests.
/// Uses a no-op memory backend so we don't have to spin up a real one.
fn make_parent(
    provider: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
) -> ParentExecutionContext {
    let tool_specs: Vec<Arc<crate::openhuman::tools::ToolSpec>> =
        tools.iter().map(|t| Arc::new(t.spec())).collect();
    ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: ["test".to_string(), "child".to_string(), "inner".to_string()]
            .into_iter()
            .collect(),
        turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(
            provider,
        ),
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(tool_specs),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.5,
        workspace_dir: std::env::temp_dir(),
        memory: noop_memory(),
        agent_config: crate::openhuman::config::AgentConfig::default(),
        workflows: Arc::new(vec![]),
        memory_context: Arc::new(None),
        session_id: "test-session".into(),
        channel: "test".into(),
        connected_integrations: vec![],
        tool_call_format: crate::openhuman::agent::context::prompt::ToolCallFormat::PFormat,
        session_key: "0_test".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

fn noop_memory() -> Arc<dyn crate::openhuman::memory::Memory> {
    struct NoopMemory;
    #[async_trait]
    impl crate::openhuman::memory::Memory for NoopMemory {
        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: crate::openhuman::memory::MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(vec![])
        }
        async fn get(
            &self,
            _namespace: &str,
            _key: &str,
        ) -> anyhow::Result<Option<crate::openhuman::memory::MemoryEntry>> {
            Ok(None)
        }
        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&crate::openhuman::memory::MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(vec![])
        }
        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(vec![])
        }
        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }
        async fn health_check(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "noop"
        }
    }
    Arc::new(NoopMemory)
}

// ── Runtime spawn-hierarchy (tier) gate (issue #4098) ───────────────────────
// `tier_gate_decision` is the pure decision the runtime gate in `run_subagent`
// applies to each delegation hop. Tested directly so the deny/allow/skip
// table is covered without standing up a global registry or a live spawn.

// Thin wrapper to call the gate with throwaway log-context ids.
fn gate(parent: Option<&AgentDefinition>, child: &AgentDefinition) -> Result<(), SubagentRunError> {
    super::runner::tier_gate_decision(parent, child, "parent-agent", "task-1")
}

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
