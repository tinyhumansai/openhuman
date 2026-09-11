//! `Agent` unit + integration tests.
//!
//! All tests exercise the agent through its public surface only (no
//! private-field access), which is why they live in a sibling file
//! rather than inline with one of the impl blocks. Shared fakes
//! (`MockProvider`, `RecordingProvider`, `MockTool`) are defined here.

use super::types::{Agent, AgentBuilder};
use crate::core::events::DomainEvent;
use crate::openhuman::agent::dispatcher::{NativeToolDispatcher, XmlToolDispatcher};
use crate::openhuman::agent::messages::ConversationMessage;
use crate::openhuman::inference::provider::ChatResponse;
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::Tool;
use anyhow::Result;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;
use tinyinference::message::Message;
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};

struct MockProvider {
    responses: Mutex<Vec<ChatResponse>>,
}

#[async_trait]
impl ChatModel<()> for MockProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(ModelProfile::default);
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        let mut guard = self.responses.lock();
        let response = if guard.is_empty() {
            ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }
        } else {
            guard.remove(0)
        };
        Ok(
            crate::openhuman::agent::tinyagents::model::native_model_response_for_request(
                &response, &request,
            ),
        )
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let response = self.invoke(state, request).await?;
        Ok(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::Completed(response),
        ])))
    }
}

/// Provider that records the system prompt bytes and model name of
/// every `chat()` call. Used by KV-cache stability tests — anything
/// that varies between turns (timestamps, re-rendered memory context,
/// flipped model hints) will show up as a diff between captures.
#[derive(Default)]
struct RecordingProvider {
    captures: Mutex<Vec<CapturedCall>>,
    responses: Mutex<Vec<ChatResponse>>,
}

#[derive(Clone)]
struct CapturedCall {
    system_prompt: Option<String>,
    model: String,
}

#[async_trait]
impl ChatModel<()> for RecordingProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(ModelProfile::default);
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        let system_prompt = request.messages.iter().find_map(|message| match message {
            Message::System(_) => Some(message.text()),
            _ => None,
        });
        self.captures.lock().push(CapturedCall {
            system_prompt,
            model: request.model.clone().unwrap_or_default(),
        });

        let mut guard = self.responses.lock();
        let response = if guard.is_empty() {
            ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }
        } else {
            guard.remove(0)
        };
        Ok(
            crate::openhuman::agent::tinyagents::model::native_model_response_for_request(
                &response, &request,
            ),
        )
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let response = self.invoke(state, request).await?;
        Ok(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::Completed(response),
        ])))
    }
}

struct MockTool;

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> Result<crate::openhuman::tools::ToolResult> {
        Ok(crate::openhuman::tools::ToolResult::success("tool-out"))
    }
}

// silence clippy — `AgentBuilder` is imported so tests can reference
// it in doc examples / type assertions if needed.
#[allow(dead_code)]
fn _assert_builder_is_exported() -> AgentBuilder {
    Agent::builder()
}

/// Minimal in-memory `Agent` build that every agent_definition_name
/// regression test reuses. Spins up a scratch workspace, a `none`
/// memory backend, a one-response `MockProvider`, and a single
/// `MockTool`, then feeds those into [`Agent::builder`]. Returns the
/// built `Agent` so individual tests can assert against the
/// [`Agent::agent_definition_name`] accessor.
fn build_minimal_agent_with_definition_name(definition_name: Option<&str>) -> Agent {
    build_minimal_agent_with_tool_sets(vec![Box::new(MockTool)], Vec::new(), definition_name)
}

/// [`build_minimal_agent_with_definition_name`] with caller-chosen durable and
/// synthesised tool sets, for the tests that pin how the two sets relate.
fn build_minimal_agent_with_tool_sets(
    tools: Vec<Box<dyn Tool>>,
    synthesized_tools: Vec<Box<dyn Tool>>,
    definition_name: Option<&str>,
) -> Agent {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![]),
    });

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut builder = Agent::builder()
        .chat_model(provider)
        .tools(tools)
        .synthesized_tools(synthesized_tools)
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path);

    if let Some(name) = definition_name {
        builder = builder.agent_definition_name(name);
    }

    builder.build().expect("minimal agent build should succeed")
}

fn integration_delegate_toolkit_enum(agent: &Agent) -> Vec<String> {
    let spec = agent
        .tool_specs()
        .iter()
        .find(|spec| spec.name == "delegate_to_integrations_agent")
        .expect("delegate_to_integrations_agent tool spec should be present");
    let mut out: Vec<String> = spec.parameters["properties"]["toolkit"]["enum"]
        .as_array()
        .expect("toolkit enum should be an array")
        .iter()
        .filter_map(|v| v.as_str().map(ToString::to_string))
        .collect();
    out.sort();
    out
}

