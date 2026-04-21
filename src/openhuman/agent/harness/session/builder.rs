//! `AgentBuilder` fluent API and the `Agent::from_config` factory.
//!
//! Everything in this file is about *constructing* an `Agent` — the
//! builder setters, the `build()` validator, and the `from_config()`
//! factory that wires together the real provider / memory / tool
//! registry from a loaded [`Config`]. Per-turn behaviour lives in
//! [`super::turn`]; accessors and run-helpers live in [`super::runtime`].

use super::types::{Agent, AgentBuilder};
use crate::openhuman::agent::dispatcher::{
    NativeToolDispatcher, PFormatToolDispatcher, ToolDispatcher, XmlToolDispatcher,
};
use crate::openhuman::agent::harness::definition::{
    AgentDefinitionRegistry, PromptSource, ToolScope,
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
            agent_definition_name: None,
            session_parent_prefix: None,
            omit_profile: None,
            omit_memory_md: None,
            payload_summarizer: None,
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

    /// Sets the agent definition id this session is running
    /// (`welcome`, `orchestrator`, `integrations_agent`, …).
    ///
    /// This value is stamped onto the built [`Agent`] and surfaces in
    /// the following places:
    ///
    /// * **Transcript filename on disk** — `transcript::write_transcript`
    ///   and `transcript::find_latest_transcript` use it as the
    ///   `{agent}` prefix in `sessions/DDMMYYYY/{agent}_{index}.md`.
    ///   Both the write path and the resume-lookup path read the same
    ///   field on `self`, so a session is always self-consistent; the
    ///   user-visible signal is which filename the transcript lands
    ///   under. Leaving it at the legacy `"main"` fallback silently
    ///   misfiles every non-orchestrator session under `main_*.md`.
    /// * **Transcript metadata header** — `transcript::write_transcript`
    ///   stamps it into the `<!-- session_transcript\nagent: {name}\n… -->`
    ///   block at the top of every `.md` file. This is the ground-truth
    ///   signal for "which agent definition ran this session" when
    ///   inspecting transcripts after the fact.
    /// * **[`PromptContext::agent_id`]** at prompt-build time (see
    ///   `turn.rs`). Today only one prompt section reads this field —
    ///   the `Connected Integrations` branch in `context/prompt.rs`
    ///   that special-cases `integrations_agent` vs every other agent — so
    ///   the current user-visible impact of a wrong id is limited to
    ///   the two bullets above. The stamped `prompt_builder` injected
    ///   by [`Agent::from_config_for_agent`] is what actually drives
    ///   prompt flavour per archetype, independent of this field. That
    ///   said, any future prompt section that branches on a
    ///   non-`integrations_agent` id (e.g. welcome-specific banner, planner-
    ///   specific rubric) would silently never fire if the field were
    ///   left at `"main"`, so keeping it correctly stamped closes a
    ///   latent foot-gun for code that hasn't been written yet.
    ///
    /// Callers building via [`Agent::from_config_for_agent`] get this
    /// wired automatically inside `build_session_agent_inner`; direct
    /// builder users (tests, CLI) must set it explicitly if they care
    /// about any of the surfaces above.
    pub fn agent_definition_name(mut self, name: impl Into<String>) -> Self {
        self.agent_definition_name = Some(name.into());
        self
    }

    /// Set the parent session-key chain for a sub-agent. Passing
    /// `Some("1713000000_orchestrator")` produces a sub-agent whose
    /// transcript filename is prefixed with the parent's session key,
    /// yielding a flat hierarchy on disk
    /// (`session_raw/DDMMYYYY/{parent}__{child}.jsonl`). Nested
    /// delegations chain further prefixes with `__`. Leave `None`
    /// (default) for root sessions.
    pub fn session_parent_prefix(mut self, prefix: Option<String>) -> Self {
        self.session_parent_prefix = prefix;
        self
    }

    /// Forward the target agent definition's `omit_profile` flag so
    /// [`Agent::build_system_prompt`] can decide whether to inject
    /// `PROFILE.md`. Only opt-in agents (welcome, orchestrator, the
    /// trigger pair) should set this to `false`.
    pub fn omit_profile(mut self, omit: bool) -> Self {
        self.omit_profile = Some(omit);
        self
    }

    /// Forward the target agent definition's `omit_memory_md` flag so
    /// [`Agent::build_system_prompt`] can decide whether to inject
    /// `MEMORY.md`. Same opt-in set as `omit_profile`.
    pub fn omit_memory_md(mut self, omit: bool) -> Self {
        self.omit_memory_md = Some(omit);
        self
    }

    /// Wire an oversized-tool-result summarizer into the agent. When
    /// set, [`Agent::execute_tool_call`] calls
    /// [`crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer::maybe_summarize`]
    /// on every successful tool output and replaces the raw payload
    /// with the compressed summary on success. Currently set only for
    /// the orchestrator session by
    /// [`Agent::build_session_agent_inner`].
    pub fn payload_summarizer(
        mut self,
        summarizer: Arc<
            dyn crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer,
        >,
    ) -> Self {
        self.payload_summarizer = Some(summarizer);
        self
    }

    /// Validates the configuration and constructs a new `Agent` instance.
    ///
    /// This method is responsible for wiring together the provided components,
    /// setting up the context manager, and initializing the conversation history.
    /// It ensures that all required fields (provider, tools, memory, etc.) are present.
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
            history: Vec::new(),
            post_turn_hooks: self.post_turn_hooks,
            learning_enabled: self.learning_enabled,
            event_session_id: self
                .event_session_id
                .unwrap_or_else(|| "standalone".to_string()),
            event_channel: self.event_channel.unwrap_or_else(|| "internal".to_string()),
            agent_definition_name: self
                .agent_definition_name
                .clone()
                .unwrap_or_else(|| "main".to_string()),
            session_transcript_path: None,
            session_key: {
                let unix_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let agent_id = self.agent_definition_name.as_deref().unwrap_or("main");
                let sanitized: String = agent_id
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("{unix_ts}_{sanitized}")
            },
            session_parent_prefix: self.session_parent_prefix,
            cached_transcript_messages: None,
            context,
            on_progress: None,
            connected_integrations: Vec::new(),
            composio_client: None,
            // Default to `true` (omit) so legacy / custom agents built
            // without a definition stay lean. Opt-in agents thread their
            // `omit_profile = false` through the builder.
            omit_profile: self.omit_profile.unwrap_or(true),
            omit_memory_md: self.omit_memory_md.unwrap_or(true),
            payload_summarizer: self.payload_summarizer,
        })
    }
}

