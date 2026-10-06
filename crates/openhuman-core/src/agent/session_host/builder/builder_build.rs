//! `SessionHostBuilder::build` — validates and assembles the final [`OpenHumanSessionHost`].

use super::{dedup_visible_tool_specs, visible_tool_specs_for_policy};
use crate::agent::context::ContextManager;
use crate::agent::session_host::types::{OpenHumanSessionHost, SessionHostBuilder};
use crate::tools::agent_policy::ToolPolicyEngine;
use anyhow::Result;
use std::sync::Arc;
use tinytools::{Tool, ToolSpec};

impl SessionHostBuilder {
    /// Validates the configuration and constructs a new `OpenHumanSessionHost` instance.
    ///
    /// This method is responsible for wiring together the provided components,
    /// setting up the context manager, and initializing the conversation history.
    /// It ensures that all required fields (provider, tools, dispatcher, …) are present.
    pub fn build(self) -> Result<OpenHumanSessionHost> {
        let mut tools = self
            .tools
            .ok_or_else(|| anyhow::anyhow!("tools are required"))?;
        // Advertise only the spawnable ids on the wire: see
        // `SpawnAsyncSubagentTool::scoped` for why the spec-view narrowing
        // alone did not reach a native-tool-calling provider.
        let spawn_scope_id = self
            .session_definition
            .as_deref()
            .map(|definition| definition.id.clone())
            .or_else(|| self.agent_definition_name.clone());
        if let Some(agent_id) = spawn_scope_id {
            if let Some(allowed) = super::allowed_subagent_ids_for(
                agent_id.trim(),
                self.runtime_config.as_deref(),
                self.session_definition.as_deref(),
            ) {
                if let Some(slot) = tools
                    .iter_mut()
                    .find(|tool| tool.name() == "spawn_async_subagent")
                {
                    tracing::debug!(agent = %agent_id, ids = allowed.len(), "[tools] scoping spawn_async_subagent schema");
                    *slot = Box::new(
                        crate::agent::orchestration::tools::SpawnAsyncSubagentTool::scoped(allowed),
                    );
                }
            }
        }
        // The synthesised set lives beside the durable registry, never inside
        // it (`OpenHumanSessionHost::synthesized_tools`); a durable name wins a collision.
        let synthesized_tools = self.synthesized_tools.unwrap_or_default();
        super::super::managed_tools::reject_synthesized_collisions(
            &self.permanent_tool_names,
            &synthesized_tools,
        )?;
        let synthesized_tools = super::drop_synthesized_name_collisions(&tools, synthesized_tools);
        let synthesized_tool_names: std::collections::HashSet<String> = synthesized_tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        // Durable specs first, synthesised after — every reader's order.
        //
        // Each schema is built once and handed out behind an `Arc`. The three
        // spec views an agent keeps (`durable_tool_specs`, `tool_specs`,
        // `visible_tool_specs`) overlap heavily — the durable set is a prefix
        // of the full set, and the visible set is a filtered subset of it — so
        // materialising them as independent `Vec<ToolSpec>` kept every
        // JSON-Schema `parameters` value resident up to three times per agent.
        // Sharing the leaves makes the extra views cost one pointer per entry
        // (openhuman#6218).
        let durable_tool_specs: Vec<Arc<ToolSpec>> =
            tools.iter().map(|tool| Arc::new(tool.spec())).collect();
        let tool_specs: Vec<Arc<ToolSpec>> = durable_tool_specs
            .iter()
            .cloned()
            .chain(synthesized_tools.iter().map(|tool| Arc::new(tool.spec())))
            .collect();

        let mut visible_names = self.visible_tool_names.unwrap_or_default();
        // Resolved here rather than at its historical position below: the pack
        // withholding is per-agent (a pack is skipped for the specialist that
        // owns its family), so the id has to exist before the strip.
        // A caller-supplied definition is resolved by the id the session was
        // stamped with (`OpenHumanDefinitionRegistry::host_definition` matches
        // `session_definition.id` against it), so the two must agree or the
        // definition is unreachable and every turn is refused for want of it.
        // Deriving the name from the definition makes the common case correct
        // without a second call, and a name that contradicts the definition is
        // a build error rather than a turn-time mystery.
        let agent_definition_name = match (
            self.agent_definition_name.clone(),
            self.session_definition.as_deref(),
        ) {
            (Some(name), Some(definition)) if name.trim() != definition.id.trim() => {
                return Err(anyhow::anyhow!(
                    "agent_definition_name `{name}` does not match the supplied agent                      definition `{}`; the session is resolved by the name it is stamped                      with, so a definition under a different id can never answer for it",
                    definition.id
                ));
            }
            // The definition's own id, not the caller's spelling of it. They
            // agree after trimming by the arm above, but only some readers
            // trim: `OpenHumanDefinitionRegistry` does,
            // `AgentDefinitionRegistry::get` does not, so a padded name would
            // resolve through one and miss through the other.
            (Some(_), Some(definition)) | (None, Some(definition)) => definition.id.clone(),
            (Some(name), None) => name,
            (None, None) => "main".to_string(),
        };
        // On-demand tool disclosure: withhold packed tools' schemas from the
        // provider and advertise `use_skill` in their place. The
        // tools stay in the registry below and stay executable — only the
        // advertised surface shrinks. Applied here, before the policy filter,
        // so the visible set and the policy session cannot disagree.
        // An empty set here is a wildcard belt; anything else was written down
        // by an agent author (or synthesised for one) and is left alone below.
        let belt_is_wildcard = visible_names.is_empty();
        if belt_is_wildcard {
            visible_names = tools
                .iter()
                .chain(synthesized_tools.iter())
                .map(|tool| tool.name().to_string())
                .collect();
        }
        crate::tools::toolpacks::strip_packed_from_visible(
            &mut visible_names,
            &agent_definition_name,
        );
        // Per-tool exposure: `Hidden` members of a collapsed tool (`todo_*`)
        // and `Deferred` tools leave the wire; they stay registered
        // and dispatchable. A wildcard belt always gets this; a hand-written
        // `[tools] named` list is already the answer to "what should this
        // agent see", so it opts into discovery by naming `tool_search` — the
        // harness's intrinsic bridge, not a registered tool, so the name is
        // taken off the allowlist here and stands for "every deferred
        // registration is reachable through the bridge".
        //
        // Only the DURABLE registry is passed, never `synthesized_tools`: every
        // `ArchetypeDelegationTool` reports `Hidden`, and on a wildcard belt the
        // synthesised delegates are the agent's only hand-off routes. Stripping
        // them would delete every `manage_tasks`/`build_workflow`/… route.
        //
        // This is the one site that turns the "all visible" sentinel into a
        // concrete set for a session, so the refresh paths never re-admit a
        // durable Hidden tool: `refresh_delegation_tools` (turn/tools.rs) only
        // swaps synthesised names, and `OpenHumanSessionHost::hide_tools` only seeds a set
        // that is still empty — see the matching strip there.
        let discovery_opted_in =
            visible_names.remove(crate::tools::implementations::meta::TOOL_SEARCH_NAME);
        let discovery_enabled = (belt_is_wildcard || discovery_opted_in)
            && !self
                .withheld_tool_names
                .contains(crate::tools::implementations::meta::TOOL_SEARCH_NAME);
        // A wildcard belt was seeded from the whole registry, so its durable
        // `Hidden` members (collapsed `todo_*`) leave here too.
        // A named belt never listed them.
        let mut deferred_names = if belt_is_wildcard {
            crate::tools::implementations::meta::strip_deferred_from_visible(
                &mut visible_names,
                tools.as_slice(),
            )
        } else {
            std::collections::HashSet::new()
        };
        if discovery_enabled {
            // Durable AND synthesised: a per-action integration tool is
            // synthesised per session (`collect_orchestrator_tools`) and
            // declares `Deferred` too. The synthesised set's `Hidden` members
            // are left alone on purpose — see the comment above.
            // Plus the tools this agent's definition defers for itself
            // (`deferred_tools`); see `meta::deferred_set`.
            deferred_names.extend(crate::tools::implementations::meta::deferred_set(
                tools.as_slice(),
                synthesized_tools.as_slice(),
                &self.deferred_tools,
            ));
            visible_names.retain(|name| !deferred_names.contains(name));
        } else {
            deferred_names.clear();
        }
        if !deferred_names.is_empty() {
            tracing::info!(
                agent = %agent_definition_name,
                deferred = deferred_names.len(),
                "[tools] withheld deferred tool schemas; reachable via the harness tool_search bridge"
            );
        }
        if !self.withheld_tool_names.is_empty() {
            visible_names.retain(|name| !self.withheld_tool_names.contains(name));
            deferred_names.retain(|name| !self.withheld_tool_names.contains(name));
            if visible_names.is_empty() {
                visible_names
                    .insert(crate::agent::harness::definition::NO_TOOLS_SENTINEL.to_string());
            }
        }
        visible_names.extend(self.permanent_tool_names.iter().cloned());
        deferred_names.retain(|name| !self.permanent_tool_names.contains(name));
        // What the policy classifies and the harness registers: the advertised
        // set plus the deferred set. A deferred tool outside this union would
        // be `HideFromPrompt`, and the direct-call gate refuses those.
        let reachable_names: std::collections::HashSet<String> = visible_names
            .iter()
            .chain(deferred_names.iter())
            .cloned()
            .collect();
        let config = self.config.clone().unwrap_or_default();
        // The turn harness is assembled without a config in hand; record the
        // `tool_search` settings here so every later turn ranks as configured.
        crate::agent::tinyagents::discovery::apply_tool_search_config(&config.tool_search);
        let event_session_id = self
            .event_session_id
            .clone()
            .unwrap_or_else(|| "standalone".to_string());
        let event_channel = self
            .event_channel
            .clone()
            .unwrap_or_else(|| "internal".to_string());
        // Classify both sets: a synthesised delegate needs a decision too.
        let all_tools: Vec<&dyn Tool> = tools
            .iter()
            .chain(synthesized_tools.iter())
            .map(|tool| tool.as_ref())
            .collect();
        let mut tool_policy_session = ToolPolicyEngine::build_session_from_refs(
            &agent_definition_name,
            &event_channel,
            "session",
            &config.channel_permissions,
            &all_tools,
            &reachable_names,
        );
        // A pack whose owner this agent can hand off to directly is that
        // specialist's belt, not this agent's: close it (#6302).
        crate::tools::toolpacks::close_handed_off_packs(
            &mut tool_policy_session,
            &agent_definition_name,
            &all_tools,
        );

        // A child agent inherits explicit profile and channel restrictions, but
        // not the primary agent's own role-specific tool scope. The Master OpenHumanSessionHost
        // can write directly, while specialists may still need tools outside its
        // intentionally compact default surface. Conflating those two surfaces
        // silently strips specialist capabilities (#5118 merge).
        //
        // Build a second policy snapshot without the role visibility filter.
        // `tool_policy_session` marks both channel-blocked and role-hidden tools
        // as restricted, so deriving the child ceiling from it would reintroduce
        // exactly that conflation.
        let channel_policy_session = ToolPolicyEngine::build_session_from_refs(
            &agent_definition_name,
            &event_channel,
            "session",
            &config.channel_permissions,
            &all_tools,
            &std::collections::HashSet::new(),
        );
        let mut subagent_tool_ceiling_names = self.subagent_tool_ceiling_names.unwrap_or_default();
        if channel_policy_session.has_restrictions() {
            let policy_allowed: std::collections::HashSet<String> = tool_specs
                .iter()
                .filter(|spec| channel_policy_session.is_allowed(&spec.name))
                .map(|spec| spec.name.clone())
                .collect();
            if subagent_tool_ceiling_names.is_empty() {
                subagent_tool_ceiling_names = policy_allowed;
            } else {
                subagent_tool_ceiling_names.retain(|name| policy_allowed.contains(name));
                if subagent_tool_ceiling_names.is_empty() {
                    subagent_tool_ceiling_names.insert("__subagent_no_tools__".to_string());
                }
            }
        }

        // Build the filtered spec list that the main agent sends to the
        // provider. The explicit visible-tool allowlist and the resolved
        // channel permission policy must stay aligned so prompt-visible
        // tools cannot exceed the runtime execution boundary.
        let visible_tool_specs_unfiltered = visible_tool_specs_for_policy(
            &tool_specs,
            &visible_names,
            &tool_policy_session,
            self.runtime_config.as_deref(),
            self.session_definition.as_deref(),
        );

        // Dedupe by tool name. Anthropic (and other strict providers)
        // rejects a chat/completions request that lists two tools with
        // the same name — OpenHuman's own backend and OpenAI silently
        // accept duplicates, which hid this bug until #1710's per-role
        // routing started sending the same tool list to Anthropic.
        let visible_tool_specs: Vec<Arc<ToolSpec>> =
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

        // Pull the model source out of the builder once; the OpenHumanSessionHost holds it and
        // builds a fresh tiered crate `ChatModel` set from it per turn.
        let mut turn_model_source = self
            .turn_model_source
            .ok_or_else(|| anyhow::anyhow!("provider is required"))?;

        let prompt_builder = self
            .prompt_builder
            .unwrap_or_else(crate::agent::prompts::SystemPromptBuilder::with_defaults);

        let model_name = self
            .model_name
            .unwrap_or_else(|| crate::config::DEFAULT_MODEL.into());

        // Assemble the per-session ContextManager. The manager owns
        // the prompt builder, the reduction pipeline, and the
        // summarizer — every concern that touches "what's in the
        // model's context window" routes through this single handle.
        let context_config = self.context_config.unwrap_or_default();

        // Live history reduction moved to the tinyagents graph
        // (`ContextCompressionMiddleware` + `MessageTrimMiddleware`, issue
        // #4249), so the session no longer constructs an in-turn summarizer
        // here.
        let context = ContextManager::new(&context_config, prompt_builder);

        let workspace_dir = self.workspace_dir.unwrap_or_else(|| {
            self.runtime_config
                .as_ref()
                .map(|config| config.workspace_dir.clone())
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        });
        let action_dir = self.action_dir.unwrap_or_else(|| {
            self.runtime_config
                .as_ref()
                .map(|config| config.action_dir.clone())
                .unwrap_or_else(|| workspace_dir.clone())
        });
        let runtime_config = self.runtime_config.map(|config| {
            let mut config = (*config).clone();
            config.workspace_dir = workspace_dir.clone();
            config.action_dir = action_dir.clone();
            Arc::new(config)
        });
        if let Some(config) = &runtime_config {
            turn_model_source = turn_model_source.with_attachment_config(config.clone());
        }

        // Direct builder callers (notably unit fixtures) do not pass through
        // `build_session_agent_inner`, which normally creates the durable host
        // authority for a root TinyAgents invocation. Unit-test binaries do
        // not promise an ordering for global-registry initialization, so use
        // the built-in test definitions when the process registry is absent.
        // Production callers keep the explicit hosted-authority error: a
        // builtins-only fallback there could hide a missing workspace load.
        let mut hosted_config = runtime_config.as_deref().cloned().unwrap_or_default();
        hosted_config.workspace_dir = workspace_dir.clone();
        hosted_config.action_dir = action_dir.clone();
        let hosted_config = Arc::new(hosted_config);
        #[cfg(test)]
        let definitions = Some(
            crate::agent::harness::AgentDefinitionRegistry::global_arc().unwrap_or_else(|| {
                Arc::new(crate::agent::harness::AgentDefinitionRegistry::builtins_only())
            }),
        );
        #[cfg(not(test))]
        let definitions = crate::agent::harness::AgentDefinitionRegistry::global_arc();
        // A caller that brought its own definition is the authority for this
        // session, so it does not need a process registry to exist before it
        // may run a turn. The stand-in is deliberately *empty* rather than
        // `builtins_only()`: such a caller has not asked for OpenHuman's
        // agents, and materialising them would make `orchestrator` and every
        // other built-in id silently resolvable in a host that never declared
        // one. The session's own definition outranks whichever registry this
        // is, so where a process registry does exist nothing about its
        // resolution changes.
        let session_definition = self.session_definition;
        let definitions = definitions.or_else(|| {
            session_definition
                .is_some()
                .then(|| Arc::new(crate::agent::harness::AgentDefinitionRegistry::default()))
        });
        let hosted_base = definitions.map(|definitions| {
            Arc::new(crate::agent::tinyagents::host::OpenHumanHostBase {
                security_policy: Arc::new(crate::security::SecurityPolicy::from_config(
                    &hosted_config.autonomy,
                    &workspace_dir,
                    &action_dir,
                )),
                config: Arc::clone(&hosted_config),
                definitions,
                post_turn_hooks: self.post_turn_hooks.clone(),
                // Usually this path names a registry id, and carries no
                // definition of its own. A caller that supplied one with
                // `agent_definition` is stamped with it here, the way
                // `build_session_agent_inner` stamps the one it resolved.
                session_definition: session_definition.clone(),
            })
        });

        let tools = Arc::new(tools);
        let synthesized_tools = Arc::new(synthesized_tools);
        // The pack tool lives inside this registry, so it can only be pointed
        // at it once it exists. Re-bind after any later rebuild of this `Arc`.
        crate::tools::toolpacks::bind_pack_registry(&tools);
        // And at the delegates, which are NOT in that registry: every
        // `delegate_*` is synthesised into its own `Arc`, so a packed delegate
        // is unreachable through `use_skill` until this second binding runs.
        crate::tools::toolpacks::bind_synthesized_pack_registry(&tools, &synthesized_tools);

        Ok(OpenHumanSessionHost {
            runtime_session: None,
            runtime_state: Arc::new(std::sync::Mutex::new(
                super::super::runtime_session::OpenHumanSessionState::default(),
            )),
            turn_model_source,
            tools,
            synthesized_tools,
            tool_specs: Arc::new(tool_specs),
            durable_tool_specs: Arc::new(durable_tool_specs),
            visible_tool_specs: Arc::new(visible_tool_specs),
            visible_tool_names: visible_names,
            deferred_tool_names: deferred_names,
            permanent_tool_names: self.permanent_tool_names,
            discovery_enabled,
            requested_deferred_tools: Arc::from(self.deferred_tools.clone()),
            subagent_tool_ceiling_names,
            tool_policy_session,
            tool_dispatcher: std::sync::Arc::from(
                self.tool_dispatcher
                    .ok_or_else(|| anyhow::anyhow!("tool_dispatcher is required"))?,
            ),
            config,
            model_name,
            model_vision: self.model_vision.unwrap_or(false),
            temperature: self.temperature.unwrap_or(0.7),
            workspace_dir,
            action_dir,
            workspace_descriptor: self.workspace_descriptor,
            workflows: self.workflows.unwrap_or_default(),
            last_memory_context: None,
            post_turn_hooks: self.post_turn_hooks,
            event_session_id,
            event_channel,
            thread_id: None,
            agent_definition_name: agent_definition_name.clone(),
            // Canonical registry id — captured here at build time
            // before any caller can call `set_agent_definition_name`
            // and clobber the transcript-facing name. Used by
            // `refresh_delegation_tools` to re-resolve the agent's
            // `subagents` declaration against the global registry.
            agent_definition_id: agent_definition_name.clone(),
            session_history_locator: self.session_history_locator,
            session_history_locator_memo: std::sync::OnceLock::new(),
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
            session: None,
            context: std::sync::Arc::new(std::sync::Mutex::new(context)),
            on_progress: None,
            run_queue: None,
            connected_integrations: Vec::new(),
            connected_integrations_initialized: false,
            runtime_config,
            hosted_base,
            definition: session_definition,
            omit_memory_context: self.omit_memory_context.unwrap_or(false),
            payload_summarizer: self.payload_summarizer,
            tokenjuice_compression: self.tokenjuice_compression,
            tool_policy: self
                .tool_policy
                .unwrap_or_else(|| Arc::new(crate::agent::tool_policy::AllowAllToolPolicy)),
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
            synthesized_tool_names,
        })
    }
}