/// Every synthesised delegate the agent advertises must have an executable
/// instance behind it, and vice versa.
///
/// This is the invariant #6145 broke: `tool_specs` reconciled unconditionally
/// while the instances only reconciled when the `tools` `Arc` happened to be
/// uniquely owned. A mid-session connect published a `delegate_*` spec with no
/// instance behind it — and, the policy snapshot being built from the
/// instances, no decision either, so the fail-closed visibility filter hid it
/// — while a revoke withdrew the spec and left the instance registered and
/// callable. Asserted through the public surface only, like every other test
/// in this file.
fn assert_synthesized_delegates_are_executable(agent: &Agent) {
    let instances = agent.synthesized_tools_arc();
    let executable: std::collections::HashSet<String> =
        instances.iter().map(|t| t.name().to_string()).collect();
    assert_eq!(
        executable.len(),
        instances.len(),
        "the synthesised set must not contain duplicate tool names"
    );

    let spec_names: std::collections::HashSet<String> = agent
        .tool_specs()
        .iter()
        .map(|spec| spec.name.clone())
        .collect();
    for name in &executable {
        assert!(
            spec_names.contains(name),
            "executable synthesised tool `{name}` is not advertised in tool_specs"
        );
    }

    // The other direction is the connect case: a `delegate_*` spec with
    // nothing registered to run it. Checked against the whole callable
    // surface, since a durable tool may legitimately own a `delegate_*` name
    // (in which case the synthesised one is dropped, not the durable one).
    let all_names: Vec<&str> = agent.all_tool_refs().iter().map(|t| t.name()).collect();
    let callable: std::collections::HashSet<&str> = all_names.iter().copied().collect();
    let advertised_delegates: Vec<&String> = spec_names
        .iter()
        .filter(|name| name.starts_with("delegate_"))
        .collect();
    for name in advertised_delegates {
        assert!(
            callable.contains(name.as_str()),
            "advertised delegate `{name}` has no executable instance in either set; \
             callable={callable:?}"
        );
    }

    // Presence is not enough. A stale instance under a fresh spec is the same
    // class of bug one step quieter: the model reads the new `toolkit` enum
    // and routes to an instance built from the previous connection set. Pin
    // that each instance's own schema is byte-identical to what is advertised.
    for tool in instances.iter() {
        let advertised = agent
            .tool_specs()
            .iter()
            .find(|spec| spec.name == tool.name())
            .unwrap_or_else(|| panic!("no spec advertised for synthesised tool `{}`", tool.name()));
        assert_eq!(
            tool.spec().parameters,
            advertised.parameters,
            "synthesised instance `{}` carries a stale schema — advertised and executable \
             must be rebuilt in the same pass",
            tool.name()
        );
    }

    // The two sets are disjoint by construction, so a name never resolves to
    // two instances however the readers order them.
    assert_eq!(
        callable.len(),
        all_names.len(),
        "the durable and synthesised sets must not share a name: {all_names:?}"
    );

    // And every synthesised tool carries a policy decision — without one the
    // fail-closed visibility filter would hide it, which is how the connect
    // direction of #6145 stayed invisible.
    for name in &executable {
        assert!(
            agent.tool_policy_session.decisions.contains_key(name),
            "synthesised tool `{name}` has no policy decision"
        );
    }
}

async fn turn_dispatches_spawn_subagent_through_full_path_inner() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;
    use crate::openhuman::tools::SpawnSubagentTool;

    // Idempotent — other tests may have already initialised it.
    AgentDefinitionRegistry::init_global_builtins().unwrap();

    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    // Scripted responses, in the exact order MockProvider will see them:
    //   1. Parent turn iter 0 — emit a spawn_subagent tool call.
    //   2. Sub-agent (researcher) iter 0 — return final text "X is Y".
    //   3. Parent turn iter 1 — fold sub-agent result into "Based on the research, X is Y."
    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                    id: "call-spawn".into(),
                    name: "spawn_subagent".into(),
                    arguments: serde_json::json!({
                        "agent_id": "__test_inherit_echo",
                        "prompt": "find out about X",
                        "blocking": true
                    })
                    .to_string(),
                    extra_content: None,
                }],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("X is Y".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("Based on the research, X is Y.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
        ]),
    });

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    // Tools include SpawnSubagentTool so the parent can call it.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(SpawnSubagentTool::new())];

    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(tools)
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent.turn("tell me about X").await.unwrap();
    assert_eq!(response, "Based on the research, X is Y.");

    // The parent's history should contain the spawn_subagent
    // assistant tool call AND a tool-result message carrying the
    // sub-agent's compact output.
    let has_spawn_call = agent.history().iter().any(|msg| match msg {
        ConversationMessage::AssistantToolCalls { tool_calls, .. } => {
            tool_calls.iter().any(|c| c.name == "spawn_subagent")
        }
        _ => false,
    });
    assert!(
        has_spawn_call,
        "parent history should contain the spawn_subagent assistant tool call"
    );

    let tool_result_contains_subagent_output = agent.history().iter().any(|msg| match msg {
        ConversationMessage::ToolResults(results) => {
            results.iter().any(|r| r.content.contains("X is Y"))
        }
        ConversationMessage::Chat(chat) if chat.role == "tool" => chat.content.contains("X is Y"),
        _ => false,
    });
    assert!(
        tool_result_contains_subagent_output,
        "parent history should contain a tool-result entry with the sub-agent's output"
    );
}

