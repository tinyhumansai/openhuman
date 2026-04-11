//! `AgentBuilder` fluent API and the `Agent::from_config` factory.
//!
//! Everything in this file is about *constructing* an `Agent` — the
//! builder setters, the `build()` validator, and the `from_config()`
//! factory that wires together the real provider / memory / tool
//! registry from a loaded [`Config`]. Per-turn behaviour lives in
//! [`super::turn`]; accessors and run-helpers live in [`super::runtime`].

use super::types::{Agent, AgentBuilder};
use crate::openhuman::agent::dispatcher::{
    NativeToolDispatcher, ToolDispatcher, XmlToolDispatcher,
};
use crate::openhuman::agent::host_runtime;
use crate::openhuman::agent::memory_loader::{DefaultMemoryLoader, MemoryLoader};
use crate::openhuman::config::{Config, ContextConfig};
use crate::openhuman::context::prompt::SystemPromptBuilder;
use crate::openhuman::context::{ContextManager, ProviderSummarizer};
use crate::openhuman::memory::{self, Memory};
use crate::openhuman::providers::{self, Provider};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::{self, Tool, ToolSpec};
use anyhow::Result;
use std::sync::Arc;

impl AgentBuilder {
    /// Creates a new `AgentBuilder` with default values.
    pub fn new() -> Self {
        Self {
            provider: None,
            tools: None,
            visible_tool_names: None,
            memory: None,
            prompt_builder: None,
            tool_dispatcher: None,
            memory_loader: None,
            config: None,
            context_config: None,
            model_name: None,
            temperature: None,
            workspace_dir: None,
            skills: None,
            auto_save: None,
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

    /// Restricts which tools the main agent can see and call directly.
    /// Tools not in this set are still available to sub-agents via the
    /// runner. Pass `None` (default) to make all tools visible.
    pub fn visible_tool_names(mut self, names: std::collections::HashSet<String>) -> Self {
        self.visible_tool_names = Some(names);
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

    /// Sets the global context-management configuration. Threaded
    /// into the [`ContextManager`] constructed in [`Self::build`]. If
    /// not set the manager is constructed with
    /// [`ContextConfig::default`].
    pub fn context_config(mut self, context_config: ContextConfig) -> Self {
        self.context_config = Some(context_config);
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

    /// Sets the post-turn hooks to be executed after each turn.
    pub fn post_turn_hooks(
        mut self,
        hooks: Vec<Arc<dyn crate::openhuman::agent::hooks::PostTurnHook>>,
    ) -> Self {
        self.post_turn_hooks = hooks;
        self
    }

    /// Enables or disables learning features.
    pub fn learning_enabled(mut self, enabled: bool) -> Self {
        self.learning_enabled = enabled;
        self
    }

    /// Sets the event-bus `session_id` and `channel` used to tag
    /// `DomainEvent`s emitted by this agent.
    ///
    /// - `session_id` groups all events for a single user / conversation so
    ///   downstream subscribers can correlate turns, tool calls, and errors.
    /// - `channel` labels the source or stream the events originated from
    ///   (e.g. `"cli"`, `"telegram"`, `"rpc"`) — useful when multiple front
    ///   ends share the same subscriber pipeline.
    ///
    /// Both parameters are converted into owned `String`s and stored in
    /// `event_session_id` / `event_channel` respectively.
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

        let visible_names = self.visible_tool_names.unwrap_or_default();

        // Build the filtered spec list that the main agent sends to the
        // provider. When the filter is empty every tool is visible
        // (backward compat). When populated, only allowlisted tools
        // appear in the function-calling schema so the LLM literally
        // cannot call skill tools directly — it must use spawn_subagent.
        let visible_tool_specs: Vec<ToolSpec> = if visible_names.is_empty() {
            tool_specs.clone()
        } else {
            tool_specs
                .iter()
                .filter(|spec| visible_names.contains(&spec.name))
                .cloned()
                .collect()
        };

        log::info!(
            "[agent] tool spec filter: total={} visible={} (filter_active={})",
            tool_specs.len(),
            visible_tool_specs.len(),
            !visible_names.is_empty()
        );

        // Pull the provider out of the builder once. We store it on
        // the Agent (for normal turn chat calls) and also clone the
        // Arc into the ProviderSummarizer so the context manager can
        // dispatch autocompaction through the same provider.
        let provider = self
            .provider
            .ok_or_else(|| anyhow::anyhow!("provider is required"))?;

        let prompt_builder = self
            .prompt_builder
            .unwrap_or_else(SystemPromptBuilder::with_defaults);

        let model_name = self
            .model_name
            .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.into());

        // Assemble the per-session ContextManager. The manager owns
        // the prompt builder, the reduction pipeline, and the
        // summarizer — every concern that touches "what's in the
        // model's context window" routes through this single handle.
        let context_config = self.context_config.unwrap_or_default();
        let summarizer = Arc::new(ProviderSummarizer::new(provider.clone()));
        let context = ContextManager::new(
            &context_config,
            summarizer,
            model_name.clone(),
            prompt_builder,
        );

        Ok(Agent {
            provider,
            tools: Arc::new(tools),
            tool_specs: Arc::new(tool_specs),
            visible_tool_specs: Arc::new(visible_tool_specs),
            visible_tool_names: visible_names,
            memory: self
                .memory
                .ok_or_else(|| anyhow::anyhow!("memory is required"))?,
            tool_dispatcher: self
                .tool_dispatcher
                .ok_or_else(|| anyhow::anyhow!("tool_dispatcher is required"))?,
            memory_loader: self
                .memory_loader
                .unwrap_or_else(|| Box::new(DefaultMemoryLoader::default())),
            config: self.config.unwrap_or_default(),
            model_name,
            temperature: self.temperature.unwrap_or(0.7),
            workspace_dir: self
                .workspace_dir
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
            skills: self.skills.unwrap_or_default(),
            auto_save: self.auto_save.unwrap_or(false),
            last_memory_context: None,
            system_prompt_cache_boundary: None,
            history: Vec::new(),
            post_turn_hooks: self.post_turn_hooks,
            learning_enabled: self.learning_enabled,
            event_session_id: self
                .event_session_id
                .unwrap_or_else(|| "standalone".to_string()),
            event_channel: self.event_channel.unwrap_or_else(|| "internal".to_string()),
            context,
        })
    }
}

impl Agent {
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
        let skill_tools = tools::collect_skill_tools();
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
        let mut post_turn_hooks: Vec<Arc<dyn crate::openhuman::agent::hooks::PostTurnHook>> =
            Vec::new();
        if config.learning.enabled {
            if config.learning.reflection_enabled {
                // Only the reflection hook needs an owned snapshot of the
                // full config, so create the `Arc` lazily inside this
                // branch instead of paying for the clone whenever
                // `learning.enabled` is true.
                let full_config = Arc::new(config.clone());
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

        // Generate the orchestrator's tool set: one tool per skill +
        // one tool per archetype (research, run_code, etc.) + spawn_subagent
        // as a fallback. These are the only tools the LLM sees in its
        // function-calling schema. Sub-agents still access the full `tools`
        // registry via ParentExecutionContext.
        let orchestrator_tools = tools::orchestrator_tools::collect_orchestrator_tools();
        let visible: std::collections::HashSet<String> = orchestrator_tools
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        // De-duplicate: spawn_subagent is already in the base registry.
        let existing_names: std::collections::HashSet<String> =
            tools.iter().map(|t| t.name().to_string()).collect();
        tools.extend(
            orchestrator_tools
                .into_iter()
                .filter(|t| !existing_names.contains(t.name())),
        );

        Agent::builder()
            .provider(provider)
            .tools(tools)
            .visible_tool_names(visible)
            .memory(memory)
            .tool_dispatcher(tool_dispatcher)
            .memory_loader(Box::new(
                DefaultMemoryLoader::new(5, config.memory.min_relevance_score)
                    .with_max_chars(config.agent.max_memory_context_chars),
            ))
            .prompt_builder(prompt_builder)
            .config(config.agent.clone())
            .context_config(config.context.clone())
            .model_name(model_name)
            .temperature(config.default_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .skills(crate::openhuman::skills::load_skills(&config.workspace_dir))
            .auto_save(config.memory.auto_save)
            .post_turn_hooks(post_turn_hooks)
            .learning_enabled(config.learning.enabled)
            .build()
    }
}
