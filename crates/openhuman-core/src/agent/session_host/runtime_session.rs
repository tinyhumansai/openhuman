//! The live OpenHuman composition over `tinyagents_runtime::Session`.
//!
//! The mutex is intentionally narrow: it carries host preparation and
//! finalization observations only. Generic model history, transcript rows,
//! prefix reconciliation, tool snapshots, resume and persistence remain inside
//! the runtime session.

#[path = "runtime_session_attachment_input.rs"]
mod attachment_input;
mod memory_ingest;
mod permanent;
mod post_commit;
mod tool_rules;
mod transcript;
#[path = "runtime_session_turn.rs"]
mod turn;
pub(super) use turn::begin_turn_resume;

use std::sync::Arc;

use anyhow::Result;
use tinyagents_runtime::{
    CommitReceipt, ResumePreparation, SessionBuilder, SessionTerminal, ToolSnapshot,
    TranscriptTarget, TurnPreparation,
};
use tinyinference_llm::message::Message;

use crate::agent::{
    message_convert::{user_message_from_text, user_text_with_markers},
    session_host::{
        driver::OpenHumanSessionDriver, OpenHumanSessionHooks, OpenHumanTranscriptCodec,
    },
    tinyagents::{host::OpenHumanRunContext, TurnContextMiddleware},
};

use super::announcement_notes::{
    integration_announcement_note, mcp_announcement_note, skill_announcement_note,
    skill_retraction_note,
};
use super::types::OpenHumanSessionHost;

#[path = "runtime_session_progress.rs"]
mod progress;

/// Mutable product state observed by the runtime hooks.
///
/// This type has no message accumulator, raw transcript rows, prefix matching,
/// resume cache, or persistence handle. Those are exclusively `Session` state.
#[derive(Default)]
pub(super) struct OpenHumanSessionState {
    terminals: Vec<SessionTerminal>,
    pub(super) last_turn_hit_cap: bool,
    pub(super) last_turn_usage: Option<crate::agent::tinyagents::host::LastTurnUsage>,
    pub(super) context_middleware: Option<TurnContextMiddleware>,
    required_output: Option<tinyagents_harness::config::RequiredOutput>,
    pub(crate) pending_turn_overrides: super::types::TurnOverrides,
    pub(super) active_turn_overrides: super::types::TurnOverrides,
    pub(super) reply_language_directive: Option<String>,
    pub(super) time_zone: Option<String>,
    prelude: Option<OpenHumanTurnPrelude>,
    /// The previous turn's deferred post-commit work (thread-goal
    /// accounting). The next turn awaits it before loading the goal
    /// (`post_commit::await_pending`).
    pending_post_commit: Option<tokio::task::JoinHandle<()>>,
}

