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
            ToolSnapshot::new(granted_tool_specs(
                &surface.visible_tool_specs,
                &surface.tool_specs,
                &surface.deferred_tool_names,
                self.thread_id.is_some(),
            ))
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
        };
        let prefix = if cold {
            Some(self.cold_prompt_prefix().await?)
        } else {
            None
        };
        Ok(TurnPreparation {
            prefix,
            tools: Some(tools),
        })
    }

    /// The prompt a new session starts with, rendered now: one system
    /// message per cache tier, with the definition's flow archetype when it
    /// has one. A resumed thread that refreshes its prompt
    /// (`prompt_refresh.rs`) renders it here too, so an unchanged prompt
    /// compares equal to the one its transcript recorded.
    pub(super) async fn cold_prompt_prefix(&self) -> Result<tinyagents_runtime::PrefixSnapshot> {
        let flow_archetype_text = super::prompt::flow_archetype_text(self, true).await?;
        let connected_identities_md =
            crate::integrations::render_connected_identities_async(&self.dispatch_ctx()).await;
        let tiered = self.build_system_prompt_tiered_with_flow_archetype_and_connected_identities(
            flow_archetype_text.as_deref(),
            &connected_identities_md,
        )?;
        Ok(tinyagents_runtime::PrefixSnapshot::new(
            tiered
                .system_messages()
                .into_iter()
                .map(Message::system)
                .collect(),
        ))
    }

    pub(super) fn build_system_prompt_tiered(&self) -> Result<crate::agent::prompts::TieredPrompt> {
        self.build_system_prompt_tiered_with_flow_archetype(None)
    }

    pub(crate) fn build_system_prompt_tiered_with_flow_archetype(
        &self,
        flow_archetype_text: Option<&str>,
    ) -> Result<crate::agent::prompts::TieredPrompt> {
        let connected_identities_md = self.connected_identities_md();
        self.build_system_prompt_tiered_with_flow_archetype_and_connected_identities(
            flow_archetype_text,
            &connected_identities_md,
        )
    }

    fn build_system_prompt_tiered_with_flow_archetype_and_connected_identities(
        &self,
        flow_archetype_text: Option<&str>,
        connected_identities_md: &str,
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
            connected_identities_md: connected_identities_md.to_string(),
            user_identity: crate::credentials::identity::peek_credential_user_identity(),
            personality_roster: vec![],
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };
        let mut tiered = self.prompt_builder.build_tiered(&context)?;
        // Appended, not prepended (#5704): every line of the boundary is
        // session-scoped (agent id, channel, entry point, risk level), so
        // putting it first would move the prompt's first diverging byte to
        // offset 0 and cost the backend's prefix cache everything behind it.
        // It rides the volatile tier, which is the tail of the system prompt.
        if let Some(boundary) =
            tinytools::render_tool_policy_boundary(&surface.tool_policy_session, 2048)
        {
            let boundary = boundary.trim_end().to_string();
            if !tiered.text.is_empty() && !tiered.text.ends_with('\n') {
                tiered.text.push('\n');
            }
            tiered.text.push('\n');
            tiered.text.push_str(&boundary);
            tiered.text.push('\n');
            tiered.parts.push((
                tinyagents_harness::prompt::PromptTier::Volatile,
                format!("\n\n{boundary}"),
            ));
        }
        Ok(tiered)
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

        // A paused goal stays paused: the model reads the message and resumes,
        // replaces or leaves it with the goal tools. Turns with the run-state
        // block see the goal there, current on every call, so it is not also
        // prepended to the user's message.
        run_context.goal_suppressed = turn.overrides.suppress_active_goal;
        let active_goal = if turn.overrides.suppress_active_goal {
            None
        } else {
            crate::agent::goals::load_for_thread(
                &self.workspace_dir,
                &self.dispatch_ctx(),
                self.thread_id.as_deref(),
            )
            .await
        };
        let state_block_shows_goal =
            crate::agent::tinyagents::state_block::applies(false, Some(&self.agent_definition_id));
        if let Some(goal) = &active_goal {
            if !state_block_shows_goal {
                if let Some(block) =
                    tinyagents_harness_tools::goals::active_goal_context_block(goal)
                {
                    context.push_str(&block);
                }
            }
            if let Some(hook) = tinyagents_hooks::stop::GoalBudgetStopHook::for_goal(
                crate::agent::goals::goal_store(&self.workspace_dir, &self.dispatch_ctx())
                    .backing_store(),
                goal,
            ) {
                run_context.stop_hooks.push(Arc::new(hook));
            }
        }
        if let Some(block) =
            crate::agent::orchestration::running_subagents::active_subagents_context_block(
                &self.dispatch_ctx(),
                &self.event_session_id,
                &self.workspace_dir,
                self.workspace_descriptor.as_ref(),
            )
            .await
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
        // A plan that finished this turn hands its notes over: they stay for
        // the thread's next turns and go to long-term memory with the reply.
        let finished_plan = crate::agent::tinyagents::state_block::close_finished_plan(
            &self.workspace_dir,
            self.thread_id.as_deref(),
            &self.dispatch_ctx(),
        )
        .await;
        self.memory_post_turn(receipt, finished_plan);
    }
}

/// The declarations a turn's snapshot grants: the advertised specs, then the
/// deferred ones, name-unique.
///
/// A deferred name can have two registrations -- a host tool spliced in ahead
/// of a config-derived built-in of the same name (`HostTurnTools::merge_into`).
/// The advertised list is already deduplicated, but `tool_specs` is the raw
/// registry, so both would reach `ToolSnapshot::new`, which refuses two
/// different declarations under one name and fails the whole turn. The first
/// occurrence wins, as it does for the advertised list, so the host's tool is
/// the one granted.
fn granted_tool_specs(
    visible: &[Arc<tinytools::ToolSpec>],
    registered: &[Arc<tinytools::ToolSpec>],
    deferred: &std::collections::HashSet<String>,
    has_thread: bool,
) -> Vec<tinytools::ToolSpec> {
    let specs: Vec<tinytools::ToolSpec> = visible
        .iter()
        .chain(
            registered
                .iter()
                .filter(|spec| deferred.contains(&spec.name)),
        )
        .filter(|spec| {
            has_thread
                || !crate::agent::tinyagents::tool_registration::is_thread_goal_tool(&spec.name)
        })
        .map(|spec| spec.as_ref().clone())
        .collect();
    tinyagents_runtime::dedup_visible_tool_specs(specs.into_iter().map(Arc::new).collect())
        .into_iter()
        .map(|spec| spec.as_ref().clone())
        .collect()
}

#[cfg(test)]
#[path = "prelude_tests.rs"]
mod tests;
