//! Assembling a [`Harness`].
//!
//! The builder splits its inputs between the two things a harness is: the
//! runtime-wide ones (workspace, services, domains, backend URL, session,
//! base config) go to a [`RuntimeBuilder`], and the agent-shaped ones
//! (provider, access, action directory, skills, MCP servers) go to one
//! [`AgentSpec`] named `harness`. Nothing here mutates the process
//! environment.

use std::path::PathBuf;
use std::sync::Arc;

use super::access::Access;
use super::error::HarnessError;
use super::provider::Provider;
use super::workspace::Workspace;
use super::Harness;
use crate::agent::AgentSpec;
use crate::runtime::RuntimeBuilder;
use crate::Session;
use openhuman_core::config::Config;
use openhuman_core::core::runtime::{DomainSet, ServiceSet};
use openhuman_core::core::types::HostKind;

/// Builder for a [`Harness`]. Obtain with [`Harness::builder`].
pub struct HarnessBuilder {
    workspace: Workspace,
    action_dir: Option<PathBuf>,
    provider: Provider,
    access: Access,
    #[cfg(feature = "skills")]
    skills_dir: Option<PathBuf>,
    #[cfg(feature = "mcp")]
    mcp_servers: Vec<super::mcp::McpServer>,
    host_tools: Option<openhuman_core::agent::HostTools>,
    services: Option<ServiceSet>,
    domains: Option<DomainSet>,
    tool_groups: Option<openhuman_core::tools::toolpacks::ToolGroups>,
    host_kind: HostKind,
    config: Option<Config>,
    session: Option<Session>,
    session_store: Option<Arc<dyn crate::SessionStoreProvider>>,
    backend_url: Option<String>,
}

impl Default for HarnessBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl HarnessBuilder {
    /// A builder with safe defaults: an ephemeral workspace, the machine's
    /// configured inference, and the supervised access tier.
    pub fn new() -> Self {
        Self {
            workspace: Workspace::default(),
            action_dir: None,
            provider: Provider::inherit(),
            access: Access::default(),
            #[cfg(feature = "skills")]
            skills_dir: None,
            #[cfg(feature = "mcp")]
            mcp_servers: Vec::new(),
            host_tools: None,
            services: None,
            domains: None,
            tool_groups: None,
            host_kind: HostKind::Library,
            config: None,
            session: None,
            session_store: None,
            backend_url: None,
        }
    }

    /// Where the harness keeps sessions, memory and skills.
    pub fn workspace(mut self, workspace: Workspace) -> Self {
        self.workspace = workspace;
        self
    }

