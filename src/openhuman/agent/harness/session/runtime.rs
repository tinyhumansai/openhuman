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

    /// Clears the agent's conversation history.
    pub fn clear_history(&mut self) {
        self.history.clear();
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
    /// Each incoming message is dispatched through [`Agent::run_single`] so
    /// the unified lifecycle events (`AgentTurnStarted`, `AgentTurnCompleted`,
    /// `AgentError`) and error sanitisation run for interactive turns just
    /// like they do for one-shot invocations.
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
