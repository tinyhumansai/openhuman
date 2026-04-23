//! Public accessors, `run_single` / `run_interactive` CLI helpers, and
//! assorted per-turn static helpers (id-fallback injection, event-error
//! sanitisation, history diffing).
//!
//! These used to live alongside the turn loop in `agent.rs`. Splitting
//! them out keeps `turn.rs` focused on the interaction lifecycle and
//! makes it obvious which methods are cheap getters vs which actually
//! drive the model.

use super::types::{Agent, AgentBuilder};
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::dispatcher::ParsedToolCall;
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{self, ConversationMessage, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolSpec};
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::sync::Arc;

impl Agent {
    const EVENT_ERROR_MAX_CHARS: usize = 256;

    // ─────────────────────────────────────────────────────────────────
    // Small accessors used by `run_single` + `turn` + sub-agent runner
    // ─────────────────────────────────────────────────────────────────

    pub(super) fn event_session_id(&self) -> &str {
        &self.event_session_id
    }

    pub(super) fn event_channel(&self) -> &str {
        &self.event_channel
    }

    /// The agent definition id this session is running
    /// (`"welcome"`, `"orchestrator"`, `"integrations_agent"`, …).
    ///
    /// Exposed so callers that build sessions via
    /// [`Agent::from_config_for_agent`] can stamp the resolved id onto
    /// correlation logs and progress events without reaching for the
    /// source `Config`. See [`AgentBuilder::agent_definition_name`]
    /// for the full list of downstream surfaces (transcript filename,
    /// transcript metadata header, and `PromptContext::agent_id`) that
    /// read this field.
    pub fn agent_definition_name(&self) -> &str {
        &self.agent_definition_name
    }

    /// Returns a new `AgentBuilder`.
    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    /// Borrow the agent's provider as an `Arc`. Used by the sub-agent
    /// runner to share the parent's provider instance with spawned
    /// sub-agents (so they share connection pools, retry budgets, and
    /// rate-limit state).
    pub fn provider_arc(&self) -> Arc<dyn Provider> {
        Arc::clone(&self.provider)
    }

    /// Borrow the agent's tools as a slice. Used by the sub-agent runner
    /// to filter the parent's tool registry per-archetype.
    pub fn tools(&self) -> &[Box<dyn Tool>] {
        self.tools.as_slice()
    }

    /// Clone the agent's tools `Arc` for sharing with sub-agents.
    pub fn tools_arc(&self) -> Arc<Vec<Box<dyn Tool>>> {
        Arc::clone(&self.tools)
    }

    /// Borrow the agent's tool specs (pre-serialised). Captured at
    /// turn-start so sub-agents can pass byte-identical schemas to the
    /// provider for prefix-cache reuse.
    pub fn tool_specs(&self) -> &[ToolSpec] {
        self.tool_specs.as_slice()
    }

    /// Clone the agent's tool specs `Arc` for sharing with sub-agents.
    pub fn tool_specs_arc(&self) -> Arc<Vec<ToolSpec>> {
        Arc::clone(&self.tool_specs)
    }

    /// Borrow the agent's memory backing store as an `Arc`.
    pub fn memory_arc(&self) -> Arc<dyn Memory> {
        Arc::clone(&self.memory)
    }

    /// The agent's working directory.
    pub fn workspace_dir(&self) -> &std::path::Path {
        &self.workspace_dir
    }

    /// The agent's currently-configured model name (before per-turn
    /// auto-classification).
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// The agent's currently-configured temperature.
    pub fn temperature(&self) -> f64 {
        self.temperature
    }

    /// The agent's loaded skills, if any.
    pub fn skills(&self) -> &[crate::openhuman::skills::Skill] {
        &self.skills
    }

    /// Active Composio integrations fetched at session start.
    pub fn connected_integrations(
        &self,
    ) -> &[crate::openhuman::context::prompt::ConnectedIntegration] {
        &self.connected_integrations
    }

    /// The Composio client cached on the session, if any. Populated by
    /// [`Agent::fetch_connected_integrations`]; remains `None` when the
    /// user is not signed in.
    pub fn composio_client(&self) -> Option<&crate::openhuman::composio::ComposioClient> {
        self.composio_client.as_ref()
    }

    /// This session's transcript key — `"{unix_ts}_{agent_id}"`,
    /// generated once at build time. Sub-agents chain this into their
    /// own transcript filenames so the parent → child hierarchy is
    /// visible on disk.
    pub fn session_key(&self) -> &str {
        &self.session_key
    }

