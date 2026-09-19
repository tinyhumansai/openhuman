//! `OpenHumanSessionHost::from_config` factory methods and the internal
//! `build_session_agent_inner` constructor.

use super::helpers::prefetch_tool_memory_rules_blocking;
use super::should_synthesize_delegation_tools;
use crate::agent::harness::definition::NO_TOOLS_SENTINEL;
use crate::agent::harness::definition::{AgentDefinitionRegistry, PromptSource, ToolScope};
use crate::agent::host_runtime;
use crate::agent::prompts::SystemPromptBuilder;
use crate::agent::session_host::types::OpenHumanSessionHost;
use crate::config::Config;
use crate::inference::provider;
use crate::memory::Memory;
use crate::memory::tool_memory::capture::ToolMemoryCaptureHook;
use crate::security::SecurityPolicy;
use crate::tools;
use anyhow::Result;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool};
use tinytools_agent::dialect::{NativeDialect, PFormatDialect, ToolDialect, XmlDialect};

impl OpenHumanSessionHost {
    /// Constructs an `OpenHumanSessionHost` instance from a global system configuration.
    ///
    /// Thin wrapper around [`OpenHumanSessionHost::from_config_for_agent`] that always
    /// targets the orchestrator definition. This preserves the legacy
    /// "main agent = orchestrator" behaviour for CLI / REPL / any caller
    /// that does not participate in the #525 onboarding-routing flow.
    ///
    /// Callers that need to select a different agent at session-build
    /// time (for example the Tauri web chat path, which routes to the
    /// welcome agent pre-onboarding) should call
    /// [`OpenHumanSessionHost::from_config_for_agent`] directly.
    pub fn from_config(config: &Config) -> Result<Self> {
        Self::from_config_for_agent(config, "orchestrator")
    }

    /// Constructs an `OpenHumanSessionHost` instance scoped to a specific agent
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
        // discovering the id is unknown. See `resolve_target_definition`
        // for the full resolution order (harness registry, then the
        // config-backed custom agent registry, then the orchestrator's
        // legacy pre-startup fallback).
        let target_def = resolve_target_definition(config, agent_id)?;

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

