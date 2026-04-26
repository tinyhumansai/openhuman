use super::*;
use crate::core::event_bus::{global, init_global, DomainEvent};
use crate::openhuman::agent::dispatcher::XmlToolDispatcher;
use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::agent::memory_loader::MemoryLoader;
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ChatRequest, ChatResponse, Provider};
use crate::openhuman::tools::Tool;
use crate::openhuman::tools::ToolResult;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout, Duration};

struct DummyProvider;

#[async_trait]
impl Provider for DummyProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("unused".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some("unused".into()),
            tool_calls: vec![],
            usage: None,
        })
    }
}

struct SequenceProvider {
    responses: AsyncMutex<Vec<anyhow::Result<ChatResponse>>>,
    requests: AsyncMutex<Vec<Vec<ChatMessage>>>,
}

#[async_trait]
impl Provider for SequenceProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("unused".into())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().await.push(request.messages.to_vec());
        self.responses.lock().await.remove(0)
    }
}

struct FixedMemoryLoader {
    context: String,
}

#[async_trait]
impl MemoryLoader for FixedMemoryLoader {
    async fn load_context(
        &self,
        _memory: &dyn Memory,
        _user_message: &str,
    ) -> anyhow::Result<String> {
        Ok(self.context.clone())
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("echo-output"))
    }
}

struct LongTool;

#[async_trait]
impl Tool for LongTool {
    fn name(&self) -> &str {
        "long"
    }

    fn description(&self) -> &str {
        "long"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("x".repeat(800)))
    }
}

struct RecordingHook {
    calls: Arc<AsyncMutex<Vec<TurnContext>>>,
    notify: Arc<Notify>,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        "recording"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        self.calls.lock().await.push(ctx.clone());
        self.notify.notify_waiters();
        Ok(())
    }
}

