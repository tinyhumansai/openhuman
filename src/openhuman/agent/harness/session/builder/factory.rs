//! `Agent::from_config` factory methods and the internal
//! `build_session_agent_inner` constructor.

use super::helpers::prefetch_tool_memory_rules_blocking;
use super::should_synthesize_delegation_tools;
use crate::openhuman::agent::dispatcher::{
    NativeToolDispatcher, PFormatToolDispatcher, XmlToolDispatcher,
};
use crate::openhuman::agent::harness::definition::{
    AgentDefinitionRegistry, PromptSource, ToolScope,
};
use crate::openhuman::agent::harness::session::types::Agent;
use crate::openhuman::agent::host_runtime;
use crate::openhuman::agent::memory_loader::DefaultMemoryLoader;
use crate::openhuman::config::Config;
use crate::openhuman::context::prompt::SystemPromptBuilder;
use crate::openhuman::inference::provider::{self, Provider};
use crate::openhuman::memory::Memory;
use crate::openhuman::memory_store;
use crate::openhuman::memory_tools::ToolMemoryCaptureHook;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::{self, Tool};
use anyhow::Result;
use std::sync::Arc;

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
    ///   the agent declares `[subagents] allowlist = [...]`). `ToolScope::Wildcard`
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
    /// Tauri web channel (see `channels::provider::web::build_session_agent`).
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

        Self::build_session_agent_inner(
            config,
            agent_id,
            target_def.as_ref(),
            None,
            None,
            false,
            None,
        )
    }

    /// Same as [`Self::from_config_for_agent`] but also appends a
    /// `ReflectionMemoryContextSection` to the assembled
    /// [`SystemPromptBuilder`], seeded with the `source_chunks` snapshot
    /// from the spawning subconscious reflection (#623).
    ///
    /// Used by `channels::provider::web::build_session_agent` when a
    /// chat thread's seed message metadata flags
    /// `origin == "subconscious_reflection"` — the orchestrator then
    /// has the same memory context the reflection-LLM had, so the user's
    /// follow-up questions stay grounded in the underlying chunks.
    pub fn from_config_for_agent_with_reflection_chunks(
        config: &Config,
        agent_id: &str,
        reflection_chunks: Vec<crate::openhuman::subconscious::SourceChunk>,
    ) -> Result<Self> {
        // Reuse the same registry-resolution path the canonical
        // `from_config_for_agent` walks, then route through the inner
        // constructor with the chunks attached.
        let target_def: Option<crate::openhuman::agent::harness::definition::AgentDefinition> =
            match AgentDefinitionRegistry::global() {
                Some(reg) => reg.get(agent_id).cloned(),
                None => None,
            };
        Self::build_session_agent_inner(
            config,
            agent_id,
            target_def.as_ref(),
            Some(reflection_chunks),
            None,
            false,
            None,
        )
    }

    /// Construct a session agent with optional reflection memory chunks and an
    /// additional profile prompt section. Used by the web channel when the user
    /// selects a persistent agent profile for the thread.
    pub fn from_config_for_agent_with_profile(
        config: &Config,
        agent_id: &str,
        reflection_chunks: Option<Vec<crate::openhuman::subconscious::SourceChunk>>,
        profile_prompt_suffix: Option<String>,
        profile: Option<&crate::openhuman::profiles::AgentProfile>,
    ) -> Result<Self> {
        let target_def: Option<crate::openhuman::agent::harness::definition::AgentDefinition> =
            match AgentDefinitionRegistry::global() {
                Some(reg) => match reg.get(agent_id) {
                    Some(def) => Some(def.clone()),
                    None if agent_id == "orchestrator" => None,
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
                    None
                }
            };
        Self::build_session_agent_inner(
            config,
            agent_id,
            target_def.as_ref(),
            reflection_chunks,
            profile_prompt_suffix,
            false,
            profile,
        )
    }

    /// Constructs a council juror that runs the normal agent tool loop with
    /// only read-only tools visible/executable.
    ///
    /// Model council calls need research/memory/search before a juror writes a
    /// turn, but they must not mutate files, memory, schedules, wallets, or the
    /// host. This constructor reuses the standard harness and provider wiring
    /// while filtering the registry before tool specs and policy are built.
    pub fn from_config_for_read_only_council_juror(
        config: &Config,
        juror_name: &str,
        model_override: Option<String>,
        temperature: Option<f64>,
        prompt_suffix: String,
    ) -> Result<Self> {
        let mut agent = Self::build_session_agent_inner(
            config,
            "orchestrator",
            None,
            None,
            Some(prompt_suffix),
            true,
            None,
        )?;
        let safe_name: String = juror_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        agent.set_event_context(
            format!("model-council-{safe_name}"),
            "model_council_readonly",
        );
        agent.set_agent_definition_name(format!("model_council_{safe_name}"));
        if let Some(model) = model_override
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
        {
            agent.model_name = model;
        }
        if let Some(temp) = temperature {
            agent.temperature = temp;
        }
        agent.auto_save = false;
        Ok(agent)
    }

    /// Internal constructor that consumes the optionally-resolved agent
    /// definition. Split out from [`Agent::from_config_for_agent`] so
    /// the lookup + logging live in one place and the heavy-lifting
    /// body stays readable.
    ///
    /// `reflection_chunks`, when present, are appended to the assembled
    /// `SystemPromptBuilder` as a [`ReflectionMemoryContextSection`] so
    /// the orchestrator's system prompt carries the same memory context
    /// the subconscious LLM cited when it produced the spawning
    /// reflection (#623). Empty / `None` is the default for normal chat
    /// threads — the section is omitted entirely.
    #[allow(clippy::too_many_arguments)]
    fn build_session_agent_inner(
        config: &Config,
        agent_id: &str,
        target_def: Option<&crate::openhuman::agent::harness::definition::AgentDefinition>,
        reflection_chunks: Option<Vec<crate::openhuman::subconscious::SourceChunk>>,
        profile_prompt_suffix: Option<String>,
        read_only_tools_only: bool,
        profile: Option<&crate::openhuman::profiles::AgentProfile>,
    ) -> Result<Self> {
        if let Some(p) = profile {
            tracing::debug!(
                profile_id = %p.id,
                include_agent_conversations = p.include_agent_conversations,
                allowed_tools = p.allowed_tools.as_ref().map_or(0, |t| t.len()),
                allowed_skills = p.allowed_skills.as_ref().map_or(0, |s| s.len()),
                allowed_mcp_servers = p.allowed_mcp_servers.as_ref().map_or(0, |m| m.len()),
                memory_sources = p.memory_sources.as_ref().map_or(0, |s| s.len()),
                "[profiles] applying per-profile session gate"
            );
        }
        let runtime: Arc<dyn host_runtime::RuntimeAdapter> = Arc::from(
            host_runtime::create_runtime(&config.runtime, config.shell.hide_window)?,
        );
        let security = Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        ));
        // Phase 1 of #1401: see comment in channels/runtime/startup.rs.
        let audit = crate::openhuman::security::get_or_create_workspace_audit_logger(
            crate::openhuman::config::AuditConfig::default(),
            config.workspace_dir.clone(),
        )?;

        let local_embedding = config.workload_local_model("embeddings");
        let embedding_api_key = crate::openhuman::embeddings::resolve_api_key(
            config,
            &config.memory.embedding_provider,
        );
        let memory: Arc<dyn Memory> = Arc::from(memory_store::create_memory_with_local_ai(
            &config.memory,
            local_embedding.as_deref(),
            &embedding_api_key,
            &config.embedding_routes,
            Some(&config.storage.provider.config),
            &config.workspace_dir,
        )?);

        // Per-profile skill (workflow) + MCP-server allowlists. `None` = all.
        let profile_skill_allowlist: Option<std::collections::HashSet<String>> = profile
            .and_then(|p| p.allowed_skills.clone())
            .map(|v| v.into_iter().collect());
        let profile_mcp_allowlist: Option<Vec<String>> =
            profile.and_then(|p| p.allowed_mcp_servers.clone());

        let mut tools = tools::all_tools_with_runtime(
            Arc::new(config.clone()),
            &security,
            runtime,
            audit,
            memory.clone(),
            &config.browser,
            &config.http_request,
            &config.action_dir,
            &config.agents,
            config,
            profile_skill_allowlist.as_ref(),
            profile_mcp_allowlist.as_deref(),
        );

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

        if read_only_tools_only {
            let before = tools.len();
            tools.retain(|tool| {
                tool.permission_level() <= tools::PermissionLevel::ReadOnly
                    && !matches!(tool.scope(), tools::ToolScope::CliRpcOnly)
            });
            log::info!(
                "[agent::builder] read-only tool filter applied: before={} after={}",
                before,
                tools.len()
            );
        }

        // Route the main agent's chat through the unified per-workload
        // factory so the user's "Reasoning" routing in the AI settings
        // panel (e.g. `reasoning_provider = "anthropic:claude-..."`)
        // actually takes effect. The factory returns a (Provider, model)
        // tuple — the resolved model wins over the legacy `default_model`
        // fallback so explicit picks like `anthropic:claude-sonnet-4-5`
        // actually use claude-sonnet-4-5 end to end (sending the abstract
        // "reasoning-v1" tier name to Anthropic would 404).
        //
        // When `reasoning_provider` is unset or `"cloud"`, the factory
        // resolves to the primary cloud (OpenHuman by default), so the
        // baseline behaviour is identical to the legacy
        // `create_intelligent_routing_provider` path.
        //
        // What we deliberately lose for now: the ReliableProvider retry
        // wrapper, model_routes translation, and intelligent local/cloud
        // task hinting that the legacy router added on top of the raw
        // backend. Those are valuable but orthogonal — they can be layered
        // back on top of the factory's output in a follow-up without
        // re-introducing the routing bypass.
        let _ = provider::ProviderRuntimeOptions {
            auth_profile_override: None,
            openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
            secrets_encrypt: config.secrets.encrypt,
            reasoning_enabled: config.runtime.reasoning_enabled,
        };
        // Explicit `hint:<role>` and known-tier model strings route to the
        // matching workload (so a subagent declaring `hint:reasoning` still
        // gets the user's `reasoning_provider`). Everything else — including
        // the orchestrator/lead, which has no specialised hint — falls
        // through to the `chat` workload, so `config.chat_provider` (the
        // "Chat" routing row, "Direct conversational back-and-forth") drives
        // the user-facing chat turn.
        // Only the explicit `hint:<role>` form routes to a specialised
        // workload — legacy tier literals like `reasoning-v1` (which the
        // bootstrap historically pinned as `default_model` for everyone)
        // fall through to `chat`. This is what makes
        // `config.chat_provider` actually drive the orchestrator's chat
        // turn for the install base; without it, every existing user's
        // `default_model = "reasoning-v1"` would silently route the main
        // chat to the `reasoning` workload regardless of their
        // `chat_provider` selection. Subagents still set their own role
        // through `ModelSpec::Hint(...)` in the subagent runner.
        let provider_role = match config.default_model.as_deref().map(str::trim) {
            Some("hint:agentic") => "agentic",
            Some("hint:coding") => "coding",
            Some("hint:summarization") => "summarization",
            Some("hint:reasoning") => "reasoning",
            _ => "chat",
        };
        let (provider, mut model_name): (Box<dyn Provider>, String) =
            crate::openhuman::inference::provider::create_chat_provider(provider_role, config)?;
        log::info!(
            "[session-builder] agent_id={} provider_role={} resolved_model={} supports_native_tools={}",
            agent_id,
            provider_role,
            model_name,
            provider.supports_native_tools()
        );
        let target_agent_id = target_def
            .map(|def| def.id.as_str())
            .unwrap_or("orchestrator");
        let target_is_lead = target_def
            .map(|def| !def.subagents.is_empty())
            .unwrap_or(true);
        if let Some(pinned_model) = config.configured_agent_model(target_agent_id, target_is_lead) {
            log::debug!(
                "[session-builder] agent_id={} using config-level model pin model={}",
                target_agent_id,
                pinned_model
            );
            model_name = pinned_model.to_string();
        }

        // Resolve the user-configured vision flag for the (now-final) model while
        // the full `Config` / `model_registry` is in scope — the turn engine only
        // sees `MultimodalConfig`. Stored on the session and surfaced to the image
        // gate via the `current_model_vision` task-local (covers custom/BYOK models
        // the provider can't introspect). Computed with `&model_name` since it's
        // moved into the builder below.
        let model_vision =
            crate::openhuman::inference::model_context::model_supports_vision(&model_name, config);

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
        // non-orchestrator definition from the registry. The orchestrator
        // continues to use `with_defaults` so its prompt stays
        // byte-identical to the legacy CLI/REPL behaviour except for the
        // tool-scope tightening we already landed in earlier commits.
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
                    let prompt_root = config.workspace_dir.join("agent").join("prompts");
                    let workspace_path = prompt_root.join(path);
                    let body_text = if workspace_path.is_file() {
                        match crate::openhuman::security::validate_path_within_root(
                            &workspace_path,
                            &prompt_root,
                        ) {
                            Ok(resolved) => {
                                std::fs::read_to_string(&resolved).unwrap_or_else(|e| {
                                    log::warn!(
                                        "[agent::builder] failed to read prompt {}: {e} — using empty body",
                                        workspace_path.display()
                                    );
                                    String::new()
                                })
                            }
                            Err(e) => {
                                log::warn!(
                                    "[agent::builder] prompt path rejected: {e} — using empty body"
                                );
                                String::new()
                            }
                        }
                    } else {
                        log::debug!(
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
            // Insert the privileged reflection block ahead of the
            // generic `user_memory` section when one is already
            // present (the `with_defaults` chain includes it). For
            // builders that do not contain `user_memory` (dynamic /
            // sub-agent prompts), the helper falls back to appending,
            // which still keeps reflections ahead of the
            // learned-context / user-profile blocks added immediately
            // after.
            prompt_builder = prompt_builder
                .insert_section_before(
                    "user_memory",
                    Box::new(crate::openhuman::context::prompt::UserReflectionsSection),
                )
                .add_section(Box::new(
                    crate::openhuman::learning::LearnedContextSection::new(memory.clone()),
                ))
                .add_section(Box::new(
                    crate::openhuman::learning::UserProfileSection::new(memory.clone()),
                ));
            // NOTE: MemoryAccessSection is added after tool-filtering so we can
            // gate it on retrieval-tool visibility — see below.
            log::info!(
                "[learning] prompt sections registered (user_reflections, learned_context, user_profile)"
            );
        }

        // Explicit-preferences injection — independent of the full learning
        // subsystem.  When `explicit_preferences_enabled` is true (the default)
        // and the full learning subsystem is NOT already wiring UserProfileSection,
        // we add it here so pinned preferences written by `remember_preference`
        // reach every session prompt.  The `fetch_learned_context` gate is
        // widened by `explicit_preferences_enabled` on the Agent (see
        // `session/turn.rs`) so the data is actually fetched and populated.
        if config.learning.explicit_preferences_enabled && !config.learning.enabled {
            prompt_builder = prompt_builder.add_section(Box::new(
                crate::openhuman::learning::UserProfileSection::new(memory.clone()),
            ));
            log::info!(
                "[learning] explicit-preference UserProfileSection registered \
                 (learning.enabled=false, explicit_preferences_enabled=true)"
            );
        }

        // (#623) Memory context for threads spawned from a subconscious
        // reflection: append the resolved `source_chunks` snapshot from
        // the reflection row as a `ReflectionMemoryContextSection`. The
        // resulting system prompt stays byte-stable for the session, so
        // every chat turn in the thread sees the same memory chunks the
        // subconscious LLM cited — without re-fetching per turn and
        // without polluting the visible conversation. No-op when the
        // caller passes `None` (regular chat threads).
        if let Some(chunks) = reflection_chunks {
            if !chunks.is_empty() {
                log::info!(
                    "[#623] injecting reflection memory context: {} chunks",
                    chunks.len()
                );
                prompt_builder = prompt_builder.with_reflection_context(chunks);
            }
        }
        if let Some(suffix) = profile_prompt_suffix
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            log::debug!(
                "[agent:builder] profile prompt section injected suffix_chars={}",
                suffix.chars().count()
            );
            prompt_builder = prompt_builder.add_section(Box::new(
                crate::openhuman::profiles::AgentProfilePromptSection::new(suffix),
            ));
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
                let reflection_provider: Option<
                    Arc<dyn crate::openhuman::inference::provider::Provider>,
                > = if config.learning.reflection_source
                    == crate::openhuman::config::ReflectionSource::Cloud
                {
                    Some(Arc::from(provider::create_routed_provider(
                        config.inference_url.as_deref(),
                        config.api_url.as_deref(),
                        config.api_key.as_deref(),
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

            if config.learning.tool_memory_capture_enabled {
                post_turn_hooks.push(Arc::new(ToolMemoryCaptureHook::new(memory.clone(), true)));
                log::info!("[learning] tool_memory_capture hook registered");
            }

            if config.learning.tool_memory_capture_enabled {
                post_turn_hooks.push(Arc::new(
                    crate::openhuman::agent_experience::AgentExperienceCaptureHook::new(
                        memory.clone(),
                        true,
                    ),
                ));
                log::info!("[learning] agent_experience_capture hook registered");
            }
        }

        // ── ArchivistHook — register independently of learning.enabled ──────
        //
        // Episodic capture (FTS5 index, segment lifecycle, LLM recap, embedding)
        // is the system-of-record for chat turns and must stay active even when
        // the inference stack (`reflection`, `stability_detector`) is disabled.
        // Gated only on `config.learning.episodic_capture_enabled` (default: true)
        // and on the memory backend exposing a SQLite connection.
        let archivist_hook_arc: Option<
            Arc<crate::openhuman::agent::harness::archivist::ArchivistHook>,
        > = if config.learning.episodic_capture_enabled {
            match memory.sqlite_conn() {
                Some(conn) => {
                    let hook = Arc::new(
                        crate::openhuman::agent::harness::archivist::ArchivistHook::new(conn, true)
                            .with_config(config.clone()),
                    );
                    post_turn_hooks
                        .push(Arc::clone(&hook)
                            as Arc<dyn crate::openhuman::agent::hooks::PostTurnHook>);
                    log::info!(
                        "[archivist] episodic capture hook registered (learning.enabled={})",
                        config.learning.enabled
                    );
                    Some(hook)
                }
                None => {
                    log::warn!(
                        "[archivist] no SQLite connection available from memory backend — \
                         episodic capture disabled"
                    );
                    None
                }
            }
        } else {
            log::info!(
                "[archivist] episodic_capture_enabled=false — archivist hook not registered"
            );
            None
        };

        // Best-effort prewarm from the shared Composio cache. This avoids
        // building the session with a knowingly stale `&[]` integration view
        // and then paying a repair pass on turn 1 just to recover the real
        // delegation surface.
        let prewarmed_integrations = crate::openhuman::composio::cached_active_integrations(config);
        // Per-profile connector gate: scope the connected-integration view to the
        // active profile's `composio_integrations` allowlist (None = all). This
        // governs both the system-prompt "connected integrations" surface and the
        // agent's `connected_integrations` field below, so a profile only ever
        // sees the toolkits it was granted.
        let prewarmed_integrations = match (
            prewarmed_integrations,
            profile.and_then(|p| p.composio_integrations.as_deref()),
        ) {
            (Some(list), Some(allow)) => {
                let filtered = crate::openhuman::profiles::filter_integrations(&list, Some(allow));
                tracing::debug!(
                    before = list.len(),
                    after = filtered.len(),
                    "[profiles] composio connectors scoped to profile allowlist"
                );
                Some(filtered)
            }
            (other, _) => other,
        };
        let prewarmed_integrations_slice = prewarmed_integrations.as_deref().unwrap_or(&[]);

        // Resolve the per-agent delegation tool set and visible-tool
        // whitelist from the target definition (when we have one) or
        // fall back to the orchestrator's synthesis path.
        //
        // For an agent with `[subagents] allowlist = [...]` in its TOML (today:
        // orchestrator), `collect_orchestrator_tools` synthesises one
        // `ArchetypeDelegationTool` per named sub-agent plus a single
        // collapsed `SkillDelegationTool`
        // (`delegate_to_integrations_agent`) whose `toolkit` argument
        // selects among the connected Composio toolkits (#1335).
        //
        // For an agent without `subagents` (today: welcome, critic,
        // archivist, etc.), no delegation tools are synthesised — the
        // LLM only sees the agent's own `ToolScope::Named` entries
        // from the global registry, narrowed by the visible-tool
        // filter.
        //
        // This builder is synchronous and sits on the CLI / REPL /
        // Tauri-web code path. It still opportunistically reuses the
        // process-wide Composio cache when one is already warm, which
        // lets the session start with the right `delegate_<toolkit>`
        // surface and prompt block without paying a turn-1 fetch. On a
        // cold cache we still fall back to the empty slice and let the
        // first turn repair the session state if needed.
        let (delegation_tools, filter_from_scope): (
            Vec<Box<dyn Tool>>,
            Option<std::collections::HashSet<String>>,
        ) = match (
            target_def,
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global(),
        ) {
            (Some(def), Some(reg)) => {
                let synthed = if should_synthesize_delegation_tools(def) {
                    tools::orchestrator_tools::collect_orchestrator_tools(
                        def,
                        reg,
                        prewarmed_integrations_slice,
                    )
                } else {
                    Vec::new()
                };
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
                    Some(orch_def) => tools::orchestrator_tools::collect_orchestrator_tools(
                        orch_def,
                        reg,
                        prewarmed_integrations_slice,
                    ),
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
        let mut visible: std::collections::HashSet<String> = match filter_from_scope {
            Some(set) => set,
            None => delegation_tools
                .iter()
                .map(|t| t.name().to_string())
                .collect(),
        };
        // Compaction applies to every agent's tool output, so the CCR recovery
        // tool must be a *real* member of any non-empty allowlist — this is the
        // single source of truth that the policy session, advertised specs, and
        // the run-time visible-name gate all consume, so adding it here makes a
        // `retrieve_tool_output("…")` footer actionable for Named-scope agents
        // (e.g. the orchestrator's curated list). An empty set already means
        // "no filter", so it needs nothing. Added BEFORE the disallow filter
        // below so an agent that explicitly disallows it still has it removed.
        super::ensure_recovery_tool_visible(&mut visible);

        if let Some(def) = target_def {
            if !def.disallowed_tools.is_empty() {
                match &def.tools {
                    ToolScope::Wildcard => {
                        visible = tools
                            .iter()
                            .map(|t| t.name().to_string())
                            .chain(delegation_tools.iter().map(|t| t.name().to_string()))
                            .filter(|name| !definition_disallows_tool(&def.disallowed_tools, name))
                            .collect();
                    }
                    ToolScope::Named(_) => {
                        visible
                            .retain(|name| !definition_disallows_tool(&def.disallowed_tools, name));
                    }
                }
            }
        }

        // Phase 4 (#566): add the MemoryAccessSection bias instruction only
        // when at least one retrieval tool is actually loaded AND survives
        // filtering. We require both because:
        //   - the tool may be filtered out by the agent's scope config
        //   - the tool may not be registered at all on this agent (tool
        //     listing is build-time configurable)
        // An empty `visible` set means "no filter" (wildcard / orchestrator
        // path); in that case any registered retrieval tool is reachable.
        if config.learning.enabled {
            let recall_tools = ["memory_recall", "memory_search"];
            let has_retrieval = recall_tools.iter().any(|name| {
                let registered = tools.iter().any(|t| t.name() == *name)
                    || delegation_tools.iter().any(|t| t.name() == *name);
                let allowed_by_filter = visible.is_empty() || visible.contains(*name);
                registered && allowed_by_filter
            });
            if has_retrieval {
                prompt_builder = prompt_builder
                    .add_section(Box::new(crate::openhuman::learning::MemoryAccessSection));
                log::debug!("[learning] memory_access prompt section registered");
            } else {
                log::debug!(
                    "[learning] skipping MemoryAccessSection — neither memory_recall nor \
                     memory_search is registered+visible for agent={agent_id}"
                );
            }
        }

        // De-duplicate: some synthesised tool names may collide with
        // already-registered tools (unlikely for `delegate_*` names but
        // cheap to guard against).
        let existing_names: std::collections::HashSet<String> =
            tools.iter().map(|t| t.name().to_string()).collect();
        let inserted_delegation_tools: Vec<Box<dyn Tool>> = delegation_tools
            .into_iter()
            .filter(|t| !existing_names.contains(t.name()))
            .collect();
        let synthesized_tool_names: std::collections::HashSet<String> = inserted_delegation_tools
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        tools.extend(inserted_delegation_tools);

        // Pre-fetch Critical + High priority tool-scoped memory rules so they
        // pin into the (compression-resistant) system prompt for the whole
        // session. Done here — after the tool list is finalised — so we only
        // fetch rules for tools this agent can actually use.  Skipped when
        // `learning.enabled` is false (no new rules are written in that mode,
        // and users who opt out of learning expect no stored rules to surface)
        // or when the runtime cannot host a synchronous bridge (single-threaded
        // test harnesses).
        if config.learning.enabled && config.learning.tool_memory_capture_enabled {
            let agent_tool_names: Vec<String> =
                tools.iter().map(|t| t.name().to_string()).collect();
            let pinned = prefetch_tool_memory_rules_blocking(memory.clone(), &agent_tool_names);
            if !pinned.is_empty() {
                log::info!(
                    "[memory::tool_memory] pinning {} tool-scoped rule(s) into system prompt",
                    pinned.len()
                );
                prompt_builder = prompt_builder.with_tool_memory_rules(pinned);
            }
        }

        // Build the P-Format registry AFTER the tool list is finalised
        // (including orchestrator tools) so every tool gets a signature
        // entry. The registry is self-contained — it doesn't hold a
        // reference back into the tools Vec.
        let pformat_registry = crate::openhuman::agent::pformat::build_registry(&tools);
        let tool_dispatcher: Box<dyn crate::openhuman::agent::dispatcher::ToolDispatcher> =
            match dispatcher_choice.as_str() {
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
        let tool_dispatcher: Box<dyn crate::openhuman::agent::dispatcher::ToolDispatcher> =
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
        let effective_trigger_memory_agent = target_def
            .map(|def| def.trigger_memory_agent)
            .unwrap_or_default();

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
        // Workflows_agent and every other typed sub-agent are unaffected
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
                    .with_max_chars(
                        config
                            .agent
                            .resolved_memory_limits()
                            .max_memory_context_chars,
                    )
                    .with_workspace_dir(config.workspace_dir.clone())
                    // Per-profile memory gate: when the active profile opts out
                    // of agent-conversation recall, suppress the prior-chat and
                    // cross-chat blocks. Defaults to on for None / unset.
                    .with_agent_conversations(
                        profile.map_or(true, |p| p.include_agent_conversations),
                    ),
            ))
            .prompt_builder(prompt_builder)
            .config(config.agent.clone())
            .context_config(config.context.clone())
            .model_name(model_name)
            .model_vision(model_vision)
            .temperature(effective_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .action_dir(config.action_dir.clone())
            .workflows(crate::openhuman::workflows::load_workflow_metadata(
                &config.workspace_dir,
            ))
            .auto_save(config.memory.auto_save)
            .post_turn_hooks(post_turn_hooks)
            .learning_enabled(config.learning.enabled)
            .explicit_preferences_enabled(config.learning.explicit_preferences_enabled)
            .agent_definition_name(agent_id.to_string())
            .omit_profile(effective_omit_profile)
            .omit_memory_md(effective_omit_memory_md)
            .trigger_memory_agent(effective_trigger_memory_agent);
        if let Some(ps) = payload_summarizer {
            builder = builder.payload_summarizer(ps);
        }
        builder = builder.archivist_hook(archivist_hook_arc);
        builder = builder.unified_compaction_enabled(config.learning.unified_compaction_enabled);
        let mut agent = builder.build()?;
        let connected_integrations_initialized = prewarmed_integrations.is_some();
        agent.connected_integrations = prewarmed_integrations.unwrap_or_default();
        agent.connected_integrations_initialized = connected_integrations_initialized;
        agent.integration_runtime_config = Some(config.clone());
        agent.last_seen_integrations_hash =
            crate::openhuman::composio::connected_set_hash(&agent.connected_integrations);
        agent.synthesized_tool_names = synthesized_tool_names;
        Ok(agent)
    }
}

fn definition_disallows_tool(disallowed: &[String], name: &str) -> bool {
    disallowed.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            entry == name
        }
    })
}
