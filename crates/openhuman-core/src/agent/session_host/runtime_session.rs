//! The live OpenHuman composition over `tinyagents_runtime::Session`.
//!
//! The mutex is intentionally narrow: it carries host preparation and
//! finalization observations only. Generic model history, transcript rows,
//! prefix reconciliation, tool snapshots, resume and persistence remain inside
//! the runtime session.

use std::sync::Arc;

use anyhow::Result;
use tinyagents_runtime::{
    CommitReceipt, PrefixSnapshot, ResumeMode, ResumePreparation, SessionBuilder, SessionTerminal,
    SessionTurnRequest, ToolSnapshot, TranscriptCodec, TranscriptTarget, TurnOptions,
    TurnPreparation,
};
use tinyagents_session::transcript::TranscriptMeta;
use tinyinference_llm::message::Message;

use crate::agent::{
    session_host::{
        OpenHumanSessionHooks, OpenHumanTranscriptCodec, driver::OpenHumanSessionDriver,
    },
    tinyagents::{TurnContextMiddleware, host::OpenHumanRunContext},
};

use super::types::OpenHumanSessionHost;

/// Mutable product state observed by the runtime hooks.
///
/// This type has no message accumulator, raw transcript rows, prefix matching,
/// resume cache, or persistence handle. Those are exclusively `Session` state.
#[derive(Default)]
pub(super) struct OpenHumanSessionState {
    /// Host-selected transcript destination. The runtime binds and owns the
    /// resulting history handle, raw rows and append/reconcile state.
    resume_target: Option<TranscriptTarget>,
    last_commit: Option<CommitReceipt<OpenHumanRunContext>>,
    terminals: Vec<SessionTerminal>,
    pub(super) last_turn_hit_cap: bool,
    pub(super) last_turn_usage: Option<crate::agent::tinyagents::host::LastTurnUsage>,
    context_middleware: Option<TurnContextMiddleware>,
    required_output: Option<tinyagents_harness::config::RequiredOutput>,
    pub(crate) pending_turn_overrides: super::types::TurnOverrides,
    prelude: Option<OpenHumanTurnPrelude>,
    pub(crate) pending_citations:
        Option<tokio::task::JoinHandle<Vec<crate::memory::agent::memory_loader::MemoryCitation>>>,
    pub(crate) last_turn_citations: Vec<crate::memory::agent::memory_loader::MemoryCitation>,
}

/// Owned host-only inputs used by the async runtime preparation hook.
/// It deliberately excludes transcript/history/tool snapshots, which belong to
/// `tinyagents_runtime::Session`.
#[derive(Clone)]
struct OpenHumanTurnPrelude {
    memory: Arc<dyn crate::memory::Memory>,
    learning_enabled: bool,
    explicit_preferences_enabled: bool,
    config: crate::config::AgentConfig,
    /// Host session-memory state, shared by the runtime hooks. It is not
    /// generic conversation state and the runtime never persists it.
    context: Arc<std::sync::Mutex<crate::agent::context::ContextManager>>,
    tool_policy: Arc<dyn crate::agent::tool_policy::ToolPolicy>,
    tool_dispatcher: Arc<dyn tinytools_agent::dialect::ToolDialect>,
    workspace_dir: std::path::PathBuf,
    action_dir: std::path::PathBuf,
    model_name: String,
    agent_definition_name: String,
    omit_profile: bool,
    omit_memory_md: bool,
    auto_save: bool,
    thread_id: Option<String>,
    auto_recall: Option<Arc<crate::memory::auto_recall::AutoRecall>>,
    agent_definition_id: String,
    event_session_id: String,
    event_channel: String,
    trigger_memory_agent: crate::agent::harness::definition::TriggerMemoryAgent,
    subagent_tool_ceiling_names: std::collections::HashSet<String>,
    turn_model_source: crate::agent::tinyagents::TurnModelSource,
    temperature: f64,
    workspace_descriptor: Option<tinytools::WorkspaceDescriptor>,
    session_key: String,
    session_parent_prefix: Option<String>,
    on_progress: Option<tokio::sync::mpsc::Sender<crate::agent::progress::AgentProgress>>,
    run_queue:
        Option<Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    allowed_subagent_ids: std::collections::HashSet<String>,
    sandbox_mode: crate::agent::harness::definition::SandboxMode,
    runtime_config: Option<Arc<crate::config::Config>>,
    archivist_hook: Option<Arc<crate::agent::harness::archivist::ArchivistHook>>,
    /// The one authoritative, request-refreshable composition of executable
    /// tools, policy, and provider schema. Generic runtime owns the immutable
    /// `ToolSnapshot`; this host surface is the source used to create it.
    tool_surface: Arc<std::sync::Mutex<OpenHumanTurnToolSurface>>,
    mutable: Arc<std::sync::Mutex<OpenHumanTurnPreludeMutable>>,
}

/// Host-owned tool composition from which one runtime request is prepared.
/// All three views are rebuilt together so a prompt schema, execution source,
/// and fail-closed policy cannot describe different authority.
struct OpenHumanTurnToolSurface {
    tools: Arc<Vec<Box<dyn tinytools::Tool>>>,
    synthesized_tools: Arc<Vec<Box<dyn tinytools::Tool>>>,
    tool_specs: Arc<Vec<Arc<tinytools::ToolSpec>>>,
    durable_tool_specs: Arc<Vec<Arc<tinytools::ToolSpec>>>,
    visible_tool_specs: Arc<Vec<Arc<tinytools::ToolSpec>>>,
    visible_tool_names: std::collections::HashSet<String>,
    /// Whether newly connected delegates may enter the visible belt without a
    /// caller explicitly allowing them. A hide/named restriction turns this
    /// off so refresh cannot reopen withdrawn authority.
    auto_include_new_synthesized_tools: bool,
    synthesized_tool_names: std::collections::HashSet<String>,
    tool_policy_session: crate::tools::agent_policy::ToolPolicySession,
    event_session_id: String,
    event_channel: String,
    agent_definition_name: String,
}

/// Per-session product observations that were formerly scattered across the
/// legacy `core_turn` loop. Generic history, raw transcript data and prefix
/// state intentionally do not appear here.
#[derive(Default)]
struct OpenHumanTurnPreludeMutable {
    last_memory_context: Option<String>,
    announced_integrations: std::collections::HashSet<String>,
    pending_integration_announcement: Vec<String>,
    announced_mcp_servers: std::collections::HashSet<String>,
    pending_mcp_announcement: Vec<String>,
    announced_skills: std::collections::HashSet<String>,
    pending_skill_announcement: Vec<String>,
    pending_skill_retraction: Vec<String>,
    connected_integrations: Vec<crate::agent::prompts::ConnectedIntegration>,
    connected_integrations_initialized: bool,
    workflows: Vec<crate::skills::Workflow>,
    composio_events: Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    skill_events: Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    pending_user_autosave: Option<String>,
}