    /// The ancestor chain of session keys for a sub-agent, joined with
    /// `__`. `None` for a root session. Root + prefix together produce
    /// the full transcript stem.
    pub fn session_parent_prefix(&self) -> Option<&str> {
        self.session_parent_prefix.as_deref()
    }

    /// Session-scoped curated-memory snapshot. `None` until the first
    /// turn takes it, or when the curated-memory runtime isn't
    /// initialised (unit tests).
    pub fn curated_snapshot(
        &self,
    ) -> Option<std::sync::Arc<crate::openhuman::curated_memory::MemorySnapshot>> {
        self.curated_snapshot.clone()
    }

    /// Replace the agent's connected integrations (e.g. from a cached
    /// fetch result when the agent was built outside the normal turn loop).
    pub fn set_connected_integrations(
        &mut self,
        integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration>,
    ) {
        self.connected_integrations = integrations;
    }

    /// The agent's runtime config snapshot.
    pub fn agent_config(&self) -> &crate::openhuman::config::AgentConfig {
        &self.config
    }

    /// Returns the current conversation history.
    pub fn history(&self) -> &[ConversationMessage] {
        &self.history
    }

    pub fn set_event_context(&mut self, session_id: impl Into<String>, channel: impl Into<String>) {
        self.event_session_id = session_id.into();
        self.event_channel = channel.into();
    }

    /// Override the agent definition name used for session transcript
    /// file paths. Callers (e.g. the web channel) use this to scope
    /// transcripts per thread so each conversation thread gets its own
    /// transcript namespace instead of sharing one by agent type.
    pub fn set_agent_definition_name(&mut self, name: impl Into<String>) {
        self.agent_definition_name = name.into();
    }

    /// Attach a progress event sender for real-time turn updates.
    ///
    /// When set, the turn loop emits [`AgentProgress`] events so
    /// callers (e.g. the web channel) can surface live tool-call and
    /// iteration updates to the UI. Pass `None` to disable.
    pub fn set_on_progress(
        &mut self,
        tx: Option<tokio::sync::mpsc::Sender<crate::openhuman::agent::progress::AgentProgress>>,
    ) {
        self.on_progress = tx;
    }

    /// Clears the agent's conversation history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Drain and return memory citations collected for the latest completed turn.
    pub fn take_last_turn_citations(
        &mut self,
    ) -> Vec<crate::openhuman::agent::memory_loader::MemoryCitation> {
        std::mem::take(&mut self.last_turn_citations)
    }

    // ─────────────────────────────────────────────────────────────────
    // Static helpers for turn parsing + telemetry
    // ─────────────────────────────────────────────────────────────────

    pub(super) fn count_iterations(messages: &[ConversationMessage]) -> usize {
        messages
            .iter()
            .filter(|message| matches!(message, ConversationMessage::AssistantToolCalls { .. }))
            .count()
            + 1
    }

    fn conversation_message_eq(left: &ConversationMessage, right: &ConversationMessage) -> bool {
        serde_json::to_string(left).ok() == serde_json::to_string(right).ok()
    }