/// Owned host-only inputs used by the async runtime preparation hook.
/// It deliberately excludes transcript/history/tool snapshots, which belong to
/// `tinyagents_runtime::Session`.
#[derive(Clone)]
struct OpenHumanTurnPrelude {
    config: crate::config::AgentConfig,
    /// Host prompt/utilisation state, shared by the runtime hooks. It is not
    /// generic conversation state and the runtime never persists it.
    context: Arc<std::sync::Mutex<crate::agent::context::ContextManager>>,
    tool_policy: Arc<dyn crate::agent::tool_policy::ToolPolicy>,
    tool_dispatcher: Arc<dyn tinytools_agent::dialect::ToolDialect>,
    workspace_dir: std::path::PathBuf,
    action_dir: std::path::PathBuf,
    model_name: String,
    agent_definition_name: String,
    /// No per-turn memory pack for this agent (definition's
    /// `omit_memory_context`); its turns are still logged.
    omit_memory_context: bool,
    thread_id: Option<String>,
    agent_definition_id: String,
    event_session_id: String,
    event_channel: String,
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
    definition: Option<Arc<crate::agent::harness::definition::AgentDefinition>>,
    runtime_config: Option<Arc<crate::config::Config>>,
    /// This session's tool-rule layers; see `tool_rules.rs`.
    tool_rules: Arc<tinytools::ToolRuleSet>,
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
    /// Registered but never advertised: the `Deferred` tools the harness's
    /// `tool_search` bridge can reach. Part of the snapshot the driver treats
    /// as the final allowlist, and classified `Allow` by the policy, so a
    /// found tool is callable. See `OpenHumanSessionHost::deferred_tool_names`.
    deferred_tool_names: std::collections::HashSet<String>,
    permanent_tool_names: std::collections::HashSet<String>,
    /// Whether this belt reaches deferred tools at all; fixed at build.
    discovery_enabled: bool,
    /// The definition's own `deferred_tools`; see `meta::deferred_set`.
    requested_deferred_tools: Arc<[String]>,
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

/// Per-session product observations formerly scattered across the legacy
/// `core_turn` loop. Generic history, raw transcript and prefix state are not here.
#[derive(Default)]
struct OpenHumanTurnPreludeMutable {
    last_memory_context: Option<String>,
    announced_integrations: std::collections::HashSet<String>,
    pending_integration_announcement: Vec<String>,
    announced_mcp_servers: std::collections::HashSet<String>,
    pending_mcp_announcement: Vec<String>,
    /// Live MCP tool definitions for this workspace, refreshed before each
    /// turn so disconnects remove their deferred executors immediately.
    #[cfg(feature = "mcp")]
    connected_mcp_tools: Vec<crate::mcp::registry::types::ConnectedServerOverview>,
    /// `mcp_*` tool names a resumed thread was sent, so a tool recorded under
    /// its pre-readable hashed name is restored under that name too.
    #[cfg(feature = "mcp")]
    recorded_mcp_tool_names: std::collections::HashSet<String>,
    announced_skills: std::collections::HashSet<String>,
    pending_skill_announcement: Vec<String>,
    pending_skill_retraction: Vec<String>,
    connected_integrations: Vec<crate::agent::prompts::ConnectedIntegration>,
    connected_integrations_initialized: bool,
    connected_integrations_authoritative: bool,
    /// A cold hydration seeded the announced sets; later ones diff instead.
    integration_announcements_seeded: bool,
    /// Integration action declarations this thread was already sent, restored by
    /// the tinyagents session on resume. Rebuilt into deferred executors whenever
    /// the live integrations list does not supply them (see `recorded_tools`).
    recorded_integration_actions: Vec<tinytools::ToolSpec>,
    /// TinyJuice REPL declarations restored by a resumed thread. These are
    /// rebuilt independently of the current setting that creates new handles.
    recorded_repl_tools: Vec<tinytools::ToolSpec>,
    /// Distinguishes a new session from a resumed session whose recorded tool
    /// snapshot simply had no TinyJuice declarations.
    recorded_tool_snapshot_adopted: bool,
    workflows: Vec<crate::skills::Workflow>,
    composio_events: Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    skill_events: Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    /// The user-authored text of the in-flight turn, held until commit so the
    /// committed turn can be handed to memory.
    pending_user_text: Option<String>,
    /// The in-flight turn's memory identity and index, from pre-turn to
    /// commit (`memory_ingest`).
    pending_memory_turn: Option<memory_ingest::PendingMemoryTurn>,
}

impl OpenHumanTurnPrelude {
    #[allow(clippy::type_complexity)]
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

