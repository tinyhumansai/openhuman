//! `Agent::from_config` factory methods and the internal
//! `build_session_agent_inner` constructor.

use super::helpers::prefetch_tool_memory_rules_blocking;
use super::should_synthesize_delegation_tools;
use crate::openhuman::agent::context::prompt::SystemPromptBuilder;
use crate::openhuman::agent::dispatcher::{
    NativeToolDispatcher, PFormatToolDispatcher, XmlToolDispatcher,
};
use crate::openhuman::agent::harness::definition::{
    AgentDefinitionRegistry, PromptSource, ToolScope,
};
use crate::openhuman::agent::harness::session::types::Agent;
use crate::openhuman::agent::host_runtime;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider;
use crate::openhuman::memory::agent::memory_loader::DefaultMemoryLoader;
use crate::openhuman::memory::store as memory_store;
use crate::openhuman::memory::tool_memory::ToolMemoryCaptureHook;
use crate::openhuman::memory::Memory;
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
        profile: Option<&crate::openhuman::agent::profiles::AgentProfile>,
    ) -> Result<Self> {
        let target_def = resolve_target_definition(config, agent_id)?;
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
    // `pub(crate)` (rather than private) so `builder_tests` can drive the
    // definition-cap resolution logic (issue #4868) directly with a
    // hand-picked `target_def`, independent of the process-global
    // `AgentDefinitionRegistry` singleton's init-once state. Still not part
    // of the crate's public API.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_session_agent_inner(
        config: &Config,
        agent_id: &str,
        target_def: Option<&crate::openhuman::agent::harness::definition::AgentDefinition>,
        reflection_chunks: Option<Vec<crate::openhuman::subconscious::SourceChunk>>,
        profile_prompt_suffix: Option<String>,
        read_only_tools_only: bool,
        profile: Option<&crate::openhuman::agent::profiles::AgentProfile>,
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

        // Section D — per-profile dedicated workspace. When the active profile
        // opts into `dedicated_workspace` (and its id passes validation), derive
        // a `WorkspaceDescriptor` rooted at `<action_dir>/profiles/<id>` and
        // thread it into the top-level chat turn so acting tools (shell/file/git)
        // resolve their default cwd there. Because the dir is under `action_dir`,
        // `SecurityPolicy` already permits it — no hardening change, and the
        // agent's broad write root is left intact (cross-profile write guarding
        // is a deliberate follow-up). `None` (the common case) preserves the
        // shared-`action_dir` cwd behaviour byte-for-byte.
        //
        // NOTE (deliberate): this `ctx.workspace` descriptor propagates to
        // subagents spawned from this session, so they too root under
        // `<action_dir>/profiles/<id>` rather than the bare `action_dir`. That
        // propagation is *intended* profile isolation, not a leak — a profile's
        // subagents should share its dedicated workspace. Do not "fix" it by
        // clearing the descriptor for child sessions.
        //
        // The expression is extracted into [`derive_profile_workspace_descriptor`]
        // so the unit tests exercise the *same* code path rather than a
        // hand-copied mirror.
        let profile_workspace_descriptor =
            derive_profile_workspace_descriptor(&config.action_dir, profile);

        let runtime: Arc<dyn host_runtime::RuntimeAdapter> = Arc::from(
            host_runtime::create_runtime(&config.runtime, config.shell.hide_window)?,
        );
        // 1b — arm the cross-profile write guard for every active profile,
        // independently of whether that profile uses (or successfully created)
        // a dedicated workspace. A shared/default profile still must not reach
        // another profile's `<action_dir>/profiles/<Q>` subtree from the broad
        // action root. Profile-less sessions remain byte-identical.
        let security = Arc::new(build_profile_security(config, profile));
        // Phase 1 of #1401: see comment in channels/runtime/startup.rs.
        let audit = crate::openhuman::security::get_or_create_workspace_audit_logger(
            crate::openhuman::config::AuditConfig::default(),
            config.workspace_dir.clone(),
        )?;

        let local_embedding = config.workload_local_model("embeddings");
        let embedding_api_key = crate::openhuman::inference::embeddings::resolve_api_key(
            config,
            &config.memory.embedding_provider,
        );
        // Route this session's captures + recall into the active profile's memory
        // subtree so `dedicatedMemory` isolation takes effect on the ordinary
        // session path (web chat, cron), not just delegation preambles. The
        // profile-less / default / shared cases resolve to `"memory"`
        // (byte-identical): `effective_memory_suffix` returns `""` for them and
        // `memory_subdir_for_suffix("")` == `"memory"`. A dedicated-memory profile
        // yields `"memory-<id>"`; a legacy numeric-suffix profile `"memory-<n>"`.
        let memory_subdir = profile
            .map(|p| {
                crate::openhuman::agent::profiles::memory_subdir_for_suffix(
                    &crate::openhuman::agent::profiles::effective_memory_suffix(p),
                )
            })
            .unwrap_or_else(|| "memory".to_string());
        let memory_suffix = profile
            .map(crate::openhuman::agent::profiles::effective_memory_suffix)
            .unwrap_or_default();
        let session_raw_subdir =
            crate::openhuman::agent::profiles::session_raw_subdir_for_suffix(&memory_suffix);
        tracing::debug!(
            memory_subdir = %memory_subdir,
            has_profile = profile.is_some(),
            "[profiles] session memory subtree selected"
        );
        let session_memory = memory_store::factories::create_session_memory_with_local_ai(
            &config.memory,
            local_embedding.as_deref(),
            &embedding_api_key,
            &config.embedding_routes,
            Some(&config.storage.provider.config),
            &config.workspace_dir,
            &memory_subdir,
        )?;
        let archivist_connection = session_memory.sqlite_connection;
        let memory: Arc<dyn Memory> = Arc::from(session_memory.memory);
        // Dedicated profiles still recall unstamped experiences written by
        // pre-profile versions from the shared memory DB. Retain the global
        // shared handle explicitly on the session rather than making the hot
        // turn path reload config or reach into process-global state.
        let shared_experience_memory = if memory_subdir == "memory" {
            None
        } else {
            Some(
                crate::openhuman::memory::global::init(config.workspace_dir.clone())
                    .map_err(anyhow::Error::msg)?
                    .memory_handle(),
            )
        };

        // Per-profile skill (workflow) + MCP-server allowlists. `None` = all.
        let profile_skill_allowlist: Option<std::collections::HashSet<String>> = profile
            .and_then(|p| p.allowed_skills.clone())
            .map(|v| v.into_iter().collect());
        let profile_mcp_allowlist: Option<Vec<String>> =
            profile.and_then(|p| p.allowed_mcp_servers.clone());

        // 2a — profile-local skills root (`<workspace>/personalities/<id>/skills/`).
        // Threaded into the harness workflow catalog AND the discovery/list tools
        // so a turn running under this profile sees its private skills (implicitly
        // allowed for their owner, winning same-name collisions). `None` for the
        // profile-less session / legacy ids keeps discovery byte-identical.
        let profile_skills_root: Option<std::path::PathBuf> = profile.and_then(|p| {
            crate::openhuman::agent::profiles::profile_skills_root(&config.workspace_dir, &p.id)
        });
        if let Some(root) = profile_skills_root.as_deref() {
            tracing::debug!(
                skills_root = %root.display(),
                "[profiles] profile-local skills root active for this session"
            );
        }

        // Load the user's persisted tool preferences once. They drive two
        // things below: granting the App UI Control / App Automation mutation
        // opt-in (#3762) and filtering the tool set to the enabled snapshot.
        let enabled_tools: Vec<String> = {
            use crate::openhuman::desktop::app_state::load_stored_app_state;
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

        let mut tools = tools::all_tools_with_runtime(
            Arc::clone(&tool_config),
            &security,
            runtime,
            audit,
            memory.clone(),
            &tool_config.browser,
            &tool_config.http_request,
            &tool_config.action_dir,
            &tool_config.agents,
            &tool_config,
            profile,
            profile_skill_allowlist.as_ref(),
            profile_mcp_allowlist.as_deref(),
            profile_skills_root.as_deref(),
            profile_workspace_descriptor
                .as_ref()
                .map(|descriptor| descriptor.root.as_path()),
        );

        // Filter tools by the user preference loaded above.
        if !enabled_tools.is_empty() {
            crate::openhuman::tools::filter_tools_by_user_preference(&mut tools, &enabled_tools);
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
        // The ReliableProvider retry/backoff + model-fallback wrapper is
        // re-layered on top of the factory's resolved backend below (issue
        // #4249, 1c). `model_routes` translation and intelligent local/cloud
        // task hinting now live in the unified routing layer (router.rs) rather
        // than a per-session wrapper, so they are not re-wrapped here.
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
        let provider_role = provider_role_for(agent_id, config.default_model.as_deref());
        // Retry/backoff is now owned by the crate `RetryPolicy` at the harness
        // model call (issue #4249, Phase 3a) — see `tinyagents::run_policy_for`.
        // The turn path therefore no longer wraps the resolved provider in
        // `ReliableProvider`; wrapping it here plus the crate retry would
        // double-retry every transient error. Cross-route fallback is likewise
        // the crate registry `FallbackPolicy`, so `config.reliability.*` no longer
        // layers on the turn path (it still governs the non-seam provider paths).
        let (resolved_chat_model, mut model_name) =
            crate::openhuman::inference::provider::create_chat_model_with_model_id(
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
            crate::openhuman::inference::model_context::model_supports_vision(&model_name, config);

        // #5146 §2.1/§2.3: when the active model can't take images the turn
        // engine silently strips them, and the user gets a confident answer
        // about an image the model never saw. Log the actionable reason (which
        // model, and what to switch to) at the moment the decision is made.
        if let Err(reason) =
            crate::openhuman::inference::provider::fallback_diagnostics::vision_preflight(
                &model_name,
                config,
            )
        {
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
                    Box::new(crate::openhuman::agent::context::prompt::UserReflectionsSection),
                )
                .add_section(Box::new(
                    crate::openhuman::agent::learning::LearnedContextSection::new(memory.clone()),
                ))
                .add_section(Box::new(
                    crate::openhuman::agent::learning::UserProfileSection::new(memory.clone()),
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
                crate::openhuman::agent::learning::UserProfileSection::new(memory.clone()),
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
        // Compose the profile prompt section: the persona suffix, plus (1b) the
        // cross-profile workspace notice when a dedicated workspace is active.
        // The notice discloses the boundary the guard enforces, so it is added
        // even when the profile carries no persona suffix.
        let profile_suffix = profile_prompt_suffix
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let workspace_notice = profile_workspace_descriptor
            .as_ref()
            .and_then(|descriptor| {
                profile.map(|p| {
                    crate::openhuman::agent::profiles::cross_profile_workspace_notice(
                        &p.id,
                        &descriptor.root,
                    )
                })
            });
        if profile_suffix.is_some() || workspace_notice.is_some() {
            log::debug!(
                "[agent:builder] profile prompt section injected suffix_chars={} workspace_notice={}",
                profile_suffix.as_deref().map(|s| s.chars().count()).unwrap_or(0),
                workspace_notice.is_some(),
            );
            let mut section = crate::openhuman::agent::profiles::AgentProfilePromptSection::new(
                profile_suffix.unwrap_or_default(),
            );
            if let Some(notice) = workspace_notice {
                section = section.with_workspace_notice(notice);
            }
            prompt_builder = prompt_builder.add_section(Box::new(section));
        }

        // Build post-turn hooks when learning is enabled
        let mut post_turn_hooks: Vec<Arc<dyn crate::openhuman::agent::hooks::PostTurnHook>> =
            Vec::new();
        if config.learning.enabled {
            if config.learning.reflection_enabled {
                // The reflection hook needs an owned `Arc<Config>`; reuse the
                // shared base config (a refcount bump) rather than a second deep
                // clone of the full config (#5050, Fix 1).
                let full_config = Arc::clone(&base_config);
                // For cloud reflection, wrap the provider in an Arc.
                // For local, no provider needed.
                let reflection_provider: Option<
                    Arc<dyn tinyagents::harness::model::ChatModel<()>>,
                > = if config.learning.reflection_source
                    == crate::openhuman::config::ReflectionSource::Cloud
                {
                    let (model, resolved_model) =
                        provider::create_chat_model_with_model_id("reasoning", config, 0.3)?;
                    log::debug!(
                        "[learning] built crate-native reflection model resolved_model={resolved_model}"
                    );
                    Some(model)
                } else {
                    None
                };
                post_turn_hooks.push(Arc::new(
                    crate::openhuman::agent::learning::ReflectionHook::new(
                        config.learning.clone(),
                        full_config.clone(),
                        memory.clone(),
                        reflection_provider,
                    ),
                ));
                log::info!(
                    "[learning] reflection hook registered (source={:?})",
                    config.learning.reflection_source
                );
            }

            if config.learning.user_profile_enabled {
                post_turn_hooks.push(Arc::new(
                    crate::openhuman::agent::learning::UserProfileHook::new(
                        config.learning.clone(),
                        memory.clone(),
                    ),
                ));
                log::info!("[learning] user_profile hook registered");
            }

            if config.learning.tool_tracking_enabled {
                post_turn_hooks.push(Arc::new(
                    crate::openhuman::agent::learning::ToolTrackerHook::new(
                        config.learning.clone(),
                        memory.clone(),
                    ),
                ));
                log::info!("[learning] tool_tracker hook registered");
            }

            if config.learning.tool_memory_capture_enabled {
                post_turn_hooks.push(Arc::new(ToolMemoryCaptureHook::new(memory.clone(), true)));
                log::info!("[learning] tool_memory_capture hook registered");
            }

            if config.learning.tool_memory_capture_enabled {
                // 1c — stamp captured experiences with the active profile id so
                // retrieval can partition them. `None` for the profile-less
                // session leaves records unstamped (shared/legacy).
                post_turn_hooks.push(Arc::new(
                    crate::openhuman::agent::experience::AgentExperienceCaptureHook::with_profile(
                        memory.clone(),
                        true,
                        profile.map(|p| p.id.clone()),
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
        // using the explicit SQLite resource returned by the session factory.
        let archivist_hook_arc: Option<
            Arc<crate::openhuman::agent::harness::archivist::ArchivistHook>,
        > = if config.learning.episodic_capture_enabled {
            let hook = Arc::new(
                crate::openhuman::agent::harness::archivist::ArchivistHook::new(
                    archivist_connection,
                    true,
                )
                .with_config(config.clone()),
            );
            post_turn_hooks
                .push(Arc::clone(&hook) as Arc<dyn crate::openhuman::agent::hooks::PostTurnHook>);
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

        // Best-effort prewarm from the shared Composio cache. This avoids
        // building the session with a knowingly stale `&[]` integration view
        // and then paying a repair pass on turn 1 just to recover the real
        // delegation surface.
        let prewarmed_integrations =
            crate::openhuman::integrations::composio::cached_active_integrations(config);
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
                let filtered =
                    crate::openhuman::agent::profiles::filter_integrations(&list, Some(allow));
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
                    ToolScope::Named(names) => Some(names.iter().cloned().collect()),
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

        // Profile tool selection is a restriction on the resolved agent
        // definition, never a replacement for it. Apply it here at the shared
        // session-builder seam so web chat, cron, tasks, and delegated profile
        // runs all enforce the same callable surface. The web wrapper used to
        // replace this set after construction, which both missed background
        // runs and could broaden a named agent definition.
        if let Some(allowed_tools) = profile
            .and_then(|profile| profile.allowed_tools.as_ref())
            .filter(|tools| !tools.is_empty())
        {
            let profile_visible: std::collections::HashSet<&str> = allowed_tools
                .iter()
                .map(|tool| tool.trim())
                .filter(|tool| !tool.is_empty())
                .collect();
            if visible.is_empty() {
                visible = profile_visible.into_iter().map(str::to_string).collect();
            } else {
                visible.retain(|tool| profile_visible.contains(tool.as_str()));
                // Empty is the Agent's historical "all tools" sentinel. A
                // disjoint profile/definition intersection must instead stay
                // non-empty with an unregistered name so it advertises and
                // permits zero tools rather than accidentally broadening.
                if visible.is_empty() {
                    visible.insert("__profile_no_tools__".to_string());
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
                prompt_builder = prompt_builder.add_section(Box::new(
                    crate::openhuman::agent::learning::MemoryAccessSection,
                ));
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
        let dispatcher_kind =
            resolve_dispatcher_kind(&dispatcher_choice, supports_native, agent_id);
        let tool_dispatcher: Box<dyn crate::openhuman::agent::dispatcher::ToolDispatcher> =
            match dispatcher_kind {
                DispatcherKind::Native => Box::new(NativeToolDispatcher),
                DispatcherKind::Xml => Box::new(XmlToolDispatcher),
                DispatcherKind::PFormat => {
                    Box::new(PFormatToolDispatcher::new(pformat_registry.clone()))
                }
            };

        log::debug!(
            "[agent] tool dispatcher selected: choice={dispatcher_choice} agent_id={agent_id} \
             kind={dispatcher_kind:?} sends_tool_specs={} pformat_registry_entries={}",
            tool_dispatcher.should_send_tool_specs(),
            pformat_registry.len()
        );

        // Temperature override: an active profile is the user-selected runtime
        // default; otherwise use the target definition's TOML value (welcome is
        // 0.7, orchestrator is 0.4, etc). Fall back to config for the legacy
        // no-definition path.
        let effective_temperature = profile
            .and_then(|profile| profile.temperature)
            .or_else(|| target_def.map(|def| def.temperature))
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
            .unwrap_or(crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Full);

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
                dyn crate::openhuman::agent::tinyagents::payload_summarizer::PayloadSummarizer,
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
                            crate::openhuman::agent::tinyagents::payload_summarizer::SubagentPayloadSummarizer::new(
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
        let profile_subagent_tool_ceiling = profile
            .and_then(|profile| profile.allowed_tools.as_ref())
            .filter(|tools| !tools.is_empty())
            .map(|tools| {
                tools
                    .iter()
                    .map(|tool| tool.trim())
                    .filter(|tool| !tool.is_empty())
                    .map(str::to_string)
                    .collect()
            });
        let mut builder = Agent::builder()
            .crate_native_provider(provider_role, Arc::clone(&base_config))
            .tools(tools)
            .visible_tool_names(visible)
            .memory(memory)
            .shared_experience_memory(shared_experience_memory)
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
                        profile.is_none_or(|p| p.include_agent_conversations),
                    ),
            ))
            .prompt_builder(prompt_builder)
            .config(effective_agent_config)
            .context_config(config.context.clone())
            .model_name(model_name)
            .model_vision(model_vision)
            .temperature(effective_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .action_dir(config.action_dir.clone())
            .workspace_descriptor(profile_workspace_descriptor)
            // 1a — carry the active profile id (any active profile, not just
            // dedicated-workspace ones) so profile-scoped post-turn hooks can
            // see which profile the turn ran under. `None` for the profile-less
            // session keeps every consumer byte-identical.
            .active_profile_id(profile.map(|p| p.id.clone()))
            .personality_soul_md(profile.and_then(|profile| {
                crate::openhuman::agent::profiles::resolve_personality_soul(
                    &config.workspace_dir,
                    profile,
                )
            }))
            .personality_memory_md(profile.and_then(|profile| {
                crate::openhuman::agent::profiles::resolve_personality_memory_md(
                    &config.workspace_dir,
                    profile,
                )
            }))
            .profile_memory_storage(memory_subdir, session_raw_subdir)
            .workflows(
                crate::openhuman::skills::load_workflow_metadata_for_profile(
                    &config.workspace_dir,
                    profile_skills_root.as_deref(),
                ),
            )
            .auto_save(config.memory.auto_save)
            .post_turn_hooks(post_turn_hooks)
            .learning_enabled(config.learning.enabled)
            .explicit_preferences_enabled(config.learning.explicit_preferences_enabled)
            .agent_definition_name(agent_id.to_string())
            .omit_profile(effective_omit_profile)
            .omit_memory_md(effective_omit_memory_md)
            .trigger_memory_agent(effective_trigger_memory_agent)
            .tokenjuice_compression(effective_tokenjuice_compression);
        if let Some(ceiling) = profile_subagent_tool_ceiling {
            builder = builder.subagent_tool_ceiling_names(ceiling);
        }
        if let Some(ps) = payload_summarizer {
            builder = builder.payload_summarizer(ps);
        }
        builder = builder.archivist_hook(archivist_hook_arc);
        let mut agent = builder.build()?;
        let connected_integrations_initialized = prewarmed_integrations.is_some();
        agent.connected_integrations = prewarmed_integrations.unwrap_or_default();
        agent.connected_integrations_initialized = connected_integrations_initialized;
        agent.runtime_config = Some(Arc::new(config.clone()));
        agent.last_seen_integrations_hash =
            crate::openhuman::integrations::composio::connected_set_hash(
                &agent.connected_integrations,
            );
        agent.synthesized_tool_names = synthesized_tool_names;
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
) -> Result<Option<crate::openhuman::agent::harness::definition::AgentDefinition>> {
    let registry = AgentDefinitionRegistry::global();

    if let Some(reg) = registry {
        if let Some(def) = reg.get(agent_id) {
            return Ok(Some(def.clone()));
        }
    }

    // Harness registry miss (or not yet initialised). Before failing, check
    // the config-backed custom agent registry — the one place custom
    // (non-shipped) agents live.
    if let Some(entry) = crate::openhuman::agent::registry::find_custom_in_config(config, agent_id)
    {
        log::info!(
            "[agent::builder] agent_id={} not found in the harness AgentDefinitionRegistry — \
             synthesizing a definition from its custom agent_registry entry so it runs with its \
             real tool belt instead of persona-only / erroring",
            agent_id
        );
        return Ok(Some(
            crate::openhuman::agent::registry::definition_from_registry_entry(&entry),
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
/// - the cloud tick builds via `Agent::from_config` (agent_id `"orchestrator"`)
///   with `default_model = "hint:subconscious"`;
/// - the event-driven long-lived session builds via
///   `Agent::from_config_for_agent(_, "subconscious")` and does NOT set the hint.
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
mod provider_role_tests {
    use super::provider_role_for;
    use super::{resolve_dispatcher_kind, DispatcherKind};

    #[test]
    fn orchestrator_defaults_to_chat() {
        assert_eq!(provider_role_for("orchestrator", Some("chat-v1")), "chat");
        assert_eq!(provider_role_for("orchestrator", None), "chat");
        // A legacy heavy default_model tier still falls through to chat.
        assert_eq!(
            provider_role_for("orchestrator", Some("reasoning-v1")),
            "chat"
        );
    }

    #[test]
    fn explicit_hints_route_to_workload() {
        assert_eq!(
            provider_role_for("orchestrator", Some("hint:agentic")),
            "agentic"
        );
        assert_eq!(
            provider_role_for("orchestrator", Some("hint:reasoning")),
            "reasoning"
        );
        // The cloud tick: orchestrator agent_id + the subconscious hint.
        assert_eq!(
            provider_role_for("orchestrator", Some("hint:subconscious")),
            "subconscious"
        );
    }

    #[test]
    fn subconscious_agent_id_routes_to_subconscious_without_hint() {
        // The event-driven long-lived session builds with agent_id="subconscious"
        // and no hint — it must still resolve the subconscious workload (Codex P2).
        assert_eq!(provider_role_for("subconscious", None), "subconscious");
        assert_eq!(
            provider_role_for("subconscious", Some("chat-v1")),
            "subconscious"
        );
        assert_eq!(provider_role_for(" subconscious ", None), "subconscious");
    }

    #[test]
    fn auto_prefers_native_when_supported_never_pformat() {
        assert_eq!(
            resolve_dispatcher_kind("auto", true, "chat"),
            DispatcherKind::Native
        );
        // Text-only provider defaults to JSON-in-tag, NOT P-Format.
        assert_eq!(
            resolve_dispatcher_kind("auto", false, "chat"),
            DispatcherKind::Xml
        );
        // An unrecognized value behaves like "auto".
        assert_eq!(
            resolve_dispatcher_kind("bogus", false, "chat"),
            DispatcherKind::Xml
        );
    }

    #[test]
    fn explicit_choices_are_honoured_including_opt_in_pformat() {
        assert_eq!(
            resolve_dispatcher_kind("native", false, "chat"),
            DispatcherKind::Native
        );
        assert_eq!(
            resolve_dispatcher_kind("xml", true, "chat"),
            DispatcherKind::Xml
        );
        // P-Format is only ever selected when explicitly requested.
        assert_eq!(
            resolve_dispatcher_kind("pformat", true, "chat"),
            DispatcherKind::PFormat
        );
    }

    #[test]
    fn integrations_agent_falls_off_native_to_json_in_tag() {
        // Native would ship JSON tool specs and blow the provider grammar-rule
        // ceiling on large Composio toolkits → force JSON-in-tag.
        assert_eq!(
            resolve_dispatcher_kind("auto", true, "integrations_agent"),
            DispatcherKind::Xml
        );
        assert_eq!(
            resolve_dispatcher_kind("native", true, "integrations_agent"),
            DispatcherKind::Xml
        );
        // An explicit non-native choice is left untouched for that agent.
        assert_eq!(
            resolve_dispatcher_kind("pformat", true, "integrations_agent"),
            DispatcherKind::PFormat
        );
    }
}

/// Section D — derive the top-level chat turn's per-profile workspace
/// descriptor. Shared by [`Agent::build_session_agent_inner`] and its unit tests
/// so the two can never drift.
///
/// Returns a [`WorkspaceDescriptor`](tinyagents::harness::workspace::WorkspaceDescriptor)
/// rooted at `<action_dir>/profiles/<id>` when `profile` opts into
/// `dedicated_workspace` and its id passes validation (via
/// [`dedicated_workspace_dir`](crate::openhuman::agent::profiles::dedicated_workspace_dir)),
/// creating the dir as a side effect; `None` for the shared-workspace common case,
/// for legacy ids that fail validation, and when the directory can't be created
/// (all three fall back to the shared `action_dir` cwd rather than binding tools
/// to a nonexistent dir). The returned descriptor propagates to subagents — see
/// the deliberate-isolation note at the call site.
pub(crate) fn derive_profile_workspace_descriptor(
    action_dir: &std::path::Path,
    profile: Option<&crate::openhuman::agent::profiles::AgentProfile>,
) -> Option<tinyagents::harness::workspace::WorkspaceDescriptor> {
    let (profile_id, dir) = profile.and_then(|p| {
        crate::openhuman::agent::profiles::dedicated_workspace_dir(action_dir, p)
            .map(|dir| (p.id.clone(), dir))
    })?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            profile_id = %profile_id,
            dir = %dir.display(),
            error = %e,
            "[profiles] failed to create dedicated workspace dir — \
             falling back to the shared action_dir cwd for this session"
        );
        // Return None so callers fall back to the shared action_dir rather than
        // binding every acting tool (shell/file/git) to a cwd that doesn't exist.
        return None;
    }
    tracing::debug!(
        profile_id = %profile_id,
        dir = %dir.display(),
        "[profiles] session bound to dedicated workspace as default cwd"
    );
    Some(
        tinyagents::harness::workspace::WorkspaceDescriptor::new(dir).with_policy_id(
            crate::openhuman::agent::profiles::workspace_policy_id(&profile_id),
        ),
    )
}

fn build_profile_security(
    config: &crate::openhuman::config::Config,
    profile: Option<&crate::openhuman::agent::profiles::AgentProfile>,
) -> SecurityPolicy {
    let base =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    match profile {
        Some(profile) => base.with_active_profile(profile.id.clone(), config.action_dir.clone()),
        None => base,
    }
}

/// Section D — per-profile dedicated-workspace descriptor seam.
///
/// These tests exercise the **production** [`derive_profile_workspace_descriptor`]
/// directly (the same function the session builder calls), so they cannot drift
/// from the real seam. They pin that the descriptor root points at
/// `<action_dir>/profiles/<id>` for an opted-in profile, and that shared/legacy
/// profiles produce no descriptor (so the shared `action_dir` cwd is preserved).
#[cfg(test)]
mod profile_workspace_descriptor_tests {
    use super::{build_profile_security, derive_profile_workspace_descriptor};
    use crate::openhuman::agent::profiles::store::built_in_default_profile;

    fn profile(
        id: &str,
        dedicated_workspace: bool,
    ) -> crate::openhuman::agent::profiles::AgentProfile {
        let mut p = built_in_default_profile();
        p.id = id.to_string();
        p.name = id.to_string();
        p.built_in = false;
        p.is_master = false;
        p.memory_dir_suffix = None;
        p.dedicated_workspace = dedicated_workspace;
        p
    }

    #[test]
    fn dedicated_workspace_profile_roots_descriptor_at_profile_dir() {
        // Real temp action_dir: the production fn creates the profile dir as a
        // side effect, so assert against the resolved path suffix.
        let action = tempfile::tempdir().expect("action tempdir");
        let p = profile("alice", true);
        let desc = derive_profile_workspace_descriptor(action.path(), Some(&p))
            .expect("dedicated_workspace profile yields a descriptor");
        let expected = action.path().join("profiles").join("alice");
        assert_eq!(desc.root.as_path(), expected.as_path());
        // The production path really created the dir.
        assert!(desc.root.is_dir());
    }

    #[test]
    fn shared_profile_yields_no_descriptor() {
        let action = tempfile::tempdir().expect("action tempdir");
        let p = profile("bob", false);
        assert!(derive_profile_workspace_descriptor(action.path(), Some(&p)).is_none());
    }

    #[test]
    fn none_profile_yields_no_descriptor() {
        let action = tempfile::tempdir().expect("action tempdir");
        assert!(derive_profile_workspace_descriptor(action.path(), None).is_none());
    }

    #[test]
    fn legacy_invalid_id_yields_no_descriptor_even_when_opted_in() {
        let action = tempfile::tempdir().expect("action tempdir");
        // An id that fails validation can't mint a workspace path → no descriptor,
        // so the session falls back to the shared action_dir cwd.
        let p = profile("Bad Id", true);
        assert!(derive_profile_workspace_descriptor(action.path(), Some(&p)).is_none());
    }

    #[test]
    fn create_dir_failure_yields_no_descriptor() {
        // Point `action_dir` at a regular file so `profiles/…` can't be created.
        // The function must fall back to `None` (shared action_dir cwd) rather
        // than hand tools a descriptor rooted at a nonexistent dir.
        let tmp = tempfile::tempdir().expect("tempdir");
        let action_file = tmp.path().join("not-a-dir");
        std::fs::write(&action_file, b"x").expect("write file");
        let p = profile("alice", true);
        assert!(
            derive_profile_workspace_descriptor(&action_file, Some(&p)).is_none(),
            "a create_dir_all failure must fall back to None"
        );
    }

    #[test]
    fn shared_profile_still_arms_cross_profile_guard() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = crate::openhuman::config::Config::default();
        config.action_dir = temp.path().join("actions");
        config.workspace_dir = temp.path().join("state");
        let profile = profile("default", false);

        let security = build_profile_security(&config, Some(&profile));

        let guard = security
            .active_profile
            .expect("every active profile must arm the guard");
        assert_eq!(guard.profile_id, "default");
        assert_eq!(guard.action_dir, config.action_dir);
    }

    #[test]
    fn profile_less_session_leaves_cross_profile_guard_disarmed() {
        let config = crate::openhuman::config::Config::default();
        assert!(build_profile_security(&config, None)
            .active_profile
            .is_none());
    }
}
