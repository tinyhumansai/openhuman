//! The live OpenHuman composition over `tinyagents_runtime::Session`.
//!
//! Host hooks carry product state; `tinyagents_runtime` owns history,
//! transcripts, prefixes, snapshots, resume, and persistence.

mod permanent;
mod post_commit;
#[path = "runtime_session/prelude.rs"]
mod prelude;
#[path = "runtime_session/prompt.rs"]
mod prompt;
#[path = "runtime_session/prompt_surface.rs"]
mod prompt_surface;
#[path = "runtime_session_usage.rs"]
mod usage;
pub(super) use usage::{account_committed_turn_against_goal, holistic_last_turn_usage};
#[path = "../../memory/lifecycle/session_hooks.rs"]
mod session_hooks;
mod tool_rules;
mod transcript;
#[path = "runtime_session_turn.rs"]
mod turn;
pub(super) use turn::begin_turn_resume;

type ActiveTerminalSidecar = std::sync::Arc<
    std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
>;

use std::sync::Arc;

use anyhow::Result;
use tinyagents_harness::multimodal::markers::extract_image_placeholders_in_text;
use tinyagents_harness::tool::ToolSnapshot;
use tinyagents_harness_tools::goals::ThreadGoalStatus;
use tinyagents_runtime::{
    CommitReceipt, ResumePreparation, SessionBuilder, SessionTerminal, TranscriptTarget,
    TurnPreparation,
};
use tinyinference_llm::message::Message;

use super::types::OpenHumanSessionHost;
use crate::agent::{
    message_convert::{user_message_from_text, user_text_with_markers},
    session_host::{
        driver::OpenHumanSessionDriver, OpenHumanSessionHooks, OpenHumanTranscriptCodec,
    },
    tinyagents::{host::OpenHumanRunContext, middleware::turn_context::TurnContextMiddleware},
};
use crate::integrations::announcement_notes::{
    integration_announcement_note, mcp_announcement_note, skill_announcement_note,
    skill_retraction_note,
};

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
    pub(super) last_turn_iterations: u32,
    pub(super) context_middleware: Option<TurnContextMiddleware>,
    required_output: Option<tinyagents_harness::config::RequiredOutput>,
    pub(crate) pending_turn_overrides: super::types::TurnOverrides,
    pub(super) active_turn_overrides: super::types::TurnOverrides,
    pub(crate) pending_response_shape: super::types::PendingResponseShape,
    pub(super) reply_language_directive: Option<String>,
    pub(super) time_zone: Option<String>,
    prelude: Option<OpenHumanTurnPrelude>,
    active_terminal_sidecar: Option<ActiveTerminalSidecar>,
    /// Previous turn's deferred goal accounting, awaited before the next turn.
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
    prompt_builder: Arc<crate::agent::prompts::SystemPromptBuilder>,
    context_config: crate::config::ContextConfig,
    tool_policy: Arc<
        dyn tinyagents_harness::middleware::ToolCallPolicy<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        >,
    >,
    pack_registry_handle: Option<tinyagents_harness_tools::skills::PackRegistryHandle>,
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
    on_progress: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
    run_queue:
        Option<Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    host_env: crate::agent::turn_env::HostTurnEnv,
    allowed_subagent_ids: std::collections::HashSet<String>,
    sandbox_mode: tinyagents_definition::SandboxMode,
    definition: Option<Arc<crate::agent_registry::host_definition::AgentDefinition>>,
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
    tool_policy_session: tinytools::ToolPolicySession,
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
    /// Current catalogs and frozen names retained by the module tool adapter.
    #[cfg(feature = "mcp")]
    mcp_tools: prelude_integrations::McpToolCatalog,
    announced_skills: std::collections::HashSet<String>,
    pending_skill_announcement: Vec<String>,
    pending_skill_retraction: Vec<String>,
    connected_integrations: Vec<crate::integrations::ConnectedIntegration>,
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
    workflows: Vec<tinyskills::Skill>,
    composio_events: Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    skill_events: Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    /// The user-authored text of the in-flight turn, held until commit so the
    /// committed turn can be handed to memory.
    pending_user_text: Option<String>,
    /// The in-flight turn's memory identity and index, from pre-turn to
    /// commit (`memory::lifecycle::session_hooks`).
    pending_memory_turn: Option<session_hooks::PendingMemoryTurn>,
}

