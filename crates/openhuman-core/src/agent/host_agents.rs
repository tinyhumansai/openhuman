//! Host-registered agents the core's own schedulers can run.
//!
//! A library host (`openhuman-embed`) defines agents the core's registries
//! know nothing about: each carries its own [`AgentDefinition`], its own
//! in-process tools ([`HostTools`]) and its own derived [`CoreContext`]. Its
//! turns reach the harness directly, with all three in hand. The core's
//! *own* drivers -- the cron scheduler's agent jobs and a workflow's `agent`
//! node -- only have an agent **id**, and resolving an id through
//! `AgentDefinitionRegistry` or `config.agent_registry` finds either nothing
//! or a definition without the host's tools.
//!
//! This is the port that closes that gap. A host installs one
//! [`HostAgentResolver`]; a driver that is about to build a session for an id
//! asks it first ([`resolve`]) and, on a hit, builds the session from the
//! [`HostAgent`] it returns -- definition, host belt and config -- and runs
//! the turn inside [`HostAgent::scope`], so every ambient read (config loader,
//! DomainSet gate, tool groups, skill roots, session store) sees the host's
//! agent rather than the process default. A miss changes nothing: the driver
//! falls back to the registries exactly as before.
//!
//! Like the session store ([`crate::storage::sessions`]) the slot is
//! process-wide, because OpenHuman runs one runtime per process. The
//! installer keeps the `Arc` it installed and removes it with [`clear_if`],
//! which never removes a replacement someone else installed later.
//!
//! The driver keeps its own authority: a cron turn still runs under
//! `TrustedAutomation { Cron }` and a workflow node under its workflow
//! origin. Resolving an agent does not change who is asking.

use crate::core::runtime::DispatchCtx;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use crate::agent::{HostTools, OpenHumanSessionHost};
use crate::agent_registry::host_definition::{AgentDefinition, AgentDefinitionRegistry};
use crate::config::Config;
use crate::core::runtime::CoreContext;

/// Everything a driver needs to build a session for a host-registered agent.
#[derive(Clone)]
pub struct HostAgent {
    /// The agent's definition: prompt, tool scope, sandbox, iteration cap.
    pub definition: AgentDefinition,
    /// The config a session for this agent is built from, with the agent's
    /// provider model and route already applied. Drivers clone it before
    /// layering their own per-run overrides (a cron job's `model`).
    pub config: Config,
    /// The agent's own in-process tools, rebuilt per session.
    pub host_tools: Option<HostTools>,
    /// The context every read during the agent's turn must see.
    pub context: Arc<CoreContext>,
    /// Agent-owned callbacks inherited by core-scheduled turns.
    pub hooks: tinyagents_hooks::scope::HookScope<
        crate::agent::tinyagents::host::OpenHumanRunContext,
        tinyagents_harness::cost::TurnCost,
    >,
}

