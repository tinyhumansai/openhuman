use super::*;

#[path = "build_helpers.rs"]
mod helpers;

impl OpenHumanSessionHost {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn build_session_agent_inner_with_turn_model_source(
        ctx: &DispatchCtx,
        config: &Config,
        agent_id: &str,
        target_def: Option<&crate::agent_registry::host_definition::AgentDefinition>,
        host_only: bool,
        host: Option<&super::super::HostTools>,
        session_id: Option<&str>,
        turn_model_source: Option<(crate::agent::tinyagents::TurnModelSource, String)>,
    ) -> Result<Self> {
        let workspace_descriptor: Option<tinytools::WorkspaceDescriptor> = None;

        let runtime: Arc<dyn host_runtime::RuntimeAdapter> = Arc::from(
            host_runtime::create_runtime(&config.runtime, config.shell.hide_window)?,
        );
        let security = Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        ));
        let audit = crate::security::get_or_create_workspace_audit_sink(
            config.audit.clone(),
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

        // Host-only (`host_only.rs`): no config-derived tool is even built.
        let (mut tools, pack_registry_handle) =
            tool_registry::make_tool_registry(host_only, || {
                tools::ops::all_tools_with_runtime_and_pack_handle(
                    Arc::clone(&tool_config),
                    &security,
                    runtime,
                    audit,
                    &tool_config.browser,
                    &tool_config.http_request,
                    &tool_config.action_dir,
                    &tool_config,
                    workspace_descriptor
                        .as_ref()
                        .map(|descriptor| descriptor.root.as_path()),
                )
            })
            .await;

        // Filter tools by the user preference loaded above.
        if !enabled_tools.is_empty() {
            crate::tools::filter_tools_by_user_preference(&mut tools, &enabled_tools);
        }

        let (provider_role, model_name, supports_native) =
            super::super::factory_resolution::resolve_session_model(
                ctx,
                config,
                agent_id,
                target_def,
                turn_model_source.as_ref(),
            )
            .await?;
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
        // definition's archetype body and respects its `omit_*` flags.
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
        // builder against the live [`PromptContext`] at `build_system_prompt`
        // time, so late integrations and the flow archetype land in the prompt.
        // `Inline`/`File` sources still resolve to just the archetype
        // body and get wrapped by [`SystemPromptBuilder::for_subagent`].
        let prompt_builder = helpers::prompt_builder(target_def, config)?;

        let post_turn_hooks: Vec<Arc<dyn tinyagents_harness::host::LearningSink>> =
            crate::agent::hooks::embedder_post_turn_hooks_in(ctx);

        // Best-effort prewarm from the shared Composio cache. This avoids
        // building the session with a knowingly stale `&[]` integration view
        // and then paying a repair pass on turn 1 just to recover the real
        // delegation surface.
        let prewarmed_integrations =
            crate::integrations::composio::cached_active_integrations(config).await;
        let prewarmed_integrations_slice = prewarmed_integrations.as_deref().unwrap_or(&[]);

        // Resolve the per-agent deferred integration actions and visible-tool
        // whitelist from the target definition (when we have one) or fall
        // back to the orchestrator's action-expansion path.
        //
        // For an agent with `[subagents] allowlist = [...]` in its TOML (today:
        // orchestrator), `collect_orchestrator_tools` returns only `Deferred`
        // connected Composio actions from the skills wildcard. Named
        // specialists are routed through the durable `spawn_subagent` tool.
        //
        // For an agent without `subagents` (today: welcome, critic,
        // summarizer, etc.), no deferred integration actions are synthesized — the
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
            crate::agent_registry::host_definition::AgentDefinitionRegistry::current(),
        ) {
            // Host-only: no delegation; the host's names join an empty belt.
            _ if host_only => (Vec::new(), Some(super::super::host_policy::empty_belt())),
            (Some(def), Some(_reg)) => {
                let synthed = if should_synthesize_delegation_tools(def) {
                    crate::integrations::composio::orchestrator_actions::collect_orchestrator_tools(
                        def,
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
                        // reports `ToolExposure::Hidden` stays off the wire.
                        // Inserting it here would defeat the exposure policy and
                        // duplicate the route through a second surface.
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
                    Some(orch_def) => crate::integrations::composio::orchestrator_actions::collect_orchestrator_tools(
                        orch_def,
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
                // ever consulting `AgentDefinitionRegistry::current()`, see
                // `agent_registry::find_custom_in_config`). Integration
                // expansion is skipped here, but `def.tools` is a
                // real scope the caller authored and MUST still gate
                // visibility — silently dropping it into the `(_, None)`
                // "no registry, no filter" catch-all would leave a custom
                // agent's `ToolScope::Named` allowlist entirely
                // unenforced (visible tools empty rather than the named
                // set), regressing the least-privilege contract this
                // synthesis path exists to provide.
                log::debug!(
                    "[agent::builder] AgentDefinitionRegistry not initialised — skipping \
                     integration action expansion, but still applying target definition's own \
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
        if !host_only {
            super::super::ensure_tinyjuice_tools_visible(&mut visible, agent_id, config);
        }

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
                            .filter(|name| {
                                !crate::tools::rules::glob_list_matches(&def.disallowed_tools, name)
                            })
                            .collect();
                    }
                    ToolScope::Named(_) => {
                        visible.retain(|name| {
                            !crate::tools::rules::glob_list_matches(&def.disallowed_tools, name)
                        });
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

        let tool_dispatcher = helpers::tool_dispatcher(
            &tools,
            &delegation_tools,
            &dispatcher_choice,
            supports_native,
            agent_id,
        );

        let (
            effective_temperature,
            effective_omit_memory_context,
            effective_tokenjuice_compression,
        ) = helpers::effective_settings(target_def, config);

        helpers::log_agent_definition_name(agent_id);

        let payload_summarizer = helpers::payload_summarizer(host_only, agent_id, config);

        // Crate-native turn models (Phase 3 P3-B): the production main-turn agent
        // builds crate `ChatModel`s from `(provider_role, config)` without retaining
        // a host provider. The `agent_harness_e2e` mock now serves SSE for streaming,
        // so the crate-native streaming path is exercised end-to-end.
        //
        let effective_agent_config = helpers::effective_agent_config(config, target_def, agent_id);
        let merged_host_tools = super::super::host_tools::merge_for_turn(
            ctx,
            host,
            agent_id,
            session_id,
            &mut tools,
            &mut visible,
        )
        .await?;
        let session_definition =
            super::super::host_tools::scope_def(target_def, &merged_host_tools);
        let host_policy =
            super::super::host_policy::session_policy(host_only, &tools, merged_host_tools.policy);
        let withheld_tool_names = merged_host_tools.withheld;
        let mut builder = OpenHumanSessionHost::builder()
            .tools(tools)
            .pack_registry_handle(pack_registry_handle)
            .synthesized_tools(delegation_tools)
            .visible_tool_names(visible)
            .withheld_tool_names(withheld_tool_names)
            .permanent_tool_names(merged_host_tools.permanent)
            .deferred_tools(target_def.map_or_else(Vec::new, |d| d.deferred_tools.clone()))
            .tool_dispatcher(tool_dispatcher)
            .prompt_builder(prompt_builder)
            .config(effective_agent_config)
            .context_config(config.context.harness.clone())
            .model_name(model_name)
            .model_vision(model_vision)
            .temperature(effective_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .action_dir(config.action_dir.clone())
            .workspace_descriptor(workspace_descriptor)
            .workflows(workflows::session_workflows(ctx, config, host_only))
            .post_turn_hooks(post_turn_hooks)
            .agent_definition_name(agent_id.to_string())
            .omit_memory_context(effective_omit_memory_context)
            .tokenjuice_compression(effective_tokenjuice_compression);
        builder = match turn_model_source {
            Some((source, _)) => builder.turn_model_source(source),
            None => builder.crate_native_provider(ctx, provider_role, Arc::clone(&base_config)),
        };
        if let Some(ps) = payload_summarizer {
            builder = builder.payload_summarizer(ps);
        }
        // A host gate REPLACES the session's rather than fronting it --
        // `tool_policy` assigns. `HostTurnTools::with_policy` says why, and
        // what it costs a host that gates only its own names.
        if let Some(policy) = host_policy {
            builder = builder.tool_policy(policy);
        }
        let mut agent = builder.build()?;
        let connected_integrations_initialized = prewarmed_integrations.is_some();
        agent.connected_integrations = prewarmed_integrations.unwrap_or_default();
        agent.connected_integrations_initialized = connected_integrations_initialized;
        // The same snapshot `base_config` already holds — `Config` is immutable
        // after construction, so a second deep clone bought nothing but a
        // second resident copy of a 95-field struct with nested `Vec`s
        // (openhuman#6218).
        agent.runtime_config = Some(Arc::clone(&base_config));
        agent.hosted_base = AgentDefinitionRegistry::current().map(|definitions| {
            Arc::new(crate::agent::tinyagents::host::OpenHumanHostBase {
                config: Arc::clone(&base_config),
                definitions,
                security_policy: security,
                session_definition: session_definition.clone().map(|definition| {
                    Arc::new(super::super::host_tools::add_permanent_tools(
                        definition,
                        &agent.permanent_tool_names,
                    ))
                }),
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