    /// The session's deferred set for this turn; see
    /// `OpenHumanRunContext::deferred_tool_names`.
    fn current_deferred_tool_names(&self) -> std::collections::HashSet<String> {
        self.tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .deferred_tool_names
            .clone()
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
                            || !crate::agent::tinyagents::harness_tool_registration::is_thread_goal_tool(
                                &spec.name,
                            )
                    })
                    .map(|spec| spec.as_ref().clone())
                    .collect(),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
        };
        let prefix = if cold {
            let tiered = self.build_system_prompt_tiered()?;
            Some(super::prefix_snapshot::tiered_prefix_snapshot(&tiered))
        } else {
            None
        };
        Ok(TurnPreparation {
            prefix,
            tools: Some(tools),
        })
    }

    fn build_system_prompt_tiered(&self) -> Result<crate::agent::prompts::TieredPrompt> {
        use crate::agent::prompts::{tool_call_format_from_dialect, PromptContext, PromptTool};
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
                    || !crate::agent::tinyagents::harness_tool_registration::is_thread_goal_tool(
                        &spec.name,
                    )
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
                    || !crate::agent::tinyagents::harness_tool_registration::is_thread_goal_tool(
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
                !crate::agent::tinyagents::harness_tool_registration::is_thread_goal_tool(name)
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
            tools: &prompt_tools,
            workflows: &mutable.workflows,
            dispatcher_instructions: &instructions,
            visible_tool_names: &visible_tool_names,
            tool_call_format: tool_call_format_from_dialect(
                self.tool_dispatcher.tool_call_format(),
            ),
            connected_integrations: &mutable.connected_integrations,
            connected_identities_md: crate::agent::prompts::render_connected_identities(),
            user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
            personality_roster: vec![],
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };
        self.context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .build_system_prompt_tiered(&context)
    }

    #[cfg(test)]
    fn synthesized_tool_names_for_test(&self) -> std::collections::HashSet<String> {
        self.tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .synthesized_tool_names
            .clone()
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
        turn: &super::types::TurnInputs,
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
        // Build the roster from this turn's *effective* visible tool set
        // (snapshotted under the tool-surface lock, then released) rather
        // than the parent definition's static scope alone: a hide or named
        // restriction can narrow what this turn can actually call below the
        // definition's baseline, and the roster must not advertise a fleet
        // control the turn cannot invoke.
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
        run_context.attach_parent(self.parent_context());
        self.apply_pending_announcements(&mut enriched);

        run_context.attachment_placeholders = Arc::new(
            crate::agent::multimodal::extract_image_placeholders_in_text(original_user_message),
        );
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
        run_context.tool_dialect = crate::agent::prompts::tool_call_format_from_dialect(
            self.tool_dispatcher.tool_call_format(),
        )
        .harness_dispatcher();
        format!("{}\n\n{enriched}", turn.preamble())
    }

    fn parent_context(&self) -> crate::agent::harness::ParentExecutionContext {
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (workflows, memory_context, connected_integrations) = {
            let mutable = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                mutable.workflows.clone(),
                mutable.last_memory_context.clone(),
                mutable.connected_integrations.clone(),
            )
        };
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
            agent_config: self.config.clone(),
            workflows: Arc::new(workflows),
            memory_context: Arc::new(memory_context),
            session_id: self.event_session_id.clone(),
            channel: self.event_channel.clone(),
            connected_integrations,
            tool_call_format: crate::agent::prompts::tool_call_format_from_dialect(
                self.tool_dispatcher.tool_call_format(),
            ),
            session_key: self.session_key.clone(),
            session_parent_prefix: self.session_parent_prefix.clone(),
            on_progress: self.on_progress.clone(),
            run_queue: self.run_queue.clone(),
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
        self.mirror_transcript_after_commit(receipt);
        self.memory_post_turn(receipt);
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
            if let Err(error) = tinyagents_session::transcript::import::live::write_live_turn(
                &workspace,
                &stem,
                &transcript,
                crate::agent::session_import::projector::journal_message_from_transcript,
            )
            .await
            {
                log::warn!("[session-store] dual-write failed stem={stem}: {error:#}");
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

#[path = "runtime_session_usage.rs"]
mod usage;
pub(super) use usage::{account_committed_turn_against_goal, holistic_last_turn_usage};

impl OpenHumanSessionHost {
    pub(super) fn update_runtime_prelude_progress(
        &mut self,
        tx: Option<tokio::sync::mpsc::Sender<crate::agent::progress::AgentProgress>>,
    ) {
        if let Some(prelude) = self
            .runtime_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prelude
            .as_mut()
        {
            prelude.on_progress = tx;
        }
    }

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

    /// Dispatch one public OpenHuman turn through the neutral runtime.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        self.turn_with_origin(user_message, None).await
    }

    pub(in crate::agent::session_host) fn ensure_runtime_session(&mut self) -> Result<()> {
        if self.runtime_session.is_some() {
            return Ok(());
        }
        let (
            tool_result_budget_bytes,
            tokenjuice_compaction_enabled,
            microcompact_keep_recent,
            autocompact_enabled,
            compaction,
        ) = {
            let context = self
                .context
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                context.tool_result_budget_bytes(),
                context.compaction_enabled(),
                context.microcompact_keep_recent(),
                context.autocompact_enabled(),
                context.compaction(),
            )
        };
        let artifact_store = super::artifact_wiring::build_artifact_store(
            &self.workspace_dir,
            self.workspace_descriptor.as_ref(),
            &self.action_dir,
            &self.event_session_id,
        );

        let context_mw = TurnContextMiddleware {
            tool_result_budget_bytes,
            payload_summarizer: self.payload_summarizer.clone(),
            // Rooted outside the project; see `artifact_wiring` (#6408, #6483).
            artifact_store: Some(artifact_store),
            tokenjuice_compaction_enabled,
            tokenjuice_compression: self.tokenjuice_compression,
            runtime_config: self.runtime_config.clone(),
            microcompact_keep_recent,
            autocompact_enabled,
            compaction,
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
        // A thread-bound root session addresses its transcript by durable
        // identity, so a restart appends to the conversation's own file rather
        // than minting a new stem and resuming whichever one happens to be
        // newest. Everything else — sub-agents, unthreaded CLI turns — keeps
        // the stem path, where a fresh transcript per run is correct.
        // The builder's initial binding and the per-turn resume hook must
        // share one locator allocation. The runtime compares locator identity
        // once a durable transcript is bound; separately constructed locators
        // for the same workspace reject the first turn after a cold resume.
        let session_locator = self.session_locator();
        let resume_target = match self.session.clone() {
            Some(session) => TranscriptTarget::for_session(
                session_locator.clone(),
                session,
                self.runtime_transcript_meta(),
            ),
            None => TranscriptTarget::new(
                session_locator.clone(),
                self.runtime_transcript_stem(),
                self.runtime_transcript_meta(),
            )
            .with_resume_agent(self.agent_definition_name.clone()),
        };
        {
            let mut state = self
                .runtime_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.context_middleware = Some(context_mw.clone());
            state.required_output = self
                .config
                .required_output
                .as_ref()
                .map(crate::agent::tinyagents::config::required_output_from);
            state.prelude = Some(OpenHumanTurnPrelude {
                config: self.config.clone(),
                context: self.context.clone(),
                tool_policy: self.tool_policy.clone(),
                tool_dispatcher: self.tool_dispatcher.clone(),
                workspace_dir: self.workspace_dir.clone(),
                action_dir: self.action_dir.clone(),
                model_name: self.model_name.clone(),
                agent_definition_name: self.agent_definition_name.clone(),
                omit_memory_context: self.omit_memory_context,
                thread_id: self.thread_id.clone(),
                agent_definition_id: self.agent_definition_id.clone(),
                event_session_id: self.event_session_id.clone(),
                event_channel: self.event_channel.clone(),
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
                    .map(|definition| definition.allowed_subagent_ids().into_iter().collect())
                    .unwrap_or_default(),
                sandbox_mode: self
                    .resolved_definition()
                    .map(|definition| definition.sandbox_mode)
                    .unwrap_or(crate::agent::harness::definition::SandboxMode::None),
                definition: self.resolved_definition(),
                runtime_config: self.runtime_config.clone(),
                tool_rules: self.session_tool_rules(),
                tool_surface: Arc::new(std::sync::Mutex::new(OpenHumanTurnToolSurface {
                    tools: self.tools.clone(),
                    synthesized_tools: self.synthesized_tools.clone(),
                    tool_specs: self.tool_specs.clone(),
                    durable_tool_specs: self.durable_tool_specs.clone(),
                    visible_tool_specs: self.visible_tool_specs.clone(),
                    visible_tool_names: self.visible_tool_names.clone(),
                    deferred_tool_names: self.deferred_tool_names.clone(),
                    permanent_tool_names: self.permanent_tool_names.clone(),
                    discovery_enabled: self.discovery_enabled,
                    requested_deferred_tools: self.requested_deferred_tools.clone(),
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
                    #[cfg(feature = "mcp")]
                    connected_mcp_tools: Vec::new(),
                    #[cfg(feature = "mcp")]
                    recorded_mcp_tool_names: std::collections::HashSet::new(),
                    announced_skills: self.announced_skills.clone(),
                    pending_skill_announcement: self.pending_skill_announcement.clone(),
                    pending_skill_retraction: self.pending_skill_retraction.clone(),
                    connected_integrations: self.connected_integrations.clone(),
                    connected_integrations_initialized: self.connected_integrations_initialized,
                    // Builder-provided integrations are not yet verified this session.
                    connected_integrations_authoritative: false,
                    integration_announcements_seeded: false,
                    recorded_integration_actions: Vec::new(),
                    recorded_repl_tools: Vec::new(),
                    recorded_tool_snapshot_adopted: false,
                    workflows: self.workflows.clone(),
                    composio_events: None,
                    skill_events: None,
                    pending_user_text: None,
                    pending_memory_turn: None,
                })),
            });
        }
        let hooks = Arc::new(OpenHumanSessionHooks::new(
            {
                let resume_target = resume_target.clone();
                let state = self.runtime_state.clone();
                move |_, options, _| {
                    let resume_target = resume_target.clone();
                    begin_turn_resume(
                        &mut state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()),
                        &mut options.resume,
                    );
                    Box::pin(async move {
                        Ok(ResumePreparation {
                            transcript: Some(resume_target),
                        })
                    })
                }
            },
            {
                let state = self.runtime_state.clone();
                move |request, options, view| {
                    let state = state.clone();
                    let request_base_len = view.history.len()
                        + usize::from(view.history.last() != Some(&request.input));
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
                        let prelude = prelude.ok_or({
                            tinyagents_runtime::RuntimeError::MissingDependency(
                                "OpenHumanTurnPrelude",
                            )
                        })?;
                        // The previous turn's goal accounting was deferred past
                        // its completion; settle it before this turn reads the goal.
                        let pending_post_commit = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .pending_post_commit
                            .take();
                        post_commit::await_pending(pending_post_commit).await;
                        let new_session = !view.resumed && view.history.is_empty();
                        prelude
                            .refresh_turn_boundary(new_session)
                            .await
                            .map_err(|error| {
                                tinyagents_runtime::RuntimeError::Driver(error.to_string())
                            })?;
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
                        let original_user_message = user_text_with_markers(&request.input);
                        prelude.begin_user_effects(request);
                        let turn = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take_turn_inputs();
                        let origin = options.run_context.data.origin.clone();
                        let (enriched, memory_turn) = futures::join!(
                            prelude.enrich_request(
                                &original_user_message,
                                &turn,
                                &mut options.run_context.data,
                            ),
                            // Boxed: the hook's future (config load, engine
                            // calls) would otherwise be inlined into this
                            // already-large hook future.
                            Box::pin(prelude.memory_pre_turn(
                                view.history,
                                view.committed_turns,
                                view.history.last().filter(|last| **last == request.input),
                                origin,
                            )),
                        );
                        options.run_context.data.memory_turn = memory_turn;
                        request.input = user_message_from_text(&enriched);
                        let mut preparation =
                            prelude.prepare(new_session).await.map_err(|error| {
                                tinyagents_runtime::RuntimeError::Driver(error.to_string())
                            })?;
                        prelude.refresh_permanent_prefix(&mut preparation, view.prefix);
                        if turn.overrides.suppress_tools {
                            // One-off tool-less turn: must not become the
                            // thread's recorded tool list.
                            preparation.tools = Some(ToolSnapshot::default().exact());
                        }
                        let (
                            mut current_tools,
                            mut current_synthesized_tools,
                            policy_session,
                            policy_session_id,
                            policy_channel,
                        ) = prelude.current_tool_source();
                        if turn.overrides.suppress_tools {
                            current_tools = Arc::new(Vec::new());
                            current_synthesized_tools = Arc::new(Vec::new());
                        }
                        let mut middleware = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .context_middleware
                            .clone()
                            .ok_or({
                                tinyagents_runtime::RuntimeError::MissingDependency(
                                    "TurnContextMiddleware",
                                )
                            })?;
                        middleware.transcript_snapshot = Some(transcript_snapshot);
                        options.run_context.data.context_middleware = Some(middleware);
                        options.run_context.data.current_tools = Some(current_tools);
                        if !turn.overrides.suppress_tools {
                            options.run_context.data.deferred_tool_names =
                                Arc::new(prelude.current_deferred_tool_names());
                        }
                        options.run_context.data.current_synthesized_tools =
                            Some(current_synthesized_tools);
                        options.run_context.data.tool_rules =
                            prelude.turn_tool_rules(&policy_channel, &options.run_context.data);
                        options.run_context.data.tool_policy =
                            Some(crate::agent::tinyagents::ToolPolicyEnforcement {
                                policy: prelude.tool_policy.clone(),
                                session: policy_session,
                                session_id: policy_session_id,
                                channel: policy_channel,
                                agent_definition_id: prelude.agent_definition_id.clone(),
                            });
                        options.run_context.data.required_output = state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .required_output
                            .clone();
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
                let post_turn_hooks = self.post_turn_hooks.clone();
                let session_id = self.event_session_id.clone();
                let agent_id = self.agent_definition_id.clone();
                let channel = self.event_channel.clone();
                move |receipt| {
                    let state = state.clone();
                    let post_turn_hooks = post_turn_hooks.clone();
                    let session_id = session_id.clone();
                    let agent_id = agent_id.clone();
                    let channel = channel.clone();
                    Box::pin(async move {
                        post_commit::finalize_committed_turn(
                            &state,
                            &post_turn_hooks,
                            post_commit::TurnIdentity {
                                session_id,
                                agent_id,
                                channel,
                            },
                            receipt,
                        )
                        .await;
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
        // Bind the transcript at construction for a thread-bound session,
        // rather than leaving it to the per-turn `before_resume` hook. The
        // hook still supplies the same target, but binding it here means the
        // session knows its own durable destination before any turn runs —
        // which is what lets a host read the conversation back without
        // driving a provider first.
        let mut builder = SessionBuilder::new(driver)
            .codec(Arc::new(OpenHumanTranscriptCodec))
            .hooks(hooks)
            .retain_recorded_tools(
                !self.host_only && crate::agent::tool_snapshot_scope::retain_recorded_tools(),
            );
        if let Some(session) = self.session.clone() {
            builder = builder.session(session_locator, session, self.runtime_transcript_meta());
        }
        self.runtime_session = Some(
            builder
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
            deferred_tool_names: self.deferred_tool_names.clone(),
            permanent_tool_names: self.permanent_tool_names.clone(),
            discovery_enabled: self.discovery_enabled,
            requested_deferred_tools: self.requested_deferred_tools.clone(),
            auto_include_new_synthesized_tools: auto_include_new_synthesized_tools
                .unwrap_or(prior_auto_include),
            synthesized_tool_names: self.synthesized_tool_names.clone(),
            tool_policy_session: self.tool_policy_session.clone(),
            event_session_id: self.event_session_id.clone(),
            event_channel: self.event_channel.clone(),
            agent_definition_name: self.agent_definition_name.clone(),
        });
    }
}

#[path = "runtime_session_tool_surface.rs"]
mod tool_surface;

#[cfg(test)]
#[path = "runtime_session_juice_tests.rs"]
mod juice_tests;

#[path = "prelude_integrations.rs"]
mod prelude_integrations;

#[cfg(test)]
#[path = "runtime_session_tests.rs"]
mod tests;
