//! Core agent implementation for the OpenHuman platform.
//!
//! This module provides the `Agent` struct, which orchestrates the interaction
//! between the AI provider, available tools, memory systems, and the user.
//! It handles the agent's "turn" logic, including tool execution and history
//! management.

use super::dispatcher::{
    NativeToolDispatcher, ParsedToolCall, ToolDispatcher, ToolExecutionResult, XmlToolDispatcher,
};
use super::error::AgentError;
use super::hooks::{self, sanitize_tool_output, PostTurnHook, ToolCallRecord, TurnContext};
use super::memory_loader::{DefaultMemoryLoader, MemoryLoader};
use super::prompt::{PromptContext, SystemPromptBuilder};
use crate::openhuman::agent::host_runtime;
use crate::openhuman::config::Config;
use crate::openhuman::event_bus::{publish_global, DomainEvent};
use crate::openhuman::memory::{self, Memory, MemoryCategory};
use crate::openhuman::providers::{
    self, ChatMessage, ChatRequest, ConversationMessage, Provider, ToolCall,
};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::{self, Tool, ToolSpec};
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::io::Write as IoWrite;
use std::sync::Arc;

/// An autonomous or semi-autonomous AI agent.
///
/// The `Agent` is the central component that manages conversation state,
/// executes tools based on model requests, and interacts with the memory system
/// to maintain context across turns.
pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Arc<Vec<Box<dyn Tool>>>,
    tool_specs: Arc<Vec<ToolSpec>>,
    memory: Arc<dyn Memory>,
    prompt_builder: SystemPromptBuilder,
    tool_dispatcher: Box<dyn ToolDispatcher>,
    memory_loader: Box<dyn MemoryLoader>,
    config: crate::openhuman::config::AgentConfig,
    model_name: String,
    temperature: f64,
    workspace_dir: std::path::PathBuf,
    identity_config: crate::openhuman::config::IdentityConfig,
    skills: Vec<crate::openhuman::skills::Skill>,
    auto_save: bool,
    history: Vec<ConversationMessage>,
    classification_config: crate::openhuman::config::QueryClassificationConfig,
    available_hints: Vec<String>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    learning_enabled: bool,
    event_session_id: String,
    event_channel: String,
    /// Layered context reduction pipeline (tool-result budget →
    /// microcompact → autocompact signal → session-memory extraction
    /// trigger). Owned by the agent so its state (token counters,
    /// session-memory extraction deltas, compaction circuit breaker)
    /// persists across turns. See
    /// [`crate::openhuman::agent::context_pipeline`] for the stage
    /// ordering and cache-safety contract.
    context_pipeline: super::context_pipeline::ContextPipeline,
}