    /// The agent's read/write root for acting tools.
    ///
    /// Defaults to a directory alongside the workspace. Set it to point the
    /// agent at a project you want it to work in — this is the directory whose
    /// contents it can change.
    pub fn action_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.action_dir = Some(dir.into());
        self
    }

    /// Which model answers, and where the request goes.
    pub fn provider(mut self, provider: Provider) -> Self {
        self.provider = provider;
        self
    }

    /// What the agent is allowed to do. Defaults to [`Access::supervised`].
    pub fn access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// Make the skill bundles in `dir` available to the agent.
    ///
    /// The bundles are **copied** into the workspace's skills root — see the
    /// internal skill-copy module docs for why linking cannot work. Not
    /// permitted with [`Workspace::Inherit`], which would leave them in the
    /// operator's own install.
    #[cfg(feature = "skills")]
    pub fn skills_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.skills_dir = Some(dir.into());
        self
    }

    /// Declare an MCP server the agent may call tools on.
    ///
    /// Call repeatedly to add several. Servers are fixed once the harness is
    /// built; the static registry has no add-at-runtime path.
    #[cfg(feature = "mcp")]
    pub fn mcp(mut self, server: super::mcp::McpServer) -> Self {
        self.mcp_servers.push(server);
        self
    }

    /// The harness agent's own in-process tools, built fresh for every turn.
    ///
    /// [`AgentSpec::tools`](crate::AgentSpec::tools) for the one-agent
    /// shorthand, with the same contract — including that `f` runs per turn
    /// and may return a different belt each time.
    #[must_use]
    pub fn tools(
        mut self,
        f: impl for<'a> Fn(
                openhuman_core::agent::TurnContext<'a>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = openhuman_core::agent::HostTurnTools>
                        + Send
                        + 'a,
                >,
            > + Send
            + Sync
            + 'static,
    ) -> Self {
        self.host_tools = Some(std::sync::Arc::new(f));
        self
    }

    /// Override which background services run.
    ///
    /// The default is deliberately minimal — see [`Harness`] — because cron,
    /// the login-gated services and the memory queue are what make a second core in the same
    /// process corrupt shared state. Widen it only if you need what they do.
    pub fn services(mut self, services: ServiceSet) -> Self {
        self.services = Some(services);
        self
    }

    /// Override which domain families exist at runtime.
    ///
    /// The default derives from what you configured — enabling `mcp` when you
    /// declared a server, `skills` when you pointed at a directory — so this is
    /// for narrowing further or for reaching a family the builder does not
    /// model.
    pub fn domains(mut self, domains: DomainSet) -> Self {
        self.domains = Some(domains);
        self
    }

    /// Choose how each tool group reaches the model.
    ///
    /// [`domains`](Self::domains) decides which families *exist*; this decides
    /// how the tools of the families that do exist are disclosed — schemas on
    /// the wire, withheld behind `use_skill`, or not registered
    /// at all.
    ///
    /// Defaults to every group withheld, matching the desktop app. Reach for
    /// [`ToolGroups::advertised`] when the host does its own routing and wants
    /// native function calling instead of the `use_skill` envelope, and for
    /// [`crate::ToolGroups::none`] plus [`with`](crate::ToolGroups::with) when the embedding
    /// product should not carry a family at all.
    ///
    /// ```no_run
    /// # use openhuman_embed::{GroupMode, Harness, ToolGroups};
    /// Harness::builder().tool_groups(
    ///     ToolGroups::none().with("documents", GroupMode::Advertised),
    /// );
    /// ```
    ///
    /// [`ToolGroups::advertised`]: openhuman_core::tools::toolpacks::ToolGroups::advertised
    /// [`crate::ToolGroups::none`]: openhuman_core::tools::toolpacks::ToolGroups::none
    /// [`ToolGroups::with`]: openhuman_core::tools::toolpacks::ToolGroups::with
    pub fn tool_groups(
        mut self,
        tool_groups: openhuman_core::tools::toolpacks::ToolGroups,
    ) -> Self {
        self.tool_groups = Some(tool_groups);
        self
    }

    /// Identify the host to the core. Defaults to [`HostKind::Library`], which
    /// accepts caller-supplied provider credentials without OpenHuman app login.
    pub fn host_kind(mut self, host_kind: HostKind) -> Self {
        self.host_kind = host_kind;
        self
    }

    /// Point the core's backend calls at `url`.
    ///
    /// Even a harness running entirely on its own inference endpoint still
    /// can talk to a backend for everything that is not a completion —
    /// integrations, billing, telemetry. Left unset, that is whatever
    /// [`Config`] resolves to, which for a fresh config is the hosted
    /// TinyHumans backend: a harness with no real account will make live calls
    /// there and be rejected. Library-routed inference is independent of those
    /// calls, but the backend features themselves will still fail.
    ///
    /// Set it to a stub (or a self-hosted backend) whenever the harness is not
    /// signed in to the real one.
    pub fn backend_url(mut self, url: impl Into<String>) -> Self {
        self.backend_url = Some(url.into());
        self
    }

    /// Install a session before the first turn.
    ///
    /// The default [`HostKind::Library`] does not require an OpenHuman app
    /// login when the caller supplies a provider. Use [`Session::backend`] only
    /// when the embedded workload also calls authenticated TinyHumans backend
    /// services. [`Session::local`] remains available for compatibility with
    /// non-library host modes and offline tests.
    pub fn session(mut self, session: Session) -> Self {
        self.session = Some(session);
        self
    }

    /// Keep the harness agent's conversation state in a host-owned store.
    ///
    /// The provider is asked for the stores of the fixed `harness` agent. Pair
    /// this with [`Workspace::Stateless`] when the harness must leave no
    /// durable session data on disk.
    pub fn session_store(mut self, provider: Arc<dyn crate::SessionStoreProvider>) -> Self {
        self.session_store = Some(provider);
        self
    }

    /// Start from a caller-supplied [`Config`] instead of the default.
    ///
    /// Every other builder method is applied **on top** of it, so this is the
    /// escape hatch for the ~200 config fields the harness does not model — not
    /// a way to bypass them.
    pub fn config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the core and return a harness ready to run turns.
    ///
    /// # Errors
    ///
    /// [`HarnessError::AlreadyRunning`] if this process already has a
    /// runtime; see that variant's docs for why that is a property of the
    /// core rather than of the harness.
    pub async fn build(self) -> Result<Harness, HarnessError> {
        let inherit = self.workspace.is_operator_owned();

        // Refused before any runtime is built, so a bad input never claims
        // the process slot. The new API allows skills on an inherited
        // workspace because it copies into the agent's own home; the
        // harness keeps installing into `<workspace>/skills`, which under
        // `Inherit` is the operator's, so it keeps refusing.
        #[cfg(feature = "skills")]
        if self.skills_dir.is_some() && inherit {
            return Err(HarnessError::Invalid(
                "skills_dir cannot be used with Workspace::Inherit: installing them would \
                 write bundles into the operator's own skills root, where they would \
                 outlive this process and shadow installed skills. Use \
                 Workspace::Ephemeral or Workspace::Dir."
                    .to_string(),
            ));
        }

        let domains = self.domains.unwrap_or_else(|| {
            // Preserve the harness's historical default: only what was asked
            // for. An MCP domain with no servers costs ~19 agent tools of
            // prompt budget on every turn for nothing.
            #[allow(unused_mut)]
            let mut domains = DomainSet::embedded();
            #[cfg(feature = "mcp")]
            {
                domains.mcp = !self.mcp_servers.is_empty();
            }
            #[cfg(feature = "skills")]
            {
                domains.skills = self.skills_dir.is_some();
            }
            domains
        });

        let mut runtime = RuntimeBuilder::new()
            .workspace(self.workspace)
            .host_kind(self.host_kind)
            .provider(self.provider.clone())
            .access(self.access.clone())
            .domains(domains);
        if let Some(config) = self.config {
            runtime = runtime.config(config);
        }
        if let Some(url) = self.backend_url {
            runtime = runtime.backend_url(url);
        }
        if let Some(services) = self.services {
            runtime = runtime.services(services);
        }
        if let Some(groups) = self.tool_groups {
            runtime = runtime.tool_groups(groups);
        }
        if let Some(session) = self.session {
            runtime = runtime.session(session);
        }
        if let Some(session_store) = self.session_store {
            runtime = runtime.session_store(session_store);
        }
        let runtime = runtime.build().await?;

        let mut spec = AgentSpec::new(HARNESS_AGENT_ID)
            .provider(self.provider)
            .access(self.access);
        if let Some(host_tools) = self.host_tools {
            spec = spec.host_tools(host_tools);
        }
        if let Some(dir) = self.action_dir {
            spec = spec.action_dir(dir);
        } else if !inherit {
            // Preserve the resolved workspace's own action directory
            // (`ResolvedWorkspace::resolve` already created it) rather than
            // falling through to `agent::build`'s per-agent default of
            // `<root>/agents/harness/action`. That default is right for a
            // `Runtime::agent` caller juggling several agents, but a
            // `Harness` caller — one runtime, one agent — expects its action
            // directory to be the workspace's, exactly where the resolved
            // workspace put it (and where `Workspace::Dir`'s sibling
            // `action` dir already lives on disk). `inherit` keeps the
            // per-agent-subdirectory default: an operator-owned workspace
            // must not let the harness act directly in the operator's own
            // directory.
            spec = spec.action_dir(runtime.base_config().action_dir.clone());
        }
        #[cfg(feature = "skills")]
        if let Some(dir) = self.skills_dir {
            spec = spec.skills_dir_legacy(dir);
        }
        #[cfg(feature = "mcp")]
        for server in self.mcp_servers {
            spec = spec.mcp(server);
        }
        let agent = runtime.agent(spec)?;

        log::debug!("[embed][harness] built inherit_workspace={inherit}");
        Ok(Harness::from_parts(runtime, agent))
    }
}

/// The id of the one agent a [`Harness`] creates.
pub const HARNESS_AGENT_ID: &str = "harness";

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