impl OpenHumanSessionHost {
    pub(super) fn update_runtime_prelude_progress(
        &mut self,
        tx: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
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
        let context_config = &self.context_config;
        let tool_result_budget_bytes = context_config.tool_result_budget_bytes;
        let tokenjuice_compaction_enabled = context_config.compaction_enabled;
        let microcompact_keep_recent = if context_config.microcompact_enabled {
            context_config.microcompact_keep_recent
        } else {
            0
        };
        let autocompact_enabled = context_config.enabled && context_config.autocompact_enabled;
        let compaction = context_config.compaction_settings();
        let artifact_store = crate::artifacts::artifact_wiring::build_artifact_store(
            &self.workspace_dir,
            &self.event_session_id,
        );

        // `[components] juice = false`: truncation only, no compaction/summary.
        let juice_on =
            (self.runtime_config.as_ref()).is_none_or(|c| crate::inference::tokenjuice::enabled(c));
        let context_mw = TurnContextMiddleware {
            tool_result_budget_bytes,
            payload_summarizer: self.payload_summarizer.clone().filter(|_| juice_on),
            // Rooted outside the project; see `artifact_wiring` (#6408, #6483).
            artifact_store: Some(artifact_store),
            tokenjuice_compaction_enabled: tokenjuice_compaction_enabled && juice_on,
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
                .unwrap_or(tinyagents_definition::SandboxMode::None),
            self.hosted_base.clone(),
            self.agent_definition_id.clone(),
        ));
        // Thread-bound roots resume their deterministic transcript; other
        // turns keep the per-run stem path. Reuse one locator for the initial
        // binding and each resume because the runtime compares locator identity.
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
            state.prelude = Some(self.create_turn_prelude(false));
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
                            crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink::default();
                        transcript_snapshot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .request_base_len = request_base_len;
                        let (prelude, pending_post_commit) = {
                            let mut state = state
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            state.active_terminal_sidecar =
                                Some(options.run_context.data.session_sidecar.clone());
                            (state.prelude.clone(), state.pending_post_commit.take())
                        };
                        let prelude = prelude.ok_or({
                            tinyagents_runtime::RuntimeError::MissingDependency(
                                "OpenHumanTurnPrelude",
                            )
                        })?;
                        // The previous turn's goal accounting was deferred past
                        // its completion; settle it before this turn reads the goal.
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
                        prelude.begin_user_effects(request, &options.run_context.data);
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
                        let sidecar = post_commit::take_terminal_sidecar(&state, terminal.clone());
                        if let Some(sidecar) = sidecar {
                            post_commit::finalize_deferred_terminal(&sidecar, &terminal).await;
                        }
                        Ok(())
                    })
                }
            },
        ));
        // Bind the hook's native target early so both thread and latest-agent
        // resumes can read recorded declarations before host admission.
        let mut builder = SessionBuilder::new(driver)
            .codec(Arc::new(OpenHumanTranscriptCodec))
            .hooks(hooks)
            .retain_recorded_tools(!self.host_only && !self.host_env.fresh_tool_snapshot);
        let meta = self.runtime_transcript_meta();
        builder = match self.session.clone() {
            Some(session) => builder.session(session_locator, session, meta),
            None => builder
                .transcript(session_locator, self.runtime_transcript_stem(), meta)
                .resume_agent(self.agent_definition_name.clone()),
        };
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
pub(super) mod tool_surface;

#[cfg(test)]
#[path = "runtime_session_juice_tests.rs"]
mod juice_tests;

mod parent_snapshot;
#[path = "../../integrations/prelude_integrations.rs"]
mod prelude_integrations;

#[cfg(test)]
#[path = "runtime_session_tests.rs"]
mod tests;
