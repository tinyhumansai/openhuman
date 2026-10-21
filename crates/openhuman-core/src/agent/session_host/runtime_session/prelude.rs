//! Host-specific model/tool/prompt preparation for one runtime turn.

use super::*;

impl OpenHumanTurnPrelude {
    #[allow(clippy::type_complexity)]
    pub(super) fn current_tool_source(
        &self,
    ) -> (
        Arc<Vec<Box<dyn tinytools::Tool>>>,
        Arc<Vec<Box<dyn tinytools::Tool>>>,
        tinytools::ToolPolicySession,
        String,
    ) {
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            surface.tools.clone(),
            surface.synthesized_tools.clone(),
            surface.tool_policy_session.clone(),
            surface.event_channel.clone(),
        )
    }

    /// The session's deferred set for this turn; see
    /// `OpenHumanRunContext::deferred_tool_names`.
    pub(super) fn current_deferred_tool_names(&self) -> std::collections::HashSet<String> {
        self.tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .deferred_tool_names
            .clone()
    }

    pub(super) fn replace_tool_surface(&self, surface: OpenHumanTurnToolSurface) {
        *self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = surface;
    }

    pub(super) async fn prepare(&self, cold: bool) -> Result<TurnPreparation> {
        let tools = {
            let surface = self
                .tool_surface
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // The snapshot is the declaration set the session grants — the
            // driver reads it back as the harness allowlist — so it carries
            // the deferred tools beside the advertised ones. The harness
            // still advertises only the `Direct` registrations and reaches
            // the rest through its `tool_search` bridge.
            ToolSnapshot::new(
                surface
                    .visible_tool_specs
                    .iter()
                    .chain(
                        surface
                            .tool_specs
                            .iter()
                            .filter(|spec| surface.deferred_tool_names.contains(&spec.name)),
                    )
                    .filter(|spec| {
                        self.thread_id.is_some()
                            || !crate::agent::tinyagents::tool_registration::is_thread_goal_tool(
                                &spec.name,
                            )
                    })
                    .map(|spec| spec.as_ref().clone())
                    .collect(),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
        };
        let flow_archetype_text = super::prompt::flow_archetype_text(self, cold).await?;
        let prefix = if cold {
            let tiered = self
                .build_system_prompt_tiered_with_flow_archetype(flow_archetype_text.as_deref())?;
            Some(tinyagents_runtime::PrefixSnapshot::new(
                tiered
                    .system_messages()
                    .into_iter()
                    .map(Message::system)
                    .collect(),
            ))
        } else {
            None
        };
        Ok(TurnPreparation {
            prefix,
            tools: Some(tools),
        })
    }

    pub(super) fn build_system_prompt_tiered(&self) -> Result<crate::agent::prompts::TieredPrompt> {
        self.build_system_prompt_tiered_with_flow_archetype(None)
    }

    pub(super) fn build_system_prompt_tiered_with_flow_archetype(
        &self,
        flow_archetype_text: Option<&str>,
    ) -> Result<crate::agent::prompts::TieredPrompt> {
        use crate::agent::prompts::{PromptContext, PromptTool};
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let withheld = self.rule_withheld_tools(&surface);
        let specs = surface
            .visible_tool_specs
            .iter()
            .filter(|spec| !withheld.contains(&spec.name))
            .filter(|spec| {
                self.thread_id.is_some()
                    || !crate::agent::tinyagents::tool_registration::is_thread_goal_tool(&spec.name)
            })
            .map(|spec| spec.as_ref().clone())
            .collect::<Vec<_>>();
        let instructions = self.tool_dispatcher.prompt_instructions(&specs);
        let tool_refs = surface
            .tools
            .iter()
            .chain(surface.synthesized_tools.iter())
            .filter(|tool| {
                self.thread_id.is_some()
                    || !crate::agent::tinyagents::tool_registration::is_thread_goal_tool(
                        tool.name(),
                    )
            })
            .map(|tool| tool.as_ref())
            .collect::<Vec<_>>();
        let mut prompt_tools = PromptTool::from_tool_refs(tool_refs.iter().copied());
        prompt_tools.retain(|tool| !surface.permanent_tool_names.contains(tool.name.as_ref()));
        prompt_tools.retain(|tool| !withheld.contains(tool.name.as_ref()));
        let mut visible_tool_names = surface.tool_policy_session.visible_tool_names_for_prompt();
        visible_tool_names.retain(|name| !withheld.contains(name));
        visible_tool_names.retain(|name| !surface.permanent_tool_names.contains(name));
        if self.thread_id.is_none() {
            visible_tool_names.retain(|name| {
                !crate::agent::tinyagents::tool_registration::is_thread_goal_tool(name)
            });
        }
        crate::agent::prompts::swap_deferred_for_discovery_bridge(
            &mut prompt_tools,
            &mut visible_tool_names,
            &surface.deferred_tool_names,
        );
        let agents_md = if self.config.agents_md_enabled {
            crate::agent::prompts::load_agents_md_layers(&self.workspace_dir, &self.action_dir)
        } else {
            Default::default()
        };
        let mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            agent_id: &self.agent_definition_name,
            flow_archetype_text,
            tools: &prompt_tools,
            workflows: &mutable.workflows,
            dispatcher_instructions: &instructions,
            visible_tool_names: &visible_tool_names,
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            connected_integrations: &mutable.connected_integrations,
            connected_identities_md: self.connected_identities_md(),
            user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
            personality_roster: vec![],
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };
        self.prompt_builder.build_tiered(&context)
    }

    pub(super) fn drain_host_events(&self) -> bool {
        let mut mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if mutable.composio_events.is_none() {
            mutable.composio_events = crate::core::bus::BUS.get().map(|bus| bus.receiver());
        }
        if mutable.skill_events.is_none() {
            mutable.skill_events = crate::core::bus::BUS.get().map(|bus| bus.receiver());
        }
        let drain = |receiver: &mut Option<
            tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
        >| {
            let Some(receiver) = receiver.as_mut() else {
                return false;
            };
            let mut skills_changed = false;
            loop {
                use tinybus::TryRecvError;
                match receiver.try_recv() {
                    Ok(crate::core::events::DomainEvent::WorkflowsChanged { .. }) => {
                        skills_changed = true
                    }
                    Ok(crate::core::events::DomainEvent::ComposioIntegrationsChanged {
                        ..
                    })
                    | Ok(_) => {}
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Lagged(_)) => {
                        skills_changed = true;
                        break;
                    }
                    Err(TryRecvError::Closed) => break,
                }
            }
            skills_changed
        };
        let composio_skills_changed = drain(&mut mutable.composio_events);
        composio_skills_changed | drain(&mut mutable.skill_events)
    }

    /// Assemble every dynamic, host-owned user-turn addition after the runtime
    /// has selected/resumed its transcript.  The returned message is then the
    /// sole input which `Session` appends to generic history.
    pub(super) async fn enrich_request(
        &self,
        original_user_message: &str,
        turn: &super::super::types::TurnInputs,
        run_context: &mut OpenHumanRunContext,
    ) -> String {
        let mut context = String::new();

        let active_goal = if turn.overrides.suppress_active_goal {
            None
        } else {
            let loaded = crate::agent::goals::runtime::load_for_thread(
                &self.workspace_dir,
                self.thread_id.as_deref(),
            )
            .await;
            match loaded {
                Some(goal) if matches!(goal.status, ThreadGoalStatus::Paused) => {
                    crate::agent::goals::runtime::resume_for_thread(
                        &self.workspace_dir,
                        self.thread_id.as_deref(),
                    )
                    .await
                    .unwrap_or(Some(goal))
                }
                other => other,
            }
        };
        if let Some(goal) = &active_goal {
            if let Some(block) = tinyagents_harness_tools::goals::active_goal_context_block(goal) {
                context.push_str(&block);
            }
            if let Some(hook) = tinyagents_hooks::stop::GoalBudgetStopHook::for_goal(
                crate::storage::sessions::stores_or_ephemeral(&self.workspace_dir).kv,
                goal,
            ) {
                run_context.stop_hooks.push(Arc::new(hook));
            }
        }
        // Build the roster from this turn's *effective* visible tool set
        // (snapshotted under the tool-surface lock, then released), not the
        // definition's static scope: a hide or named restriction can narrow what
        // this turn can call, and the roster must not advertise a control it cannot invoke.
        let turn_fleet = {
            let visible = self
                .tool_surface
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .visible_tool_names
                .clone();
            if visible.is_empty() {
                crate::agent::orchestration::fleet_tools::FleetToolSet::for_parent(
                    &self.agent_definition_id,
                )
            } else {
                crate::agent::orchestration::fleet_tools::FleetToolSet::from_visible_tool_names(
                    &visible,
                )
            }
        };
        if let Some(block) =
            crate::agent::orchestration::running_subagents::active_subagents_context_block(
                &self.dispatch_ctx(),
                &self.event_session_id,
                &self.workspace_dir,
                &turn_fleet,
            )
        {
            context.push_str(&block);
        }

        let mut enriched = if context.is_empty() {
            original_user_message.to_string()
        } else {
            format!("{context}{original_user_message}")
        };
        {
            let mut mutable = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            mutable.last_memory_context = (!context.is_empty()).then_some(context);
        }
        run_context.response_shape = turn.response_shape.clone();
        run_context.attach_parent(self.parent_context(run_context.parent.as_ref()));
        self.apply_pending_announcements(&mut enriched);
        run_context.attachment_placeholders =
            Arc::new(extract_image_placeholders_in_text(original_user_message));
        run_context.dispatch = Some(Arc::new(
            crate::agent::tinyagents::host::TurnDispatchState::new(
                crate::agent::tinyagents::agent_turn_wall_clock_ms_for(
                    self.runtime_config
                        .as_deref()
                        .is_some_and(crate::agent::tinyagents::chat_provider_is_local),
                )
                .map(std::time::Duration::from_millis),
            ),
        ));
        run_context.sandbox_mode = Some(self.sandbox_mode);
        // Same pin as `SessionDriver::run_turn`: the harness speaks the dialect
        // the prompt was composed for, so a text dialect keeps its schemas off
        // the wire and renders the catalogue itself (`ToolsSection` no longer
        // does), and a code call is recovered against the positional registry.
        run_context.tool_dialect = tinyagents_harness::dialect::dispatcher_for_format(
            self.tool_dispatcher.tool_call_format(),
        );
        format!("{}\n\n{enriched}", turn.preamble())
    }

    fn apply_pending_announcements(&self, enriched: &mut String) {
        let mut mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for note in [
            integration_announcement_note(&std::mem::take(
                &mut mutable.pending_integration_announcement,
            )),
            mcp_announcement_note(&std::mem::take(&mut mutable.pending_mcp_announcement)),
            skill_announcement_note(&std::mem::take(&mut mutable.pending_skill_announcement)),
            skill_retraction_note(&std::mem::take(&mut mutable.pending_skill_retraction)),
        ]
        .into_iter()
        .flatten()
        {
            *enriched = format!("{note}\n\n{enriched}");
        }
    }

    pub(super) async fn finalize_after_durable_commit(
        &self,
        receipt: &CommitReceipt<OpenHumanRunContext>,
    ) {
        self.memory_post_turn(receipt);
    }
}