impl Agent {
    /// Constructs an `Agent` instance from a global system configuration.
    ///
    /// Thin wrapper around [`Agent::from_config_for_agent`] that always
    /// targets the orchestrator definition. This preserves the legacy
    /// "main agent = orchestrator" behaviour for CLI / REPL / any caller
    /// that does not participate in the #525 onboarding-routing flow.
    ///
    /// Callers that need to select a different agent at session-build
    /// time (for example the Tauri web chat path, which routes to the
    /// welcome agent pre-onboarding) should call
    /// [`Agent::from_config_for_agent`] directly.
    pub fn from_config(config: &Config) -> Result<Self> {
        Self::from_config_for_agent(config, "orchestrator")
    }

    /// Constructs an `Agent` instance scoped to a specific agent
    /// definition loaded from the global [`AgentDefinitionRegistry`].
    ///
    /// `agent_id` is looked up in the registry; the returned agent
    /// inherits that definition's `ToolScope`, `system_prompt`,
    /// `temperature`, `max_iterations`, and `omit_*` flags. Unknown
    /// agent ids produce a registry-lookup error rather than silently
    /// falling back to the orchestrator.
    ///
    /// Shared infrastructure between agent ids is identical:
    /// 1. Initializing the host runtime (native or docker).
    /// 2. Setting up security policies.
    /// 3. Initializing memory and embedding services.
    /// 4. Registering all built-in and orchestrator tools.
    /// 5. Configuring the routed AI provider.
    /// 6. Setting up the learning system and post-turn hooks.
    ///
    /// What differs per agent id:
    /// * `visible_tool_names` is the agent's `ToolScope::Named` list
    ///   (unioned with the names of synthesised delegation tools when
    ///   the agent declares `subagents = [...]`). `ToolScope::Wildcard`
    ///   yields an empty filter, matching the legacy unfiltered path.
    /// * `prompt_builder` uses [`SystemPromptBuilder::for_subagent`]
    ///   with the agent's inline/file prompt body and `omit_*` flags,
    ///   so each agent renders its own persona rather than the default
    ///   orchestrator workspace-files identity dump.
    /// * `temperature` comes from the agent's TOML (falls back to
    ///   `config.default_temperature` for the orchestrator to preserve
    ///   legacy behaviour).
    ///
    /// The welcome agent uses this entry point when routed from the
    /// Tauri web channel (see `channels::providers::web::build_session_agent`).
    pub fn from_config_for_agent(config: &Config, agent_id: &str) -> Result<Self> {
        // Look up the target definition up front so we can fail fast
        // with a clear error instead of building half an agent and then
        // discovering the id is unknown. The registry is a singleton
        // initialised at startup; if it's not yet populated we
        // conservatively fall back to the legacy "orchestrator-shaped"
        // build by proceeding without a definition override.
        let target_def: Option<crate::openhuman::agent::harness::definition::AgentDefinition> =
            match AgentDefinitionRegistry::global() {
                Some(reg) => match reg.get(agent_id) {
                    Some(def) => Some(def.clone()),
                    None if agent_id == "orchestrator" => {
                        // Orchestrator is allowed to be missing from the
                        // registry (legacy path, tests, pre-startup) —
                        // fall back to default behaviour.
                        log::debug!(
                            "[agent::builder] orchestrator definition not in registry — \
                         using legacy default prompt + filter"
                        );
                        None
                    }
                    None => {
                        return Err(anyhow::anyhow!(
                            "agent definition '{}' not found in registry",
                            agent_id
                        ));
                    }
                },
                None => {
                    if agent_id != "orchestrator" {
                        return Err(anyhow::anyhow!(
                            "AgentDefinitionRegistry is not initialised — cannot \
                         resolve agent '{}'. Call AgentDefinitionRegistry::init_global \
                         at startup.",
                            agent_id
                        ));
                    }
                    log::debug!(
                        "[agent::builder] registry not initialised, orchestrator requested — \
                     using legacy default prompt + filter"
                    );
                    None
                }
            };

        log::info!(
            "[agent::builder] building session agent id={} \
             (scope={}, omit_identity={}, omit_profile={}, omit_memory_md={}, temperature={:.2})",
            agent_id,
            target_def
                .as_ref()
                .map(|d| match &d.tools {
                    ToolScope::Named(names) => format!("named({})", names.len()),
                    ToolScope::Wildcard => "wildcard".to_string(),
                })
                .unwrap_or_else(|| "legacy".to_string()),
            target_def
                .as_ref()
                .map(|d| d.omit_identity)
                .unwrap_or(false),
            target_def.as_ref().map(|d| d.omit_profile).unwrap_or(true),
            target_def
                .as_ref()
                .map(|d| d.omit_memory_md)
                .unwrap_or(true),
            target_def
                .as_ref()
                .map(|d| d.temperature)
                .unwrap_or(config.default_temperature)
        );

        Self::build_session_agent_inner(config, agent_id, target_def.as_ref())
    }

