//! The runtime's agents, as the core's own drivers see them.
//!
//! The core's cron scheduler and workflow `agent` nodes only know an agent
//! id. [`RuntimeAgents`] answers for the ids alive on this runtime with
//! everything a session for that agent needs: its definition, its host tools
//! (spec belt plus attachments), its config with the provider model and route
//! applied, and its context. The runtime installs it on build and removes it
//! when the last owner of the core drops.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};

use openhuman_core::agent::host_agents::{HostAgent, HostAgentResolver, OpenedHostAgent};

use crate::agent::AgentInner;

/// The live agents of one runtime, by id.
pub(crate) type AgentMap = Mutex<AgentRegistry>;

/// Failed cleanup retains its captured owner and purge intent, without keeping
/// an agent or the runtime guard alive. The original live runtime's
/// `remove_agent` can retry this record. Its process-local directory fence
/// survives a lost record, without automatic durable retry or reclamation.
#[derive(Clone)]
pub(crate) struct PendingCleanup {
    pub(crate) cleanup: Option<openhuman_core::mcp::host::AgentMcpCleanup>,
    pub(crate) directory: openhuman_core::mcp::host::AgentDirectoryCleanup,
    pub(crate) instance: Weak<AgentInner>,
    pub(crate) home: std::path::PathBuf,
    pub(crate) purge: bool,
    pub(crate) retry_running: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default)]
pub(crate) struct AgentRegistry {
    live: HashMap<String, Weak<AgentInner>>,
    pub(crate) pending: HashMap<String, PendingCleanup>,
}

impl std::ops::Deref for AgentRegistry {
    type Target = HashMap<String, Weak<AgentInner>>;
    fn deref(&self) -> &Self::Target {
        &self.live
    }
}

impl std::ops::DerefMut for AgentRegistry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.live
    }
}

/// The host's way to bring up an agent that is not live (see
/// [`RuntimeBuilder::on_missing_agent`](super::RuntimeBuilder::on_missing_agent)).
pub(crate) type AgentOpener =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Option<crate::Agent>> + Send>> + Send + Sync>;

pub(crate) struct RuntimeAgents {
    pub(crate) agents: Arc<AgentMap>,
    pub(crate) opener: Option<AgentOpener>,
    /// Serialises opens, so two due jobs for one missing agent open it once.
    opening: tokio::sync::Mutex<()>,
}

impl RuntimeAgents {
    pub(crate) fn new(agents: Arc<AgentMap>, opener: Option<AgentOpener>) -> Self {
        Self {
            agents,
            opener,
            opening: tokio::sync::Mutex::new(()),
        }
    }

    fn live_inner(&self, agent_id: &str) -> Option<Arc<AgentInner>> {
        self.agents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(agent_id)?
            .upgrade()
    }

    fn live(&self, agent_id: &str) -> Option<HostAgent> {
        self.live_inner(agent_id)?.host_agent()
    }

    /// A live agent with a strong handle as its lease, so a run that reuses an
    /// agent another run opened keeps it from being torn down when that run
    /// drops the last other handle.
    fn live_leased(&self, agent_id: &str) -> Option<OpenedHostAgent> {
        let inner = self.live_inner(agent_id)?;
        let host = inner.host_agent()?;
        Some(OpenedHostAgent::new(host).with_lease(inner))
    }
}

impl HostAgentResolver for RuntimeAgents {
    fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
        self.live(agent_id)
    }

    fn resolve_leased(&self, agent_id: &str) -> Option<OpenedHostAgent> {
        self.live_leased(agent_id)
    }

    fn open<'a>(
        &'a self,
        agent_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<OpenedHostAgent>> + Send + 'a>> {
        Box::pin(async move {
            let opener = self.opener.as_ref()?;
            let _opening = self.opening.lock().await;
            // Another run may have opened it while this one waited.
            if let Some(live) = self.live_leased(agent_id) {
                return Some(live);
            }
            log::debug!("[embed][agent] opening missing agent id={agent_id}");
            let Some(agent) = opener(agent_id.to_string()).await else {
                log::warn!("[embed][agent] host did not open agent id={agent_id}");
                return None;
            };
            if agent.id() != agent_id {
                log::warn!(
                    "[embed][agent] host opened id={} for a request for id={agent_id}; refusing",
                    agent.id()
                );
                return None;
            }
            let host = agent.host_agent()?;
            // The handle is the lease: the agent lives for the run, then the
            // host's drop of the last clone closes it.
            Some(OpenedHostAgent::new(host).with_lease(agent))
        })
    }
}

impl AgentInner {
    /// This agent as a core driver builds it. `None` when its provider route
    /// pairs a bearer with an endpoint a turn would refuse (plain `http:` to
    /// a remote host): the driver then fails to find the agent rather than
    /// sending the credential in the clear.
    pub(crate) fn host_agent(self: &Arc<Self>) -> Option<HostAgent> {
        if *self.lifecycle.removed().borrow() {
            return None;
        }
        let mut config = self.config.clone();
        if let Some(model) = self.provider.model_id() {
            config.default_model = Some(model.to_string());
        }
        if let Some(route) = self.provider.route() {
            if !crate::security::is_safe_endpoint_for_bearer(&route.base_url) {
                log::warn!(
                    "[embed][agent] id={} has an insecure route; not offered to core drivers",
                    self.id
                );
                return None;
            }
            if let Some(route) = openhuman_core::config::schema::EphemeralRoute::from_params(
                Some(route.base_url.clone()),
                Some(route.api_key.clone()),
            )
            .map(|scoped| {
                scoped
                    .with_headers(route.headers.clone())
                    .with_provider_options(route.provider_options.clone())
                    .with_reasoning_effort(
                        route
                            .reasoning_effort
                            .map(|effort| effort.as_str().to_string()),
                    )
            }) {
                openhuman_core::config::schema::ephemeral_route::apply(&mut config, route);
            }
        }
        Some(HostAgent {
            definition: self.definition.clone(),
            config,
            host_tools: self.composed_host_tools(),
            context: Arc::clone(&self.ctx),
            hooks: self.hooks.clone(),
        })
    }
}