impl HostAgent {
    /// Resolve a config-owned agent into the same trusted carrier used for
    /// embedder-owned agents. Channel entry points use this after routing has
    /// selected an agent; the caller still supplies the untrusted origin and
    /// approval context at execution time.
    pub(crate) fn from_config(
        dispatch: &DispatchCtx,
        config: Config,
        agent_id: &str,
    ) -> anyhow::Result<Self> {
        let definition = AgentDefinitionRegistry::load(&config.workspace_dir)
            .ok()
            .and_then(|registry| registry.get(agent_id).cloned())
            .ok_or_else(|| anyhow::anyhow!("agent definition `{agent_id}` is unavailable"))?;
        let context = dispatch
            .core()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("agent turn has no core context"))?;
        Ok(Self {
            definition,
            config,
            host_tools: None,
            context,
            hooks: Default::default(),
        })
    }

    /// Execute one durable, externally-authored turn with the resolved host
    /// agent's context and hooks. `session_key` is the trusted caller's
    /// canonical conversation identity (already scoped by tenant/provider/
    /// account/conversation); TinyAgents adds the agent id through
    /// `set_thread_id` when it binds the SessionRef.
    #[allow(clippy::too_many_arguments)]
    #[cfg(feature = "channels")]
    pub(crate) async fn run_external_channel_turn(
        self,
        session_key: &str,
        event_session_id: &str,
        channel: &str,
        message: &str,
        message_input: Option<tinyinference_llm::message::Message>,
        origin: crate::security::turn_origin::AgentTurnOrigin,
        approval: crate::security::approval::ApprovalTurnContext,
        progress: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
        run_queue: Option<
            std::sync::Arc<
                tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>,
            >,
        >,
        cancellation: tinyagents_harness::CancellationToken,
        turn_deadline: Option<std::time::Duration>,
        imported_history: &[tinyagents_session::transcript::TranscriptMessage],
        legacy_history: Option<crate::channels::providers::relay::LegacyHistoryImport>,
        staged_parts: Vec<tinychannels_bus::ContentPart>,
        staged_lease: Option<crate::threads::attachments::NativeStagedLease>,
        staged_message_id: Option<String>,
    ) -> anyhow::Result<String> {
        let config = self.config.clone();
        let agent_id = self.definition.id.clone();
        let mut session = self.session_host(&config, Some(session_key))?;
        session.set_thread_id(Some(session_key));
        session.set_event_context(event_session_id, channel);
        session.adopt_imported_history(imported_history)?;
        if let Some(source) = legacy_history {
            let dispatch = crate::core::runtime::DispatchCtx::new(Arc::clone(&self.context));
            session
                .adopt_legacy_history_if_needed(|| async {
                    source.load(&dispatch).await.map_err(anyhow::Error::msg)
                })
                .await
                .map_err(|_| {
                    anyhow::anyhow!(crate::channels::failure_copy::LEGACY_HISTORY_UNAVAILABLE)
                })?;
        }
        self.scope(crate::agent::turn_host::AgentTurnHost::run(
            &mut session,
            crate::agent::turn_host::AgentTurnRequest {
                message: message.to_owned(),
                message_input,
                native_staged_parts: staged_parts,
                native_staged_lease: staged_lease,
                native_message_id: staged_message_id,
                thread_id: Some(session_key.to_owned()),
                origin,
                approval: approval.clone(),
                progress,
                run_queue,
                cancellation,
                turn_deadline,
                workspace: crate::agent::turn_host::WorkspaceBinding::Preserve,
                host_env: crate::agent::turn_env::HostTurnEnv {
                    hooks: self.hooks.clone(),
                    core: Some(Arc::downgrade(&self.context)),
                    approval,
                    memory_identity: Some(
                        tinymemory_tools::layout::identity::Identity::root().child(&agent_id),
                    ),
                    ..Default::default()
                },
            },
        ))
        .await
    }

    /// Reset the durable session used by an external channel command. The
    /// caller serializes this operation with turns for the same trusted
    /// tenant, agent, and canonical session identity.
    pub(crate) async fn reset_external_channel_history(
        &self,
        session_key: &str,
        event_session_id: &str,
        channel: &str,
    ) -> anyhow::Result<bool> {
        let mut session = self.session_host(&self.config, Some(session_key))?;
        session.set_thread_id(Some(session_key));
        session.set_event_context(event_session_id, channel);
        self.scope(session.reset_bound_history()).await
    }

    /// Build a session for this agent from `config` -- normally a clone of
    /// [`HostAgent::config`] carrying a driver's overrides.
    ///
    /// Builds under the agent's context, because the session builder reads
    /// the ambient context (tool groups, skill roots) while it assembles the
    /// belt. `session_id` reaches the host belt factory as the turn's
    /// session; `None` when the driver names none.
    pub async fn session_host(
        &self,
        config: &Config,
        session_id: Option<&str>,
    ) -> anyhow::Result<OpenHumanSessionHost> {
        tracing::debug!(
            agent_id = %self.definition.id,
            host_tools = self.host_tools.is_some(),
            "[host_agents] building session for host agent"
        );
        let dispatch = DispatchCtx::new(Arc::clone(&self.context));
        let mut session = CoreContext::scope(Arc::clone(&self.context), async {
            if crate::profiles::capabilities::ProfileCapabilities::in_ctx(&dispatch).is_some() {
                return OpenHumanSessionHost::from_config_host_only(
                    &dispatch,
                    config,
                    &self.definition,
                    self.host_tools.as_ref(),
                    session_id,
                )
                .await;
            }
            match &self.host_tools {
                Some(host) => {
                    OpenHumanSessionHost::from_config_with_host_tools(
                        &dispatch,
                        config,
                        &self.definition,
                        host,
                        session_id,
                    )
                    .await
                }
                None => {
                    OpenHumanSessionHost::from_config_with_definition(
                        &dispatch,
                        config,
                        &self.definition,
                    )
                    .await
                }
            }
        })
        .await?;
        // The agent's own callbacks ride the session explicitly: its tool and
        // stop hooks reach every turn's run context, its post-turn hooks join
        // the learning sink.
        session.set_host_env(crate::agent::turn_env::HostTurnEnv {
            hooks: self.hooks.clone(),
            core: Some(Arc::downgrade(&self.context)),
            ..Default::default()
        });
        Ok(session)
    }

    /// Run `fut` with this agent's context as the ambient [`CoreContext`].
    pub async fn scope<F: std::future::Future>(&self, fut: F) -> F::Output {
        CoreContext::scope(Arc::clone(&self.context), fut).await
    }
}

impl std::fmt::Debug for HostAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The config carries credentials and resolved paths; the belt is a
        // closure. Name the agent and say whether it has a belt.
        f.debug_struct("HostAgent")
            .field("id", &self.definition.id)
            .field("host_tools", &self.host_tools.is_some())
            .finish_non_exhaustive()
    }
}

/// What a host installs so the core's drivers can find its agents by id.
pub trait HostAgentResolver: Send + Sync {
    /// The agent registered under `agent_id`, if the host has one.
    fn resolve(&self, agent_id: &str) -> Option<HostAgent>;
}

static RESOLVER: LazyLock<RwLock<Option<Arc<dyn HostAgentResolver>>>> =
    LazyLock::new(|| RwLock::new(None));

/// Install `resolver`, replacing any earlier one, which is returned.
pub fn install(resolver: Arc<dyn HostAgentResolver>) -> Option<Arc<dyn HostAgentResolver>> {
    tracing::info!("[host_agents] host agent resolver installed");
    RESOLVER
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(resolver)
}

/// Remove the installed resolver only while it is still `expected`. Returns
/// whether it was removed.
pub fn clear_if(expected: &Arc<dyn HostAgentResolver>) -> bool {
    let mut installed = RESOLVER.write().unwrap_or_else(PoisonError::into_inner);
    if installed
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, expected))
    {
        installed.take();
        tracing::info!("[host_agents] host agent resolver removed");
        true
    } else {
        false
    }
}

/// Ask the installed resolver for `agent_id`. `None` when nothing is
/// installed or the host has no such agent.
pub fn resolve(agent_id: &str) -> Option<HostAgent> {
    let resolver = RESOLVER
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()?;
    let hit = resolver.resolve(agent_id);
    tracing::debug!(
        agent_id,
        hit = hit.is_some(),
        "[host_agents] resolve host agent"
    );
    hit
}

#[cfg(test)]
#[path = "host_agents_tests.rs"]
pub(crate) mod tests;
