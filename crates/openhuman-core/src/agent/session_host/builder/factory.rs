//! `OpenHumanSessionHost::from_config` factory methods and the internal
//! `build_session_agent_inner` constructor.

use super::dispatcher::{resolve_dispatcher_kind, DispatcherKind};
use super::should_synthesize_delegation_tools;
use crate::agent::harness::definition::NO_TOOLS_SENTINEL;
use crate::agent::harness::definition::{AgentDefinitionRegistry, PromptSource, ToolScope};
use crate::agent::host_runtime;
use crate::agent::prompts::SystemPromptBuilder;
use crate::agent::session_host::types::OpenHumanSessionHost;
use crate::config::Config;
use crate::security::SecurityPolicy;
use crate::tools;
use anyhow::Result;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool};
use tinytools_agent::dialect::{
    CodeDialect, NativeDialect, PFormatDialect, ToolDialect, XmlDialect,
};

impl OpenHumanSessionHost {
    /// Returns whether `agent_id` resolves to a runnable definition for this
    /// configuration. This is deliberately the same resolution path used by
    /// [`Self::from_config_for_agent`], so configuration writers cannot save
    /// a web-chat route that the session factory would later reject.
    pub(crate) fn is_runnable_agent_id(config: &Config, agent_id: &str) -> bool {
        resolve_target_definition(config, agent_id).is_ok()
    }

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
    /// 3. Registering all built-in and orchestrator tools (the `memory` tool
    ///    only while memory is on).
    /// 4. Configuring the routed AI provider.
    /// 5. Setting up post-turn hooks.
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
             (scope={}, omit_identity={}, omit_memory_context={}, temperature={:.2})",
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
            target_def
                .as_ref()
                .map(|d| d.omit_memory_context)
                .unwrap_or(false),
            target_def
                .as_ref()
                .map(|d| d.temperature)
                .unwrap_or(config.default_temperature)
        );

        Self::build_session_agent_inner(config, agent_id, target_def.as_ref(), false, None, None)
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
        Self::build_session_agent_inner(config, &definition.id, Some(definition), false, None, None)
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
        host: Option<&super::HostTools>,
        session_id: Option<&str>,
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

        // Load the user's persisted tool preferences once. They drive two
        // things below: granting the App UI Control / App Automation mutation
        // opt-in (#3762) and filtering the tool set to the enabled snapshot.
        let mut enabled_tools: Vec<String> = {
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
        if config.browser.enabled && !enabled_tools.is_empty() {
            // Browser's explicit opt-in outranks a positive-only onboarding snapshot.
            enabled_tools.extend(["browser".to_string(), "browser_open".to_string()]);
        }
        // Share a single `Arc<Config>` across the heavyweight per-build consumers
        // (the tool registry, the turn provider) instead of
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
        // "hint:reasoning" tier name to Anthropic would 404).
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
        // workload — legacy tier literals like `hint:reasoning` (which the
        // bootstrap historically pinned as `default_model` for everyone)
        // fall through to `chat`. This preserves `config.chat_provider` for
        // legacy callers. A built-in Master OpenHumanSessionHost has an explicit
        // `hint:coding`, while existing pre-registry callers continue using
        // `config.default_model` unchanged.
        let provider_role =
            provider_role_for_definition(config.default_model.as_deref(), target_def);
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
        let prompt_builder = match target_def {
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
        let post_turn_hooks: Vec<Arc<dyn crate::agent::hooks::PostTurnHook>> =
            crate::agent::hooks::embedder_post_turn_hooks();

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
        // `ArchetypeDelegationTool` per named sub-agent plus, for the
        // `{ skills = "*" }` wildcard, one `Deferred` action tool per
        // connected Composio action (reached through `tool_search`).
        //
        // For an agent without `subagents` (today: welcome, critic,
        // summarizer, etc.), no delegation tools are synthesised — the
        // LLM only sees the agent's own `ToolScope::Named` entries
        // from the global registry, narrowed by the visible-tool
        // filter.
        //
        // This builder is synchronous and sits on the CLI / REPL /
        // Tauri-web code path. It still opportunistically reuses the
        // process-wide Composio cache when one is already warm, which
        // lets the session start with the right integration action
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
                        //
                        // A synthesised `Deferred` tool (a per-action
                        // integration tool) is likewise not advertised: it is
                        // reachable through `tool_search` when the belt opts
                        // in, and the session builder keeps it in the deferred
                        // set beside the visible one.
                        for t in &synthed {
                            if t.exposure() != tinytools::ToolExposure::Direct {
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
        // `⟦tj:…⟧` marker actionable for Named-scope agents
        // (e.g. the orchestrator's curated list). An empty set already means
        // "no filter", so it needs nothing. Added BEFORE the disallow filter
        // below so an agent that explicitly disallows it still has it removed.
        // A summary names the tool in its footer too, and summaries run with
        // the router off, so either one makes the tool necessary.
        super::ensure_tinyjuice_tools_visible(&mut visible, agent_id, config);

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

        // The delegation tools stay beside the durable registry rather than
        // inside it: the builder holds them in `OpenHumanSessionHost::synthesized_tools`,
        // drops any name a durable tool already owns, and
        // `refresh_delegation_tools` replaces the whole set later (#6145).

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
        let dispatcher_kind = resolve_dispatcher_kind(&dispatcher_choice, supports_native);
        let tool_dispatcher: Box<dyn ToolDialect> = match dispatcher_kind {
            DispatcherKind::Native => Box::new(NativeDialect),
            DispatcherKind::Xml => Box::new(XmlDialect),
            DispatcherKind::PFormat => Box::new(PFormatDialect::new(pformat_registry.clone())),
            DispatcherKind::Code(style) => {
                Box::new(CodeDialect::new(style, pformat_registry.clone()))
            }
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

        // Whether a new session gets `context.md` prepended to its first user
        // message; agents without a definition get it.
        let effective_omit_memory_context = target_def
            .map(|def| def.omit_memory_context)
            .unwrap_or(false);
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
        // are spawned through `subagent_host` which constructs its
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
        // by TinyJuice's summary stage before entering the orchestrator's
        // history. TinyJuice owns the prompt and the thresholds (installed
        // from [`ContextConfig`]); the host supplies only the model call,
        // through a `SubagentPayloadSummarizer` built from the `summarizer`
        // agent definition. Every other agent id gets
        // `None` and their tool results stay untouched (the summarizer
        // itself MUST be `None` to avoid recursive self-summarization).
        let payload_summarizer: Option<
            std::sync::Arc<dyn crate::agent::tinyagents::payload_summarizer::PayloadSummarizer>,
        > = if super::summarizes_tool_output(agent_id, config) {
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
        // Resolve the per-turn iteration cap in one place so an explicit
        // operator override takes precedence over the selected definition,
        // while definitions still take precedence over the global default.
        let mut effective_agent_config = config.agent.clone();
        let effective_cap =
            super::iteration_cap::resolve_max_tool_iterations(&config.agent, target_def);
        if let Some(def) = target_def {
            log::info!(
                "[agent::builder] resolved iteration cap for agent_id={}: \
                 definition.max_iterations={} iteration_policy={:?} -> effective={} \
                 (global default {}, explicit override {:?})",
                agent_id,
                def.max_iterations,
                def.iteration_policy,
                effective_cap,
                config.agent.max_tool_iterations,
                config.agent.max_tool_iterations_override,
            );
        }
        effective_agent_config.max_tool_iterations = effective_cap;
        let merged_host_tools = super::host_tools::merge_for_turn(
            host,
            agent_id,
            session_id,
            &mut tools,
            &mut visible,
        )?;
        let session_definition = super::host_tools::scope_def(target_def, &merged_host_tools);
        let host_policy = merged_host_tools.policy;
        let withheld_tool_names = merged_host_tools.withheld;
        let mut builder = OpenHumanSessionHost::builder()
            .crate_native_provider(provider_role, Arc::clone(&base_config))
            .tools(tools)
            .synthesized_tools(delegation_tools)
            .visible_tool_names(visible)
            .withheld_tool_names(withheld_tool_names)
            .permanent_tool_names(merged_host_tools.permanent)
            .deferred_tools(target_def.map_or_else(Vec::new, |d| d.deferred_tools.clone()))
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
            .post_turn_hooks(post_turn_hooks)
            .agent_definition_name(agent_id.to_string())
            .omit_memory_context(effective_omit_memory_context)
            .tokenjuice_compression(effective_tokenjuice_compression);
        if let Some(ps) = payload_summarizer {
            builder = builder.payload_summarizer(ps);
        }
        // A host gate REPLACES the session's rather than fronting it --
        // `tool_policy` assigns. `HostTurnTools::with_policy` says why, and
        // what it costs a host that gates only its own names.
        if let Some(policy) = host_policy {
            builder = builder.tool_policy(policy);
        }
        // Before `build()` so the spawn enum honours a saved `subagents` override
        // (#6934); re-shared after it, as `build` keeps its own copy (#6218).
        builder = builder.runtime_config(Arc::clone(&base_config));
        let mut agent = builder.build()?;
        let connected_integrations_initialized = prewarmed_integrations.is_some();
        agent.connected_integrations = prewarmed_integrations.unwrap_or_default();
        agent.connected_integrations_initialized = connected_integrations_initialized;
        agent.runtime_config = Some(Arc::clone(&base_config));
        agent.hosted_base = AgentDefinitionRegistry::global_arc().map(|definitions| {
            Arc::new(crate::agent::tinyagents::host::OpenHumanHostBase {
                config: Arc::clone(&base_config),
                definitions,
                security_policy: security,
                post_turn_hooks: agent.post_turn_hooks.clone(),
                // The caller's own definition, when this session was built from
                // one rather than from a registry id. `AgentSpec::into_core`
                // re-stamps the built-in orchestrator under the caller's id, so
                // ids like `harness`/`alpha`/`beta` reach hosted resolution as
                // names no registry holds; without handing the definition over
                // here the lookup misses and the turn is rejected as a policy
                // failure before any provider call (#6404/#6392/#6393).
                session_definition: session_definition.clone().map(Arc::new),
            })
        });
        if agent.hosted_base.is_none() {
            tracing::warn!(
                "[tinyagents] hosted invocation base unavailable: agent definition registry was not initialized"
            );
        }
        agent.definition = session_definition.map(Arc::new);
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
///    (chat) or silently ran tool-less/persona-only (flows'
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

/// Resolve the provider/workload role for a session build.
///
/// Explicit `hint:<role>` markers route to their workload; everything else
/// (incl. the legacy `default_model` tier the bootstrap pinned) falls through
/// to `chat` so `chat_provider` drives the user-facing turn.
pub(crate) fn provider_role_for(default_model: Option<&str>) -> &'static str {
    match default_model.map(str::trim) {
        Some("hint:agentic") => "agentic",
        Some("hint:coding") => "coding",
        Some("hint:summarization") => "summarization",
        Some("hint:reasoning") => "reasoning",
        _ => "chat",
    }
}

#[cfg(test)]
#[path = "factory_provider_role_tests_tests.rs"]
mod provider_role_tests;

/// Resolve the initial harness workload without constructing a session.
/// Flow readiness shares this path so it checks the provider used at runtime.
pub(crate) fn provider_role_for_definition(
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
    provider_role_for(master_hint.as_deref().or(default_model))
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
