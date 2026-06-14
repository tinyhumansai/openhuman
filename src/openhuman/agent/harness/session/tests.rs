//! `Agent` unit + integration tests.
//!
//! All tests exercise the agent through its public surface only (no
//! private-field access), which is why they live in a sibling file
//! rather than inline with one of the impl blocks. Shared fakes
//! (`MockProvider`, `RecordingProvider`, `MockTool`) are defined here.

use super::types::{Agent, AgentBuilder};
use crate::core::event_bus::{init_global, publish_global, DomainEvent};
use crate::openhuman::agent::dispatcher::{NativeToolDispatcher, XmlToolDispatcher};
use crate::openhuman::inference::provider::{ChatRequest, ConversationMessage, Provider};
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::Tool;
use anyhow::Result;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;

struct MockProvider {
    responses: Mutex<Vec<crate::openhuman::inference::provider::ChatResponse>>,
}

#[async_trait]
impl Provider for MockProvider {
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
    ) -> Result<crate::openhuman::inference::provider::ChatResponse> {
        let mut guard = self.responses.lock();
        if guard.is_empty() {
            return Ok(crate::openhuman::inference::provider::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            });
        }
        Ok(guard.remove(0))
    }
}

/// Provider that records the system prompt bytes and model name of
/// every `chat()` call. Used by KV-cache stability tests — anything
/// that varies between turns (timestamps, re-rendered memory context,
/// flipped model hints) will show up as a diff between captures.
#[derive(Default)]
struct RecordingProvider {
    captures: Mutex<Vec<CapturedCall>>,
    responses: Mutex<Vec<crate::openhuman::inference::provider::ChatResponse>>,
}

#[derive(Clone)]
struct CapturedCall {
    system_prompt: Option<String>,
    model: String,
}

#[async_trait]
impl Provider for RecordingProvider {
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
        model: &str,
        _temperature: f64,
    ) -> Result<crate::openhuman::inference::provider::ChatResponse> {
        let system_prompt = request
            .messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.clone());
        self.captures.lock().push(CapturedCall {
            system_prompt,
            model: model.to_string(),
        });

        let mut guard = self.responses.lock();
        if guard.is_empty() {
            return Ok(crate::openhuman::inference::provider::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            });
        }
        Ok(guard.remove(0))
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
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Box::new(MockProvider {
        responses: Mutex::new(vec![]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    let mut builder = Agent::builder()
        .provider(provider)
        .tools(vec![Box::new(MockTool)])
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

/// Regression test for the `build_session_agent_inner` agent-id
/// threading bug.
///
/// Prior to the fix, `build_session_agent_inner` took an `agent_id:
/// &str` parameter but never threaded it into the `Agent::builder()`
/// chain. The builder's `.build()` then fell back to the legacy
/// `"main"` default, and every session built via
/// `Agent::from_config_for_agent` carried `agent_definition_name =
/// "main"` at runtime regardless of which id the caller asked for.
///
/// In the current codebase the user-facing path is `"orchestrator"`,
/// and the same builder is also used by several direct session agents.
/// A fallback to `"main"` silently misfiles transcripts on disk and
/// stamps the wrong agent metadata into them. Typed sub-agents are
/// unaffected because they're spawned through `subagent_runner` and
/// never touch the `from_config_for_agent` / builder fallback path.
///
/// This test pins the builder contract the fix relies on: calling
/// `.agent_definition_name(id)` on the builder chain produces an
/// `Agent` whose [`Agent::agent_definition_name`] accessor returns
/// that id verbatim. `"orchestrator"` covers the user-facing chat path;
/// the others are defensive coverage so a future top-level caller still
/// inherits the contract.
#[test]
fn agent_builder_threads_agent_definition_name_when_set() {
    for expected in ["integrations_agent", "orchestrator", "trigger_triage"] {
        let agent = build_minimal_agent_with_definition_name(Some(expected));
        assert_eq!(
            agent.agent_definition_name(),
            expected,
            "agent.agent_definition_name() should return the value passed to the builder"
        );
    }
}

/// Complementary to [`agent_builder_threads_agent_definition_name_when_set`]:
/// when a caller builds an `Agent` without ever calling
/// [`AgentBuilder::agent_definition_name`], the legacy `"main"`
/// fallback still applies. This pins the fallback contract that
/// direct builder users (tests, CLI harnesses) rely on, and
/// documents the exact misbehaviour the threading fix prevents —
/// `build_session_agent_inner` used to hit this fallback even when
/// a caller asked for a concrete agent id, because the
/// `.agent_definition_name` setter was missing from the builder chain.
#[test]
fn agent_builder_falls_back_to_main_when_definition_name_unset() {
    let agent = build_minimal_agent_with_definition_name(None);
    assert_eq!(
        agent.agent_definition_name(),
        "main",
        "AgentBuilder::build should default agent_definition_name to \"main\" when unset"
    );
}

#[test]
fn set_connected_integrations_marks_session_initialized_and_updates_hash() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    assert!(
        !agent.connected_integrations_initialized,
        "fresh builder-built agents should start with placeholder integration state"
    );

    agent.set_connected_integrations(vec![
        crate::openhuman::context::prompt::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email".into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        },
    ]);

    assert!(agent.connected_integrations_initialized);
    assert_eq!(agent.connected_integrations().len(), 1);
    assert_eq!(agent.connected_integrations()[0].toolkit, "gmail");
    assert_eq!(
        agent.last_seen_integrations_hash,
        crate::openhuman::composio::connected_set_hash(agent.connected_integrations())
    );
}

#[test]
fn refresh_delegation_tools_updates_schema_even_when_tool_arc_is_shared() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.set_connected_integrations(vec![
        crate::openhuman::context::prompt::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email".into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        },
    ]);

    assert!(agent.refresh_delegation_tools());
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec!["gmail".to_string()]
    );

    // Simulate an in-flight turn holding a shared Arc clone.
    let _shared_tools = agent.tools_arc();
    agent.set_connected_integrations(vec![
        crate::openhuman::context::prompt::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email".into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        },
        crate::openhuman::context::prompt::ConnectedIntegration {
            toolkit: "notion".into(),
            description: "Docs".into(),
            tools: vec![],
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        },
    ]);

    assert!(agent.refresh_delegation_tools());
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec!["gmail".to_string(), "notion".to_string()]
    );
}