impl OpenHumanTurnPrelude {
    fn current_tool_source(
        &self,
    ) -> (
        Arc<Vec<Box<dyn tinytools::Tool>>>,
        Arc<Vec<Box<dyn tinytools::Tool>>>,
        crate::tools::agent_policy::ToolPolicySession,
        String,
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
            surface.event_session_id.clone(),
            surface.event_channel.clone(),
        )
    }

    fn replace_tool_surface(&self, surface: OpenHumanTurnToolSurface) {
        *self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = surface;
    }

    async fn prepare(&self, cold: bool) -> Result<TurnPreparation> {
        let tools = {
            let surface = self
                .tool_surface
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            ToolSnapshot::new(
                surface
                    .visible_tool_specs
                    .iter()
                    .map(|spec| spec.as_ref().clone())
                    .collect(),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
        };
        let prefix = if cold {
            let learned = self.fetch_learned_context().await;
            Some(PrefixSnapshot::new(vec![Message::system(
                self.build_system_prompt(learned)?,
            )]))
        } else {
            None
        };
        Ok(TurnPreparation {
            prefix,
            tools: Some(tools),
        })
    }

    async fn refresh_turn_boundary(&self, cold: bool) {
        if cold {
            self.refresh_cold_integrations().await;
        } else {
            self.refresh_dynamic_announcements().await;
        }
        // Integration changes are authority changes, not only display
        // announcements. Refresh the delegation executable set and rebuild
        // its schema/policy in the same hook pass before the driver sees it.
        self.refresh_delegation_tool_surface();
    }

    fn begin_user_effects(&self, state: &mut OpenHumanSessionState, request: &SessionTurnRequest) {
        let user_text = request.input.text();
        if self.auto_save && crate::agent::turn_origin::current_is_user_authored() {
            self.mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pending_user_autosave = Some(user_text.clone());
        }
        state.last_turn_citations.clear();
        if let Some(previous) = state.pending_citations.take() {
            previous.abort();
        }
        let memory = self.memory.clone();
        state.pending_citations = Some(tokio::spawn(async move {
            crate::memory::agent::memory_loader::collect_recall_citations(
                memory.as_ref(),
                &user_text,
                5,
                0.4,
            )
            .await
            .unwrap_or_default()
        }));
    }

    async fn fetch_learned_context(&self) -> crate::agent::prompts::LearnedContextData {
        if !self.learning_enabled && !self.explicit_preferences_enabled {
            return Default::default();
        }
        if !self.learning_enabled && self.explicit_preferences_enabled {
            return crate::agent::prompts::LearnedContextData {
                user_profile: crate::memory::preferences::load_general_preferences_on(
                    &self.memory,
                    crate::memory::preferences::STANDING_PREFS_LIMIT,
                )
                .await,
                ..Default::default()
            };
        }
        use crate::memory::MemoryCategory;
        let observations = self
            .memory
            .list(
                Some("learning_observations"),
                Some(&MemoryCategory::Custom("learning_observations".into())),
                None,
            )
            .await
            .unwrap_or_default();
        let patterns = self
            .memory
            .list(
                Some("learning_patterns"),
                Some(&MemoryCategory::Custom("learning_patterns".into())),
                None,
            )
            .await
            .unwrap_or_default();
        let reflections = self
            .memory
            .list(
                Some(crate::agent::learning::reflection::REFLECTIONS_NAMESPACE),
                Some(&MemoryCategory::Custom(
                    crate::agent::learning::reflection::REFLECTIONS_NAMESPACE.into(),
                )),
                None,
            )
            .await
            .unwrap_or_default();
        let limits = self.config.resolved_memory_limits();
        crate::agent::prompts::LearnedContextData {
            observations: observations
                .iter()
                .rev()
                .take(5)
                .map(|entry| sanitize_prelude_entry(&entry.content))
                .collect(),
            patterns: patterns
                .iter()
                .take(3)
                .map(|entry| sanitize_prelude_entry(&entry.content))
                .collect(),
            user_profile: crate::memory::preferences::load_general_preferences_on(
                &self.memory,
                crate::memory::preferences::STANDING_PREFS_LIMIT,
            )
            .await,
            reflections: reflections
                .iter()
                .rev()
                .take(10)
                .map(|entry| sanitize_prelude_entry(&entry.content))
                .collect(),
            tree_root_summaries: collect_prelude_tree_roots(
                limits.per_namespace_max_chars,
                limits.total_tree_max_chars,
            )
            .await,
        }
    }

    fn build_system_prompt(
        &self,
        learned: crate::agent::prompts::LearnedContextData,
    ) -> Result<String> {
        use crate::agent::prompts::{PromptContext, PromptTool, tool_call_format_from_dialect};
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let specs = surface
            .visible_tool_specs
            .iter()
            .map(|spec| spec.as_ref().clone())
            .collect::<Vec<_>>();
        let instructions = self.tool_dispatcher.prompt_instructions(&specs);
        let tool_refs = surface
            .tools
            .iter()
            .chain(surface.synthesized_tools.iter())
            .map(|tool| tool.as_ref())
            .collect::<Vec<_>>();
        let prompt_tools = PromptTool::from_tool_refs(tool_refs.iter().copied());
        let visible_tool_names = surface.tool_policy_session.visible_tool_names_for_prompt();
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
            tools: &prompt_tools,
            workflows: &mutable.workflows,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &visible_tool_names,
            tool_call_format: tool_call_format_from_dialect(
                self.tool_dispatcher.tool_call_format(),
            ),
            connected_integrations: &mutable.connected_integrations,
            connected_identities_md: crate::agent::prompts::render_connected_identities(),
            include_profile: !self.omit_profile,
            include_memory_md: !self.omit_memory_md,
            curated_snapshot: None,
            user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
            personality_roster: vec![],
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };
        self.context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .build_system_prompt(&context)
            .map_err(Into::into)
    }

    async fn refresh_cold_integrations(&self) {
        let should_fetch = !self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .connected_integrations_initialized;
        if !should_fetch {
            return;
        }
        let config = match self.runtime_config.clone() {
            Some(config) => Some(config),
            None => crate::config::Config::load_or_init()
                .await
                .ok()
                .map(Arc::new),
        };
        let Some(config) = config else {
            return;
        };
        let connected = crate::integrations::composio::fetch_connected_integrations(&config).await;
        let mcp_servers = crate::mcp::registry::connections::connected_overview()
            .await
            .into_iter()
            .map(|server| server.qualified_name)
            .collect::<std::collections::HashSet<_>>();
        let mut mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        mutable.connected_integrations = connected;
        mutable.connected_integrations_initialized = true;
        mutable.announced_integrations = mutable
            .connected_integrations
            .iter()
            .map(|item| item.toolkit.clone())
            .collect();
        mutable.announced_mcp_servers = mcp_servers;
    }

    async fn refresh_dynamic_announcements(&self) {
        let skills_changed = self.drain_host_events();
        if let Some(config) = self.runtime_config.as_deref() {
            if let Some(current) = crate::integrations::composio::cached_active_integrations(config)
            {
                let mut mutable = self
                    .mutable
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let current_slugs: std::collections::HashSet<_> =
                    current.iter().map(|item| item.toolkit.clone()).collect();
                for slug in &current_slugs {
                    if mutable.announced_integrations.insert(slug.clone())
                        && !mutable.pending_integration_announcement.contains(slug)
                    {
                        mutable.pending_integration_announcement.push(slug.clone());
                    }
                }
                mutable.connected_integrations = current;
            }
        }
        let connected_mcp = crate::mcp::registry::connections::connected_overview()
            .await
            .into_iter()
            .map(|server| server.qualified_name)
            .collect::<Vec<_>>();
        let mut mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for server in connected_mcp {
            if mutable.announced_mcp_servers.insert(server.clone())
                && !mutable.pending_mcp_announcement.contains(&server)
            {
                mutable.pending_mcp_announcement.push(server);
            }
        }
        if !skills_changed {
            return;
        }
        // Event-driven metadata refresh keeps the steady-state hot path free
        // of the old per-turn filesystem scan.
        let latest = crate::skills::load_workflow_metadata(&self.workspace_dir);
        let id = |workflow: &crate::skills::Workflow| {
            if workflow.dir_name.is_empty() {
                workflow.name.clone()
            } else {
                workflow.dir_name.clone()
            }
        };
        let previous: std::collections::HashSet<_> = mutable.workflows.iter().map(&id).collect();
        let current: std::collections::HashSet<_> = latest.iter().map(&id).collect();
        for id in current.difference(&previous) {
            if mutable.announced_skills.insert((*id).clone())
                && !mutable.pending_skill_announcement.contains(id)
            {
                mutable.pending_skill_announcement.push((*id).clone());
            }
        }
        for id in previous.difference(&current) {
            mutable.announced_skills.remove(id);
            mutable
                .pending_skill_announcement
                .retain(|pending| pending != id);
            if !mutable.pending_skill_retraction.contains(id) {
                mutable.pending_skill_retraction.push((*id).clone());
            }
        }
        mutable.workflows = latest;
    }

    /// Rebuild every delegation-dependent tool view from the current cached
    /// integration set. This mirrors the legacy refresh's replace-not-append
    /// semantics, but keeps the mutable authority in hook state rather than a
    /// second turn loop. A revoked delegate is removed from the executable
    /// source, schema, and policy together before this request is prepared.
    fn refresh_delegation_tool_surface(&self) {
        use crate::agent::harness::definition::AgentDefinitionRegistry;
        use crate::tools::agent_policy::ToolPolicyEngine;
        use crate::tools::orchestrator_tools::collect_orchestrator_tools;

        let Some(registry) = AgentDefinitionRegistry::global() else {
            return;
        };
        let Some(definition) = registry.get(&self.agent_definition_id).cloned() else {
            return;
        };
        if definition.subagents.is_empty() {
            return;
        }
        let integrations = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .connected_integrations
            .clone();
        let mut surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let synthesized = super::builder::drop_synthesized_name_collisions(
            &surface.tools,
            collect_orchestrator_tools(&definition, registry, &integrations),
        );
        let synthesized_names = synthesized
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<std::collections::HashSet<_>>();
        let previous_synthesized = std::mem::replace(
            &mut surface.synthesized_tool_names,
            synthesized_names.clone(),
        );
        let auto_include_new_synthesized_tools = surface.auto_include_new_synthesized_tools;
        let agent_definition_name = surface.agent_definition_name.clone();
        reconcile_synthesized_visibility(
            &mut surface.visible_tool_names,
            &previous_synthesized,
            &synthesized_names,
            auto_include_new_synthesized_tools,
        );
        crate::tools::toolpacks::strip_packed_from_visible(
            &mut surface.visible_tool_names,
            &agent_definition_name,
        );

        let specs = surface
            .durable_tool_specs
            .iter()
            .cloned()
            .chain(synthesized.iter().map(|tool| Arc::new(tool.spec())))
            .collect::<Vec<_>>();
        let synthesized_tools = Arc::new(synthesized);
        crate::tools::toolpacks::bind_synthesized_pack_registry(&surface.tools, &synthesized_tools);
        let all_tools = surface
            .tools
            .iter()
            .chain(synthesized_tools.iter())
            .map(|tool| tool.as_ref())
            .collect::<Vec<_>>();
        let mut policy = ToolPolicyEngine::build_session_from_refs(
            &surface.agent_definition_name,
            &surface.event_channel,
            "session",
            &self.config.channel_permissions,
            &all_tools,
            &surface.visible_tool_names,
        );
        crate::tools::toolpacks::close_handed_off_packs(
            &mut policy,
            &surface.agent_definition_name,
            &all_tools,
        );
        let visible = super::builder::dedup_visible_tool_specs(
            super::builder::visible_tool_specs_for_policy(
                &specs,
                &surface.visible_tool_names,
                &policy,
            ),
        );
        surface.tool_specs = Arc::new(specs);
        surface.synthesized_tools = synthesized_tools;
        surface.visible_tool_specs = Arc::new(visible);
        surface.tool_policy_session = policy;
    }

    fn drain_host_events(&self) -> bool {
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
    async fn enrich_request(
        &self,
        original_user_message: &str,
        overrides: &super::types::TurnOverrides,
        run_context: &mut OpenHumanRunContext,
    ) -> String {
        let mut context = String::new();
        self.append_recall_lanes(original_user_message, &mut context)
            .await;

        let active_goal = if overrides.suppress_active_goal {
            None
        } else {
            let loaded = crate::agent::goals::runtime::load_for_thread(
                &self.workspace_dir,
                self.thread_id.as_deref(),
            )
            .await;
            match loaded {
                Some(goal)
                    if matches!(goal.status, crate::agent::goals::ThreadGoalStatus::Paused) =>
                {
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
            if let Some(block) = tinyagents_graph::goals::active_goal_context_block(goal) {
                context.push_str(&block);
            }
            if let Some(hook) = crate::agent::goals::runtime::GoalBudgetStopHook::for_goal(
                &self.workspace_dir,
                goal,
            ) {
                run_context.stop_hooks.push(Arc::new(hook));
            }
        }
        if let Some(block) =
            crate::agent::orchestration::running_subagents::active_subagents_context_block(
                &self.event_session_id,
                &self.workspace_dir,
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
        enriched = self
            .inject_agent_experience_context(original_user_message, enriched)
            .await;

        let parent = self.parent_context();
        let (enriched_with_memory_agent, memory_agent_context_injected) = self
            .inject_triggered_memory_agent_context(
                original_user_message,
                enriched,
                &parent,
                overrides.suppress_memory_agent,
            )
            .await;
        enriched = enriched_with_memory_agent;
        let mut prepared_sources = Vec::new();
        if memory_agent_context_injected {
            prepared_sources.push(crate::agent::harness::AgentContextPreparedSource {
                source: "memory agent context retrieval".to_string(),
                has_enough_context: None,
            });
        }
        if !prepared_sources.is_empty() {
            enriched = format!(
                "{}\n\n{enriched}",
                render_agent_context_status_note(&prepared_sources)
            );
        }
        self.apply_pending_announcements(&mut enriched);

        run_context.parent = Some(parent);
        run_context.prepared_context_sources = Arc::new(prepared_sources);
        run_context.attachment_placeholders = Arc::new(
            crate::agent::multimodal::extract_image_placeholders_in_text(original_user_message),
        );
        run_context.dispatch = Some(Arc::new(
            crate::agent::tinyagents::host::TurnDispatchState::new(
                crate::agent::tinyagents::agent_turn_wall_clock_ms()
                    .map(std::time::Duration::from_millis),
            ),
        ));
        run_context.sandbox_mode = Some(self.sandbox_mode);
        run_context
            .stop_hooks
            .extend(crate::agent::stop_hooks::current_stop_hooks());
        self.context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tick_turn();
        format!(
            "{}\n\n{enriched}",
            crate::agent::prompts::current_datetime_line()
        )
    }

    async fn append_recall_lanes(&self, user_message: &str, context: &mut String) {
        const BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
        let preferences = tokio::time::timeout(
            BUDGET,
            crate::memory::preferences::recall_situational_preferences_on(
                &self.memory,
                user_message,
            ),
        )
        .await
        .unwrap_or_default();
        if !preferences.is_empty() {
            context.push_str("## Relevant preferences\n\n");
            for preference in preferences {
                context.push_str(preference.trim());
                context.push('\n');
            }
            context.push('\n');
        }
        if let Some(auto_recall) = &self.auto_recall {
            if let Some(block) = auto_recall.block_for(user_message).await {
                context.push_str(&block);
            }
        }
    }

    async fn inject_agent_experience_context(
        &self,
        user_message: &str,
        enriched: String,
    ) -> String {
        if !self.learning_enabled {
            return enriched;
        }
        let visible_tools = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .visible_tool_specs
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        let query = crate::agent::experience::ExperienceQuery {
            query: user_message.to_string(),
            tools: visible_tools,
            tags: Vec::new(),
            agent_id: Some(self.agent_definition_id.clone()).filter(|id| !id.trim().is_empty()),
            entrypoint: Some(self.event_channel.clone())
                .filter(|channel| !channel.trim().is_empty()),
            max_hits: 3,
        };
        let stores = vec![crate::agent::experience::AgentExperienceStore::new(
            self.memory.clone(),
        )];
        match crate::agent::experience::retrieve_across_stores(&stores, query).await {
            Ok(hits) => {
                let matched: Vec<_> = hits
                    .into_iter()
                    .filter(|hit| !hit.match_reasons.is_empty())
                    .collect();
                let block = crate::agent::experience::render_experience_hits(&matched, 2048);
                if block.is_empty() {
                    enriched
                } else {
                    crate::agent::experience::prepend_experience_block(&enriched, &block)
                }
            }
            Err(error) => {
                log::warn!("[agent-experience] retrieval failed (non-fatal): {error}");
                enriched
            }
        }
    }

    fn parent_context(&self) -> crate::agent::harness::ParentExecutionContext {
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::agent::harness::ParentExecutionContext {
            agent_definition_id: self.agent_definition_id.clone(),
            allowed_subagent_ids: self.allowed_subagent_ids.clone(),
            turn_model_source: self.turn_model_source.clone(),
            all_tools: surface.tools.clone(),
            all_tool_specs: surface.durable_tool_specs.clone(),
            visible_tool_names: surface.visible_tool_names.clone(),
            visible_tool_specs: surface.visible_tool_specs.clone(),
            subagent_tool_ceiling_names: self.subagent_tool_ceiling_names.clone(),
            model_name: self.model_name.clone(),
            temperature: self.temperature,
            workspace_dir: self.workspace_dir.clone(),
            workspace_descriptor: crate::agent::harness::current_parent()
                .and_then(|parent| parent.workspace_descriptor)
                .or_else(|| self.workspace_descriptor.clone()),
            memory: self.memory.clone(),
            agent_config: self.config.clone(),
            workflows: Arc::new(
                self.mutable
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .workflows
                    .clone(),
            ),
            memory_context: Arc::new(
                self.mutable
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .last_memory_context
                    .clone(),
            ),
            session_id: self.event_session_id.clone(),
            channel: self.event_channel.clone(),
            connected_integrations: self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .connected_integrations
                .clone(),
            tool_call_format: crate::agent::prompts::tool_call_format_from_dialect(
                self.tool_dispatcher.tool_call_format(),
            ),
            session_key: self.session_key.clone(),
            session_parent_prefix: self.session_parent_prefix.clone(),
            on_progress: self.on_progress.clone(),
            run_queue: self.run_queue.clone(),
        }
    }

    async fn inject_triggered_memory_agent_context(
        &self,
        user_message: &str,
        enriched: String,
        parent_context: &crate::agent::harness::ParentExecutionContext,
        force_skip: bool,
    ) -> (String, bool) {
        use crate::agent::harness::definition::TriggerMemoryAgent;
        if force_skip
            || self.trigger_memory_agent != TriggerMemoryAgent::Always
            || self.agent_definition_id == "agent_memory"
        {
            return (enriched, false);
        }
        let Some(registry) = crate::agent::harness::AgentDefinitionRegistry::global() else {
            return (enriched, false);
        };
        let Some(definition) = registry.get("agent_memory").cloned() else {
            return (enriched, false);
        };
        let prompt = format!(
            "Search the user's memory tree and return only context relevant to the next agent turn.\n\nUser prompt:\n{user_message}"
        );
        let options = crate::agent::harness::SubagentRunOptions {
            task_id: Some(format!("mem-trigger-{}", uuid::Uuid::new_v4())),
            model_override: Some(parent_context.model_name.clone()),
            run_context: OpenHumanRunContext::new().with_parent(parent_context.clone()),
            ..Default::default()
        };
        match crate::agent::harness::run_subagent(&definition, &prompt, options).await {
            Ok(outcome) if !outcome.output.trim().is_empty() => (
                format!(
                    "## Memory agent context\n\n{}\n\n---\n\n{enriched}",
                    crate::util::truncate_with_ellipsis(&outcome.output, 8000)
                ),
                true,
            ),
            Ok(_) => (enriched, false),
            Err(error) => {
                log::warn!("[agent_memory:trigger] failed: {error:#}");
                (enriched, false)
            }
        }
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

    async fn finalize_after_durable_commit(&self, receipt: &CommitReceipt<OpenHumanRunContext>) {
        self.flush_user_autosave().await;
        self.mirror_transcript_after_commit(receipt);
        self.spawn_transcript_ingestion_after_commit(receipt);
        self.spawn_session_memory_extraction_after_commit(receipt)
            .await;
    }

    async fn flush_user_autosave(&self) {
        let message = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_user_autosave
            .take();
        let Some(message) = message else {
            return;
        };
        let key = format!("user_msg:{}", uuid::Uuid::new_v4());
        if let Err(error) = self
            .memory
            .store(
                crate::agent::learning::transcript_ingest::CONVERSATION_RAW_NAMESPACE,
                &key,
                &message,
                crate::memory::MemoryCategory::Conversation,
                self.thread_id.as_deref(),
            )
            .await
        {
            log::warn!(
                "[agent_autosave] durable user-message autosave failed key={key} err={error}"
            );
        }
    }

    fn mirror_transcript_after_commit(&self, receipt: &CommitReceipt<OpenHumanRunContext>) {
        let Some(path) = receipt
            .transcript
            .as_ref()
            .map(|commit| commit.path.clone())
        else {
            return;
        };
        if !crate::agent::session_import::live::dual_write_enabled(self.config.session_dual_write) {
            return;
        }
        let Some(stem) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
        else {
            return;
        };
        let workspace = self.workspace_dir.clone();
        tokio::spawn(async move {
            let read_path = path.clone();
            let transcript = tokio::task::spawn_blocking(move || {
                tinyagents_session::transcript::read_transcript(&read_path)
            })
            .await;
            let Ok(Ok(transcript)) = transcript else {
                log::warn!("[session-store] dual-write transcript read-back failed");
                return;
            };
            if let Err(error) =
                crate::agent::session_import::live::write_live_turn(&workspace, &stem, &transcript)
                    .await
            {
                log::warn!("[session-store] dual-write failed stem={stem}: {error:#}");
            }
        });
    }

    fn spawn_transcript_ingestion_after_commit(
        &self,
        receipt: &CommitReceipt<OpenHumanRunContext>,
    ) {
        let Some(path) = receipt
            .transcript
            .as_ref()
            .map(|commit| commit.path.clone())
        else {
            return;
        };
        let memory = self.memory.clone();
        tokio::spawn(async move {
            if let Err(error) = crate::agent::learning::transcript_ingest::ingest_transcript_path(
                memory.as_ref(),
                &path,
            )
            .await
            {
                log::warn!("[transcript_ingest] background ingest failed: {error}");
            }
        });
    }

    async fn spawn_session_memory_extraction_after_commit(
        &self,
        receipt: &CommitReceipt<OpenHumanRunContext>,
    ) {
        let should_extract = self
            .context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .should_extract_session_memory();
        if !should_extract {
            return;
        }
        if let Some(archivist) = &self.archivist_hook {
            archivist.flush_open_segment(&self.event_session_id).await;
        }
        // Resolve all fallible launch inputs before mutating the extraction
        // state.  A missing registry/definition/parent is a no-op, not a
        // permanently "in progress" session-memory extraction.
        let Some(registry) = crate::agent::harness::AgentDefinitionRegistry::global() else {
            return;
        };
        let Some(definition) = registry.get("archivist").cloned() else {
            return;
        };
        let Some(parent) = receipt.options.context.parent.clone() else {
            return;
        };
        let (handle, stats) = {
            let mut context = self
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let stats = context.stats();
            context.mark_session_memory_started();
            (context.session_memory_handle(), stats)
        };
        log::info!(
            "[session_memory] scheduling durable archivist extraction turn={}",
            stats.session_memory_current_turn
        );
        tokio::spawn(async move {
            let result = crate::agent::harness::run_subagent(
                &definition,
                crate::agent::context::ARCHIVIST_EXTRACTION_PROMPT,
                crate::agent::harness::SubagentRunOptions {
                    run_context: OpenHumanRunContext::new().with_parent(parent),
                    ..Default::default()
                },
            )
            .await;
            if let Ok(mut state) = handle.lock() {
                if result.is_ok() {
                    state.mark_extraction_complete();
                } else {
                    state.mark_extraction_failed();
                }
            }
        });
    }
}

/// Reconcile visible delegate names without reopening a caller's explicit
/// restriction. Revoked names disappear; a newly connected name is admitted
/// only for the wildcard belt.
pub(super) fn reconcile_synthesized_visibility(
    visible: &mut std::collections::HashSet<String>,
    previous: &std::collections::HashSet<String>,
    next: &std::collections::HashSet<String>,
    auto_include_new: bool,
) {
    for name in previous.difference(next) {
        visible.remove(name);
    }
    if auto_include_new {
        visible.extend(next.iter().cloned());
    }
}

/// Account only a receipt-backed turn, using the same direct-plus-completed
/// child total the codec attached to the atomic transcript append.
pub(super) async fn account_committed_turn_against_goal(
    workspace_dir: &std::path::Path,
    thread_id: Option<&str>,
    sidecar: &crate::agent::tinyagents::host::run_context::SessionTurnSidecar,
) {
    let child_input = sidecar
        .subagents
        .iter()
        .map(|entry| entry.usage.input_tokens)
        .sum::<u64>();
    let child_output = sidecar
        .subagents
        .iter()
        .map(|entry| entry.usage.output_tokens)
        .sum::<u64>();
    crate::agent::goals::runtime::account_turn_against_goal(
        workspace_dir,
        thread_id,
        sidecar.input_tokens.saturating_add(child_input),
        sidecar.output_tokens.saturating_add(child_output),
        sidecar
            .duration
            .map(|duration| duration.as_secs())
            .unwrap_or_default(),
    )
    .await;
}

/// UI billing projection for a committed turn. It keeps child records for a
/// detailed display while reporting the same all-in totals the codec persists.
pub(super) fn holistic_last_turn_usage(
    sidecar: &crate::agent::tinyagents::host::run_context::SessionTurnSidecar,
) -> crate::agent::tinyagents::host::LastTurnUsage {
    let input_tokens = sidecar
        .subagents
        .iter()
        .fold(sidecar.input_tokens, |total, entry| {
            total.saturating_add(entry.usage.input_tokens)
        });
    let output_tokens = sidecar
        .subagents
        .iter()
        .fold(sidecar.output_tokens, |total, entry| {
            total.saturating_add(entry.usage.output_tokens)
        });
    let cached_input_tokens = sidecar
        .subagents
        .iter()
        .fold(sidecar.cached_input_tokens, |total, entry| {
            total.saturating_add(entry.usage.cached_input_tokens)
        });
    let cost_usd = sidecar
        .subagents
        .iter()
        .fold(sidecar.cost_usd, |total, entry| {
            total + entry.usage.charged_amount_usd
        });
    crate::agent::tinyagents::host::LastTurnUsage {
        input_tokens,
        output_tokens,
        cached_input_tokens,
        cost_usd,
        context_window: sidecar.context_window,
        subagents: sidecar.subagents.clone(),
    }
}

fn sanitize_prelude_entry(content: &str) -> String {
    let value: String = content.trim().chars().take(200).collect();
    if value.contains("Bearer ")
        || value.contains("sk-")
        || value.contains("ghp_")
        || value.contains("-----BEGIN")
    {
        "[redacted: potential secret]".into()
    } else {
        value
    }
}

fn render_agent_context_status_note(
    sources: &[crate::agent::harness::AgentContextPreparedSource],
) -> String {
    let sources = if sources.is_empty() {
        "the OpenHuman harness".to_string()
    } else {
        sources
            .iter()
            .map(|source| source.source.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "## Agent context status\n\nAgent context retrieval/preparation has already run once \
         for this turn in code via {sources}. Do not call `agent_prepare_context` again for \
         general context preparation. Use the prepared context below, and call only specific \
         follow-up tools if a concrete missing detail is required."
    )
}

fn integration_announcement_note(slugs: &[String]) -> Option<String> {
    (!slugs.is_empty()).then(|| format!(
        "[integration update] These integration(s) connected during this conversation and are available right now: {}. \
Use delegate_to_integrations_agent with the matching toolkit slug to act on them immediately — do not tell the user to reconnect or restart.",
        slugs.join(", ")
    ))
}

fn mcp_announcement_note(servers: &[String]) -> Option<String> {
    (!servers.is_empty()).then(|| format!(
        "[MCP update] These MCP server(s) connected during this conversation and are available right now: {}. \
Use the use_mcp_server delegate to act on them immediately — do not tell the user to reconnect or restart.",
        servers.join(", ")
    ))
}

fn skill_announcement_note(skill_ids: &[String]) -> Option<String> {
    (!skill_ids.is_empty()).then(|| format!(
        "[skills update] These skill(s) were installed during this conversation and are available right now: {}. \
They are in your `## Installed Skills` list — run one with `run_skill` immediately; do not tell the user to reinstall or restart.",
        skill_ids.join(", ")
    ))
}

fn skill_retraction_note(skill_ids: &[String]) -> Option<String> {
    (!skill_ids.is_empty()).then(|| format!(
        "[skills retracted] These skill(s) were uninstalled during this conversation and are no longer available: {}. \
Do not attempt to run them with `run_skill` — they have been removed. Tell the user to reinstall if they want to use them again.",
        skill_ids.join(", ")
    ))
}

async fn collect_prelude_tree_roots(
    per_namespace_cap: usize,
    total_cap: usize,
) -> Vec<crate::agent::prompts::NamespaceSummary> {
    use crate::memory::api::provider::MemoryProvider;
    let Ok(guard) = crate::memory::ops::guard::active_memory_guard().await else {
        return Vec::new();
    };
    let Some(tree) = guard.as_tree() else {
        return Vec::new();
    };
    tree.root_summaries_with_caps(per_namespace_cap, total_cap)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| crate::agent::prompts::NamespaceSummary {
            namespace: row.namespace,
            body: row.body,
            updated_at: row.updated_at,
        })
        .collect()
}

impl OpenHumanSessionHost {
    /// Seed a cold runtime session from a host-provided message log.
    ///
    /// The runtime receives both the seed and any subsequent append; the host
    /// does not retain a parallel history cache. This lower-fidelity fallback
    /// intentionally has no raw transcript rows because its source is a
    /// role/content log rather than an authoritative transcript.
    pub fn seed_resume_from_messages(
        &mut self,
        mut messages: Vec<(String, String)>,
        current_user_message: &str,
    ) -> Result<()> {
        if self.runtime_session.is_some() {
            return Ok(());
        }
        if messages.last().is_some_and(|(role, text)| {
            role == "user" && text.trim() == current_user_message.trim()
        }) {
            messages.pop();
        }
        if messages.is_empty() {
            return Ok(());
        }
        self.ensure_runtime_session()?;
        let history = messages
            .into_iter()
            .map(|(role, text)| match role.as_str() {
                "system" => Message::system(text),
                "assistant" | "agent" => Message::assistant(text),
                _ => Message::user(text),
            })
            .collect();
        self.runtime_session
            .as_mut()
            .expect("runtime session initialized")
            .seed_history(history, Vec::new())
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    /// Seed a cold runtime session from the lossless transcript selected for a
    /// conversation thread. The original raw rows travel with the seed so the
    /// next runtime append can reconcile rather than reserialize them.
    pub fn seed_resume_from_thread_transcript(&mut self, thread_id: &str) -> bool {
        self.seed_resume_from_thread_transcript_scoped(thread_id, None)
    }

    pub fn seed_resume_from_thread_transcript_scoped(
        &mut self,
        thread_id: &str,
        agent_id: Option<&str>,
    ) -> bool {
        if self.runtime_session.is_some() {
            return false;
        }
        let Some(handle) = self
            .session_locator()
            .root_for_thread_scoped(thread_id, agent_id)
        else {
            return false;
        };
        let Ok(Some(transcript)) = handle.read_session() else {
            return false;
        };
        let Ok(history) = OpenHumanTranscriptCodec.decode_history(&transcript) else {
            return false;
        };
        if history.is_empty() || self.ensure_runtime_session().is_err() {
            return false;
        }
        self.runtime_session
            .as_mut()
            .expect("runtime session initialized")
            .seed_history(history, transcript.messages)
            .is_ok()
    }

    /// Dispatch one public OpenHuman turn through the neutral runtime.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        self.ensure_runtime_session()?;

        let mut context = OpenHumanRunContext::new();
        context.progress = self.on_progress.clone();
        context.thread_id = self.thread_id.clone();
        context.workspace = self.workspace_descriptor.clone();
        let cancellation = context.cancellation.clone();
        let options = TurnOptions {
            request_id: crate::agent::turn_origin::current_request_id(),
            thread_id: self.thread_id.clone(),
            stream: self.on_progress.is_some(),
            resume: if self
                .runtime_session
                .as_ref()
                .is_some_and(|session| session.history().is_empty())
            {
                ResumeMode::LatestForAgent
            } else {
                ResumeMode::Never
            },
            cancellation,
            run_context: context.into_tinyagents(tinyagents_harness::context::RunConfig::new(
                "openhuman-session",
            )),
        };
        let outcome = self
            .runtime_session
            .as_mut()
            .expect("runtime session initialized")
            .turn(
                SessionTurnRequest::new(Message::user(user_message)),
                options,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(outcome.output.unwrap_or_default())
    }

    fn ensure_runtime_session(&mut self) -> Result<()> {
        if self.runtime_session.is_some() {
            return Ok(());
        }
        let context_mw = TurnContextMiddleware {
            tool_result_budget_bytes: self
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .tool_result_budget_bytes(),
            payload_summarizer: self.payload_summarizer.clone(),
            task_hint: None,
            artifact_store: None,
            tokenjuice_compaction_enabled: self
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .compaction_enabled(),
            tokenjuice_compression: self.tokenjuice_compression,
            runtime_config: self.runtime_config.clone(),
            microcompact_keep_recent: self
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .microcompact_keep_recent(),
            autocompact_enabled: self
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .autocompact_enabled(),
            handoff: None,
            transcript_snapshot: None,
        };
        let driver = Arc::new(OpenHumanSessionDriver::new(
            self.turn_model_source.clone(),
            self.tool_dispatcher.clone(),
            self.model_name.clone(),
            self.temperature,
            self.config.max_tool_iterations,
            self.model_vision,
            self.run_queue.clone(),
            self.workspace_descriptor.clone(),
            self.resolved_definition()
                .map(|definition| definition.sandbox_mode)
                .unwrap_or(crate::agent::harness::definition::SandboxMode::None),
            self.hosted_base.clone(),
            self.agent_definition_id.clone(),
        ));
        let shared = self.runtime_state.clone();
        let resume_target = TranscriptTarget::new(
            self.session_locator(),
            self.runtime_transcript_stem(),
            self.runtime_transcript_meta(),
        )
        .with_resume_agent(self.agent_definition_name.clone());
        {
            let mut state = self
                .runtime_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.resume_target = Some(resume_target);
            state.context_middleware = Some(context_mw.clone());
            state.required_output = self
                .config
                .required_output
                .as_ref()
                .map(crate::agent::tinyagents::config::required_output_from);
            state.prelude = Some(OpenHumanTurnPrelude {
                memory: self.memory.clone(),
                learning_enabled: self.learning_enabled,
                explicit_preferences_enabled: self.explicit_preferences_enabled,
                config: self.config.clone(),
                context: self.context.clone(),
                tool_policy: self.tool_policy.clone(),
                tool_dispatcher: self.tool_dispatcher.clone(),
                workspace_dir: self.workspace_dir.clone(),
                action_dir: self.action_dir.clone(),
                model_name: self.model_name.clone(),
                agent_definition_name: self.agent_definition_name.clone(),
                omit_profile: self.omit_profile,
                omit_memory_md: self.omit_memory_md,
                auto_save: self.auto_save,
                thread_id: self.thread_id.clone(),
                auto_recall: self.auto_recall.clone(),
                agent_definition_id: self.agent_definition_id.clone(),
                event_session_id: self.event_session_id.clone(),
                event_channel: self.event_channel.clone(),
                trigger_memory_agent: self.trigger_memory_agent,
                subagent_tool_ceiling_names: self.subagent_tool_ceiling_names.clone(),
                turn_model_source: self.turn_model_source.clone(),
                temperature: self.temperature,
                workspace_descriptor: self.workspace_descriptor.clone(),
                session_key: self.session_key.clone(),
                session_parent_prefix: self.session_parent_prefix.clone(),
                on_progress: self.on_progress.clone(),
                run_queue: self.run_queue.clone(),
                allowed_subagent_ids: self
                    .resolved_definition()
                    .map(|definition| {
                        definition
                            .subagents
                            .iter()
                            .filter_map(|entry| match entry {
                                crate::agent::harness::definition::SubagentEntry::AgentId(id) => {
                                    Some(id.clone())
                                }
                                crate::agent::harness::definition::SubagentEntry::Skills(
                                    wildcard,
                                ) if wildcard.matches_all() => {
                                    Some("integrations_agent".to_string())
                                }
                                crate::agent::harness::definition::SubagentEntry::Skills(_) => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                sandbox_mode: self
                    .resolved_definition()
                    .map(|definition| definition.sandbox_mode)
                    .unwrap_or(crate::agent::harness::definition::SandboxMode::None),
                runtime_config: self.runtime_config.clone(),
                archivist_hook: self.archivist_hook.clone(),
                tool_surface: Arc::new(std::sync::Mutex::new(OpenHumanTurnToolSurface {
                    tools: self.tools.clone(),
                    synthesized_tools: self.synthesized_tools.clone(),
                    tool_specs: self.tool_specs.clone(),
                    durable_tool_specs: self.durable_tool_specs.clone(),
                    visible_tool_specs: self.visible_tool_specs.clone(),
                    visible_tool_names: self.visible_tool_names.clone(),
                    auto_include_new_synthesized_tools: true,
                    synthesized_tool_names: self.synthesized_tool_names.clone(),
                    tool_policy_session: self.tool_policy_session.clone(),
                    event_session_id: self.event_session_id.clone(),
                    event_channel: self.event_channel.clone(),
                    agent_definition_name: self.agent_definition_name.clone(),
                })),
                mutable: Arc::new(std::sync::Mutex::new(OpenHumanTurnPreludeMutable {
                    last_memory_context: self.last_memory_context.clone(),
                    announced_integrations: self.announced_integrations.clone(),
                    pending_integration_announcement: self.pending_integration_announcement.clone(),
                    announced_mcp_servers: self.announced_mcp_servers.clone(),
                    pending_mcp_announcement: self.pending_mcp_announcement.clone(),
                    announced_skills: self.announced_skills.clone(),
                    pending_skill_announcement: self.pending_skill_announcement.clone(),
                    pending_skill_retraction: self.pending_skill_retraction.clone(),
                    connected_integrations: self.connected_integrations.clone(),
                    connected_integrations_initialized: self.connected_integrations_initialized,
                    workflows: self.workflows.clone(),
                    composio_events: None,
                    skill_events: None,
                    pending_user_autosave: None,
                })),
            });
        }
        let hooks = Arc::new(OpenHumanSessionHooks::new(
            move |_, _, _| {
                let shared = shared.clone();
                Box::pin(async move {
                    Ok(ResumePreparation {
                        transcript: shared
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .resume_target
                            .clone(),
                    })
                })
            },
            {
                let state = self.runtime_state.clone();
                move |request, options, view| {
                    let state = state.clone();
                    let request_base_len = view.history.len()
                        + usize::from(view.history.last() != Some(&request.input));
                    let resumed_prefix = view.resumed.then(|| {
                        view.history
                            .first()
                            .filter(|message| matches!(message, Message::System(_)))
                            .cloned()
                            .map(|message| PrefixSnapshot::new(vec![message]))
                    });
                    Box::pin(async move {
                        let transcript_snapshot =
                            crate::agent::tinyagents::TranscriptSnapshotSink::default();
                        transcript_snapshot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .request_base_len = request_base_len;
                        let prelude = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .prelude
                            .clone();
                        let prelude = prelude.ok_or_else(|| {
                            tinyagents_runtime::RuntimeError::MissingDependency(
                                "OpenHumanTurnPrelude",
                            )
                        })?;
                        prelude
                            .refresh_turn_boundary(!view.resumed && view.history.is_empty())
                            .await;
                        // The driver resolves the same model source, but the
                        // host sidecar needs this metadata before either the
                        // successful or partial runtime append asks the codec
                        // for atomic billing data.
                        let context_window = prelude
                            .turn_model_source
                            .effective_context_window(&prelude.model_name)
                            .await
                            .unwrap_or_default();
                        options
                            .run_context
                            .data
                            .session_sidecar
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .context_window = context_window;
                        let original_user_message = request.input.text();
                        prelude.begin_user_effects(
                            &mut state
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()),
                            request,
                        );
                        let overrides = std::mem::take(
                            &mut state
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .pending_turn_overrides,
                        );
                        let enriched = prelude
                            .enrich_request(
                                &original_user_message,
                                &overrides,
                                &mut options.run_context.data,
                            )
                            .await;
                        request.input = Message::user(enriched);
                        let mut preparation = prelude
                            .prepare(!view.resumed && view.history.is_empty())
                            .await
                            .map_err(|error| {
                                tinyagents_runtime::RuntimeError::Driver(error.to_string())
                            })?;
                        if overrides.suppress_tools {
                            preparation.tools = Some(ToolSnapshot::default());
                        }
                        let (
                            mut current_tools,
                            mut current_synthesized_tools,
                            policy_session,
                            policy_session_id,
                            policy_channel,
                        ) = prelude.current_tool_source();
                        if overrides.suppress_tools {
                            // The execution source must narrow with the wire
                            // snapshot. Leaving instances here would make a
                            // tool-less override advisory instead of a hard
                            // authority boundary.
                            current_tools = Arc::new(Vec::new());
                            current_synthesized_tools = Arc::new(Vec::new());
                        }
                        let mut middleware = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .context_middleware
                            .clone()
                            .ok_or_else(|| {
                                tinyagents_runtime::RuntimeError::MissingDependency(
                                    "TurnContextMiddleware",
                                )
                            })?;
                        middleware.transcript_snapshot = Some(transcript_snapshot);
                        options.run_context.data.context_middleware = Some(middleware);
                        options.run_context.data.current_tools = Some(current_tools);
                        options.run_context.data.current_synthesized_tools =
                            Some(current_synthesized_tools);
                        options.run_context.data.tool_policy =
                            Some(crate::agent::tinyagents::ToolPolicyEnforcement {
                                policy: prelude.tool_policy.clone(),
                                session: policy_session,
                                session_id: policy_session_id,
                                channel: policy_channel,
                                // This is the stable agent definition key
                                // used for driver diagnostics and policy
                                // enforcement. The mutable surface carries
                                // its current display name separately when it
                                // rebuilds the policy session.
                                agent_definition_id: prelude.agent_definition_id.clone(),
                            });
                        options.run_context.data.required_output = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .required_output
                            .clone();
                        if let Some(prefix) = resumed_prefix {
                            preparation.prefix = prefix;
                        }
                        Ok(preparation)
                    })
                }
            },
            |outcome, _| {
                Box::pin(async move {
                    if outcome
                        .output
                        .as_deref()
                        .is_none_or(|output| output.trim().is_empty())
                    {
                        return Err(tinyagents_runtime::RuntimeError::Driver(
                            "driver returned an empty final response after repair".into(),
                        ));
                    }
                    Ok(())
                })
            },
            {
                let state = self.runtime_state.clone();
                let progress = self.on_progress.clone();
                let post_turn_hooks = self.post_turn_hooks.clone();
                let session_id = self.event_session_id.clone();
                let agent_id = self.agent_definition_id.clone();
                let channel = self.event_channel.clone();
                move |receipt| {
                    let state = state.clone();
                    let progress = progress.clone();
                    let post_turn_hooks = post_turn_hooks.clone();
                    let session_id = session_id.clone();
                    let agent_id = agent_id.clone();
                    let channel = channel.clone();
                    Box::pin(async move {
                        let iterations = receipt
                            .outcome
                            .history
                            .iter()
                            .filter(|message| matches!(message, Message::Assistant(_)))
                            .count()
                            .max(1) as u32;
                        let output = receipt.outcome.output.clone().unwrap_or_default();
                        let input = receipt
                            .outcome
                            .history
                            .iter()
                            .rev()
                            .find_map(|message| match message {
                                Message::User(_) => Some(message.text()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        let sidecar = receipt
                            .options
                            .context
                            .session_sidecar
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone();
                        let usage = holistic_last_turn_usage(&sidecar);
                        let interrupted = sidecar.hit_cap || receipt.outcome.interrupted;
                        let tool_calls = sidecar
                            .tool_outcomes
                            .iter()
                            .map(|outcome| crate::agent::hooks::ToolCallRecord {
                                name: outcome.name.clone(),
                                arguments: outcome.arguments.clone(),
                                success: outcome.success,
                                output_summary: crate::agent::hooks::sanitize_tool_output(
                                    &outcome.content,
                                    &outcome.name,
                                    outcome.success,
                                ),
                                duration_ms: outcome.duration_ms,
                            })
                            .collect::<Vec<_>>();
                        let turn_duration_ms = sidecar
                            .duration
                            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                            .unwrap_or_default();
                        let prelude = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .prelude
                            .clone();
                        let citations = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .pending_citations
                            .take();
                        if let Some(prelude) = prelude {
                            // `after_commit` is only reached after runtime
                            // transcript durability. Every host write below is
                            // therefore receipt-gated.
                            prelude.finalize_after_durable_commit(&receipt).await;
                            // Account the same committed direct + child totals
                            // that the codec atomically attached to the
                            // transcript. A continuation's suppression state
                            // is intentionally cleared by the goals runtime
                            // only after this receipt exists.
                            account_committed_turn_against_goal(
                                &prelude.workspace_dir,
                                receipt.options.context.thread_id.as_deref(),
                                &sidecar,
                            )
                            .await;
                        }
                        // Citations are display-only, but their result belongs
                        // to this committed turn. Join only after durability so
                        // a failed/cancelled candidate never becomes the UI's
                        // "last turn" citation set.
                        let citations = match citations {
                            Some(task) => task.await.unwrap_or_default(),
                            None => Vec::new(),
                        };
                        state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .last_commit = Some(receipt);
                        let mut state = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        state.last_turn_hit_cap = interrupted;
                        state.last_turn_usage = Some(usage);
                        state.last_turn_citations = citations;
                        if let Some(progress) = &progress {
                            let _ = progress.try_send(
                                crate::agent::progress::AgentProgress::TurnContent {
                                    input: Some(input.clone()),
                                    output: Some(output.clone()),
                                },
                            );
                            let _ = progress.try_send(
                                crate::agent::progress::AgentProgress::TurnCompleted { iterations },
                            );
                        }
                        crate::agent::hooks::fire_hooks(
                            &post_turn_hooks,
                            crate::agent::hooks::TurnContext {
                                user_message: input,
                                assistant_response: output,
                                tool_calls,
                                turn_duration_ms,
                                session_id: Some(session_id.clone()),
                                agent_id: Some(agent_id.clone()),
                                entrypoint: Some(channel.clone()),
                                iteration_count: iterations as usize,
                            },
                        );
                        Ok(())
                    })
                }
            },
            {
                let state = self.runtime_state.clone();
                move |terminal| {
                    let state = state.clone();
                    Box::pin(async move {
                        state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .terminals
                            .push(terminal);
                        Ok(())
                    })
                }
            },
        ));
        self.runtime_session = Some(
            SessionBuilder::new(driver)
                .codec(Arc::new(OpenHumanTranscriptCodec))
                .hooks(hooks)
                .build()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?,
        );
        Ok(())
    }

    /// Publish a caller-driven visibility/policy mutation into the live hook
    /// state. The runtime keeps generic snapshots immutable per request; this
    /// simply replaces the host composition from which the *next* snapshot,
    /// execution source, and enforcement object are built.
    pub(in crate::agent::session_host) fn sync_runtime_tool_surface(&self) {
        self.sync_runtime_tool_surface_with_auto_include(None);
    }

    /// Publish a visibility mutation and record whether future dynamically
    /// connected delegates may enter the surface without an explicit allow.
    pub(in crate::agent::session_host) fn sync_runtime_tool_surface_with_auto_include(
        &self,
        auto_include_new_synthesized_tools: Option<bool>,
    ) {
        let prelude = self
            .runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .prelude
            .clone();
        let Some(prelude) = prelude else {
            return;
        };
        let prior_auto_include = prelude
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .auto_include_new_synthesized_tools;
        prelude.replace_tool_surface(OpenHumanTurnToolSurface {
            tools: self.tools.clone(),
            synthesized_tools: self.synthesized_tools.clone(),
            tool_specs: self.tool_specs.clone(),
            durable_tool_specs: self.durable_tool_specs.clone(),
            visible_tool_specs: self.visible_tool_specs.clone(),
            visible_tool_names: self.visible_tool_names.clone(),
            auto_include_new_synthesized_tools: auto_include_new_synthesized_tools
                .unwrap_or(prior_auto_include),
            synthesized_tool_names: self.synthesized_tool_names.clone(),
            tool_policy_session: self.tool_policy_session.clone(),
            event_session_id: self.event_session_id.clone(),
            event_channel: self.event_channel.clone(),
            agent_definition_name: self.agent_definition_name.clone(),
        });
    }

    fn runtime_transcript_stem(&self) -> String {
        match &self.session_parent_prefix {
            Some(prefix) => format!("{prefix}__{}", self.session_key),
            None => self.session_key.clone(),
        }
    }

    fn session_locator(&self) -> Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
        self.session_history_locator.clone().unwrap_or_else(|| {
            Arc::new(tinyagents_session::transcript::FileTranscriptLocator::new(
                self.workspace_dir.clone(),
            ))
        })
    }

    fn runtime_transcript_meta(&self) -> TranscriptMeta {
        let now = chrono::Utc::now().to_rfc3339();
        TranscriptMeta {
            agent_name: self.agent_definition_name.clone(),
            agent_id: Some(self.agent_definition_id.clone()),
            agent_type: Some(if self.session_parent_prefix.is_some() {
                "subagent".into()
            } else {
                "root".into()
            }),
            dispatcher: if self.tool_dispatcher.should_send_tool_specs() {
                "native".into()
            } else {
                "xml".into()
            },
            provider: None,
            model: Some(self.model_name.clone()),
            created: now.clone(),
            updated: now,
            turn_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            charged_amount_usd: 0.0,
            thread_id: self.thread_id.clone(),
            task_id: None,
        }
    }
}
