//! `Agent` unit + integration tests.
//!
//! All tests exercise the agent through its public surface only (no
//! private-field access), which is why they live in a sibling file
//! rather than inline with one of the impl blocks. Shared fakes
//! (`MockProvider`, `RecordingProvider`, `MockTool`) are defined here.

use super::types::{Agent, AgentBuilder};
use crate::openhuman::agent::dispatcher::{NativeToolDispatcher, XmlToolDispatcher};
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ChatRequest, ConversationMessage, Provider};
use crate::openhuman::tools::Tool;
use anyhow::Result;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;

struct MockProvider {
    responses: Mutex<Vec<crate::openhuman::providers::ChatResponse>>,
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
    ) -> Result<crate::openhuman::providers::ChatResponse> {
        let mut guard = self.responses.lock();
        if guard.is_empty() {
            return Ok(crate::openhuman::providers::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
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
    responses: Mutex<Vec<crate::openhuman::providers::ChatResponse>>,
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
    ) -> Result<crate::openhuman::providers::ChatResponse> {
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
            return Ok(crate::openhuman::providers::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
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
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

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
/// In the current codebase only two ids actually reach
/// `from_config_for_agent` in production: `"orchestrator"` (via the
/// `Agent::from_config` legacy wrapper and the post-onboarding web
/// dispatch path) and `"welcome"` (via `welcome_proactive` and the
/// pre-onboarding web dispatch path). The orchestrator case is
/// benign — `"main"` is already an alias for orchestrator everywhere
/// downstream, so the behavior is a no-op. The welcome case is the
/// one the user sees: welcome sessions were being misfiled on disk
/// as `sessions/DDMMYYYY/main_*.md` instead of `welcome_*.md`, and
/// the `agent:` line inside each transcript's `<!-- session_transcript
/// -->` metadata header stamped `agent: main` instead of
/// `agent: welcome`. Skills_agent and the other typed sub-agents are
/// unaffected because they're spawned through `subagent_runner` and
/// never touch the `from_config_for_agent` / builder fallback path.
///
/// This test pins the builder contract the fix relies on: calling
/// `.agent_definition_name(id)` on the builder chain produces an
/// `Agent` whose [`Agent::agent_definition_name`] accessor returns
/// that id verbatim. `"welcome"` and `"orchestrator"` exercise the
/// two ids that reach `from_config_for_agent` today; `"integrations_agent"`
/// and `"trigger_triage"` are defensive coverage so that if a
/// future commit adds a new top-level caller for one of those ids
/// the builder contract is already pinned.
#[test]
fn agent_builder_threads_agent_definition_name_when_set() {
    for expected in [
        "welcome",
        "integrations_agent",
        "orchestrator",
        "trigger_triage",
    ] {
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
/// a caller asked for `welcome`, because the `.agent_definition_name`
/// setter was missing from the builder chain. The result was that
/// welcome sessions landed on disk as `main_*.md` with `agent: main`
/// stamped into their transcript metadata header.
#[test]
fn agent_builder_falls_back_to_main_when_definition_name_unset() {
    let agent = build_minimal_agent_with_definition_name(None);
    assert_eq!(
        agent.agent_definition_name(),
        "main",
        "AgentBuilder::build should default agent_definition_name to \"main\" when unset"
    );
}

#[tokio::test]
async fn turn_without_tools_returns_text() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Box::new(MockProvider {
        responses: Mutex::new(vec![crate::openhuman::providers::ChatResponse {
            text: Some("hello".into()),
            tool_calls: vec![],
            usage: None,
        }]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

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
            crate::openhuman::providers::ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![crate::openhuman::providers::ToolCall {
                    id: "tc1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                }],
                usage: None,
            },
            crate::openhuman::providers::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
            },
        ]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

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
            crate::openhuman::providers::ChatResponse {
                text: Some(
                    "Checking...\n<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>"
                        .into(),
                ),
                tool_calls: vec![],
                usage: None,
            },
            crate::openhuman::providers::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
            },
        ]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

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
            crate::openhuman::providers::ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![crate::openhuman::providers::ToolCall {
                    id: "call-spawn".into(),
                    name: "spawn_subagent".into(),
                    arguments: serde_json::json!({
                        "agent_id": "researcher",
                        "prompt": "find out about X"
                    })
                    .to_string(),
                }],
                usage: None,
            },
            crate::openhuman::providers::ChatResponse {
                text: Some("X is Y".into()),
                tool_calls: vec![],
                usage: None,
            },
            crate::openhuman::providers::ChatResponse {
                text: Some("Based on the research, X is Y.".into()),
                tool_calls: vec![],
                usage: None,
            },
        ]),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

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
            crate::openhuman::providers::ChatResponse {
                text: Some("first".into()),
                tool_calls: vec![],
                usage: None,
            },
            crate::openhuman::providers::ChatResponse {
                text: Some("second".into()),
                tool_calls: vec![],
                usage: None,
            },
            crate::openhuman::providers::ChatResponse {
                text: Some("third".into()),
                tool_calls: vec![],
                usage: None,
            },
        ]),
        captures: Mutex::new(Vec::new()),
    });

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

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
        crate::openhuman::providers::ChatMessage::system("warm prefix"),
        crate::openhuman::providers::ChatMessage::user("hi"),
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
