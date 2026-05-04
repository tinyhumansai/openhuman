use super::*;
use crate::core::event_bus::{global, init_global, DomainEvent};
use crate::openhuman::agent::dispatcher::XmlToolDispatcher;
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ChatMessage, ChatRequest, ChatResponse, UsageInfo};
use anyhow::anyhow;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{sleep, Duration};

struct StaticProvider {
    response: Mutex<Option<anyhow::Result<ChatResponse>>>,
}

#[async_trait]
impl Provider for StaticProvider {
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
        self.response.lock().take().unwrap_or_else(|| {
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
            })
        })
    }
}

fn make_agent(provider: Arc<dyn Provider>) -> Agent {
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
        .tools(vec![])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("runtime-test-session", "runtime-test-channel")
        .build()
        .unwrap()
}

#[test]
fn new_entries_for_turn_detects_prefix_overlap_and_fallbacks() {
    let history_snapshot = vec![
        ConversationMessage::Chat(ChatMessage::user("a")),
        ConversationMessage::Chat(ChatMessage::assistant("b")),
    ];
    let current_history = vec![
        ConversationMessage::Chat(ChatMessage::user("a")),
        ConversationMessage::Chat(ChatMessage::assistant("b")),
        ConversationMessage::Chat(ChatMessage::assistant("c")),
    ];
    let appended = Agent::new_entries_for_turn(&history_snapshot, &current_history);
    assert_eq!(appended.len(), 1);

    let shifted_history = vec![
        ConversationMessage::Chat(ChatMessage::assistant("b")),
        ConversationMessage::Chat(ChatMessage::assistant("c")),
    ];
    let overlap = Agent::new_entries_for_turn(&history_snapshot, &shifted_history);
    assert_eq!(overlap.len(), 1);
    assert!(matches!(&overlap[0], ConversationMessage::Chat(msg) if msg.content == "c"));
}

#[test]
fn sanitizers_and_tool_call_helpers_cover_fallback_paths() {
    let err = anyhow!(AgentError::PermissionDenied {
        tool_name: "shell".into(),
        required_level: "Execute".into(),
        channel_max_level: "ReadOnly".into(),
    });
    assert_eq!(
        Agent::sanitize_event_error_message(&err),
        "permission_denied"
    );

    let generic = anyhow!("bad key sk-123456789012345678901234567890\nwith\twhitespace");
    let sanitized = Agent::sanitize_event_error_message(&generic);
    assert!(!sanitized.contains('\n'));
    assert!(!sanitized.contains('\t'));

    let calls = vec![
        crate::openhuman::agent::dispatcher::ParsedToolCall {
            name: "a".into(),
            arguments: serde_json::json!({}),
            tool_call_id: None,
        },
        crate::openhuman::agent::dispatcher::ParsedToolCall {
            name: "b".into(),
            arguments: serde_json::json!({"x":1}),
            tool_call_id: Some("keep".into()),
        },
    ];
    let calls = Agent::with_fallback_tool_call_ids(calls, 2);
    assert_eq!(calls[0].tool_call_id.as_deref(), Some("parsed-3-1"));
    assert_eq!(calls[1].tool_call_id.as_deref(), Some("keep"));

    let response = crate::openhuman::providers::ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![],
        usage: None,
    };
    let persisted = Agent::persisted_tool_calls_for_history(&response, &calls, 2);
    assert_eq!(persisted[0].id, "parsed-3-1");
    assert_eq!(persisted[1].id, "keep");

    let history = vec![
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![],
        },
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![],
        },
    ];
    assert_eq!(Agent::count_iterations(&history), 3);
}

