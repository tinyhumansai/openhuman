//! `AgentBuilder` fluent setters and the `build()` validator.
//!
//! All setter methods return `Self` for chaining. `build()` validates that
//! required fields are present and assembles the final [`Agent`].

use super::{dedup_visible_tool_specs, visible_tool_specs_for_policy};
use crate::openhuman::agent::harness::session::types::{Agent, AgentBuilder};
use crate::openhuman::agent::harness::TriggerMemoryAgent;
use crate::openhuman::agent::memory_loader::DefaultMemoryLoader;
use crate::openhuman::agent_tool_policy::ToolPolicyEngine;
use crate::openhuman::config::ContextConfig;
use crate::openhuman::context::{ContextManager, ProviderSummarizer, SegmentRecapSummarizer};
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::{Tool, ToolSpec};
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
            model_vision: None,
            temperature: None,
            workspace_dir: None,
            action_dir: None,
            workflows: None,
            auto_save: None,
            post_turn_hooks: Vec::new(),
            learning_enabled: false,
            explicit_preferences_enabled: true,
            event_session_id: None,
            event_channel: None,
            agent_definition_name: None,
            session_parent_prefix: None,
            omit_profile: None,
            omit_memory_md: None,
            payload_summarizer: None,
            trigger_memory_agent: None,
            tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression::Full,
            tool_policy: None,
            archivist_hook: None,
            unified_compaction_enabled: true,
        }
    }

    /// Sets the AI provider for the agent.
    ///
    /// Accepts a `Box<dyn Provider>` for backward compatibility but stores
    /// the provider as an `Arc` internally so sub-agents spawned from this
    /// agent (via `spawn_subagent`) can share the same instance.
    pub fn provider(
        mut self,
        provider: Box<dyn crate::openhuman::inference::provider::Provider>,
    ) -> Self {
        self.provider = Some(Arc::from(provider));
        self
    }

    /// Sets the AI provider from an existing `Arc`. Use this when sharing
    /// a provider instance across multiple agents.
    pub fn provider_arc(
        mut self,
        provider: Arc<dyn crate::openhuman::inference::provider::Provider>,
    ) -> Self {
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
    pub fn prompt_builder(
        mut self,
        prompt_builder: crate::openhuman::context::prompt::SystemPromptBuilder,
    ) -> Self {
        self.prompt_builder = Some(prompt_builder);
        self
    }

    /// Sets the tool dispatcher for the agent.
    pub fn tool_dispatcher(
        mut self,
        tool_dispatcher: Box<dyn crate::openhuman::agent::dispatcher::ToolDispatcher>,
    ) -> Self {
        self.tool_dispatcher = Some(tool_dispatcher);
        self
    }

    /// Sets the memory loader for the agent.
    pub fn memory_loader(
        mut self,
        memory_loader: Box<dyn crate::openhuman::agent::memory_loader::MemoryLoader>,
    ) -> Self {
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

    /// Sets the user-configured vision capability for the resolved model.
    /// Surfaced to the turn engine's image gate via the `current_model_vision`
    /// task-local. Defaults to `false` when unset.
    pub fn model_vision(mut self, model_vision: bool) -> Self {
        self.model_vision = Some(model_vision);
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

    pub fn action_dir(mut self, action_dir: std::path::PathBuf) -> Self {
        self.action_dir = Some(action_dir);
        self
    }

    /// Sets the skills available to the agent.
    pub fn workflows(mut self, skills: Vec<crate::openhuman::workflows::Workflow>) -> Self {
        self.workflows = Some(skills);
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

    /// Enables or disables explicit-preference injection.
    ///
    /// When `true` (the default), preferences stored via `remember_preference`
    /// are fetched from the `user_profile` namespace and injected into the
    /// system prompt on every turn, independent of `learning_enabled`.
    pub fn explicit_preferences_enabled(mut self, enabled: bool) -> Self {
        self.explicit_preferences_enabled = enabled;
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

    /// Forward the target agent definition's pre-turn memory policy.
    pub fn trigger_memory_agent(mut self, policy: TriggerMemoryAgent) -> Self {
        self.trigger_memory_agent = Some(policy);
        self
    }

    /// Installs pre-execution policy middleware for tool calls.
    ///
    /// The default policy allows all calls. Custom policies can deny a call
    /// before `Tool::execute_with_options` runs.
    pub fn tool_policy(
        mut self,
        policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
    ) -> Self {
        self.tool_policy = Some(policy);
        self
    }

    /// Attach the production [`ArchivistHook`] instance so the session
    /// turn loop can call [`ArchivistHook::flush_open_segment`] at
    /// session-wind-down time, guaranteeing the trailing open segment is
    /// always finalized with an LLM recap + embedding.
    ///
    /// Set from `build_session_agent_inner` when
    /// `config.learning.episodic_capture_enabled` is `true` and a
    /// SQLite connection is available. Callers that construct an `Agent`
    /// directly (tests, CLI) can leave this `None` — flush is a no-op
    /// when the hook is absent.
    pub fn archivist_hook(
        mut self,
        hook: Option<Arc<crate::openhuman::agent::harness::archivist::ArchivistHook>>,
    ) -> Self {
        self.archivist_hook = hook;
        self
    }

    /// Phase 1.5 — gate the unified compaction path.
    ///
    /// When `true` (the default) and an archivist hook is wired in via
    /// [`Self::archivist_hook`], the session's `ContextManager` summarizer is
    /// wrapped with a [`SegmentRecapSummarizer`] that routes autocompaction
    /// through the archivist's rolling recap (one LLM summarizer, soft-fallback
    /// to [`ProviderSummarizer`] when the recap is unavailable).
    ///
    /// When `false` the `ProviderSummarizer` is used directly and Phase 1.5 is
    /// completely absent from the hot path — behaviour is identical to today's.
    pub fn unified_compaction_enabled(mut self, enabled: bool) -> Self {
        self.unified_compaction_enabled = enabled;
        self
    }

    /// Set the per-agent TokenJuice tool-output compression profile.
    pub fn tokenjuice_compression(
        mut self,
        profile: crate::openhuman::tokenjuice::AgentTokenjuiceCompression,
    ) -> Self {
        self.tokenjuice_compression = profile;
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
        let config = self.config.clone().unwrap_or_default();
        let event_session_id = self
            .event_session_id
            .clone()
            .unwrap_or_else(|| "standalone".to_string());
        let event_channel = self
            .event_channel
            .clone()
            .unwrap_or_else(|| "internal".to_string());
        let agent_definition_name = self
            .agent_definition_name
            .clone()
            .unwrap_or_else(|| "main".to_string());
        let tool_policy_session = ToolPolicyEngine::build_session(
            &agent_definition_name,
            &event_channel,
            "session",
            &config.channel_permissions,
            &tools,
            &visible_names,
        );

        // Build the filtered spec list that the main agent sends to the
        // provider. The explicit visible-tool allowlist and the resolved
        // channel permission policy must stay aligned so prompt-visible
        // tools cannot exceed the runtime execution boundary.
        let visible_tool_specs_unfiltered =
            visible_tool_specs_for_policy(&tool_specs, &visible_names, &tool_policy_session);

        // Dedupe by tool name. Anthropic (and other strict providers)
        // rejects a chat/completions request that lists two tools with
        // the same name — OpenHuman's own backend and OpenAI silently
        // accept duplicates, which hid this bug until #1710's per-role
        // routing started sending the same tool list to Anthropic.
        let visible_tool_specs: Vec<ToolSpec> =
            dedup_visible_tool_specs(visible_tool_specs_unfiltered);

        let visible_names_list: Vec<&str> =
            visible_tool_specs.iter().map(|s| s.name.as_str()).collect();
        log::info!(
            "[agent] tool spec filter: total={} visible={} (filter_active={} policy_restricted={}) names=[{}]",
            tool_specs.len(),
            visible_tool_specs.len(),
            !visible_names.is_empty(),
            tool_policy_session.has_restrictions(),
            visible_names_list.join(", ")
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
            .unwrap_or_else(crate::openhuman::context::prompt::SystemPromptBuilder::with_defaults);

        let model_name = self
            .model_name
            .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.into());

        // Assemble the per-session ContextManager. The manager owns
        // the prompt builder, the reduction pipeline, and the
        // summarizer — every concern that touches "what's in the
        // model's context window" routes through this single handle.
        let context_config = self.context_config.unwrap_or_default();

        // Phase 1.5 — unified compaction.
        //
        // When `unified_compaction_enabled` is true AND an archivist hook
        // is wired in, wrap the inner `ProviderSummarizer` with a
        // `SegmentRecapSummarizer`. The outer type:
        //   1. Tries the rolling segment recap from the open segment.
        //   2. Falls back to the inner `ProviderSummarizer` if unavailable.
        //
        // With the flag off OR no archivist, the plain `ProviderSummarizer`
        // is used and Phase 1.5 is completely absent from the hot path
        // — behaviour is identical to Phase 1.
        let inner_summarizer: Arc<dyn crate::openhuman::context::Summarizer> =
            Arc::new(ProviderSummarizer::new(provider.clone()));
        let session_id_for_recap = self
            .event_session_id
            .clone()
            .unwrap_or_else(|| "standalone".to_string());
        let summarizer: Arc<dyn crate::openhuman::context::Summarizer> =
            if self.unified_compaction_enabled {
                if let Some(ref archivist) = self.archivist_hook {
                    log::debug!(
                        "[agent::builder] unified_compaction_enabled=true — \
                         wrapping summarizer with SegmentRecapSummarizer \
                         session_id={session_id_for_recap}"
                    );
                    Arc::new(SegmentRecapSummarizer::new(
                        Arc::clone(archivist),
                        session_id_for_recap,
                        inner_summarizer,
                    ))
                } else {
                    log::debug!(
                        "[agent::builder] unified_compaction_enabled=true but \
                         no archivist hook — using ProviderSummarizer"
                    );
                    inner_summarizer
                }
            } else {
                log::debug!(
                    "[agent::builder] unified_compaction_enabled=false — \
                     using ProviderSummarizer (Phase 1.5 disabled)"
                );
                inner_summarizer
            };

        let context = ContextManager::new(
            &context_config,
            summarizer,
            model_name.clone(),
            prompt_builder,
        );

        let workspace_dir = self
            .workspace_dir
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let action_dir = self.action_dir.unwrap_or_else(|| workspace_dir.clone());

        Ok(Agent {
            provider,
            tools: Arc::new(tools),
            tool_specs: Arc::new(tool_specs),
            visible_tool_specs: Arc::new(visible_tool_specs),
            visible_tool_names: visible_names,
            tool_policy_session,
            memory: self
                .memory
                .ok_or_else(|| anyhow::anyhow!("memory is required"))?,
            tool_dispatcher: std::sync::Arc::from(
                self.tool_dispatcher
                    .ok_or_else(|| anyhow::anyhow!("tool_dispatcher is required"))?,
            ),
            memory_loader: self
                .memory_loader
                .unwrap_or_else(|| Box::new(DefaultMemoryLoader::default())),
            config,
            model_name,
            model_vision: self.model_vision.unwrap_or(false),
            temperature: self.temperature.unwrap_or(0.7),
            workspace_dir,
            action_dir,
            workflows: self.workflows.unwrap_or_default(),
            auto_save: self.auto_save.unwrap_or(false),
            last_memory_context: None,
            last_turn_citations: Vec::new(),
            last_turn_usage_totals: None,
            history: Vec::new(),
            post_turn_hooks: self.post_turn_hooks,
            learning_enabled: self.learning_enabled,
            explicit_preferences_enabled: self.explicit_preferences_enabled,
            event_session_id,
            event_channel,
            agent_definition_name: agent_definition_name.clone(),
            // Canonical registry id — captured here at build time
            // before any caller can call `set_agent_definition_name`
            // and clobber the transcript-facing name. Used by
            // `refresh_delegation_tools` to re-resolve the agent's
            // `subagents` declaration against the global registry.
            agent_definition_id: agent_definition_name.clone(),
            session_transcript_path: None,
            session_key: {
                let unix_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let sanitized: String = agent_definition_name
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
            run_queue: None,
            connected_integrations: Vec::new(),
            connected_integrations_initialized: false,
            integration_runtime_config: None,
            // Default to `true` (omit) so legacy / custom agents built
            // without a definition stay lean. Opt-in agents thread their
            // `omit_profile = false` through the builder.
            omit_profile: self.omit_profile.unwrap_or(true),
            omit_memory_md: self.omit_memory_md.unwrap_or(true),
            payload_summarizer: self.payload_summarizer,
            trigger_memory_agent: self.trigger_memory_agent.unwrap_or_default(),
            tokenjuice_compression: self.tokenjuice_compression,
            tool_policy: self.tool_policy.unwrap_or_else(|| {
                Arc::new(crate::openhuman::agent::tool_policy::AllowAllToolPolicy)
            }),
            last_seen_integrations_hash: 0,
            composio_integrations_rx: None,
            skill_events_rx: None,
            announced_integrations: std::collections::HashSet::new(),
            pending_integration_announcement: Vec::new(),
            announced_mcp_servers: std::collections::HashSet::new(),
            pending_mcp_announcement: Vec::new(),
            announced_skills: std::collections::HashSet::new(),
            pending_skill_announcement: Vec::new(),
            pending_skill_retraction: Vec::new(),
            archivist_hook: self.archivist_hook,
            synthesized_tool_names: std::collections::HashSet::new(),
            pending_synthesized_tools_mask: std::collections::HashSet::new(),
        })
    }
}
