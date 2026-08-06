use super::*;
use crate::core::event_bus::{global, init_global, DomainEvent};
use crate::openhuman::agent::dispatcher::XmlToolDispatcher;
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::inference::provider::{ChatResponse, UsageInfo};
use crate::openhuman::memory::Memory;
use anyhow::anyhow;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;
use tinyagents::harness::model::{
    ChatModel, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{sleep, Duration};

struct StaticModel {
    response: Mutex<Option<anyhow::Result<ChatResponse>>>,
}

#[async_trait]
impl ChatModel<()> for StaticModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let response = self.response.lock().take().unwrap_or_else(|| {
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            })
        });
        match response {
            Ok(response) => Ok(
                crate::openhuman::agent::tinyagents::model::native_model_response_for_request(
                    &response, &request,
                ),
            ),
            Err(error) => Err(tinyagents::TinyAgentsError::Model(error.to_string())),
        }
    }

    async fn stream(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelStream> {
        let response = self.invoke(state, request).await?;
        Ok(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::Completed(response),
        ])))
    }
}

/// Model that fails every call with the rendered form of a host [`AgentError`].
///
/// The default turn model (`chat-v1`) now carries a same-family cross-route
/// fallback chain (`chat-v1 → burst-v1`, issue #4249 Workstream 02.2). A mock
/// that errors only once (via `StaticModel`'s `take()`) would fail the primary
/// route and then succeed on the fallback route, masking the terminal error. To
/// exercise `run_single`'s error-surfacing path we need a model that fails on
/// every route so the harness exhausts the chain and surfaces the message.
struct PersistentErrModel {
    kind: PersistentErrKind,
}

#[derive(Clone, Copy)]
enum PersistentErrKind {
    MaxIterations { max: usize },
    PermissionDenied,
}

impl PersistentErrModel {
    fn build_error(&self) -> anyhow::Error {
        match self.kind {
            PersistentErrKind::MaxIterations { max } => {
                anyhow!(AgentError::MaxIterationsExceeded { max })
            }
            PersistentErrKind::PermissionDenied => anyhow!(AgentError::PermissionDenied {
                tool_name: "shell".into(),
                required_level: "Execute".into(),
                channel_max_level: "ReadOnly".into(),
            }),
        }
    }
}

#[async_trait]
impl ChatModel<()> for PersistentErrModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        Err(tinyagents::TinyAgentsError::Model(
            self.build_error().to_string(),
        ))
    }
}

fn make_agent(model: Arc<dyn ChatModel<()>>) -> Agent {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory::store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    Agent::builder()
        .chat_model(model)
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

    let response = crate::openhuman::inference::provider::ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    };
    let persisted = Agent::persisted_tool_calls_for_history(&response, &calls, 2);
    assert_eq!(persisted[0].id, "parsed-3-1");
    assert_eq!(persisted[1].id, "keep");

    let history = vec![
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![],
            reasoning_content: None,
            extra_metadata: None,
        },
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![],
            reasoning_content: None,
            extra_metadata: None,
        },
    ];
    assert_eq!(Agent::count_iterations(&history), 3);
}

#[tokio::test]
async fn run_single_preserves_native_model_error_text() {
    // Host-generated user-state errors remain typed and therefore retain the
    // Sentry-suppression contract at their source.
    let typed = anyhow!(AgentError::MaxIterationsExceeded { max: 8 });
    assert!(matches!(
        typed.downcast_ref::<AgentError>(),
        Some(AgentError::MaxIterationsExceeded { max: 8 })
    ));
    assert_eq!(
        Agent::sanitize_event_error_message(&typed),
        "max_iterations_exceeded"
    );

    // A crate-native model error crosses the TinyAgents boundary as its
    // provider-neutral error type rather than a downcastable host error. Its
    // user-visible text must still remain intact.
    let _ = init_global(64);

    let err_provider: Arc<dyn ChatModel<()>> = Arc::new(PersistentErrModel {
        kind: PersistentErrKind::MaxIterations { max: 8 },
    });
    let mut agent = make_agent(err_provider);
    let err = agent
        .run_single("hello")
        .await
        .expect_err("run_single should surface max-iter cap");

    // The user-visible chat string MUST stay byte-identical — the UI
    // (and `runtime_tool_calls.rs` channel test) reads this verbatim.
    assert!(
        err.to_string()
            .contains("Agent exceeded maximum tool iterations"),
        "canonical phrase missing: {err}"
    );

    assert!(
        err.downcast_ref::<AgentError>().is_none(),
        "model-boundary errors must not pretend to retain host types"
    );
    assert!(
        Agent::sanitize_event_error_message(&err)
            .contains("Agent exceeded maximum tool iterations"),
        "native error text should survive sanitization: {err}"
    );
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

    let ok_provider: Arc<dyn ChatModel<()>> = Arc::new(StaticModel {
        response: Mutex::new(Some(Ok(ChatResponse {
            text: Some("ok".into()),
            tool_calls: vec![],
            usage: Some(UsageInfo::default()),
            reasoning_content: None,
        }))),
    });
    let mut ok_agent = make_agent(ok_provider);
    let response = ok_agent.run_single("hello").await.expect("run_single ok");
    assert_eq!(response, "ok");

    let err_provider: Arc<dyn ChatModel<()>> = Arc::new(PersistentErrModel {
        kind: PersistentErrKind::PermissionDenied,
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
            && message.contains("Permission denied")
            && !recoverable
    )));
}