/// Regression for #3044: repeated mid-session connects while the `tools`
/// Arc stays shared (the normal `before_dispatch` path, where
/// `AgentToolSource` holds a clone) must not accumulate duplicate
/// synthesised `ToolSpec`s.
///
/// Before the fix, a failed `tools` reconcile rolled `synthesized_tool_names`
/// back to the *old* mask. On the next refresh the spec `retain` used that
/// stale mask and failed to drop the intervening refresh's specs, so the
/// synthesised delegate spec piled up once per connect.
#[test]
fn refresh_delegation_tools_no_duplicate_specs_across_shared_arc_connects() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));

    let conn = |slug: &str, desc: &str| crate::openhuman::context::prompt::ConnectedIntegration {
        toolkit: slug.into(),
        description: desc.into(),
        tools: vec![],
        gated_tools: vec![],
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    };

    let delegate_spec_count = |agent: &Agent| -> usize {
        agent
            .tool_specs()
            .iter()
            .filter(|s| s.name == "delegate_to_integrations_agent")
            .count()
    };

    // Turn 1: gmail connects.
    agent.set_connected_integrations(vec![conn("gmail", "Email")]);
    assert!(agent.refresh_delegation_tools());

    // Hold a shared clone across every subsequent refresh so `Arc::get_mut`
    // always fails — exactly what happens during an in-flight turn.
    let _shared_tools = agent.tools_arc();

    // Turn 2: notion connects mid-session.
    agent.set_connected_integrations(vec![conn("gmail", "Email"), conn("notion", "Docs")]);
    assert!(agent.refresh_delegation_tools());

    // Turn 3: slack connects mid-session — this is where the old code
    // produced a duplicate `delegate_to_integrations_agent` spec.
    agent.set_connected_integrations(vec![
        conn("gmail", "Email"),
        conn("notion", "Docs"),
        conn("slack", "Chat"),
    ]);
    assert!(agent.refresh_delegation_tools());

    assert_eq!(
        delegate_spec_count(&agent),
        1,
        "exactly one synthesised delegate spec must remain after repeated shared-Arc connects"
    );
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec![
            "gmail".to_string(),
            "notion".to_string(),
            "slack".to_string()
        ]
    );
}

#[test]
fn composio_listener_drains_integrations_changed_events() {
    let _ = init_global(64);
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.ensure_composio_integrations_listener();
    publish_global(DomainEvent::ComposioIntegrationsChanged {
        toolkits: vec!["gmail".into()],
    });
    assert!(agent.drain_composio_integrations_changed_events());
    assert!(
        !agent.drain_composio_integrations_changed_events(),
        "event queue should be drained after one pass"
    );
}