#[tokio::test]
async fn run_single_publishes_completed_and_error_events() {
    let _ = init_global(64);
    let events = Arc::new(AsyncMutex::new(Vec::<DomainEvent>::new()));
    let events_handler = Arc::clone(&events);
    let _handle = global().unwrap().on("runtime-events-test", move |event| {
        let events = Arc::clone(&events_handler);
        let cloned = event.clone();
        Box::pin(async move {
            events.lock().await.push(cloned);
        })
    });

    let ok_provider: Arc<dyn Provider> = Arc::new(StaticProvider {
        response: Mutex::new(Some(Ok(ChatResponse {
            text: Some("ok".into()),
            tool_calls: vec![],
            usage: Some(UsageInfo::default()),
        }))),
    });
    let mut ok_agent = make_agent(ok_provider);
    let response = ok_agent.run_single("hello").await.expect("run_single ok");
    assert_eq!(response, "ok");

    let err_provider: Arc<dyn Provider> = Arc::new(StaticProvider {
        response: Mutex::new(Some(Err(anyhow!(AgentError::PermissionDenied {
            tool_name: "shell".into(),
            required_level: "Execute".into(),
            channel_max_level: "ReadOnly".into(),
        })))),
    });
    let mut err_agent = make_agent(err_provider);
    let err = err_agent
        .run_single("hello")
        .await
        .expect_err("run_single should publish error");
    assert!(err.to_string().contains("Permission denied"));

    sleep(Duration::from_millis(20)).await;
    let captured = events.lock().await;
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::AgentTurnStarted { session_id, channel }
            if session_id == "runtime-test-session" && channel == "runtime-test-channel"
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::AgentTurnCompleted {
            session_id,
            text_chars,
            iterations,
        } if session_id == "runtime-test-session" && *text_chars == 2 && *iterations >= 1
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::AgentError {
            session_id,
            message,
            recoverable,
        } if session_id == "runtime-test-session"
            && message == "permission_denied"
            && !recoverable
    )));
}

#[test]
fn accessors_and_history_reset_expose_agent_runtime_state() {
    let provider: Arc<dyn Provider> = Arc::new(StaticProvider {
        response: Mutex::new(None),
    });
    let mut agent = make_agent(provider);
    agent.history = vec![ConversationMessage::Chat(ChatMessage::system("sys"))];
    agent.skills = vec![crate::openhuman::skills::Skill {
        name: "demo".into(),
        ..Default::default()
    }];

    assert_eq!(agent.event_session_id(), "runtime-test-session");
    assert_eq!(agent.event_channel(), "runtime-test-channel");
    assert_eq!(agent.tools().len(), 0);
    assert_eq!(agent.tool_specs().len(), 0);
    assert_eq!(agent.workspace_dir(), agent.workspace_dir.as_path());
    assert_eq!(agent.model_name(), agent.model_name);
    assert_eq!(agent.temperature(), agent.temperature);
    assert_eq!(agent.skills().len(), 1);
    assert_eq!(
        agent.agent_config().max_tool_iterations,
        agent.config.max_tool_iterations
    );
    assert_eq!(agent.history().len(), 1);
    assert!(!agent.memory_arc().name().is_empty());

    agent.set_event_context("updated-session", "updated-channel");
    assert_eq!(agent.event_session_id(), "updated-session");
    assert_eq!(agent.event_channel(), "updated-channel");

    agent.clear_history();
    assert!(agent.history().is_empty());
    assert_eq!(Agent::count_iterations(agent.history()), 1);
}

#[test]
fn helper_paths_cover_no_overlap_native_calls_and_truncation() {
    let history_snapshot = vec![ConversationMessage::Chat(ChatMessage::user("a"))];
    let current_history = vec![ConversationMessage::Chat(ChatMessage::assistant("b"))];
    let appended = Agent::new_entries_for_turn(&history_snapshot, &current_history);
    assert_eq!(appended.len(), 1);
    assert!(matches!(&appended[0], ConversationMessage::Chat(msg) if msg.content == "b"));

    let native_calls = vec![crate::openhuman::providers::ToolCall {
        id: "native-1".into(),
        name: "echo".into(),
        arguments: "{}".into(),
    }];
    let response = crate::openhuman::providers::ChatResponse {
        text: Some(String::new()),
        tool_calls: native_calls.clone(),
        usage: None,
    };
    let persisted = Agent::persisted_tool_calls_for_history(&response, &[], 0);
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].id, native_calls[0].id);
    assert_eq!(persisted[0].name, native_calls[0].name);

    let long = anyhow!("{}", "x".repeat(400));
    let sanitized = Agent::sanitize_event_error_message(&long);
    assert!(sanitized.len() <= 256);
}