#[test]
fn accessors_and_history_reset_expose_agent_runtime_state() {
    let provider: Arc<dyn ChatModel<()>> = Arc::new(StaticModel {
        response: Mutex::new(None),
    });
    let mut agent = make_agent(provider);
    agent.history = vec![ConversationMessage::Chat(ChatMessage::system("sys"))];
    agent.workflows = vec![crate::openhuman::skills::Workflow {
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
    assert_eq!(agent.workflows().len(), 1);
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

    let native_calls = vec![crate::openhuman::inference::provider::ToolCall {
        id: "native-1".into(),
        name: "echo".into(),
        arguments: "{}".into(),
        extra_content: None,
    }];
    let response = crate::openhuman::inference::provider::ChatResponse {
        text: Some(String::new()),
        tool_calls: native_calls.clone(),
        usage: None,
        reasoning_content: None,
    };
    let persisted = Agent::persisted_tool_calls_for_history(&response, &[], 0);
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].id, native_calls[0].id);
    assert_eq!(persisted[0].name, native_calls[0].name);

    let long = anyhow!("{}", "x".repeat(400));
    let sanitized = Agent::sanitize_event_error_message(&long);
    assert!(sanitized.len() <= 256);
}

// ── Host capability accessors (plan-agents Phase 4) ──────────────────────────

/// The memory-backed capabilities build from a bare-builder session.
///
/// These two are the adapters that need only `Arc<dyn Memory>`, which every
/// session has however it was assembled — so they must work on the builder path
/// too, not just behind the factory.
#[tokio::test]
async fn memory_backed_host_capabilities_build_from_session_state() {
    use tinyagents::harness::host::{AgentMemory, ExperienceStore};

    let model: Arc<dyn ChatModel<()>> = Arc::new(StaticModel {
        response: Mutex::new(None),
    });
    let agent = make_agent(model);

    // Exercised through the trait objects, not the concrete types: the point of
    // the accessor is that the runtime can hold `dyn AgentMemory`.
    let memory: &dyn AgentMemory = &agent.host_agent_memory();
    let recalled = memory
        .recall(tinyagents::harness::host::RecallRequest::new("anything"))
        .await
        .expect("recall must succeed against an empty backend");
    assert!(
        recalled.is_empty(),
        "a `backend = none` memory has nothing to recall"
    );

    let experience: &dyn ExperienceStore = &agent.host_experience_store();
    let prior = experience
        .recall_for("orchestrator", "some task")
        .await
        .expect("recall_for must succeed against an empty store");
    assert!(prior.is_empty(), "no experience has been recorded yet");
}

/// A bare-builder session reports its config-dependent capabilities as
/// unavailable rather than pretending otherwise.
///
/// `host_capabilities_available()` is what keeps "this session cannot answer
/// that" distinguishable from "the capability failed" — the same
/// absence-versus-failure rule the traits are built on. The factory path sets
/// the config (`factory.rs`); the builder path deliberately does not.
#[tokio::test]
async fn a_bare_builder_session_reports_config_capabilities_unavailable() {
    let model: Arc<dyn ChatModel<()>> = Arc::new(StaticModel {
        response: Mutex::new(None),
    });
    let agent = make_agent(model);
    assert!(
        agent.runtime_config().is_none(),
        "the bare builder path supplies no host Config"
    );
    assert!(
        !agent.host_capabilities_available(),
        "config-dependent capabilities must report unavailable, not be fabricated"
    );
}