        Self::build_session_agent_inner(config, agent_id, target_def.as_ref(), false)
    }

    /// Build a session agent from a definition the caller already holds,
    /// rather than an id resolved through `AgentDefinitionRegistry::global()`
    /// or `config.agent_registry`.
    ///
    /// The definition is authoritative for this agent's prompt, tool scope,
    /// sandbox mode and delegation allowlist; it is stamped on the session so
    /// in-turn readers ([`OpenHumanSessionHost::resolved_definition`]) never fall back to a
    /// registry entry that happens to share its id. This is the constructor a
    /// library host uses to run many independently specified agents on one
    /// booted core.
    pub fn from_config_with_definition(
        config: &Config,
        definition: &crate::agent::harness::definition::AgentDefinition,
    ) -> Result<Self> {
        log::debug!(
            "[agent] from_config_with_definition id={} sandbox={:?}",
            definition.id,
            definition.sandbox_mode,
        );
        Self::build_session_agent_inner(config, &definition.id, Some(definition), false)
    }

    /// Internal constructor that consumes the optionally-resolved agent
    /// definition. Split out from [`OpenHumanSessionHost::from_config_for_agent`] so
    /// the lookup + logging live in one place and the heavy-lifting
    /// body stays readable.
    // `pub(crate)` (rather than private) so `builder_tests` can drive the
    // definition-cap resolution logic (issue #4868) directly with a
    // hand-picked `target_def`, independent of the process-global
    // `AgentDefinitionRegistry` singleton's init-once state. Still not part
    // of the crate's public API.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_session_agent_inner(
        config: &Config,
        agent_id: &str,
        target_def: Option<&crate::agent::harness::definition::AgentDefinition>,
        read_only_tools_only: bool,
    ) -> Result<Self> {
        let workspace_descriptor = derive_turn_workspace_descriptor();

        let runtime: Arc<dyn host_runtime::RuntimeAdapter> = Arc::from(
            host_runtime::create_runtime(&config.runtime, config.shell.hide_window)?,
        );
        let security = Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        ));
        // Phase 1 of #1401: see comment in channels/runtime/startup.rs.
        let audit = crate::security::get_or_create_workspace_audit_logger(
            crate::config::AuditConfig::default(),
            config.workspace_dir.clone(),
        )?;

        // The session's store, through the same binding the archivist resolves
        // two statements down — so one subtree yields one store rather than an
        // engine handle beside a driver over the same files.
        //
        // The two reasons this was deferred are both settled. **Embedder
        // resolution**: the factory's ladder and the driver's are the same
        // code reading the same field. `Config::workload_local_model` and
        // `EngineRuntimeConfig::workload_local_model` both take
        // `embeddings_provider`, strip `"ollama:"`, trim and reject empty, and
        // that `Option` is the only input to `effective_embedding_settings`.
        // The Ollama health-gate is not a difference either: it lives inside
        // `create_unified_memory_full`, which the module runs because the
        // module *is* the engine, and its probe address is proxied back here
        // through `EmbeddingHost::ollama_base_url`. (`embedding_routes` never
        // mattered — the engine's own parameter is underscore-prefixed and
        // unused.) **The test build**: `binding::module_provider` under
        // `cfg(test)` loads the module when `TINYMEMORY_TEST_MODULE` names it
        // and degrades to the null driver otherwise, which is the same footing
        // the archivist has had here all along.
        let memory: Arc<dyn Memory> =
            crate::agent::experience::ops::DriverMemory::for_config(config)
                .map_err(|e| anyhow::anyhow!("session memory binding: {e}"))?;
        // The archivist takes the bound driver for this session's memory
        // subtree — the same subtree `session_memory` opened — rather than the
        // raw SQLite handle the factory used to strip off the engine result.
        // That handle was the #5378 `:290` blocker: a concrete connection no
        // module or remote driver can supply. The engine's connection is now
        // exclusively the engine's. Lane C (#6040) rides the same binding.
        let (archivist_provider, auto_recall) = super::helpers::bind_session_memory(config)?;

        // Load the user's persisted tool preferences once. They drive two
        // things below: granting the App UI Control / App Automation mutation
        // opt-in (#3762) and filtering the tool set to the enabled snapshot.
        let enabled_tools: Vec<String> = {
            use crate::desktop::app_state::load_stored_app_state;
            match load_stored_app_state(config) {
                Ok(stored) => stored
                    .onboarding_tasks
                    .map(|tasks| tasks.enabled_tools)
                    .unwrap_or_default(),
                Err(e) => {
                    log::warn!(
                        "[session-builder] failed to load app state for tool filtering: {e}"
                    );
                    Vec::new()
                }
            }
        };

        // Share a single `Arc<Config>` across the heavyweight per-build consumers
        // (the tool registry, the reflection hook, the turn provider) instead of
        // deep-cloning the large `Config` at each site (#5050, Fix 1). `Config` is
        // immutable after construction, so one refcounted instance is behaviourally
        // identical to N independent clones.
        let base_config: Arc<Config> = Arc::new(config.clone());
        let tool_config: Arc<Config> = Arc::clone(&base_config);

        let mut tools = tools::ops::all_tools_with_runtime(
            Arc::clone(&tool_config),
            &security,
            runtime,
            audit,
            &tool_config.browser,
            &tool_config.http_request,
            &tool_config.action_dir,
            &tool_config.agents,
            &tool_config,
            workspace_descriptor
                .as_ref()
                .map(|descriptor| descriptor.root.as_path()),
        );

        // Filter tools by the user preference loaded above.
        if !enabled_tools.is_empty() {
            crate::tools::filter_tools_by_user_preference(&mut tools, &enabled_tools);
        }

        if read_only_tools_only {
            let before = tools.len();
            tools.retain(|tool| {
                tool.permission_level() <= PermissionLevel::ReadOnly
                    && !matches!(tool.scope(), tinytools::ToolScope::CliRpcOnly)
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
        // The ReliableProvider retry/backoff + model-fallback wrapper is
        // re-layered on top of the factory's resolved backend below (issue
        // #4249, 1c). `model_routes` translation and intelligent local/cloud
        // task hinting now live in the unified routing layer (router.rs) rather
        // than a per-session wrapper, so they are not re-wrapped here.
        // The Master OpenHumanSessionHost's definition selects the coding workload so the
        // default user-facing turn has a model suitable for the direct
        // inspect → edit → verify loop. Legacy/no-definition callers retain
        // the configured chat default. Other specialised roles still select
        // their model in the sub-agent runner.
        // Only the explicit `hint:<role>` form routes to a specialised
        // workload — legacy tier literals like `reasoning-v1` (which the
        // bootstrap historically pinned as `default_model` for everyone)
        // fall through to `chat`. This preserves `config.chat_provider` for
        // legacy callers. A built-in Master OpenHumanSessionHost has an explicit
        // `hint:coding`, while existing pre-registry callers continue using
        // `config.default_model` unchanged.
        let provider_role =
            provider_role_for_definition(agent_id, config.default_model.as_deref(), target_def);
        // Retry/backoff is now owned by the crate `RetryPolicy` at the harness
        // model call (issue #4249, Phase 3a) — see `tinyagents::run_policy_for`.
        // The turn path therefore no longer wraps the resolved provider in
        // `ReliableProvider`; wrapping it here plus the crate retry would
        // double-retry every transient error. Cross-route fallback is likewise
        // the crate registry `FallbackPolicy`, so `config.reliability.*` no longer
        // layers on the turn path (it still governs the non-seam provider paths).
        let (resolved_chat_model, mut model_name) =
            crate::inference::provider::create_chat_model_with_model_id(
                provider_role,
                config,
                config.default_temperature,
            )?;
        let supports_native = resolved_chat_model
            .profile()
            .is_none_or(|profile| profile.tool_calling);
        log::info!(
            "[session-builder] agent_id={} provider_role={} resolved_model={} supports_native_tools={}",
            agent_id,
            provider_role,
            model_name,
            supports_native
        );
        let target_agent_id = target_def
            .map(|def| def.id.as_str())
            .unwrap_or("orchestrator");
        let target_is_lead = target_def
            .map(|def| !def.subagents.is_empty())
            .unwrap_or(true);
        // The `subconscious` workload's model is governed by `subconscious_provider`
        // + the managed-tier registry — NOT by an interactive agent model pin.
        // The tick reuses the orchestrator definition (agent_id="orchestrator"),
        // so without this guard a configured `[orchestrator].model` pin would
        // clobber the resolved subconscious model and send an unrelated tier to
        // the Subconscious provider (Codex P2).
        if provider_role != "subconscious" {
            if let Some(pinned_model) =
                config.configured_agent_model(target_agent_id, target_is_lead)
            {
                log::debug!(
                    "[session-builder] agent_id={} using config-level model pin model={}",
                    target_agent_id,
                    pinned_model
                );
                model_name = pinned_model.to_string();
            }
        } else {
            log::debug!(
                "[session-builder] agent_id={} provider_role=subconscious — skipping agent model \
                 pin so the subconscious provider/registry model is preserved",
                target_agent_id
            );
        }

        // Resolve the user-configured vision flag for the (now-final) model while
        // the full `Config` / `model_registry` is in scope — the turn engine only
        // sees `MultimodalConfig`. Stored on the session and surfaced to the image
        // gate via the `current_model_vision` task-local (covers custom/BYOK models
        // the provider can't introspect). Computed with `&model_name` since it's
        // moved into the builder below.
        let model_vision =
            crate::inference::model_context::model_supports_vision(&model_name, config);

        // #5146 §2.1/§2.3: when the active model can't take images the turn
        // engine silently strips them, and the user gets a confident answer
        // about an image the model never saw. Log the actionable reason (which
        // model, and what to switch to) at the moment the decision is made.
        if !model_vision {
            let reason =
                crate::inference::provider::fallback_diagnostics::local_vision_unsupported_message(
                    &model_name,
                );
            log::info!("[vision-preflight] {reason}");
        }

        // Dispatcher selection is deferred until after the tool list is
        // finalised (orchestrator tools are appended below). We capture
        // the choice string now so the provider borrow doesn't conflict
        // with the later `provider` move into the builder.
        let dispatcher_choice = config.agent.tool_dispatcher.clone();

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
                ),
                PromptSource::File { path } => {
                    let prompt_root = config.workspace_dir.join("agent").join("prompts");
                    let workspace_path = prompt_root.join(path);
                    let body_text = if workspace_path.is_file() {
                        match crate::security::validate_path_within_root(
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
                    Box::new(crate::agent::prompts::UserReflectionsSection),
                )
                .add_section(Box::new(
                    crate::agent::learning::LearnedContextSection::new(memory.clone()),
                ))
                .add_section(Box::new(crate::agent::learning::UserProfileSection::new(
                    memory.clone(),
                )));
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
        // widened by `explicit_preferences_enabled` on the OpenHumanSessionHost (see
        // `session/turn.rs`) so the data is actually fetched and populated.
        if config.learning.explicit_preferences_enabled && !config.learning.enabled {
            prompt_builder = prompt_builder.add_section(Box::new(
                crate::agent::learning::UserProfileSection::new(memory.clone()),
            ));
            log::info!(
                "[learning] explicit-preference UserProfileSection registered \
                 (learning.enabled=false, explicit_preferences_enabled=true)"
            );
        }

        // Build post-turn hooks when learning is enabled
        let mut post_turn_hooks: Vec<Arc<dyn crate::agent::hooks::PostTurnHook>> = Vec::new();
        if config.learning.enabled {
            if config.learning.reflection_enabled {
                // The reflection hook needs an owned `Arc<Config>`; reuse the
                // shared base config (a refcount bump) rather than a second deep
                // clone of the full config (#5050, Fix 1).
                let full_config = Arc::clone(&base_config);
                // For cloud reflection, wrap the provider in an Arc.
                // For local, no provider needed.
                let reflection_provider: Option<Arc<dyn tinyinference_llm::model::ChatModel<()>>> =
                    if config.learning.reflection_source == crate::config::ReflectionSource::Cloud {
                        let (model, resolved_model) =
                            provider::create_chat_model_with_model_id("reasoning", config, 0.3)?;
                        log::debug!(
                            "[learning] built crate-native reflection model resolved_model={resolved_model}"
                        );
                        Some(model)
                    } else {
                        None
                    };
                post_turn_hooks.push(Arc::new(crate::agent::learning::ReflectionHook::new(
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
                post_turn_hooks.push(Arc::new(crate::agent::learning::UserProfileHook::new(
                    config.learning.clone(),
                    memory.clone(),
                )));
                log::info!("[learning] user_profile hook registered");
            }

            if config.learning.tool_tracking_enabled {
                post_turn_hooks.push(Arc::new(crate::agent::learning::ToolTrackerHook::new(
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
                    crate::agent::experience::AgentExperienceCaptureHook::new(memory.clone(), true),
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
        // using the explicit SQLite resource returned by the session factory.
        let archivist_hook_arc: Option<Arc<crate::agent::harness::archivist::ArchivistHook>> =
            if config.learning.episodic_capture_enabled {
                let hook = Arc::new(
                    crate::agent::harness::archivist::ArchivistHook::new(archivist_provider, true)
                        .with_config(Arc::clone(&base_config)),
                );
                post_turn_hooks
                    .push(Arc::clone(&hook) as Arc<dyn crate::agent::hooks::PostTurnHook>);
                log::info!(
                    "[archivist] episodic capture hook registered (learning.enabled={})",
                    config.learning.enabled
                );
                Some(hook)
            } else {
                log::info!(
                    "[archivist] episodic_capture_enabled=false — archivist hook not registered"
                );
                None
            };

        post_turn_hooks.extend(crate::agent::hooks::embedder_post_turn_hooks());

        // Best-effort prewarm from the shared Composio cache. This avoids
        // building the session with a knowingly stale `&[]` integration view
        // and then paying a repair pass on turn 1 just to recover the real
        // delegation surface.
        let prewarmed_integrations =
            crate::integrations::composio::cached_active_integrations(config);
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
            crate::agent::harness::definition::AgentDefinitionRegistry::global(),
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
                        // Only the *advertised* ones. A synthesised tool that
                        // reports `ToolExposure::Hidden` is a member of a
                        // collapsed tool — today every `ArchetypeDelegationTool`,
                        // whose family the single `delegate_to` tool now stands
                        // for. Inserting it here would put it back on the wire
                        // beside the tool that replaced it, shipping both
                        // surfaces and saving nothing.
                        //
                        // This is not the same judgement as
                        // `strip_deferred_from_visible`, which deliberately
                        // leaves a hand-written `[tools] named` belt alone. That
                        // restraint is about not second-guessing a human's
                        // choice; these names were never chosen by a human, they
                        // are inserted right here. Hiding one removes nothing an
                        // author asked for.
                        //
                        // The tool stays in `synthed`, so it stays registered
                        // and dispatchable for a replayed transcript or a saved
                        // skill that names it — exactly like a packed tool.
                        for t in &synthed {
                            if t.exposure() == tinytools::ToolExposure::Hidden {
                                continue;
                            }
                            set.insert(t.name().to_string());
                        }
                        // `named = []` means zero tools. An empty set here is
                        // the harness's "no filter" sentinel and would advertise
                        // the whole registry instead — the exact inversion that
                        // handed `summarizer` and `trigger_triage` 109 tools
                        // each. Spell the empty belt so it survives.
                        if set.is_empty() {
                            set.insert(NO_TOOLS_SENTINEL.to_string());
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
            (Some(def), None) => {
                // We have a target definition (either a pre-populated
                // harness entry looked up before the registry singleton
                // existed, or — the common case today — a `CustomRegistry`
                // definition `resolve_target_definition` synthesizes
                // straight from `config.agent_registry.entries` without
                // ever consulting `AgentDefinitionRegistry::global()`, see
                // `agent_registry::find_custom_in_config`). Delegation-tool
                // synthesis needs the registry (to resolve named
                // subagents), so it's skipped here, but `def.tools` is a
                // real scope the caller authored and MUST still gate
                // visibility — silently dropping it into the `(_, None)`
                // "no registry, no filter" catch-all would leave a custom
                // agent's `ToolScope::Named` allowlist entirely
                // unenforced (visible tools empty rather than the named
                // set), regressing the least-privilege contract this
                // synthesis path exists to provide.
                log::debug!(
                    "[agent::builder] AgentDefinitionRegistry not initialised — skipping \
                     delegation tool synthesis, but still applying target definition's own \
                     tool scope"
                );
                let filter: Option<std::collections::HashSet<String>> = match &def.tools {
                    ToolScope::Named(names) => {
                        let mut set: std::collections::HashSet<String> =
                            names.iter().cloned().collect();
                        // Same rule as the branch above: an empty named scope
                        // is zero tools, and an empty set would mean the
                        // opposite.
                        if set.is_empty() {
                            set.insert(NO_TOOLS_SENTINEL.to_string());
                        }
                        Some(set)
                    }
                    ToolScope::Wildcard => None,
                };
                (Vec::new(), filter)
            }
            (None, None) => {
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
        // tools alone so the existing `OpenHumanSessionHost::visible_tool_names`
        // contract (empty == no filter) stays intact: an empty set
        // means "no filter" for both legacy callers and the new
        // agent-scoped path.
        let mut visible: std::collections::HashSet<String> = match filter_from_scope {
            Some(set) => set,
            None => delegation_tools
                .iter()
                .filter(|t| t.exposure() != tinytools::ToolExposure::Hidden)
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
                            .chain(
                                delegation_tools
                                    .iter()
                                    .filter(|t| t.exposure() != tinytools::ToolExposure::Hidden)
                                    .map(|t| t.name().to_string()),
                            )
                            .filter(|name| !definition_disallows_tool(&def.disallowed_tools, name))
                            .collect();
                    }
                    ToolScope::Named(_) => {
                        visible
                            .retain(|name| !definition_disallows_tool(&def.disallowed_tools, name));
                    }
                }
                // Disallowing every tool must remain a zero-tool scope. An
                // empty visible set means "no filter" to the harness.
                if visible.is_empty() {
                    visible.insert(NO_TOOLS_SENTINEL.to_string());
                }
            }
        }

        // Memory prompt sections — the read side (#566) and the write side
        // (#6048); both gates live in `helpers::add_memory_prompt_sections`.
        prompt_builder = super::helpers::add_memory_prompt_sections(
            prompt_builder,
            &tools,
            &delegation_tools,
            &visible,
            agent_id,
        );

        // The delegation tools stay beside the durable registry rather than
        // inside it: the builder holds them in `OpenHumanSessionHost::synthesized_tools`,
        // drops any name a durable tool already owns, and
        // `refresh_delegation_tools` replaces the whole set later (#6145).

        // Pre-fetch Critical + High priority tool-scoped memory rules so they
        // pin into the (compression-resistant) system prompt for the whole
        // session. Done here — after the tool list is finalised — so we only
        // fetch rules for tools this agent can actually use.  Skipped when
        // `learning.enabled` is false (no new rules are written in that mode,
        // and users who opt out of learning expect no stored rules to surface)
        // or when the runtime cannot host a synchronous bridge (single-threaded
        // test harnesses).
        if config.learning.enabled && config.learning.tool_memory_capture_enabled {
            let agent_tool_names: Vec<String> = tools
                .iter()
                .chain(delegation_tools.iter())
                .map(|t| t.name().to_string())
                .collect();
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
        let pformat_registry = tinytools_agent::build_registry(
            tools
                .iter()
                .chain(delegation_tools.iter())
                .map(|tool| (tool.name(), tool.parameters_schema())),
        );
        let dispatcher_kind =
            resolve_dispatcher_kind(&dispatcher_choice, supports_native, agent_id);
        let tool_dispatcher: Box<dyn ToolDialect> = match dispatcher_kind {
            DispatcherKind::Native => Box::new(NativeDialect),
            DispatcherKind::Xml => Box::new(XmlDialect),
            DispatcherKind::PFormat => Box::new(PFormatDialect::new(pformat_registry.clone())),
        };

        log::debug!(
            "[agent] tool dispatcher selected: choice={dispatcher_choice} agent_id={agent_id} \
             kind={dispatcher_kind:?} sends_tool_specs={} pformat_registry_entries={}",
            tool_dispatcher.should_send_tool_specs(),
            pformat_registry.len()
        );

        // The resolved definition's TOML value takes precedence over the
        // configured default for canonical agent runs.
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
        let effective_tokenjuice_compression = target_def
            .map(|def| def.effective_tokenjuice_compression())
            .unwrap_or(crate::inference::tokenjuice::AgentTokenjuiceCompression::Full);

        // Stamp the resolved agent definition id onto the OpenHumanSessionHost via the
        // builder. Without this call, `agent_definition_name` falls
        // back to the legacy `"main"` default (see `SessionHostBuilder::build`)
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
        // See the docstring on `SessionHostBuilder::agent_definition_name`
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
            std::sync::Arc<dyn crate::agent::tinyagents::payload_summarizer::PayloadSummarizer>,
        > = if agent_id == "orchestrator" && config.context.summarizer_payload_threshold_tokens > 0
        {
            match crate::agent::harness::definition::AgentDefinitionRegistry::global() {
                Some(reg) => match reg.get("summarizer") {
                    Some(summarizer_def) => {
                        log::info!(
                            "[agent::builder] wiring payload_summarizer for orchestrator: \
                             threshold_tokens={} max_tokens={}",
                            config.context.summarizer_payload_threshold_tokens,
                            config.context.summarizer_max_payload_tokens
                        );
                        Some(std::sync::Arc::new(
                            crate::agent::tinyagents::payload_summarizer::SubagentPayloadSummarizer::new(
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

        // Crate-native turn models (Phase 3 P3-B): the production main-turn agent
        // builds crate `ChatModel`s from `(provider_role, config)` without retaining
        // a host provider. The
        // `agent_harness_e2e` mock now serves SSE for streaming, so the crate-native
        // streaming path is exercised end-to-end.
        //
        // Issue #4868 — resolve the per-agent iteration cap. When a named
        // definition is present, its `effective_max_iterations()` (which honors
        // `iteration_policy = "extended"` -> 50, and the declared `max_iterations`
        // for strict agents) takes priority over the global
        // `config.agent.max_tool_iterations` (default 10). This is the single
        // shared resolution point that closes #4868 for every direct-invocation
        // path: flows_build, flows_discover, agent-node runtime, cron, MCP
        // server, etc. Falls back to the global default when there is no
        // definition for this agent_id.
        let mut effective_agent_config = config.agent.clone();
        if let Some(def) = target_def {
            let def_cap = def.effective_max_iterations();
            log::info!(
                "[agent::builder] applying definition iteration cap for agent_id={}: \
                 definition.max_iterations={} iteration_policy={:?} -> effective={} \
                 (was global default {})",
                agent_id,
                def.max_iterations,
                def.iteration_policy,
                def_cap,
                config.agent.max_tool_iterations,
            );
            effective_agent_config.max_tool_iterations = def_cap;
        }
        let mut builder = OpenHumanSessionHost::builder()
            .crate_native_provider(provider_role, Arc::clone(&base_config))
            .tools(tools)
            .synthesized_tools(delegation_tools)
            .visible_tool_names(visible)
            .memory(memory)
            .auto_recall(Some(auto_recall))
            .tool_dispatcher(tool_dispatcher)
            .prompt_builder(prompt_builder)
            .config(effective_agent_config)
            .context_config(config.context.clone())
            .model_name(model_name)
            .model_vision(model_vision)
            .temperature(effective_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .action_dir(config.action_dir.clone())
            .workspace_descriptor(workspace_descriptor)
            .workflows({
                let mut catalogue = crate::skills::load_workflow_metadata(&config.workspace_dir);
                #[cfg(feature = "flows")]
                catalogue.extend(crate::flows::catalogue::flow_entries(config));
                catalogue
            })
            .auto_save(config.memory.auto_save)
            .post_turn_hooks(post_turn_hooks)
            .learning_enabled(config.learning.enabled)
            .explicit_preferences_enabled(config.learning.explicit_preferences_enabled)
            .agent_definition_name(agent_id.to_string())
            .omit_profile(effective_omit_profile)
            .omit_memory_md(effective_omit_memory_md)
            .trigger_memory_agent(effective_trigger_memory_agent)
            .tokenjuice_compression(effective_tokenjuice_compression);
        if let Some(ps) = payload_summarizer {
            builder = builder.payload_summarizer(ps);
        }
        builder = builder.archivist_hook(archivist_hook_arc);
        let mut agent = builder.build()?;
        let connected_integrations_initialized = prewarmed_integrations.is_some();
        agent.connected_integrations = prewarmed_integrations.unwrap_or_default();
        agent.connected_integrations_initialized = connected_integrations_initialized;
        // The same snapshot `base_config` already holds — `Config` is immutable
        // after construction, so a second deep clone bought nothing but a
        // second resident copy of a 95-field struct with nested `Vec`s
        // (openhuman#6218).
        agent.runtime_config = Some(Arc::clone(&base_config));
        agent.hosted_base = AgentDefinitionRegistry::global_arc().map(|definitions| {
            Arc::new(crate::agent::tinyagents::host::OpenHumanHostBase {
                config: Arc::clone(&base_config),
                definitions,
                security_policy: security,
                memory: agent.memory_arc(),
                post_turn_hooks: agent.post_turn_hooks.clone(),
            })
        });
        if agent.hosted_base.is_none() {
            tracing::warn!(
                "[tinyagents] hosted invocation base unavailable: agent definition registry was not initialized"
            );
        }
        agent.definition = target_def.cloned().map(Arc::new);
        agent.last_seen_integrations_hash =
            crate::integrations::composio::connected_set_hash(&agent.connected_integrations);
        Ok(agent)
    }
}

/// Resolves the `AgentDefinition` a session should be built from, given the
/// requested `agent_id`, in three steps:
///
/// 1. **Harness registry** (`AgentDefinitionRegistry`, the process-global
///    singleton of built-in + workspace-TOML-override definitions) — a hit
///    here wins outright.
/// 2. **Config-backed custom agent registry** (`config.agent_registry.entries`,
///    `AgentRegistrySource::Custom`) — on a harness-registry miss (or the
///    registry not yet being initialised), a user-authored custom agent is
///    synthesized into a real `AgentDefinition` via
///    `agent_registry::definition_from_registry_entry` so it runs through
///    the exact same `build_session_agent_inner` path (and therefore the
///    exact same `SecurityPolicy` / tool-filtering / approval gate) as a
///    built-in. This closes the gap where a custom agent either hard-errored
///    (chat, task-dispatcher) or silently ran tool-less/persona-only (flows'
///    `RegistryFallback`) — see the cross-cutting fix in the PR that added
///    this function.
/// 3. **Orchestrator legacy fallback** — `orchestrator` alone is allowed to
///    resolve to `None` (pre-startup, tests): the caller then builds with the
///    default prompt/filter, matching pre-#1 behaviour.
///
/// Any other id that resolves nowhere is a hard error, exactly as before this
/// function existed — only the *search order* changed, not the failure
/// contract for a genuinely-unknown id.
fn resolve_target_definition(
    config: &Config,
    agent_id: &str,
) -> Result<Option<crate::agent::harness::definition::AgentDefinition>> {
    let registry = AgentDefinitionRegistry::global();

    if let Some(reg) = registry {
        if let Some(def) = reg.get(agent_id) {
            return Ok(Some(def.clone()));
        }
    }

    // Harness registry miss (or not yet initialised). Before failing, check
    // the config-backed custom agent registry — the one place custom
    // (non-shipped) agents live.
    if let Some(entry) = crate::agent::registry::find_custom_in_config(config, agent_id) {
        log::info!(
            "[agent::builder] agent_id={} not found in the harness AgentDefinitionRegistry — \
             synthesizing a definition from its custom agent_registry entry so it runs with its \
             real tool belt instead of persona-only / erroring",
            agent_id
        );
        return Ok(Some(
            crate::agent::registry::definition_from_registry_entry(&entry),
        ));
    }

    if agent_id == "orchestrator" {
        // Orchestrator is allowed to be missing from every source (legacy
        // path, tests, pre-startup) — fall back to default behaviour.
        log::debug!(
            "[agent::builder] orchestrator definition not in any registry — using legacy \
             default prompt + filter"
        );
        return Ok(None);
    }

    if registry.is_none() {
        return Err(anyhow::anyhow!(
            "AgentDefinitionRegistry is not initialised — cannot resolve agent '{}'. Call \
             AgentDefinitionRegistry::init_global at startup.",
            agent_id
        ));
    }

    Err(anyhow::anyhow!(
        "agent definition '{}' not found in registry",
        agent_id
    ))
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

/// Which tool-call dialect a session speaks to its provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatcherKind {
    /// Provider-native structured function calling (JSON tool specs on the wire).
    Native,
    /// JSON-in-tag: `<tool_call>{"name":…,"arguments":{…}}</tool_call>` in text.
    Xml,
    /// Compact positional P-Format (`tool[a|b]`) — opt-in only.
    PFormat,
}

/// Pick the tool-call dialect from the configured `agent.tool_dispatcher`
/// choice, the provider's native-tool support, and the agent id.
///
/// `"auto"` (and any unrecognized value) resolves to native when the provider
/// supports it, otherwise JSON-in-tag — **never** P-Format, which is opt-in
/// (`"pformat"`) because its compact positional syntax mis-parses on some
/// models.
///
/// `integrations_agent` is special-cased off native: provider-side grammar
/// decoders (e.g. Fireworks) compile every JSON tool schema into a grammar
/// indexed by a `uint16_t` (max 65 535 rules), and large Composio toolkits
/// (Notion, Salesforce, Gmail) blow past that ceiling, so a native request is
/// rejected with a 400 before any generation. Falling back to JSON-in-tag puts
/// the catalogue in the prompt as prose, so no grammar is compiled.
fn resolve_dispatcher_kind(
    dispatcher_choice: &str,
    supports_native: bool,
    agent_id: &str,
) -> DispatcherKind {
    let base = match dispatcher_choice {
        "native" => DispatcherKind::Native,
        "xml" => DispatcherKind::Xml,
        "pformat" => DispatcherKind::PFormat,
        _ if supports_native => DispatcherKind::Native,
        _ => DispatcherKind::Xml,
    };
    if agent_id == "integrations_agent" && base == DispatcherKind::Native {
        DispatcherKind::Xml
    } else {
        base
    }
}

/// Resolve the provider/workload role for a session build.
///
/// The `subconscious` workload has two entry points and both must route here:
/// - the cloud tick builds via `OpenHumanSessionHost::from_config` (agent_id `"orchestrator"`)
///   with `default_model = "hint:subconscious"`;
/// - the event-driven long-lived session builds via
///   `OpenHumanSessionHost::from_config_for_agent(_, "subconscious")` and does NOT set the hint.
///
/// Routing on `agent_id == "subconscious"` covers the second case (Codex P2:
/// otherwise promoted background turns fall through to `chat_provider` and ignore
/// Connections → API keys → LLM "Subconscious"). Other explicit `hint:<role>` markers route to
/// their workload; everything else (incl. the legacy `default_model` tier the
/// bootstrap pinned) falls through to `chat` so `chat_provider` drives the
/// user-facing turn.
pub(crate) fn provider_role_for(agent_id: &str, default_model: Option<&str>) -> &'static str {
    if agent_id.trim() == "subconscious" {
        return "subconscious";
    }
    match default_model.map(str::trim) {
        Some("hint:agentic") => "agentic",
        Some("hint:coding") => "coding",
        Some("hint:summarization") => "summarization",
        Some("hint:reasoning") => "reasoning",
        Some("hint:subconscious") => "subconscious",
        _ => "chat",
    }
}

#[cfg(test)]
#[path = "factory_provider_role_tests_tests.rs"]
mod provider_role_tests;

/// Resolve the initial harness workload without constructing a session.
/// Flow readiness shares this path so it checks the provider used at runtime.
pub(crate) fn provider_role_for_definition(
    agent_id: &str,
    default_model: Option<&str>,
    target_def: Option<&crate::agent::harness::definition::AgentDefinition>,
) -> &'static str {
    let master_hint = default_model
        .is_none()
        .then(|| {
            target_def.and_then(|def| {
                if def.id == "orchestrator" {
                    match &def.model {
                        crate::agent::harness::definition::ModelSpec::Hint(hint) => {
                            Some(format!("hint:{hint}"))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
        })
        .flatten();
    provider_role_for(agent_id, master_hint.as_deref().or(default_model))
}

fn derive_turn_workspace_descriptor() -> Option<tinytools::WorkspaceDescriptor> {
    let root = crate::agent::turn_workspace::current()?;
    if !root.is_dir() {
        tracing::warn!(
            root = %root.display(),
            "[turn_workspace] scoped root is not an existing directory — \
             falling back to the shared action_dir cwd for this turn"
        );
        return None;
    }
    tracing::debug!(
        root = %root.display(),
        "[turn_workspace] turn bound to the embedder's per-turn root as default cwd"
    );
    Some(tinytools::WorkspaceDescriptor::new(root).with_policy_id("turn-workspace"))
}