    /// Internal constructor that consumes the optionally-resolved agent
    /// definition. Split out from [`Agent::from_config_for_agent`] so
    /// the lookup + logging live in one place and the heavy-lifting
    /// body stays readable.
    fn build_session_agent_inner(
        config: &Config,
        agent_id: &str,
        target_def: Option<&crate::openhuman::agent::harness::definition::AgentDefinition>,
    ) -> Result<Self> {
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
            None,
        )?);

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
            None,
            composio_entity_id,
            &config.browser,
            &config.http_request,
            &config.workspace_dir,
            &config.agents,
            config,
        );

        // `complete_onboarding` is the terminal step of the welcome
        // flow and must never be callable from any other session.
        // Stripping it here (before prompt + delegation assembly) keeps
        // it out of both the LLM's function-calling schema and the
        // rendered `## Tools` section.
        if agent_id != "welcome" {
            tools.retain(|t| {
                !crate::openhuman::agent::harness::subagent_runner::is_welcome_only_tool(t.name())
            });
        }

        // Filter tools by user preference stored in app state.
        {
            use crate::openhuman::app_state::load_stored_app_state;
            match load_stored_app_state(config) {
                Ok(stored) => {
                    if let Some(ref tasks) = stored.onboarding_tasks {
                        if !tasks.enabled_tools.is_empty() {
                            crate::openhuman::tools::filter_tools_by_user_preference(
                                &mut tools,
                                &tasks.enabled_tools,
                            );
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[session-builder] failed to load app state for tool filtering: {e}"
                    );
                }
            }
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

        let provider: Box<dyn Provider> = providers::create_intelligent_routing_provider(
            None,
            config.api_url.as_deref(),
            config,
            &provider_runtime_options,
        )?;

        // Dispatcher selection is deferred until after the tool list is
        // finalised (orchestrator tools are appended below). We capture
        // the choice string now so the provider borrow doesn't conflict
        // with the later `provider` move into the builder.
        let dispatcher_choice = config.agent.tool_dispatcher.clone();
        let supports_native = provider.supports_native_tools();

        // Build prompt builder — either the default "orchestrator /
        // main agent" layout that bootstraps from workspace identity
        // files, OR a narrow per-agent builder that injects the target
        // definition's `prompt.md` body and respects its `omit_*` flags.
        //
        // The narrow path is selected whenever we resolved a
        // non-orchestrator definition from the registry. Welcome agent
        // is the first real consumer: its TOML sets
        // `omit_identity = true`, `omit_memory_context = false`,
        // `omit_safety_preamble = true`, `omit_skills_catalog = true`,
        // so the rendered prompt becomes:
        //
        //   (welcome persona body)
        //   ── Memory context (user profile, learned observations)
        //   ── Tools (2 entries: complete_onboarding + memory_recall)
        //   ── Workspace directory
        //
        // The orchestrator continues to use `with_defaults` so its
        // prompt stays byte-identical to the legacy CLI/REPL behaviour
        // except for the tool-scope tightening we already landed in
        // earlier commits.
        // Every agent with a resolved definition (built-in or workspace
        // override) goes through the per-agent pipeline — the legacy
        // `with_defaults()` branch only fires when the registry is
        // unavailable (pre-startup, tests). `PromptSource::Dynamic`
        // agents install a [`DynamicPromptSection`] that re-runs the
        // builder against the live [`PromptContext`] at
        // `build_system_prompt` time, so `connected_integrations`
        // fetched asynchronously on session start land in the prompt.
        // `Inline`/`File` sources still resolve to just the archetype
        // body and get wrapped by [`SystemPromptBuilder::for_subagent`].
        let mut prompt_builder = match target_def {
            Some(def) => match &def.system_prompt {
                PromptSource::Dynamic(build) => SystemPromptBuilder::from_dynamic(*build),
                PromptSource::Inline(text) => SystemPromptBuilder::for_subagent(
                    text.clone(),
                    def.omit_identity,
                    def.omit_safety_preamble,
                    def.omit_skills_catalog,
                ),
                PromptSource::File { path } => {
                    let workspace_path = config
                        .workspace_dir
                        .join("agent")
                        .join("prompts")
                        .join(path);
                    let body_text = if workspace_path.is_file() {
                        std::fs::read_to_string(&workspace_path).unwrap_or_else(|e| {
                            log::warn!(
                                "[agent::builder] failed to read prompt {}: {e} — using empty body",
                                workspace_path.display()
                            );
                            String::new()
                        })
                    } else {
                        log::warn!(
                            "[agent::builder] prompt file {} not found — using empty body",
                            path
                        );
                        String::new()
                    };
                    SystemPromptBuilder::for_subagent(
                        body_text,
                        def.omit_identity,
                        def.omit_safety_preamble,
                        def.omit_skills_catalog,
                    )
                }
            },
            None => SystemPromptBuilder::with_defaults(),
        };
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
                            None,
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

        // Resolve the per-agent delegation tool set and visible-tool
        // whitelist from the target definition (when we have one) or
        // fall back to the orchestrator's synthesis path.
        //
        // For an agent with `subagents = [...]` in its TOML (today:
        // orchestrator), `collect_orchestrator_tools` synthesises one
        // `ArchetypeDelegationTool` per named sub-agent plus one
        // `SkillDelegationTool` per connected Composio toolkit.
        //
        // For an agent without `subagents` (today: welcome, critic,
        // archivist, etc.), no delegation tools are synthesised — the
        // LLM only sees the agent's own `ToolScope::Named` entries
        // from the global registry, narrowed by the visible-tool
        // filter.
        //
        // This builder is synchronous and sits on the CLI / REPL /
        // Tauri-web code path. It does not have access to the async
        // Composio fetcher, so we pass an empty slice of connected
        // integrations here — the skill-wildcard expansion therefore
        // produces zero delegation tools. That is correct behaviour:
        // callers that need live integration expansion go through the
        // bus-based `channels::runtime::dispatch` path instead.
        let (delegation_tools, filter_from_scope): (
            Vec<Box<dyn Tool>>,
            Option<std::collections::HashSet<String>>,
        ) = match (
            target_def,
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global(),
        ) {
            (Some(def), Some(reg)) => {
                let synthed = tools::orchestrator_tools::collect_orchestrator_tools(def, reg, &[]);
                let filter: Option<std::collections::HashSet<String>> = match &def.tools {
                    ToolScope::Named(names) => {
                        let mut set: std::collections::HashSet<String> =
                            names.iter().cloned().collect();
                        for t in &synthed {
                            set.insert(t.name().to_string());
                        }
                        Some(set)
                    }
                    ToolScope::Wildcard => None,
                };
                (synthed, filter)
            }
            (None, Some(reg)) => {
                // Legacy orchestrator fallback (no target definition).
                // Keeps the pre-refactor behaviour byte-identical for
                // callers that invoke the old `from_config` on a
                // pre-startup or test registry state.
                let synthed = match reg.get("orchestrator") {
                    Some(orch_def) => {
                        tools::orchestrator_tools::collect_orchestrator_tools(orch_def, reg, &[])
                    }
                    None => {
                        log::debug!(
                            "[agent::builder] orchestrator definition not in registry — \
                             skipping delegation tool synthesis"
                        );
                        Vec::new()
                    }
                };
                (synthed, None)
            }
            (_, None) => {
                log::debug!(
                    "[agent::builder] AgentDefinitionRegistry not initialised — \
                     skipping delegation tool synthesis"
                );
                (Vec::new(), None)
            }
        };

        // The final visible-tool whitelist is the union of whatever the
        // definition scope produced (for named scopes) and every tool
        // we just synthesised as a delegation wrapper. When the
        // definition is `ToolScope::Wildcard` (legacy default, no
        // filter), we still populate `visible` from the delegation
        // tools alone so the existing `Agent::visible_tool_names`
        // contract (empty == no filter) stays intact: an empty set
        // means "no filter" for both legacy callers and the new
        // agent-scoped path.
        let visible: std::collections::HashSet<String> = match filter_from_scope {
            Some(set) => set,
            None => delegation_tools
                .iter()
                .map(|t| t.name().to_string())
                .collect(),
        };
        // De-duplicate: some synthesised tool names may collide with
        // already-registered tools (unlikely for `delegate_*` names but
        // cheap to guard against).
        let existing_names: std::collections::HashSet<String> =
            tools.iter().map(|t| t.name().to_string()).collect();
        tools.extend(
            delegation_tools
                .into_iter()
                .filter(|t| !existing_names.contains(t.name())),
        );

        // Build the P-Format registry AFTER the tool list is finalised
        // (including orchestrator tools) so every tool gets a signature
        // entry. The registry is self-contained — it doesn't hold a
        // reference back into the tools Vec.
        let pformat_registry = crate::openhuman::agent::pformat::build_registry(&tools);
        let tool_dispatcher: Box<dyn ToolDispatcher> = match dispatcher_choice.as_str() {
            "native" => Box::new(NativeToolDispatcher),
            "xml" => Box::new(XmlToolDispatcher),
            "pformat" => Box::new(PFormatToolDispatcher::new(pformat_registry.clone())),
            _ if supports_native => Box::new(NativeToolDispatcher),
            // Default for text-only providers: P-Format. Flip the
            // `agent.tool_dispatcher` config to `"xml"` to revert.
            _ => Box::new(PFormatToolDispatcher::new(pformat_registry.clone())),
        };

        // Provider-side grammar decoders (e.g. Fireworks) compile every
        // tool JSON schema into a grammar and index its rules with a
        // uint16_t — max 65 535 rules. Large Composio toolkits (Notion,
        // Salesforce, Gmail) produce per-action schemas dense enough
        // that even 16–25 of them blow past that ceiling, regardless of
        // how aggressively the fuzzy filter in `tool_filter.rs` narrows
        // the list. When that happens the provider rejects the request
        // with a 400 before any generation starts, so integrations_agent can
        // never actually invoke the toolkit.
        //
        // Workaround: if we're building integrations_agent and the selected
        // dispatcher would ship `tools: [...]` in the API payload
        // (`should_send_tool_specs() == true`, i.e. native mode), swap
        // to XML mode. XmlToolDispatcher puts the tool catalogue inside
        // the system prompt as prose instead — the provider never
        // compiles a grammar for it, so the rule-count ceiling stops
        // mattering. Downside: slightly looser tool-call formatting
        // than native; the existing `parse_tool_calls` recovers from
        // stray formatting and the loop retries on malformed output.
        let tool_dispatcher: Box<dyn ToolDispatcher> =
            if agent_id == "integrations_agent" && tool_dispatcher.should_send_tool_specs() {
                log::info!(
                    "[agent::builder] integrations_agent: overriding native tool dispatcher with \
                     XmlToolDispatcher (native mode hits provider grammar-rule limits on \
                     large Composio toolkits)"
                );
                Box::new(XmlToolDispatcher)
            } else {
                tool_dispatcher
            };

        log::debug!(
            "[agent] tool dispatcher selected: choice={dispatcher_choice} agent_id={agent_id} \
             sends_tool_specs={} default_text_format=pformat pformat_registry_entries={}",
            tool_dispatcher.should_send_tool_specs(),
            pformat_registry.len()
        );

        // Temperature override: when we have a target definition, use
        // its declared temperature from the TOML (welcome is 0.7,
        // orchestrator is 0.4, etc). Fall back to
        // `config.default_temperature` for the legacy "no definition"
        // path so existing callers keep getting their configured value.
        let effective_temperature = target_def
            .map(|def| def.temperature)
            .unwrap_or(config.default_temperature);

        // Thread PROFILE.md + MEMORY.md inclusion from the resolved
        // definition. Legacy / no-definition path stays on the safe
        // `true` default (omit) for both files.
        let effective_omit_profile = target_def.map(|def| def.omit_profile).unwrap_or(true);
        let effective_omit_memory_md = target_def.map(|def| def.omit_memory_md).unwrap_or(true);

        // Stamp the resolved agent definition id onto the Agent via the
        // builder. Without this call, `agent_definition_name` falls
        // back to the legacy `"main"` default (see `AgentBuilder::build`)
        // for every non-orchestrator caller. In the current codebase
        // that is benign for the orchestrator (which is already aliased
        // as `"main"` everywhere downstream) but causes two concrete
        // bugs for the welcome agent, which is the only other id that
        // reaches this function in practice:
        //
        //   1. Its session transcripts are misfiled on disk under
        //      `sessions/DDMMYYYY/main_*.md` instead of `welcome_*.md`.
        //   2. The `agent:` line inside each transcript's metadata
        //      header stamps `agent: main` instead of `agent: welcome`.
        //
        // Skills_agent and every other typed sub-agent are unaffected
        // because they never build via `from_config_for_agent` — they
        // are spawned through `subagent_runner` which constructs its
        // prompt and history directly.
        //
        // See the docstring on `AgentBuilder::agent_definition_name`
        // for the full list of surfaces and the latent prompt-section
        // foot-gun this call also closes.
        log::debug!(
            "[agent::builder] stamping agent_definition_name={} onto session agent",
            agent_id
        );

        // ── Orchestrator-only: wire the payload summarizer ──────────
        //
        // Issue #574 — when a tool returns a huge payload (Composio
        // dump, long file read, web scrape), it should be compressed
        // by a dedicated `summarizer` sub-agent before entering the
        // orchestrator's history. We resolve the summarizer agent
        // definition from the global registry and construct a
        // `SubagentPayloadSummarizer` parameterized from the
        // [`ContextConfig`] thresholds. Every other agent id gets
        // `None` and their tool results stay untouched (the summarizer
        // itself MUST be `None` to avoid recursive self-summarization).
        let payload_summarizer: Option<
            std::sync::Arc<
                dyn crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer,
            >,
        > = if agent_id == "orchestrator" && config.context.summarizer_payload_threshold_tokens > 0
        {
            match crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global() {
                Some(reg) => match reg.get("summarizer") {
                    Some(summarizer_def) => {
                        log::info!(
                            "[agent::builder] wiring payload_summarizer for orchestrator: \
                             threshold_tokens={} max_tokens={}",
                            config.context.summarizer_payload_threshold_tokens,
                            config.context.summarizer_max_payload_tokens
                        );
                        Some(std::sync::Arc::new(
                            crate::openhuman::agent::harness::payload_summarizer::SubagentPayloadSummarizer::new(
                                summarizer_def.clone(),
                                config.context.summarizer_payload_threshold_tokens,
                                config.context.summarizer_max_payload_tokens,
                            ),
                        ))
                    }
                    None => {
                        log::warn!(
                            "[agent::builder] orchestrator requested payload_summarizer but \
                             `summarizer` definition is not in the registry — proceeding without it"
                        );
                        None
                    }
                },
                None => {
                    log::warn!(
                        "[agent::builder] orchestrator requested payload_summarizer but \
                         AgentDefinitionRegistry is not initialised — proceeding without it"
                    );
                    None
                }
            }
        } else {
            None
        };

        let mut builder = Agent::builder()
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
            .temperature(effective_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .skills(crate::openhuman::skills::load_skills(&config.workspace_dir))
            .auto_save(config.memory.auto_save)
            .post_turn_hooks(post_turn_hooks)
            .learning_enabled(config.learning.enabled)
            .agent_definition_name(agent_id.to_string())
            .omit_profile(effective_omit_profile)
            .omit_memory_md(effective_omit_memory_md);
        if let Some(ps) = payload_summarizer {
            builder = builder.payload_summarizer(ps);
        }
        builder.build()
    }
}
