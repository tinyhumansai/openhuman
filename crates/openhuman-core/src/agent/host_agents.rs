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
//! Like the session store ([`crate::agent::session_store`]) the slot is
//! process-wide, because OpenHuman runs one runtime per process. The
//! installer keeps the `Arc` it installed and removes it with [`clear_if`],
//! which never removes a replacement someone else installed later.
//!
//! The driver keeps its own authority: a cron turn still runs under
//! `TrustedAutomation { Cron }` and a workflow node under its workflow
//! origin. Resolving an agent does not change who is asking.

use crate::core::runtime::DispatchCtx;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use crate::agent::harness::host_definition::AgentDefinition;
use crate::agent::{HostTools, OpenHumanSessionHost};
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
    pub hooks: crate::agent::hooks::HookScope,
}

impl HostAgent {
    /// Build a session for this agent from `config` -- normally a clone of
    /// [`HostAgent::config`] carrying a driver's overrides.
    ///
    /// Builds under the agent's context, because the session builder reads
    /// the ambient context (tool groups, skill roots) while it assembles the
    /// belt. `session_id` reaches the host belt factory as the turn's
    /// session; `None` when the driver names none.
    pub fn session_host(
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
        let mut session = CoreContext::sync_scope(Arc::clone(&self.context), || {
            if crate::profiles::capabilities::ProfileCapabilities::in_ctx(&dispatch).is_some() {
                return OpenHumanSessionHost::from_config_host_only(
                    &dispatch,
                    config,
                    &self.definition,
                    self.host_tools.as_ref(),
                    session_id,
                );
            }
            match &self.host_tools {
                Some(host) => OpenHumanSessionHost::from_config_with_host_tools(
                    &dispatch,
                    config,
                    &self.definition,
                    host,
                    session_id,
                ),
                None => OpenHumanSessionHost::from_config_with_definition(
                    &dispatch,
                    config,
                    &self.definition,
                ),
            }
        })?;
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
