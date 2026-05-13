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
use crate::openhuman::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::openhuman::providers::{self, ConversationMessage, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolSpec};
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::collections::HashSet;
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
    ///
    /// Also rebuilds [`Self::session_key`] so the next call to
    /// `persist_session_transcript` writes to a path keyed by the new
    /// name. Without this, persist would keep using the builder-time
    /// name (e.g. `"orchestrator"`) while
    /// `find_latest_transcript` searches for the post-rename name (e.g.
    /// `"orchestrator_thread-6ad6d"`), and resume on cold boot would
    /// silently miss every prior transcript — the LLM would then run
    /// each new turn with no conversation history.
    pub fn set_agent_definition_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        let sanitized: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        // Preserve the original unix-timestamp prefix from the builder
        // so sub-agent spawn collisions remain impossible. Falls back
        // to "0" if the existing key is in an unexpected shape.
        let prefix = self
            .session_key
            .split_once('_')
            .map(|(p, _)| p)
            .filter(|p| !p.is_empty())
            .unwrap_or("0");
        self.session_key = format!("{prefix}_{sanitized}");
        self.agent_definition_name = name;
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

    /// Restrict which tools the main agent can see and call for this
    /// session. An empty set restores the default "all visible"
    /// behavior.
    pub fn set_visible_tool_names(&mut self, names: HashSet<String>) {
        self.visible_tool_names = names;
        let visible_specs = if self.visible_tool_names.is_empty() {
            (*self.tool_specs).clone()
        } else {
            self.tool_specs
                .iter()
                .filter(|spec| self.visible_tool_names.contains(&spec.name))
                .cloned()
                .collect()
        };
        self.visible_tool_specs = Arc::new(visible_specs);
    }

    /// Clears the agent's conversation history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Seed the next turn's LLM context from an authoritative message
    /// log (e.g. the web channel's per-thread conversation JSONL).
    ///
    /// Mirrors what [`Self::try_load_session_transcript`] does on a
    /// transcript-file hit, but sources from a caller-supplied list so
    /// resume works even when no transcript file exists for this
    /// agent name (the typical situation right after the
    /// `set_agent_definition_name` / `session_key` rename fix landed —
    /// existing transcripts are written under the old name).
    ///
    /// `messages` is `(role, content)` pairs in chronological order.
    /// Recognised roles: `"user"`, `"agent"` / `"assistant"`. Any
    /// trailing user message that exactly matches `current_user_message`
    /// is dropped — the caller is about to pass that text to
    /// [`Self::run_single`], which will append it to history itself, so
    /// keeping it here would duplicate it on the wire.
    ///
    /// No-ops if the agent already has a history or a cached transcript
    /// (i.e. the per-process session cache is warm). Intended only for
    /// cold-boot priming.
    pub fn seed_resume_from_messages(
        &mut self,
        messages: Vec<(String, String)>,
        current_user_message: &str,
    ) -> Result<()> {
        if !self.history.is_empty() || self.cached_transcript_messages.is_some() {
            return Ok(());
        }
        let mut prior = messages;
        if let Some(last) = prior.last() {
            if last.0 == "user" && last.1.trim() == current_user_message.trim() {
                prior.pop();
            }
        }
        if prior.is_empty() {
            return Ok(());
        }

        // Build the system prompt fresh — there's no persisted prefix
        // to preserve here, and learned-context decoration is skipped
        // intentionally so this fallback path stays synchronous and
        // doesn't fan out to the memory store on every cold-boot turn.
        let learned = crate::openhuman::agent::prompts::LearnedContextData::default();
        let system_prompt = self.build_system_prompt(learned)?;

        let mut cached: Vec<crate::openhuman::providers::ChatMessage> =
            Vec::with_capacity(prior.len() + 1);
        cached.push(crate::openhuman::providers::ChatMessage::system(
            system_prompt,
        ));
        for (role, content) in prior {
            let chat = match role.as_str() {
                "user" => crate::openhuman::providers::ChatMessage::user(content),
                "agent" | "assistant" => {
                    crate::openhuman::providers::ChatMessage::assistant(content)
                }
                // Fall back to user role for unknown senders rather than
                // dropping the message — losing context is worse than
                // mislabelling a system/tool message.
                _ => crate::openhuman::providers::ChatMessage::user(content),
            };
            cached.push(chat);
        }

        log::info!(
            "[agent] seed_resume_from_messages — primed cached transcript with {} prior messages",
            cached.len() - 1
        );
        self.cached_transcript_messages = Some(cached);
        Ok(())
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
        let guard = enforce_prompt_input(
            message,
            PromptEnforcementContext {
                source: "agent.runtime.run_single",
                request_id: None,
                user_id: Some(self.event_channel()),
                session_id: Some(self.event_session_id()),
            },
        );
        if !matches!(guard.action, PromptEnforcementAction::Allow) {
            let user_message = match guard.action {
                PromptEnforcementAction::Allow => "Message accepted.",
                PromptEnforcementAction::Blocked => "Prompt blocked by security policy.",
                PromptEnforcementAction::ReviewBlocked => {
                    "Prompt flagged for security review and was not processed."
                }
            };
            let action_tag = match guard.action {
                PromptEnforcementAction::Allow => "allow",
                PromptEnforcementAction::Blocked => "blocked",
                PromptEnforcementAction::ReviewBlocked => "review_blocked",
            };
            crate::core::observability::report_error(
                user_message,
                "agent",
                "prompt_injection_blocked",
                &[
                    ("session_id", self.event_session_id()),
                    ("channel", self.event_channel()),
                    ("action", action_tag),
                ],
            );
            publish_global(DomainEvent::AgentError {
                session_id: self.event_session_id().to_string(),
                message: user_message.to_string(),
                recoverable: true,
            });
            return Err(anyhow::anyhow!(user_message));
        }

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
                // OPENHUMAN-TAURI-5Z: upstream transient HTTP failures
                // (408/429/502/503/504) are already retried + filtered at the
                // provider layer. When retries exhaust they bubble up here via
                // `Result::Err`, and re-reporting under `domain=agent` escapes
                // the `domain=llm_provider` filter — one Sentry event per
                // failed turn for a transient infrastructure blip. Route
                // through `report_error_or_expected` so the classifier demotes
                // those (and other expected user-state errors) to a warn-level
                // breadcrumb while preserving the AgentError publish + return.
                crate::core::observability::report_error_or_expected(
                    &err,
                    "agent",
                    "run_single",
                    &[
                        ("session_id", self.event_session_id()),
                        ("channel", self.event_channel()),
                        ("error_kind", sanitized_message.as_str()),
                    ],
                );
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
#[path = "runtime_tests.rs"]
mod tests;