#[tokio::test]
async fn turn_without_tools_returns_text() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Box::new(MockProvider {
        responses: Mutex::new(vec![crate::openhuman::inference::provider::ChatResponse {
            text: Some("hello".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        }]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    let mut agent = Agent::builder()
        .provider(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent.turn("hi").await.unwrap();
    assert_eq!(response, "hello");
}

#[tokio::test]
async fn turn_with_native_dispatcher_handles_tool_results_variant() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Box::new(MockProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                    id: "tc1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                    extra_content: None,
                }],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
        ]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    let mut agent = Agent::builder()
        .provider(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent.turn("hi").await.unwrap();
    assert_eq!(response, "done");
    assert!(agent
        .history()
        .iter()
        .any(|msg| matches!(msg, ConversationMessage::ToolResults(_))));
}

#[tokio::test]
async fn turn_with_native_dispatcher_persists_fallback_tool_calls() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Box::new(MockProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some(
                    "Checking...\n<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>"
                        .into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
        ]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    let mut agent = Agent::builder()
        .provider(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent.turn("hi").await.unwrap();
    assert_eq!(response, "done");

    let persisted_calls = agent
        .history()
        .iter()
        .find_map(|msg| match msg {
            ConversationMessage::AssistantToolCalls { tool_calls, .. } => Some(tool_calls),
            _ => None,
        })
        .expect("assistant tool calls should be persisted");
    assert_eq!(persisted_calls.len(), 1);
    assert_eq!(persisted_calls[0].name, "echo");
}

/// End-to-end: parent Agent issues a `spawn_subagent` tool call, the
/// runner dispatches a built-in sub-agent (`researcher`) using the
/// same MockProvider, and the parent's next turn folds the sub-agent's
/// text output into the final response.
///
/// This is the highest-level test that exercises:
/// - Agent::turn → execute_tool_call → SpawnSubagentTool::execute
/// - PARENT_CONTEXT task-local visibility
/// - AgentDefinitionRegistry::global lookup
/// - run_subagent → run_inner_loop with the parent's provider
/// - Result returned as a ToolResult and threaded back into history
///
/// Uses the `#[cfg(test)]`-only `__test_inherit_echo` sub-agent
/// (`ModelSpec::Inherit`) rather than `researcher`. After #1710,
/// sub-agents with a `Hint(workload)` spec build a fresh provider via
/// `create_chat_provider(...)` and therefore can't share this test's
/// `MockProvider` — so a Hint sub-agent here would leak the scripted
/// chain. `Inherit` keeps `parent.provider`, which is exactly the
/// plumbing this test asserts. Provider *routing* for Hint sub-agents
/// is covered independently by
/// `subagent_runner::ops::tests::resolve_subagent_provider_*`.
#[tokio::test]
async fn turn_dispatches_spawn_subagent_through_full_path() {
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
    let provider = Box::new(MockProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                    id: "call-spawn".into(),
                    name: "spawn_subagent".into(),
                    arguments: serde_json::json!({
                        "agent_id": "__test_inherit_echo",
                        "prompt": "find out about X"
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

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    // Tools include SpawnSubagentTool so the parent can call it.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(SpawnSubagentTool::new())];

    let mut agent = Agent::builder()
        .provider(provider)
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

/// KV-cache invariant: across multiple turns in the same session, the
/// system-prompt bytes submitted to the provider must be byte-identical,
/// and the model name must not flip. Both are required for the backend's
/// automatic prefix cache to hit — if either changes, the backend must
/// re-prefill the entire prompt every turn.
///
/// This test guards against two regressions:
///   1. A future edit that reintroduces the subsequent-turn system
///      prompt rebuild (see the `learning_enabled` branch we
///      deliberately removed in `turn()`).
///   2. A future edit that reintroduces per-message model
///      classification on the main agent (which would flip the
///      effective model between turns).
#[tokio::test]
async fn system_prompt_and_model_are_byte_stable_across_turns() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("first".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("second".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("third".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
        ]),
        captures: Mutex::new(Vec::new()),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    let mut agent = Agent::builder()
        .provider_arc(provider.clone() as Arc<dyn Provider>)
        .tools(vec![])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        // Learning flag is explicitly enabled to prove that the
        // former "rebuild system prompt on subsequent turns" branch
        // is gone — we should still see byte-stable prompts.
        .learning_enabled(true)
        .build()
        .unwrap();

    for prompt in ["first question", "second question", "third question"] {
        agent.turn(prompt).await.unwrap();
    }

    let captures = provider.captures.lock().clone();
    assert_eq!(
        captures.len(),
        3,
        "expected one provider call per turn, got {}",
        captures.len()
    );

    let first_system = captures[0]
        .system_prompt
        .as_ref()
        .expect("first turn should have a system prompt");
    for (idx, cap) in captures.iter().enumerate() {
        let sys = cap
            .system_prompt
            .as_ref()
            .expect("every turn should carry the system prompt");
        assert_eq!(
            sys, first_system,
            "system prompt drifted on turn {} — KV cache prefix broken",
            idx
        );
        assert_eq!(
            cap.model, captures[0].model,
            "model name flipped on turn {} — KV cache namespace broken",
            idx
        );
        assert!(
            !sys.contains("<!-- CACHE_BOUNDARY -->"),
            "system prompt should not leak any cache-boundary marker"
        );
    }
}

/// Regression test for the per-thread transcript resume bug.
///
/// `set_agent_definition_name` is called by the web channel after
/// `Agent::from_config_for_agent("orchestrator")` returns, to scope
/// transcripts per thread (e.g. `"orchestrator_thread-6ad6d"`). Prior
/// to the fix this only updated `agent_definition_name` and left
/// `session_key` pointing at the builder-time name. Persist would
/// then write `session_raw/<ts>_orchestrator.jsonl` while resume
/// searched for `session_raw/<ts>_orchestrator_thread-6ad6d.jsonl`,
/// so every cold-boot turn ran against an empty transcript and the
/// LLM had no conversation history.
///
/// This test pins the contract: after `set_agent_definition_name`,
/// `session_key`'s suffix matches the new (sanitised) name so the
/// next persist+resume pair land on the same file.
#[test]
fn set_agent_definition_name_rewrites_session_key_suffix() {
    let agent_first = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let original_key = agent_first.session_key().to_string();
    assert!(
        original_key.ends_with("_orchestrator"),
        "builder should seed session_key suffix from agent_definition_name; got {original_key}"
    );

    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let prefix = agent
        .session_key()
        .split_once('_')
        .map(|(p, _)| p.to_string())
        .expect("session_key must have a `<ts>_<suffix>` shape");

    agent.set_agent_definition_name("orchestrator_thread-6ad6d");

    assert_eq!(agent.agent_definition_name(), "orchestrator_thread-6ad6d");
    assert_eq!(
        agent.session_key(),
        format!("{prefix}_orchestrator_thread-6ad6d"),
        "session_key suffix must track agent_definition_name so transcript persist + \
         resume agree on the file path"
    );
}

/// `set_agent_definition_name` must sanitise non-allowed characters in
/// the new name (matching the builder's policy) so `session_key`
/// never contains anything that would escape the `session_raw/`
/// directory or break filename parsing on disk.
#[test]
fn set_agent_definition_name_sanitises_unsafe_characters() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.set_agent_definition_name("orch/../../etc/passwd thread-6ad6d");
    assert!(
        !agent.session_key().contains('/'),
        "session_key must never contain path separators; got {}",
        agent.session_key()
    );
    assert!(
        !agent.session_key().contains(' '),
        "session_key must never contain whitespace; got {}",
        agent.session_key()
    );
}

/// Cold-boot resume from the conversation JSONL works even when no
/// matching transcript file exists. The web channel calls
/// `seed_resume_from_messages` on the cache-miss path so the agent
/// sees prior conversation context immediately, instead of having to
/// wait for a transcript to be persisted under the new
/// thread-scoped name.
#[test]
fn seed_resume_from_messages_primes_cached_transcript() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let prior = vec![
        ("user".to_string(), "what is btc price".to_string()),
        ("agent".to_string(), "$80,000".to_string()),
        // Trailing user message that the caller is about to pass to
        // run_single — must be deduped from the cached prefix.
        ("user".to_string(), "what did i just ask".to_string()),
    ];
    agent
        .seed_resume_from_messages(prior, "what did i just ask")
        .expect("seed");

    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cache populated");
    // [system, user(btc), agent(80k)] — trailing user was deduped.
    assert_eq!(cached.len(), 3);
    assert_eq!(cached[0].role, "system");
    assert_eq!(cached[1].role, "user");
    assert_eq!(cached[1].content, "what is btc price");
    assert_eq!(cached[2].role, "assistant");
    assert_eq!(cached[2].content, "$80,000");
}

/// `seed_resume_from_messages` must not stomp the existing context if
/// the agent has already been warmed (in-process session cache hit).
/// Otherwise the cache-miss branch in the web channel would erase
/// real progress whenever the caller defensively invoked seeding.
#[test]
fn seed_resume_from_messages_is_noop_on_warm_agent() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.cached_transcript_messages = Some(vec![
        crate::openhuman::inference::provider::ChatMessage::system("warm prefix"),
        crate::openhuman::inference::provider::ChatMessage::user("hi"),
    ]);
    agent
        .seed_resume_from_messages(vec![("user".into(), "different".into())], "different")
        .expect("seed");
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("still populated");
    assert_eq!(cached.len(), 2);
    assert_eq!(cached[0].content, "warm prefix");
}

/// Trailing user message that does NOT match the current incoming
/// message must be preserved — the dedup heuristic only fires on
/// exact match because the conversation JSONL is the source of truth
/// and may legitimately contain back-to-back user messages (e.g. the
/// thread-7242c case where an interrupted turn left the prior user
/// message un-replied).
#[test]
fn seed_resume_from_messages_preserves_unmatched_trailing_user() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let prior = vec![
        ("user".to_string(), "earlier question".to_string()),
        ("agent".to_string(), "earlier answer".to_string()),
        ("user".to_string(), "stranded follow-up".to_string()),
    ];
    agent
        .seed_resume_from_messages(prior, "completely different new turn")
        .expect("seed");
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cache populated");
    // [system, user, agent, user] — trailing kept because it doesn't
    // match the current turn's user input.
    assert_eq!(cached.len(), 4);
    assert_eq!(cached[3].role, "user");
    assert_eq!(cached[3].content, "stranded follow-up");
}

#[test]
fn seed_resume_from_messages_respects_history_window_bound() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.config.max_history_messages = 4;
    let prior = vec![
        ("user".to_string(), "u1".to_string()),
        ("agent".to_string(), "a1".to_string()),
        ("user".to_string(), "u2".to_string()),
        ("agent".to_string(), "a2".to_string()),
        ("user".to_string(), "u3".to_string()),
        ("agent".to_string(), "a3".to_string()),
    ];
    agent
        .seed_resume_from_messages(prior, "new turn")
        .expect("seed");

    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cache populated");
    // max_history_messages=4 keeps [system + last 3 messages].
    assert_eq!(cached.len(), 4);
    assert_eq!(cached[0].role, "system");
    assert_eq!(cached[1].content, "a2");
    assert_eq!(cached[2].content, "u3");
    assert_eq!(cached[3].content, "a3");
}

#[test]
fn bound_cached_transcript_messages_without_system_prefix_keeps_tail() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.config.max_history_messages = 3;

    let messages = vec![
        crate::openhuman::inference::provider::ChatMessage::user("u1"),
        crate::openhuman::inference::provider::ChatMessage::assistant("a1"),
        crate::openhuman::inference::provider::ChatMessage::user("u2"),
        crate::openhuman::inference::provider::ChatMessage::assistant("a2"),
        crate::openhuman::inference::provider::ChatMessage::user("u3"),
    ];
    let bounded = agent.bound_cached_transcript_messages(messages);
    assert_eq!(bounded.len(), 3);
    assert_eq!(bounded[0].content, "u2");
    assert_eq!(bounded[1].content, "a2");
    assert_eq!(bounded[2].content, "u3");
}

/// The cached-transcript resume path operates on wire-form `ChatMessage`s. When
/// the window cut lands so the tail opens on a `tool` result whose `tool_calls`
/// opener fell outside the window, `bound_cached_transcript_messages` must snap
/// past it — a leading `tool` message has no preceding `tool_calls` and the
/// provider 400s (surfacing as "Something went wrong").
#[test]
fn bound_cached_transcript_messages_snaps_past_leading_orphan_tool() {
    use crate::openhuman::inference::provider::ChatMessage;

    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.config.max_history_messages = 3;

    // 5 messages, cap 3: the tail slice is [tool(a), user(u2), assistant(a2)];
    // the assistant `tool_calls` opener fell outside the window.
    let messages = vec![
        ChatMessage::assistant(
            r#"{"content":"calling","tool_calls":[{"id":"call_a","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_a","content":"orphaned"}"#),
        ChatMessage::user("u2"),
        ChatMessage::assistant("a2"),
        ChatMessage::user("u3"),
    ];

    let bounded = agent.bound_cached_transcript_messages(messages);

    assert!(
        bounded.first().map(|m| m.role.as_str()) != Some("tool"),
        "window must not open on an orphaned tool result"
    );
    assert!(
        !bounded.iter().any(|m| m.role == "tool"),
        "the orphaned tool result must be dropped"
    );
    // tail [tool, u2, a2, u3] -> drop leading tool -> [u2, a2, u3].
    assert_eq!(
        bounded
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["u2", "a2", "u3"]
    );
}