// ─────────────────────────────────────────────────────────────────────
// S4: the transcript seam is genuinely substitutable
// ─────────────────────────────────────────────────────────────────────

/// A `SessionHistory` that keeps everything in memory and touches no file.
///
/// The point of the fake is not that it is convenient — it is that it is
/// *possible*. Before the locator existed, `session_history` was an
/// `Arc<dyn …>` the turn path constructed inline, so nothing could ever be put
/// behind it; this fake failing to compile or failing to receive the turn is
/// the regression signal for that.
struct FakeSessionHistory {
    path: std::path::PathBuf,
    canned: Option<crate::openhuman::agent::harness::session::transcript::SessionTranscript>,
    appended: Mutex<Vec<Vec<crate::openhuman::agent::messages::ChatMessage>>>,
}

impl crate::openhuman::agent::harness::session::transcript_history::SessionTranscriptRead
    for FakeSessionHistory
{
    fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn read_session(
        &self,
    ) -> Result<Option<crate::openhuman::agent::harness::session::transcript::SessionTranscript>>
    {
        Ok(self.canned.clone())
    }
}

impl crate::openhuman::agent::harness::session::transcript_history::SessionHistory
    for FakeSessionHistory
{
    fn append_turn(
        &self,
        turn: crate::openhuman::agent::harness::session::transcript_history::TranscriptTurn<'_>,
    ) -> Result<()> {
        self.appended.lock().push(turn.next.to_vec());
        Ok(())
    }
}

#[async_trait]
impl tinyagents_harness::memory::ChatHistory for FakeSessionHistory {
    async fn messages(&self, _thread_id: &str) -> tinyagents_harness::Result<Vec<Message>> {
        Ok(vec![])
    }
    async fn append(&self, _thread_id: &str, _message: Message) -> tinyagents_harness::Result<()> {
        Ok(())
    }
    async fn replace(
        &self,
        _thread_id: &str,
        _messages: Vec<Message>,
    ) -> tinyagents_harness::Result<()> {
        Ok(())
    }
    async fn clear(&self, _thread_id: &str) -> tinyagents_harness::Result<()> {
        Ok(())
    }
}

/// Serves one canned transcript for every lookup and one recording write
/// handle, so a whole session's transcript I/O can be observed off-disk.
struct FakeLocator {
    handle: Arc<FakeSessionHistory>,
}

impl crate::openhuman::agent::harness::session::transcript_history::SessionHistoryLocator
    for FakeLocator
{
    fn latest_for_agent(
        &self,
        _agent_name: &str,
    ) -> Option<
        Arc<dyn crate::openhuman::agent::harness::session::transcript_history::SessionTranscriptRead>,
    >{
        Some(self.handle.clone())
    }

    fn root_for_thread(
        &self,
        _thread_id: &str,
    ) -> Option<
        Arc<dyn crate::openhuman::agent::harness::session::transcript_history::SessionTranscriptRead>,
    >{
        Some(self.handle.clone())
    }

    fn open_stem(
        &self,
        _stem: &str,
        _seed: crate::openhuman::agent::harness::session::transcript::TranscriptMeta,
    ) -> Result<
        Arc<dyn crate::openhuman::agent::harness::session::transcript_history::SessionHistory>,
    > {
        Ok(self.handle.clone())
    }
}

fn fake_transcript_meta(
    thread_id: &str,
) -> crate::openhuman::agent::harness::session::transcript::TranscriptMeta {
    crate::openhuman::agent::harness::session::transcript::TranscriptMeta {
        agent_name: "faker".into(),
        agent_id: None,
        agent_type: Some("root".into()),
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-08-08T00:00:00Z".into(),
        updated: "2026-08-08T00:00:00Z".into(),
        turn_count: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.into()),
        task_id: None,
    }
}

fn agent_with_fake_locator(
    workspace: &std::path::Path,
    canned: Option<crate::openhuman::agent::harness::session::transcript::SessionTranscript>,
) -> (Agent, Arc<FakeSessionHistory>) {
    let handle = Arc::new(FakeSessionHistory {
        path: workspace.join("session_raw").join("fake.jsonl"),
        canned,
        appended: Mutex::new(Vec::new()),
    });
    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();
    let agent = Agent::builder()
        .chat_model(Arc::new(MockProvider {
            responses: Mutex::new(vec![]),
        }))
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .agent_definition_name("faker")
        .workspace_dir(workspace.to_path_buf())
        .with_session_history_locator(Arc::new(FakeLocator {
            handle: handle.clone(),
        }))
        .build()
        .expect("agent build should succeed");
    (agent, handle)
}

#[path = "session_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "session_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "session_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "session_tests_part_04_tests.rs"]
mod part_04_tests;