/// A builder for creating `Agent` instances with custom configuration.
pub struct AgentBuilder {
    provider: Option<Arc<dyn Provider>>,
    tools: Option<Vec<Box<dyn Tool>>>,
    memory: Option<Arc<dyn Memory>>,
    prompt_builder: Option<SystemPromptBuilder>,
    tool_dispatcher: Option<Box<dyn ToolDispatcher>>,
    memory_loader: Option<Box<dyn MemoryLoader>>,
    config: Option<crate::openhuman::config::AgentConfig>,
    model_name: Option<String>,
    temperature: Option<f64>,
    workspace_dir: Option<std::path::PathBuf>,
    identity_config: Option<crate::openhuman::config::IdentityConfig>,
    skills: Option<Vec<crate::openhuman::skills::Skill>>,
    auto_save: Option<bool>,
    classification_config: Option<crate::openhuman::config::QueryClassificationConfig>,
    available_hints: Option<Vec<String>>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    learning_enabled: bool,
    event_session_id: Option<String>,
    event_channel: Option<String>,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBuilder {
    /// Creates a new `AgentBuilder` with default values.
    pub fn new() -> Self {
        Self {
            provider: None,
            tools: None,
            memory: None,
            prompt_builder: None,
            tool_dispatcher: None,
            memory_loader: None,
            config: None,
            model_name: None,
            temperature: None,
            workspace_dir: None,
            identity_config: None,
            skills: None,
            auto_save: None,
            classification_config: None,
            available_hints: None,
            post_turn_hooks: Vec::new(),
            learning_enabled: false,
            event_session_id: None,
            event_channel: None,
        }
    }

    /// Sets the AI provider for the agent.
    ///
    /// Accepts a `Box<dyn Provider>` for backward compatibility but stores
    /// the provider as an `Arc` internally so sub-agents spawned from this
    /// agent (via `spawn_subagent`) can share the same instance.
    pub fn provider(mut self, provider: Box<dyn Provider>) -> Self {
        self.provider = Some(Arc::from(provider));
        self
    }

    /// Sets the AI provider from an existing `Arc`. Use this when sharing
    /// a provider instance across multiple agents.
    pub fn provider_arc(mut self, provider: Arc<dyn Provider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Sets the available tools for the agent.
    pub fn tools(mut self, tools: Vec<Box<dyn Tool>>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Sets the memory system for the agent.
    pub fn memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Sets the system prompt builder for the agent.
    pub fn prompt_builder(mut self, prompt_builder: SystemPromptBuilder) -> Self {
        self.prompt_builder = Some(prompt_builder);
        self
    }

    /// Sets the tool dispatcher for the agent.
    pub fn tool_dispatcher(mut self, tool_dispatcher: Box<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(tool_dispatcher);
        self
    }

    /// Sets the memory loader for the agent.
    pub fn memory_loader(mut self, memory_loader: Box<dyn MemoryLoader>) -> Self {
        self.memory_loader = Some(memory_loader);
        self
    }

    /// Sets the agent configuration.
    pub fn config(mut self, config: crate::openhuman::config::AgentConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Sets the model name to use for chat requests.
    pub fn model_name(mut self, model_name: String) -> Self {
        self.model_name = Some(model_name);
        self
    }

    /// Sets the temperature for chat requests.
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Sets the workspace directory for the agent.
    pub fn workspace_dir(mut self, workspace_dir: std::path::PathBuf) -> Self {
        self.workspace_dir = Some(workspace_dir);
        self
    }

    /// Sets the identity configuration for the agent.
    pub fn identity_config(
        mut self,
        identity_config: crate::openhuman::config::IdentityConfig,
    ) -> Self {
        self.identity_config = Some(identity_config);
        self
    }

    /// Sets the skills available to the agent.
    pub fn skills(mut self, skills: Vec<crate::openhuman::skills::Skill>) -> Self {
        self.skills = Some(skills);
        self
    }

    /// Enables or disables automatic saving of conversation history to memory.
    pub fn auto_save(mut self, auto_save: bool) -> Self {
        self.auto_save = Some(auto_save);
        self
    }

    /// Sets the query classification configuration.
    pub fn classification_config(
        mut self,
        classification_config: crate::openhuman::config::QueryClassificationConfig,
    ) -> Self {
        self.classification_config = Some(classification_config);
        self
    }

    /// Sets the available model hints for auto-classification.
    pub fn available_hints(mut self, available_hints: Vec<String>) -> Self {
        self.available_hints = Some(available_hints);
        self
    }

    /// Sets the post-turn hooks to be executed after each turn.
    pub fn post_turn_hooks(mut self, hooks: Vec<Arc<dyn PostTurnHook>>) -> Self {
        self.post_turn_hooks = hooks;
        self
    }

    /// Enables or disables learning features.
    pub fn learning_enabled(mut self, enabled: bool) -> Self {
        self.learning_enabled = enabled;
        self
    }

    pub fn event_context(
        mut self,
        session_id: impl Into<String>,
        channel: impl Into<String>,
    ) -> Self {
        self.event_session_id = Some(session_id.into());
        self.event_channel = Some(channel.into());
        self
    }

    /// Validates the configuration and builds the `Agent` instance.
    pub fn build(self) -> Result<Agent> {
        let tools = self
            .tools
            .ok_or_else(|| anyhow::anyhow!("tools are required"))?;
        let tool_specs: Vec<ToolSpec> = tools.iter().map(|tool| tool.spec()).collect();

        Ok(Agent {
            provider: self
                .provider
                .ok_or_else(|| anyhow::anyhow!("provider is required"))?,
            tools: Arc::new(tools),
            tool_specs: Arc::new(tool_specs),
            memory: self
                .memory
                .ok_or_else(|| anyhow::anyhow!("memory is required"))?,
            prompt_builder: self
                .prompt_builder
                .unwrap_or_else(SystemPromptBuilder::with_defaults),
            tool_dispatcher: self
                .tool_dispatcher
                .ok_or_else(|| anyhow::anyhow!("tool_dispatcher is required"))?,
            memory_loader: self
                .memory_loader
                .unwrap_or_else(|| Box::new(DefaultMemoryLoader::default())),
            config: self.config.unwrap_or_default(),
            model_name: self
                .model_name
                .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.into()),
            temperature: self.temperature.unwrap_or(0.7),
            workspace_dir: self
                .workspace_dir
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
            identity_config: self.identity_config.unwrap_or_default(),
            skills: self.skills.unwrap_or_default(),
            auto_save: self.auto_save.unwrap_or(false),
            history: Vec::new(),
            classification_config: self.classification_config.unwrap_or_default(),
            available_hints: self.available_hints.unwrap_or_default(),
            post_turn_hooks: self.post_turn_hooks,
            learning_enabled: self.learning_enabled,
            event_session_id: self
                .event_session_id
                .unwrap_or_else(|| "standalone".to_string()),
            event_channel: self.event_channel.unwrap_or_else(|| "internal".to_string()),
            context_pipeline: super::context_pipeline::ContextPipeline::default(),
        })
    }
}

impl Agent {
    const EVENT_ERROR_MAX_CHARS: usize = 256;

    fn event_session_id(&self) -> &str {
        &self.event_session_id
    }

    fn event_channel(&self) -> &str {
        &self.event_channel
    }

    fn count_iterations(messages: &[ConversationMessage]) -> usize {
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

    fn new_entries_for_turn<'a>(
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

    fn sanitize_event_error_message(err: &anyhow::Error) -> String {
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
    fn with_fallback_tool_call_ids(
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
    fn persisted_tool_calls_for_history(
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

    /// The agent's identity config (used by sub-agent prompt building
    /// when `omit_identity = false`).
    pub fn identity_config(&self) -> &crate::openhuman::config::IdentityConfig {
        &self.identity_config
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

    /// Creates an `Agent` instance from a global configuration.
    ///
    /// This is the primary way to initialize an agent with all system
    /// integrations (memory, tools, skills, etc.) configured.
    pub fn from_config(config: &Config) -> Result<Self> {
        let runtime: Arc<dyn host_runtime::RuntimeAdapter> =
            Arc::from(host_runtime::create_runtime(&config.runtime)?);
        let security = Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
        ));

        let memory: Arc<dyn Memory> = Arc::from(memory::create_memory_with_storage_and_routes(
            &config.memory,
            &config.embedding_routes,
            Some(&config.storage.provider.config),
            &config.workspace_dir,
            config.api_key.as_deref(),
        )?);

        let composio_key = if config.composio.enabled {
            config.composio.api_key.as_deref()
        } else {
            None
        };
        let composio_entity_id = if config.composio.enabled {
            Some(config.composio.entity_id.as_str())
        } else {
            None
        };

        let mut tools = tools::all_tools_with_runtime(
            Arc::new(config.clone()),
            &security,
            runtime,
            memory.clone(),
            composio_key,
            composio_entity_id,
            &config.browser,
            &config.http_request,
            &config.workspace_dir,
            &config.agents,
            config.api_key.as_deref(),
            config,
        );

        // Bridge skill tools (Notion, Gmail, etc.) from the QuickJS runtime
        // into the agent's tool registry so the LLM can call them.
        let skill_tools = tools::skill_bridge::collect_skill_tools();
        if !skill_tools.is_empty() {
            log::info!(
                "[agent] Injecting {} skill tool(s) into agent registry",
                skill_tools.len()
            );
            tools.extend(skill_tools);
        }

        let model_name = config
            .default_model
            .as_deref()
            .unwrap_or(crate::openhuman::config::DEFAULT_MODEL)
            .to_string();

        let provider_runtime_options = providers::ProviderRuntimeOptions {
            auth_profile_override: None,
            openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
            secrets_encrypt: config.secrets.encrypt,
            reasoning_enabled: config.runtime.reasoning_enabled,
        };

        let provider: Box<dyn Provider> = providers::create_routed_provider_with_options(
            config.api_key.as_deref(),
            config.api_url.as_deref(),
            &config.reliability,
            &config.model_routes,
            &model_name,
            &provider_runtime_options,
        )?;

        let dispatcher_choice = config.agent.tool_dispatcher.as_str();
        let tool_dispatcher: Box<dyn ToolDispatcher> = match dispatcher_choice {
            "native" => Box::new(NativeToolDispatcher),
            "xml" => Box::new(XmlToolDispatcher),
            _ if provider.supports_native_tools() => Box::new(NativeToolDispatcher),
            _ => Box::new(XmlToolDispatcher),
        };

        let available_hints: Vec<String> =
            config.model_routes.iter().map(|r| r.hint.clone()).collect();

        // Build prompt builder, optionally with learning sections
        let mut prompt_builder = SystemPromptBuilder::with_defaults();
        if config.learning.enabled {
            prompt_builder = prompt_builder
                .add_section(Box::new(
                    crate::openhuman::learning::LearnedContextSection::new(memory.clone()),
                ))
                .add_section(Box::new(
                    crate::openhuman::learning::UserProfileSection::new(memory.clone()),
                ));
            log::info!("[learning] prompt sections registered (learned_context, user_profile)");
        }

        // Build post-turn hooks when learning is enabled
        let mut post_turn_hooks: Vec<Arc<dyn super::hooks::PostTurnHook>> = Vec::new();
        if config.learning.enabled {
            let full_config = Arc::new(config.clone());

            if config.learning.reflection_enabled {
                // For cloud reflection, wrap the provider in an Arc.
                // For local, no provider needed.
                let reflection_provider: Option<Arc<dyn crate::openhuman::providers::Provider>> =
                    if config.learning.reflection_source
                        == crate::openhuman::config::ReflectionSource::Cloud
                    {
                        Some(Arc::from(providers::create_routed_provider(
                            config.api_key.as_deref(),
                            config.api_url.as_deref(),
                            &config.reliability,
                            &config.model_routes,
                            &model_name,
                        )?))
                    } else {
                        None
                    };
                post_turn_hooks.push(Arc::new(crate::openhuman::learning::ReflectionHook::new(
                    config.learning.clone(),
                    full_config.clone(),
                    memory.clone(),
                    reflection_provider,
                )));
                log::info!(
                    "[learning] reflection hook registered (source={:?})",
                    config.learning.reflection_source
                );
            }

            if config.learning.user_profile_enabled {
                post_turn_hooks.push(Arc::new(crate::openhuman::learning::UserProfileHook::new(
                    config.learning.clone(),
                    memory.clone(),
                )));
                log::info!("[learning] user_profile hook registered");
            }

            if config.learning.tool_tracking_enabled {
                post_turn_hooks.push(Arc::new(crate::openhuman::learning::ToolTrackerHook::new(
                    config.learning.clone(),
                    memory.clone(),
                )));
                log::info!("[learning] tool_tracker hook registered");
            }
        }

        Agent::builder()
            .provider(provider)
            .tools(tools)
            .memory(memory)
            .tool_dispatcher(tool_dispatcher)
            .memory_loader(Box::new(
                DefaultMemoryLoader::new(5, config.memory.min_relevance_score)
                    .with_max_chars(config.agent.max_memory_context_chars),
            ))
            .prompt_builder(prompt_builder)
            .config(config.agent.clone())
            .model_name(model_name)
            .temperature(config.default_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .classification_config(config.query_classification.clone())
            .available_hints(available_hints)
            .identity_config(config.identity.clone())
            .skills(crate::openhuman::skills::load_skills(&config.workspace_dir))
            .auto_save(config.memory.auto_save)
            .post_turn_hooks(post_turn_hooks)
            .learning_enabled(config.learning.enabled)
            .build()
    }

    /// Truncates the conversation history to the configured maximum message count.
    ///
    /// System messages are always preserved. Older non-system messages are
    /// dropped first.
    fn trim_history(&mut self) {
        let max = self.config.max_history_messages;
        if self.history.len() <= max {
            return;
        }

        let mut system_messages = Vec::new();
        let mut other_messages = Vec::new();

        for msg in self.history.drain(..) {
            match &msg {
                ConversationMessage::Chat(chat) if chat.role == "system" => {
                    system_messages.push(msg);
                }
                _ => other_messages.push(msg),
            }
        }

        if other_messages.len() > max {
            let drop_count = other_messages.len() - max;
            other_messages.drain(0..drop_count);
        }

        self.history = system_messages;
        self.history.extend(other_messages);
    }

    /// Pre-fetches learned context data from memory (observations, patterns, user profile).
    ///
    /// This is an async, non-blocking operation that populates the context
    /// for the system prompt.
    async fn fetch_learned_context(&self) -> crate::openhuman::agent::prompt::LearnedContextData {
        use crate::openhuman::agent::prompt::LearnedContextData;

        if !self.learning_enabled {
            return LearnedContextData::default();
        }

        let obs_entries = self
            .memory
            .list(
                Some(&MemoryCategory::Custom("learning_observations".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let pat_entries = self
            .memory
            .list(
                Some(&MemoryCategory::Custom("learning_patterns".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let profile_entries = self
            .memory
            .list(Some(&MemoryCategory::Custom("user_profile".into())), None)
            .await
            .unwrap_or_default();

        LearnedContextData {
            observations: obs_entries
                .iter()
                .rev()
                .take(5)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            patterns: pat_entries
                .iter()
                .take(3)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            user_profile: profile_entries
                .iter()
                .take(20)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
        }
    }

    /// Builds the system prompt for the current turn, including tool
    /// instructions and learned context.
    fn build_system_prompt(
        &self,
        learned: crate::openhuman::agent::prompt::LearnedContextData,
    ) -> Result<String> {
        let tools_slice: &[Box<dyn Tool>] = self.tools.as_slice();
        let instructions = self.tool_dispatcher.prompt_instructions(tools_slice);
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            tools: tools_slice,
            skills: &self.skills,
            identity_config: Some(&self.identity_config),
            dispatcher_instructions: &instructions,
            learned,
        };
        self.prompt_builder.build(&ctx)
    }

    /// Sanitize tool output to prevent PII/secrets in learning data.
    /// Returns a safe metadata string: tool type, status, and error class.
    fn sanitize_tool_output(raw_output: &str, tool_name: &str, success: bool) -> String {
        if success {
            // For successful calls, return a structured summary without raw data
            let char_count = raw_output.chars().count();
            format!(
                "tool={} status=success output_length={}",
                tool_name, char_count
            )
        } else {
            // For errors, classify the error type without exposing details
            let error_class = if raw_output.contains("permission") || raw_output.contains("denied")
            {
                "permission_error"
            } else if raw_output.contains("not found") || raw_output.contains("404") {
                "not_found"
            } else if raw_output.contains("timeout") || raw_output.contains("timed out") {
                "timeout"
            } else if raw_output.contains("network") || raw_output.contains("connection") {
                "network_error"
            } else if raw_output.contains("invalid") || raw_output.contains("parse") {
                "validation_error"
            } else {
                "unknown_error"
            };
            format!("tool={} status=error class={}", tool_name, error_class)
        }
    }

    /// Executes a single tool call and returns the result and execution record.
    async fn execute_tool_call(
        &self,
        call: &ParsedToolCall,
    ) -> (ToolExecutionResult, ToolCallRecord) {
        let started = std::time::Instant::now();
        publish_global(DomainEvent::ToolExecutionStarted {
            tool_name: call.name.clone(),
            session_id: self.event_session_id().to_string(),
        });
        log::info!("[agent_loop] tool start name={}", call.name);

        // Special-case `spawn_subagent { mode: "fork", … }`: stash a
        // ForkContext task-local so the sub-agent runner can replay the
        // parent's exact rendered prompt + tool schemas + message prefix
        // for backend prefix-cache reuse. The branch is taken before
        // executing the tool so the task-local is visible inside
        // `tool.execute(...)`.
        let fork_context_for_call = if call.name == "spawn_subagent"
            && call
                .arguments
                .get("mode")
                .and_then(|v| v.as_str())
                .map(|s| s.eq_ignore_ascii_case("fork"))
                .unwrap_or(false)
        {
            Some(self.build_fork_context(call))
        } else {
            None
        };

        let (raw_result, success) =
            if let Some(tool) = self.tools.iter().find(|t| t.name() == call.name) {
                let exec = tool.execute(call.arguments.clone());
                let outcome = if let Some(fork_ctx) = fork_context_for_call {
                    super::harness::with_fork_context(fork_ctx, exec).await
                } else {
                    exec.await
                };
                match outcome {
                    Ok(r) => {
                        if !r.is_error {
                            (r.output(), true)
                        } else {
                            (format!("Error: {}", r.output()), false)
                        }
                    }
                    Err(e) => (format!("Error executing {}: {e}", call.name), false),
                }
            } else {
                (format!("Unknown tool: {}", call.name), false)
            };

        // Context pipeline stage 1: apply the per-result byte budget
        // *inline* before the result enters history. This is the only
        // cache-safe reduction stage — the truncated body has never
        // been sent to the backend so it creates no cache invalidation.
        let budget_bytes = self.config.tool_result_budget_bytes;
        let (result, budget_outcome) =
            super::context_pipeline::apply_tool_result_budget(raw_result, budget_bytes);
        if budget_outcome.truncated {
            log::info!(
                "[agent_loop] tool_result_budget applied name={} original_bytes={} final_bytes={} dropped_bytes={}",
                call.name,
                budget_outcome.original_bytes,
                budget_outcome.final_bytes,
                budget_outcome.original_bytes - budget_outcome.final_bytes
            );
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        publish_global(DomainEvent::ToolExecutionCompleted {
            tool_name: call.name.clone(),
            session_id: self.event_session_id().to_string(),
            success,
            elapsed_ms,
        });
        log::info!(
            "[agent_loop] tool finish name={} elapsed_ms={} output_chars={} success={}",
            call.name,
            elapsed_ms,
            result.chars().count(),
            success
        );

        let output_summary = sanitize_tool_output(&result, &call.name, success);

        let record = ToolCallRecord {
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            success,
            output_summary,
            duration_ms: elapsed_ms,
        };

        let exec_result = ToolExecutionResult {
            name: call.name.clone(),
            output: result,
            success,
            tool_call_id: call.tool_call_id.clone(),
        };

        (exec_result, record)
    }

    /// Executes multiple tool calls in sequence.
    async fn execute_tools(
        &self,
        calls: &[ParsedToolCall],
    ) -> (Vec<ToolExecutionResult>, Vec<ToolCallRecord>) {
        let mut results = Vec::with_capacity(calls.len());
        let mut records = Vec::with_capacity(calls.len());
        for call in calls {
            let (exec_result, record) = self.execute_tool_call(call).await;
            results.push(exec_result);
            records.push(record);
        }
        (results, records)
    }

    /// Snapshot the parent's runtime so spawned sub-agents can read
    /// it via the [`super::harness::PARENT_CONTEXT`] task-local.
    fn build_parent_execution_context(&self) -> super::harness::ParentExecutionContext {
        super::harness::ParentExecutionContext {
            provider: Arc::clone(&self.provider),
            all_tools: Arc::clone(&self.tools),
            all_tool_specs: Arc::clone(&self.tool_specs),
            model_name: self.model_name.clone(),
            temperature: self.temperature,
            workspace_dir: self.workspace_dir.clone(),
            memory: Arc::clone(&self.memory),
            agent_config: self.config.clone(),
            identity_config: self.identity_config.clone(),
            skills: Arc::new(self.skills.clone()),
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
        }
    }

    /// Build a [`super::harness::ForkContext`] capturing the parent's
    /// rendered system prompt + tool schemas + message prefix at the
    /// moment a `spawn_subagent { mode: "fork", … }` call fires.
    ///
    /// The system prompt is pulled from `history[0]` (the agent always
    /// stores its rendered system prompt as the first message). The
    /// message prefix is the entire current history rendered through
    /// the dispatcher — the *same* sequence the parent's next call
    /// would send, except the new fork directive replaces the parent's
    /// next continuation.
    fn build_fork_context(&self, call: &ParsedToolCall) -> super::harness::ForkContext {
        let messages = self.tool_dispatcher.to_provider_messages(&self.history);
        let system_prompt: String = messages
            .first()
            .filter(|m| m.role == "system")
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let fork_task_prompt = call
            .arguments
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        super::harness::ForkContext {
            system_prompt: Arc::new(system_prompt),
            tool_specs: Arc::clone(&self.tool_specs),
            message_prefix: Arc::new(messages),
            cache_boundary: None,
            fork_task_prompt,
        }
    }

    /// Classifies the user message to determine if a specific model hint should be used.
    ///
    /// Currently unused by `turn()` — we pin the main agent to its configured
    /// model for KV-cache stability (see the rationale in `turn()` where
    /// `effective_model` is set). Kept around because the classifier config
    /// is still surfaced via `AgentBuilder::classification_config` and
    /// external callers (e.g. eval harnesses) may want to probe it directly.
    #[allow(dead_code)]
    fn classify_model(&self, user_message: &str) -> String {
        if let Some(hint) = super::classifier::classify(&self.classification_config, user_message) {
            if self.available_hints.contains(&hint) {
                tracing::info!(hint = hint.as_str(), "Auto-classified query");
                return format!("hint:{hint}");
            }
        }
        self.model_name.clone()
    }

    /// Performs a single interaction "turn" with the agent.
    ///
    /// This is the core logic that takes user input, manages the history,
    /// calls the LLM, handles tool calls (up to `max_tool_iterations`),
    /// and returns the final assistant response.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        let turn_started = std::time::Instant::now();
        log::info!(
            "[agent_loop] turn start message_chars={} history_len={} max_tool_iterations={}",
            user_message.chars().count(),
            self.history.len(),
            self.config.max_tool_iterations
        );
        // Pre-fetch learned context async before building the system prompt
        let learned = self.fetch_learned_context().await;

        if self.history.is_empty() {
            let system_prompt = self.build_system_prompt(learned)?;
            log::info!(
                "[agent_loop] system prompt built chars={} content=\n{}",
                system_prompt.chars().count(),
                system_prompt
            );
            self.history
                .push(ConversationMessage::Chat(ChatMessage::system(
                    system_prompt,
                )));
        } else {
            // Deliberately do NOT rebuild the system prompt on subsequent
            // turns. The rendered prompt is the KV-cache prefix the inference
            // backend has already tokenised; replacing its bytes (even
            // cosmetically) forces the backend to re-prefill from scratch.
            //
            // Dynamic turn-to-turn context (memory recall, learned snippets)
            // rides on the user message via `memory_loader.load_context()`
            // — that's where the caller should inject anything that varies
            // between turns.
            let _ = learned;
            log::trace!(
                "[agent_loop] system prompt reused (history_len={}) — KV cache prefix preserved",
                self.history.len()
            );
        }

        if self.auto_save {
            let _ = self
                .memory
                .store("user_msg", user_message, MemoryCategory::Conversation, None)
                .await;
        }

        let context = self
            .memory_loader
            .load_context(self.memory.as_ref(), user_message)
            .await
            .unwrap_or_default();

        let enriched = if context.is_empty() {
            user_message.to_string()
        } else {
            format!("{context}{user_message}")
        };

        self.history
            .push(ConversationMessage::Chat(ChatMessage::user(enriched)));

        // Pin the main agent to its configured model for the lifetime of
        // the session. Per-turn classification used to run here, but it
        // would flip `effective_model` mid-conversation (e.g. reasoning →
        // coding based on a single keyword). Every flip invalidates the
        // backend's KV cache namespace for this session, costing full
        // re-prefill on the very next turn. The main agent's job is to
        // decide *which sub-agent* to spawn — that routing lives in the
        // model prompt, not in the Rust-side classifier. Sub-agents pick
        // their own tier via `ModelSpec::Hint(...)` in their definition.
        let effective_model = self.model_name.clone();
        log::info!(
            "[agent_loop] model pinned model={} (per-turn classification disabled for KV cache stability)",
            effective_model
        );

        // Snapshot the parent's runtime once per turn so any
        // `spawn_subagent` invocation that fires inside this turn can
        // read it via the PARENT_CONTEXT task-local. We override the
        // model field with the post-classification effective model.
        let mut parent_context = self.build_parent_execution_context();
        parent_context.model_name = effective_model.clone();

        // Bump the session-memory turn counter. Used later by
        // `should_extract_session_memory` to decide whether to spawn a
        // background archivist fork at end-of-turn.
        self.context_pipeline.tick_turn();

        // Collect tool call records across all iterations for post-turn hooks
        let mut all_tool_records: Vec<ToolCallRecord> = Vec::new();

        let turn_body = async {
            for iteration in 0..self.config.max_tool_iterations {
                log::info!(
                    "[agent_loop] iteration start i={} history_len={}",
                    iteration + 1,
                    self.history.len()
                );

                // Context pipeline stages 3 & 4: run the reduction
                // chain before every provider hit. Microcompact fires
                // when the guard reports we're above the soft threshold
                // and there are older tool results to clear; otherwise
                // we log an autocompaction signal (openhuman's
                // compactor lives in `loop_/history.rs` and operates on
                // the `ChatMessage` shape, so for now the
                // `ConversationMessage`-shaped Agent path lets the
                // signal bubble up as telemetry until a native
                // summariser lands).
                let outcome = self
                    .context_pipeline
                    .run_before_call(&mut self.history);
                match &outcome {
                    super::context_pipeline::PipelineOutcome::NoOp => {}
                    super::context_pipeline::PipelineOutcome::Microcompacted(stats) => {
                        log::info!(
                            "[agent_loop] context_pipeline microcompact i={} envelopes={} entries={} bytes_freed={}",
                            iteration + 1,
                            stats.envelopes_cleared,
                            stats.entries_cleared,
                            stats.bytes_freed
                        );
                    }
                    super::context_pipeline::PipelineOutcome::AutocompactionRequested {
                        utilisation_pct,
                    } => {
                        log::warn!(
                            "[agent_loop] context_pipeline autocompaction requested i={} utilisation_pct={}",
                            iteration + 1,
                            utilisation_pct
                        );
                    }
                    super::context_pipeline::PipelineOutcome::ContextExhausted {
                        utilisation_pct,
                        reason,
                    } => {
                        log::error!(
                            "[agent_loop] context_pipeline context exhausted i={} utilisation_pct={} reason={}",
                            iteration + 1,
                            utilisation_pct,
                            reason
                        );
                        return Err(anyhow::anyhow!(
                            "Context window exhausted ({utilisation_pct}% full): {reason}"
                        ));
                    }
                }

                let messages = self.tool_dispatcher.to_provider_messages(&self.history);
                log::info!(
                    "[agent_loop] provider request i={} messages={} send_tool_specs={}",
                    iteration + 1,
                    messages.len(),
                    self.tool_dispatcher.should_send_tool_specs()
                );
                let provider_started = std::time::Instant::now();
                let response = match self
                    .provider
                    .chat(
                        ChatRequest {
                            messages: &messages,
                            tools: if self.tool_dispatcher.should_send_tool_specs() {
                                Some(self.tool_specs.as_slice())
                            } else {
                                None
                            },
                            system_prompt_cache_boundary: None,
                        },
                        &effective_model,
                        self.temperature,
                    )
                    .await
                {
                    Ok(resp) => {
                        log::info!(
                        "[agent_loop] provider response i={} elapsed_ms={} text_chars={} native_tool_calls={}",
                        iteration + 1,
                        provider_started.elapsed().as_millis(),
                        resp.text.as_ref().map_or(0, |t| t.chars().count()),
                        resp.tool_calls.len()
                    );
                        log::debug!("[agent_loop] provider response: {resp:?}");
                        resp
                    }
                    Err(err) => return Err(err),
                };

                let (text, calls) = self.tool_dispatcher.parse_response(&response);
                let calls = Self::with_fallback_tool_call_ids(calls, iteration);
                log::info!(
                    "[agent_loop] parsed response i={} parsed_text_chars={} parsed_tool_calls={}",
                    iteration + 1,
                    text.chars().count(),
                    calls.len()
                );
                if calls.is_empty() {
                    let final_text = if text.is_empty() {
                        response.text.unwrap_or_default()
                    } else {
                        text
                    };
                    log::info!(
                        "[agent_loop] final response i={} final_chars={}",
                        iteration + 1,
                        final_text.chars().count()
                    );

                    self.history
                        .push(ConversationMessage::Chat(ChatMessage::assistant(
                            final_text.clone(),
                        )));
                    self.trim_history();

                    if self.auto_save {
                        let summary = truncate_with_ellipsis(&final_text, 100);
                        let _ = self
                            .memory
                            .store("assistant_resp", &summary, MemoryCategory::Daily, None)
                            .await;
                    }

                    // Fire post-turn hooks (non-blocking)
                    if !self.post_turn_hooks.is_empty() {
                        let ctx = TurnContext {
                            user_message: user_message.to_string(),
                            assistant_response: final_text.clone(),
                            tool_calls: all_tool_records,
                            turn_duration_ms: turn_started.elapsed().as_millis() as u64,
                            session_id: None,
                            iteration_count: iteration + 1,
                        };
                        hooks::fire_hooks(&self.post_turn_hooks, ctx);
                    }

                    return Ok(final_text);
                }

                if !text.is_empty() {
                    log::info!(
                        "[agent_loop] assistant pre-tool text i={} chars={}",
                        iteration + 1,
                        text.chars().count()
                    );
                    self.history
                        .push(ConversationMessage::Chat(ChatMessage::assistant(
                            text.clone(),
                        )));
                    print!("{text}");
                    let _ = std::io::stdout().flush();
                }
                let tool_names: Vec<&str> = calls.iter().map(|call| call.name.as_str()).collect();
                log::info!(
                    "[agent_loop] executing tools i={} names={:?}",
                    iteration + 1,
                    tool_names
                );
                let persisted_tool_calls =
                    Self::persisted_tool_calls_for_history(&response, &calls, iteration);
                log::info!(
                "[agent_loop] persisting assistant tool calls i={} persisted_tool_calls={} parsed_tool_calls={}",
                iteration + 1,
                persisted_tool_calls.len(),
                calls.len()
            );
                self.history.push(ConversationMessage::AssistantToolCalls {
                    text: if text.is_empty() {
                        None
                    } else {
                        Some(text.clone())
                    },
                    tool_calls: persisted_tool_calls,
                });

                let (results, records) = self.execute_tools(&calls).await;
                all_tool_records.extend(records);
                log::info!(
                    "[agent_loop] tool results complete i={} result_count={}",
                    iteration + 1,
                    results.len()
                );
                let formatted = self.tool_dispatcher.format_results(&results);
                self.history.push(formatted);
                self.trim_history();
                log::info!(
                    "[agent_loop] iteration end i={} history_len={}",
                    iteration + 1,
                    self.history.len()
                );
            }

            log::warn!(
                "[agent_loop] exceeded maximum tool iterations max={}",
                self.config.max_tool_iterations
            );
            anyhow::bail!(
                "Agent exceeded maximum tool iterations ({})",
                self.config.max_tool_iterations
            )
        }; // end of `turn_body` async block

        // Run the turn body inside the parent-execution-context scope so
        // that any `spawn_subagent` tool call fired during the loop can
        // read the parent's provider, tools, model, and workspace via
        // the PARENT_CONTEXT task-local.
        super::harness::with_parent_context(parent_context, turn_body).await
    }

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
    pub async fn run_interactive(&mut self) -> Result<()> {
        println!("🦀 OpenHuman Interactive Mode");
        println!("Type /quit to exit.\n");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let cli = crate::openhuman::channels::CliChannel::new();

        let listen_handle = tokio::spawn(async move {
            let _ = crate::openhuman::channels::Channel::listen(&cli, tx).await;
        });

        while let Some(msg) = rx.recv().await {
            let response = match self.turn(&msg.content).await {
                Ok(resp) => resp,
                Err(e) => {
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            };
            println!("\n{response}\n");
        }

        listen_handle.abort();
        Ok(())
    }
}

/// Convenience entry point to run an agent with the given configuration and message.
pub async fn run(
    config: Config,
    message: Option<String>,
    model_override: Option<String>,
    temperature: f64,
) -> Result<()> {
    let mut effective_config = config;
    if let Some(m) = model_override {
        effective_config.default_model = Some(m);
    }
    effective_config.default_temperature = temperature;

    let mut agent = Agent::from_config(&effective_config)?;

    if let Some(msg) = message {
        let response = agent.run_single(&msg).await?;
        println!("{response}");
    } else {
        agent.run_interactive().await?;
    }

    Ok(())
}

/// Sanitize a learned memory entry before injecting into the system prompt.
/// Strips raw data, limits length, and removes potential secrets.
fn sanitize_learned_entry(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Truncate to a safe length
    let max_len = 200;
    let sanitized: String = trimmed.chars().take(max_len).collect();
    // Strip anything that looks like a secret/token
    if sanitized.contains("Bearer ")
        || sanitized.contains("sk-")
        || sanitized.contains("ghp_")
        || sanitized.contains("-----BEGIN")
    {
        return "[redacted: potential secret]".to_string();
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;

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
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
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
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
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
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
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

    /// End-to-end: parent Agent issues a `spawn_subagent` tool call,
    /// the runner dispatches a built-in sub-agent (`researcher`) using
    /// the same MockProvider, and the parent's next turn folds the
    /// sub-agent's text output into the final response.
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
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
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
            ConversationMessage::Chat(chat) if chat.role == "tool" => {
                chat.content.contains("X is Y")
            }
            _ => false,
        });
        assert!(
            tool_result_contains_subagent_output,
            "parent history should contain a tool-result entry with the sub-agent's output"
        );
    }

    /// KV-cache invariant: across multiple turns in the same session,
    /// the system-prompt bytes submitted to the provider must be
    /// byte-identical, and the model name must not flip. Both are
    /// required for the backend's automatic prefix cache to hit — if
    /// either changes, the backend must re-prefill the entire prompt
    /// every turn.
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
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
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
        }
    }
}