fn make_agent(visible_tool_names: Option<HashSet<String>>) -> Agent {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

    let mut builder = Agent::builder()
        .provider(Box::new(DummyProvider))
        .tools(vec![Box::new(EchoTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("turn-test-session", "turn-test-channel")
        .config(crate::openhuman::config::AgentConfig {
            max_history_messages: 3,
            ..crate::openhuman::config::AgentConfig::default()
        });

    if let Some(names) = visible_tool_names {
        builder = builder.visible_tool_names(names);
    }

    builder.build().unwrap()
}

fn make_agent_with_builder(
    provider: Arc<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    memory_loader: Box<dyn MemoryLoader>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    config: crate::openhuman::config::AgentConfig,
    context_config: crate::openhuman::config::ContextConfig,
) -> Agent {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

    Agent::builder()
        .provider_arc(provider)
        .tools(tools)
        .memory(mem)
        .memory_loader(memory_loader)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .post_turn_hooks(post_turn_hooks)
        .config(config)
        .context_config(context_config)
        .workspace_dir(workspace_path)
        .auto_save(true)
        .event_context("turn-test-session", "turn-test-channel")
        .build()
        .unwrap()
}

#[test]
fn trim_history_preserves_system_and_keeps_latest_non_system_entries() {
    let mut agent = make_agent(None);
    agent.history = vec![
        ConversationMessage::Chat(ChatMessage::system("sys")),
        ConversationMessage::Chat(ChatMessage::user("u1")),
        ConversationMessage::Chat(ChatMessage::assistant("a1")),
        ConversationMessage::Chat(ChatMessage::user("u2")),
        ConversationMessage::Chat(ChatMessage::assistant("a2")),
    ];

    agent.trim_history();

    assert_eq!(agent.history.len(), 4);
    assert!(matches!(&agent.history[0], ConversationMessage::Chat(msg) if msg.role == "system"));
    assert!(agent
        .history
        .iter()
        .all(|msg| !matches!(msg, ConversationMessage::Chat(chat) if chat.content == "u1")));
    assert!(agent
        .history
        .iter()
        .any(|msg| matches!(msg, ConversationMessage::Chat(chat) if chat.content == "a2")));
}

#[test]
fn build_fork_context_uses_visible_specs_and_prompt_argument() {
    let mut visible = HashSet::new();
    visible.insert("echo".to_string());
    let agent = make_agent(Some(visible));
    let call = ParsedToolCall {
        name: "spawn_subagent".into(),
        arguments: serde_json::json!({ "prompt": "fork task" }),
        tool_call_id: None,
    };

    let fork = agent.build_fork_context(&call);
    assert_eq!(fork.fork_task_prompt, "fork task");
    assert_eq!(fork.tool_specs.len(), 1);
    assert_eq!(fork.tool_specs[0].name, "echo");
    assert_eq!(fork.message_prefix.len(), 0);
}

#[test]
fn build_parent_context_and_sanitize_helpers_cover_snapshot_paths() {
    let mut agent = make_agent(None);
    agent.last_memory_context = Some("remember this".into());
    agent.skills = vec![crate::openhuman::skills::Skill {
        name: "demo".into(),
        ..Default::default()
    }];

    let parent = agent.build_parent_execution_context();
    assert_eq!(parent.model_name, agent.model_name);
    assert_eq!(parent.temperature, agent.temperature);
    assert_eq!(parent.memory_context.as_deref(), Some("remember this"));
    assert_eq!(parent.session_id, "turn-test-session");
    assert_eq!(parent.channel, "turn-test-channel");
    assert_eq!(parent.skills.len(), 1);

    assert_eq!(sanitize_learned_entry("   "), "");
    assert_eq!(
        sanitize_learned_entry("Bearer abcdef"),
        "[redacted: potential secret]"
    );
    let long = "x".repeat(500);
    assert_eq!(sanitize_learned_entry(&long).chars().count(), 200);
    assert!(collect_tree_root_summaries(agent.workspace_dir()).is_empty());
}

#[tokio::test]
async fn transcript_roundtrip_work() {
    let mut agent = make_agent(None);

    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("done"),
    ];
    agent.persist_session_transcript(&messages, 10, 5, 3, 0.25, None);
    assert!(agent.session_transcript_path.is_some());

    let loaded = transcript::read_transcript(agent.session_transcript_path.as_ref().unwrap())
        .expect("transcript should be readable");
    assert_eq!(loaded.messages.len(), 3);
    assert_eq!(loaded.meta.input_tokens, 10);

    let mut resumed = make_agent(None);
    resumed.workspace_dir = agent.workspace_dir.clone();
    resumed.agent_definition_name = agent.agent_definition_name.clone();
    resumed.try_load_session_transcript();
    assert_eq!(
        resumed.cached_transcript_messages.as_ref().map(|m| m.len()),
        Some(3)
    );
}

#[tokio::test]
async fn execute_tool_call_blocks_invisible_tool_and_emits_events() {
    let _ = init_global(64);
    let events = Arc::new(AsyncMutex::new(Vec::<DomainEvent>::new()));
    let events_handler = Arc::clone(&events);
    let _handle = global().unwrap().on("turn-events-test", move |event| {
        let events = Arc::clone(&events_handler);
        let cloned = event.clone();
        Box::pin(async move {
            events.lock().await.push(cloned);
        })
    });

    let mut visible = HashSet::new();
    visible.insert("other".to_string());
    let agent = make_agent(Some(visible));
    let call = ParsedToolCall {
        name: "echo".into(),
        arguments: serde_json::json!({}),
        tool_call_id: Some("tc-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(!result.success);
    assert!(result.output.contains("not available to this agent"));
    assert_eq!(record.name, "echo");
    assert!(!record.success);

    sleep(Duration::from_millis(20)).await;
    let captured = events.lock().await;
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::ToolExecutionStarted { tool_name, session_id }
            if tool_name == "echo" && session_id == "turn-test-session"
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::ToolExecutionCompleted {
            tool_name,
            session_id,
            success,
            ..
        } if tool_name == "echo" && session_id == "turn-test-session" && !success
    )));
}

#[tokio::test]
async fn execute_tool_call_reports_unknown_tool() {
    let agent = make_agent(None);
    let call = ParsedToolCall {
        name: "missing".into(),
        arguments: serde_json::json!({}),
        tool_call_id: None,
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(!result.success);
    assert!(result.output.contains("Unknown tool: missing"));
    assert_eq!(record.name, "missing");
    assert!(!record.success);
}

#[tokio::test]
async fn turn_runs_full_tool_cycle_with_context_and_hooks() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some(
                    "preface <tool_call>{\"name\":\"echo\",\"arguments\":{\"value\":1}}</tool_call>"
                        .into(),
                ),
                tool_calls: vec![],
                usage: None,
            }),
            Ok(ChatResponse {
                text: Some("final answer".into()),
                tool_calls: vec![],
                usage: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let hook_calls = Arc::new(AsyncMutex::new(Vec::<TurnContext>::new()));
    let hook_notify = Arc::new(Notify::new());
    let hooks: Vec<Arc<dyn PostTurnHook>> = vec![Arc::new(RecordingHook {
        calls: Arc::clone(&hook_calls),
        notify: Arc::clone(&hook_notify),
    })];

    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: "[Injected]\n".into(),
        }),
        hooks,
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            max_history_messages: 10,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let response = agent
        .turn("hello world")
        .await
        .expect("turn should succeed");
    assert_eq!(response, "final answer");
    assert!(agent.last_memory_context.as_deref() == Some("[Injected]\n"));
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::AssistantToolCalls { text, tool_calls }
            if text.as_deref().is_some_and(|value| value.contains("preface")) && tool_calls.len() == 1
    )));
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::Chat(chat) if chat.role == "assistant" && chat.content == "final answer"
    )));

    timeout(Duration::from_secs(1), async {
        loop {
            if !hook_calls.lock().await.is_empty() {
                break;
            }
            hook_notify.notified().await;
        }
    })
    .await
    .expect("hook should fire");

    let recorded_hooks = hook_calls.lock().await;
    assert_eq!(recorded_hooks.len(), 1);
    assert_eq!(recorded_hooks[0].assistant_response, "final answer");
    assert_eq!(recorded_hooks[0].iteration_count, 2);
    assert_eq!(recorded_hooks[0].tool_calls.len(), 1);
    assert_eq!(recorded_hooks[0].tool_calls[0].name, "echo");
    drop(recorded_hooks);

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0][0].role, "system");
    assert!(requests[0][1].content.contains("[Injected]"));
    assert!(requests[0][1].content.contains("hello world"));
    assert!(requests[1]
        .iter()
        .any(|msg| msg.role == "assistant" && msg.content.contains("preface")));
    assert!(requests[1]
        .iter()
        .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]")));
}