    fn message_slice_eq(left: &[ConversationMessage], right: &[ConversationMessage]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right.iter())
                .all(|(left, right)| Self::conversation_message_eq(left, right))
    }

    pub(super) fn new_entries_for_turn<'a>(
        history_snapshot: &[ConversationMessage],
        current_history: &'a [ConversationMessage],
    ) -> &'a [ConversationMessage] {
        let common_prefix_len = history_snapshot
            .iter()
            .zip(current_history.iter())
            .take_while(|(left, right)| Self::conversation_message_eq(left, right))
            .count();

        if common_prefix_len == history_snapshot.len() {
            return &current_history[common_prefix_len..];
        }

        let max_overlap = history_snapshot.len().min(current_history.len());
        for overlap in (0..=max_overlap).rev() {
            let snapshot_suffix = &history_snapshot[history_snapshot.len() - overlap..];
            let current_prefix = &current_history[..overlap];
            if Self::message_slice_eq(snapshot_suffix, current_prefix) {
                return &current_history[overlap..];
            }
        }

        current_history
    }

    pub(super) fn sanitize_event_error_message(err: &anyhow::Error) -> String {
        let kind = match err.downcast_ref::<AgentError>() {
            Some(AgentError::ProviderError { .. }) => Some("provider_error"),
            Some(AgentError::ContextLimitExceeded { .. }) => Some("context_limit_exceeded"),
            Some(AgentError::ToolExecutionError { .. }) => Some("tool_execution_error"),
            Some(AgentError::CostBudgetExceeded { .. }) => Some("cost_budget_exceeded"),
            Some(AgentError::MaxIterationsExceeded { .. }) => Some("max_iterations_exceeded"),
            Some(AgentError::CompactionFailed { .. }) => Some("compaction_failed"),
            Some(AgentError::PermissionDenied { .. }) => Some("permission_denied"),
            Some(AgentError::Other(_)) | None => None,
        };

        if let Some(kind) = kind {
            return kind.to_string();
        }

        let scrubbed = providers::sanitize_api_error(&err.to_string())
            .replace(['\n', '\r', '\t'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        truncate_with_ellipsis(&scrubbed, Self::EVENT_ERROR_MAX_CHARS)
    }

    /// Injects unique IDs into tool calls that are missing them.
    ///
    /// This is necessary for some tool dispatchers to correctly track and
    /// associate results.
    pub(super) fn with_fallback_tool_call_ids(
        mut parsed_calls: Vec<ParsedToolCall>,
        iteration: usize,
    ) -> Vec<ParsedToolCall> {
        for (idx, call) in parsed_calls.iter_mut().enumerate() {
            if call.tool_call_id.is_none() {
                call.tool_call_id = Some(format!("parsed-{}-{}", iteration + 1, idx + 1));
            }
        }
        parsed_calls
    }

    /// Converts parsed tool calls into the provider-standard `ToolCall` format.
    ///
    /// If the provider response already contains native tool calls, they are
    /// returned as-is.
    pub(super) fn persisted_tool_calls_for_history(
        response: &crate::openhuman::providers::ChatResponse,
        parsed_calls: &[ParsedToolCall],
        iteration: usize,
    ) -> Vec<ToolCall> {
        if !response.tool_calls.is_empty() {
            return response.tool_calls.clone();
        }

        parsed_calls
            .iter()
            .enumerate()
            .map(|(idx, call)| ToolCall {
                id: call
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| format!("parsed-{}-{}", iteration + 1, idx + 1)),
                name: call.name.clone(),
                arguments: call.arguments.to_string(),
            })
            .collect()
    }

    // ─────────────────────────────────────────────────────────────────
    // Run helpers — single-shot and interactive loops
    // ─────────────────────────────────────────────────────────────────

    /// Runs a single turn with the given message and returns the response.
    ///
    /// This is the primary high-level method for programmatic interaction with the agent.
    /// It wraps the core `turn` logic with telemetry events (`AgentTurnStarted`,
    /// `AgentTurnCompleted`) and error sanitization.
    pub async fn run_single(&mut self, message: &str) -> Result<String> {
        let history_snapshot = self.history.clone();
        publish_global(DomainEvent::AgentTurnStarted {
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
        });

        match self.turn(message).await {
            Ok(response) => {
                let new_entries = Self::new_entries_for_turn(&history_snapshot, &self.history);
                publish_global(DomainEvent::AgentTurnCompleted {
                    session_id: self.event_session_id().to_string(),
                    text_chars: response.chars().count(),
                    iterations: Self::count_iterations(new_entries),
                });
                Ok(response)
            }
            Err(err) => {
                let sanitized_message = Self::sanitize_event_error_message(&err);
                publish_global(DomainEvent::AgentError {
                    session_id: self.event_session_id().to_string(),
                    message: sanitized_message,
                    recoverable: false,
                });
                Err(err)
            }
        }
    }

    /// Runs an interactive CLI loop, reading from standard input and printing to standard output.
    ///
    /// This method starts a persistent session where the user can chat with the agent
    /// directly from the console. It handles input until a termination command
    /// (e.g., `/quit`) is received.
    pub async fn run_interactive(&mut self) -> Result<()> {
        println!("🦀 OpenHuman Interactive Mode");
        println!("Type /quit to exit.\n");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let cli = crate::openhuman::channels::CliChannel::new();

        let listen_handle = tokio::spawn(async move {
            let _ = crate::openhuman::channels::Channel::listen(&cli, tx).await;
        });

        while let Some(msg) = rx.recv().await {
            match self.run_single(&msg.content).await {
                Ok(response) => println!("\n{response}\n"),
                Err(e) => {
                    // `run_single` already publishes `AgentError` and
                    // sanitises the payload; surface a concise line here
                    // for the CLI user and continue the loop.
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            }
        }

        listen_handle.abort();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap(),
        );

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
}
