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
    /// It ensures that all required fields (provider, tools, memory, etc.) are present.
    pub fn build(self) -> Result<OpenHumanSessionHost> {
        let tools = self
            .tools
            .ok_or_else(|| anyhow::anyhow!("tools are required"))?;
        // The synthesised set lives beside the durable registry, never inside
        // it (`OpenHumanSessionHost::synthesized_tools`); a durable name wins a collision.
        let synthesized_tools = super::drop_synthesized_name_collisions(
            &tools,
            self.synthesized_tools.unwrap_or_default(),
        );
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
        let agent_definition_name = self
            .agent_definition_name
            .clone()
            .unwrap_or_else(|| "main".to_string());
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
        // Per-tool exposure: `Hidden` members of a collapsed tool (`memory_*`,
        // `todo_*`) and `Deferred` tools leave the wire; they stay registered
        // and dispatchable. Only for a wildcard belt — a hand-written `[tools]
        // named` list is already the answer to "what should this agent see".
        //
        // Only the DURABLE registry is passed, never `synthesized_tools`: every
        // `ArchetypeDelegationTool` reports `Hidden`, and on a wildcard belt the
        // synthesised delegates are the agent's only hand-off routes. Stripping
        // them would delete every `research`/`run_code`/… route.
        // `strip_deferred_from_visible` only looks at the tools it is given.
        //
        // This is the one site that turns the "all visible" sentinel into a
        // concrete set for a session, so the refresh paths never re-admit a
        // durable Hidden tool: `refresh_delegation_tools` (turn/tools.rs) only
        // swaps synthesised names, and `OpenHumanSessionHost::hide_tools` only seeds a set
        // that is still empty — see the matching strip there.
        let deferred = if belt_is_wildcard {
            crate::tools::implementations::meta::strip_deferred_from_visible(
                &mut visible_names,
                tools.as_slice(),
            )
        } else {
            Vec::new()
        };
        if !deferred.is_empty() {
            tracing::info!(
                agent = %agent_definition_name,
                deferred = deferred.len(),
                "[tools] withheld deferred tool schemas; reachable via tool_search"
            );
        }
        // Index them where the model can find them again. Done here rather than
        // at registration because which tools are deferred depends on the belt.
        crate::tools::implementations::meta::bind_tool_search_index(tools.as_slice(), deferred);
        let config = self.config.clone().unwrap_or_default();
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
            &visible_names,
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
        let visible_tool_specs_unfiltered =
            visible_tool_specs_for_policy(&tool_specs, &visible_names, &tool_policy_session);

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
        let turn_model_source = self
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
        // here. The archivist hook still drives durable segment recaps on its
        // own post-turn path; it is no longer coupled to context compaction.
        let context = ContextManager::new(&context_config, prompt_builder);

        let workspace_dir = self
            .workspace_dir
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let action_dir = self.action_dir.unwrap_or_else(|| workspace_dir.clone());

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
            subagent_tool_ceiling_names,
            tool_policy_session,
            memory: self
                .memory
                .ok_or_else(|| anyhow::anyhow!("memory is required"))?,
            auto_recall: self.auto_recall,
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
            auto_save: self.auto_save.unwrap_or(false),
            last_memory_context: None,
            post_turn_hooks: self.post_turn_hooks,
            learning_enabled: self.learning_enabled,
            explicit_preferences_enabled: self.explicit_preferences_enabled,
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
            context: std::sync::Arc::new(std::sync::Mutex::new(context)),
            on_progress: None,
            run_queue: None,
            connected_integrations: Vec::new(),
            connected_integrations_initialized: false,
            runtime_config: None,
            hosted_base: None,
            definition: None,
            // Default to `true` (omit) so legacy / custom agents built
            // without a definition stay lean. Opt-in agents thread their
            // `omit_profile = false` through the builder.
            omit_profile: self.omit_profile.unwrap_or(true),
            omit_memory_md: self.omit_memory_md.unwrap_or(true),
            payload_summarizer: self.payload_summarizer,
            trigger_memory_agent: self.trigger_memory_agent.unwrap_or_default(),
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
            archivist_hook: self.archivist_hook,
            synthesized_tool_names,
        })
    }
}