#[tokio::test]
async fn turn_uses_cached_transcript_prefix_on_first_iteration() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("cached-final".into()),
            tool_calls: vec![],
            usage: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );
    agent.cached_transcript_messages = Some(vec![
        ChatMessage::system("cached-system"),
        ChatMessage::assistant("cached-assistant"),
    ]);

    let response = agent.turn("fresh").await.expect("turn should succeed");
    assert_eq!(response, "cached-final");
    assert!(agent.cached_transcript_messages.is_none());

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].len(), 3);
    assert_eq!(requests[0][0].content, "cached-system");
    assert_eq!(requests[0][1].content, "cached-assistant");
    assert_eq!(requests[0][2].role, "user");
    assert_eq!(requests[0][2].content, "fresh");
}

#[tokio::test]
async fn turn_errors_when_max_tool_iterations_are_exceeded() {
    let provider: Arc<dyn Provider> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
            tool_calls: vec![],
            usage: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let err = agent
        .turn("hello")
        .await
        .expect_err("turn should stop at configured iteration budget");
    assert!(err
        .to_string()
        .contains("Agent exceeded maximum tool iterations (1)"));
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::AssistantToolCalls { tool_calls, .. } if tool_calls.len() == 1
    )));
}

#[tokio::test]
async fn execute_tool_call_applies_inline_result_budget() {
    let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
    let agent = make_agent_with_builder(
        provider,
        vec![Box::new(LongTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig {
            tool_result_budget_bytes: 300,
            ..crate::openhuman::config::ContextConfig::default()
        },
    );
    let call = ParsedToolCall {
        name: "long".into(),
        arguments: serde_json::json!({}),
        tool_call_id: Some("long-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(result.success);
    assert!(result.output.contains("truncated by tool_result_budget"));
    assert!(record.output_summary.starts_with("long: ok ("));
}
