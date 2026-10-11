//! Everything a host says about one agent before it exists.
//!
//! An [`AgentSpec`] is pure data: it is applied by
//! [`Runtime::agent`](crate::Runtime::agent), in a fixed order, onto a clone
//! of the runtime's base config — access tier, provider model, MCP servers,
//! Composio credential, then the [`config`](AgentSpec::config) escape hatch
//! last — and onto a
//! [`AgentDefinitionSpec`].

use std::path::PathBuf;

use openhuman_core::config::{ComposioHostCredential, Config};
use openhuman_core::core::runtime::DomainSet;
use openhuman_core::security::TrustedAccess;
use openhuman_core::tools::toolpacks::ToolGroups;

use super::AgentDefinitionSpec;
use crate::harness::{Access, Provider};

/// The [`AgentSpec::config`] escape hatch, applied last onto the agent's config.
type ConfigEdit = Box<dyn FnOnce(&mut Config) + Send>;

/// Where [`AgentSpec::skills_dir`] bundles are copied.
#[cfg(feature = "skills")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkillsDest {
    /// `<workspace>/agents/<id>/skills/` — seen only by this agent.
    AgentLocal,
    /// `<workspace>/skills/` — the legacy workspace root the one-agent
    /// [`Harness`](crate::Harness) always used; kept for its callers.
    WorkspaceLegacy,
}

/// Description of an agent to instantiate on a [`Runtime`](crate::Runtime).
pub struct AgentSpec {
    id: String,
    post_turn_hooks: Vec<std::sync::Arc<dyn crate::seams::PostTurnHook>>,
    tool_hooks: Vec<std::sync::Arc<dyn crate::seams::ToolHook>>,
    session_store: Option<std::sync::Arc<dyn crate::SessionStoreProvider>>,
    approval_handler: Option<std::sync::Arc<dyn crate::ApprovalHandler>>,
    definition: AgentDefinitionSpec,
    template: Option<String>,
    model_defaults: crate::ModelDefaults,
    provider: Option<Provider>,
    inherit_provider_route: bool,
    access: Option<Access>,
    tool_groups: Option<ToolGroups>,
    domains: Option<DomainSet>,
    #[cfg(feature = "mcp")]
    mcp_servers: Vec<crate::harness::McpServer>,
    #[cfg(feature = "skills")]
    skills_dir: Option<PathBuf>,
    #[cfg(feature = "skills")]
    skills_dest: SkillsDest,
    include_user_skills: Option<bool>,
    action_dir: Option<PathBuf>,
    trusted: Vec<(String, TrustedAccess)>,
    composio: Option<ComposioHostCredential>,
    config_fn: Option<ConfigEdit>,
    host_tools: Option<openhuman_core::agent::HostTools>,
    hooks: openhuman_core::agent::hooks::HookScope,
    memory: Option<MemoryBinding>,
    subagents: Vec<(String, AgentDefinitionSpec)>,
}

/// Whose memory an agent reads and writes: the memory agent id its turns are
/// logged under, and the layout root (tenant) its memory lives below.
///
/// Without one, an agent's memory agent id is its definition id and its
/// root is the runtime's default — every agent on the runtime shares
/// learnings and the brain, and keeps its own conversations. A coordinating
/// host that reuses OpenHuman agents as, say, employees of a company binds
/// each to its own id (and, per tenant, its own root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBinding {
    agent_id: String,
    root: Option<String>,
}

impl MemoryBinding {
    /// Memory under `agent_id`, at the runtime's default root.
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            root: None,
        }
    }

    /// The layout root (`team:acme`, `project:q4`): memory below it is
    /// shared by every agent bound there, and invisible to other roots.
    #[must_use]
    pub fn root(mut self, root: impl Into<String>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// The memory agent id.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// The layout root, when one is set.
    pub fn root_namespace(&self) -> Option<&str> {
        self.root.as_deref()
    }
}

impl AgentSpec {
    /// An agent named `id`, with every setting at the runtime's default.
    ///
    /// The id must match `^[a-z0-9][a-z0-9_-]{0,63}$` (checked at
    /// [`Runtime::agent`](crate::Runtime::agent)); it names the agent's
    /// directories and transcripts. The built-in ids (`orchestrator`,
    /// `summarizer`, …) are reserved for the shipped definitions and refused
    /// with [`AgentError::ReservedId`](crate::AgentError::ReservedId).
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            post_turn_hooks: Vec::new(),
            tool_hooks: Vec::new(),
            session_store: None,
            approval_handler: None,
            definition: AgentDefinitionSpec::new(),
            template: None,
            model_defaults: crate::ModelDefaults::default(),
            provider: None,
            inherit_provider_route: false,
            access: None,
            tool_groups: None,
            domains: None,
            #[cfg(feature = "mcp")]
            mcp_servers: Vec::new(),
            #[cfg(feature = "skills")]
            skills_dir: None,
            #[cfg(feature = "skills")]
            skills_dest: SkillsDest::AgentLocal,
            include_user_skills: None,
            action_dir: None,
            trusted: Vec::new(),
            composio: None,
            config_fn: None,
            host_tools: None,
            hooks: Default::default(),
            memory: None,
            subagents: Vec::new(),
        }
    }

    /// Register a post-turn hook local to this agent, in addition to runtime hooks.
    pub fn post_turn_hook(mut self, hook: std::sync::Arc<dyn crate::seams::PostTurnHook>) -> Self {
        self.post_turn_hooks.push(hook);
        self
    }

    /// Register a tool hook local to this agent, in addition to runtime hooks.
    pub fn tool_hook(mut self, hook: std::sync::Arc<dyn crate::seams::ToolHook>) -> Self {
        self.tool_hooks.push(hook);
        self
    }

    /// Override session storage for this agent; credentials and the bus remain shared.
    pub fn session_store(mut self, store: std::sync::Arc<dyn crate::SessionStoreProvider>) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Decide permissions through a callback rather than polling.
    pub fn approval_handler(mut self, handler: std::sync::Arc<dyn crate::ApprovalHandler>) -> Self {
        self.approval_handler = Some(handler);
        self
    }

    /// The id given to [`new`](Self::new).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Extend a definition template registered on this runtime.
    pub fn extends(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }
    /// Sampling defaults overriding the runtime; turn parameters override these.
    pub fn model_defaults(mut self, defaults: crate::ModelDefaults) -> Self {
        self.model_defaults = defaults;
        self
    }

    /// What the agent is: prompt, tool scope, sandbox, iteration cap.
    pub fn definition(mut self, definition: AgentDefinitionSpec) -> Self {
        self.definition = definition;
        self
    }

    /// Shorthand for [`AgentDefinitionSpec::system_prompt`].
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.definition = self.definition.system_prompt(prompt);
        self
    }

    /// Which model answers and where the request goes. Unset means the
    /// runtime's default provider — the managed TinyHumans backend via the
    /// runtime's API key when no route was given.
    pub fn provider(mut self, provider: Provider) -> Self {
        self.provider = Some(provider);
        self.inherit_provider_route = false;
        self
    }

    /// Pin the model without changing the route.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        if self.provider.is_none() {
            self.inherit_provider_route = true;
        }
        let provider = self.provider.take().unwrap_or_else(Provider::inherit);
        self.provider = Some(provider.model(model));
        self
    }

    /// What the agent is allowed to do. Unset means the runtime's default.
    pub fn access(mut self, access: Access) -> Self {
        self.access = Some(access);
        self
    }

    /// Narrow how tool groups are disclosed to this agent. Must not advertise
    /// a group the runtime withheld or turned off.
    pub fn tool_groups(mut self, groups: ToolGroups) -> Self {
        self.tool_groups = Some(groups);
        self
    }

    /// Narrow which domain families this agent sees. Must be a subset of the
    /// runtime's.
    pub fn domains(mut self, domains: DomainSet) -> Self {
        self.domains = Some(domains);
        self
    }

    /// Declare an MCP server this agent may call tools on. Call repeatedly to
    /// add several. Other agents on the runtime do not see it.
    #[cfg(feature = "mcp")]
    pub fn mcp(mut self, server: crate::harness::McpServer) -> Self {
        self.mcp_servers.push(server);
        self
    }

    /// Make the skill bundles in `dir` available to this agent alone.
    ///
    /// Copied into `<workspace>/agents/<id>/skills/` — copied rather
    /// than linked because skill discovery rejects symlinked bundles.
    #[cfg(feature = "skills")]
    pub fn skills_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.skills_dir = Some(dir.into());
        self.skills_dest = SkillsDest::AgentLocal;
        self
    }

    /// [`skills_dir`](Self::skills_dir), but into the workspace's shared
    /// `skills/` root. What the one-agent [`Harness`](crate::Harness) does.
    #[cfg(feature = "skills")]
    pub(crate) fn skills_dir_legacy(mut self, dir: PathBuf) -> Self {
        self.skills_dir = Some(dir);
        self.skills_dest = SkillsDest::WorkspaceLegacy;
        self
    }

    /// Also let this agent discover the operator's user-scope skills
    /// (`~/.openhuman/skills`, `~/.agents/skills`). Off by default: an
    /// embedded agent sees what its host installed, not what the machine's
    /// user did.
    pub fn include_user_skills(mut self, include: bool) -> Self {
        self.include_user_skills = Some(include);
        self
    }

    /// The agent's read/write root for acting tools.
    ///
    /// Defaults to `<root>/agents/<id>/action` (or, on an inherited
    /// workspace, `<action_dir>/agents/<id>`). Point it at the project the
    /// agent should work in — this is the directory whose contents it can
    /// change.
    pub fn action_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.action_dir = Some(dir.into());
        self
    }

    /// Grant access to a directory outside the action root. Convenience over
    /// [`Access::trust`].
    pub fn trust(mut self, path: impl Into<String>, access: TrustedAccess) -> Self {
        self.trusted.push((path.into(), access));
        self
    }

    /// This agent's own Composio credential.
    ///
    /// The built-in Composio tools then call Composio with this key and
    /// entity only: never the runtime's stored Composio key, and never
    /// another agent's. Connected-integration caches are partitioned by it.
    #[must_use]
    pub fn composio(mut self, credential: ComposioHostCredential) -> Self {
        self.composio = Some(credential);
        self
    }

    /// Binds the agent's memory: the memory agent id its turns are logged
    /// under and, optionally, its layout root. See [`MemoryBinding`].
    #[must_use]
    pub fn memory(mut self, binding: MemoryBinding) -> Self {
        self.memory = Some(binding);
        self
    }

    /// Sub-agents only this agent can delegate to, each under its own id.
    ///
    /// Each becomes a `delegate_<id>` tool on this agent and a worker in this
    /// agent's own catalogue; other agents on the runtime never see it. Ids
    /// follow the agent-id rules and must not name a built-in definition.
    pub fn subagents<I, S>(mut self, subagents: I) -> Self
    where
        I: IntoIterator<Item = (S, AgentDefinitionSpec)>,
        S: Into<String>,
    {
        self.subagents
            .extend(subagents.into_iter().map(|(id, spec)| (id.into(), spec)));
        self
    }

    /// Await the host's permission decision before each tool executes.
    /// The callback may wait for UI approval, then return `Proceed`, `Deny`,
    /// or `ProceedWith`. Returning `Ask` denies the call; this callback itself
    /// owns the approval wait. Static tool/security restrictions still apply.
    /// Agent and turn callbacks are additive: a denial cannot be overridden.
    pub fn can_use_tool<F>(self, callback: F) -> Self
    where
        F: for<'a> Fn(&'a crate::seams::ToolHookContext) -> crate::PermissionFuture<'a>
            + Send
            + Sync
            + 'static,
    {
        self.tool_hook(std::sync::Arc::new(crate::permission::PermissionHook::new(
            callback,
        )))
    }

    /// Observe cumulative usage after each model call or vote to stop before
    /// the next call. Return `StopDecision::Continue` for observation alone;
    /// a budget policy can return `StopDecision::Stop`. Scoped to this agent; no runtime-global policy is replaced.
    pub fn stop_hook(mut self, hook: std::sync::Arc<dyn crate::seams::StopHook>) -> Self {
        self.hooks.push_stop(hook);
        self
    }

    /// Arbitrary edits to the agent's config, applied last.
    ///
    /// The escape hatch for the config fields the spec does not model — not
    /// a way to bypass them. `config_path` is reset afterwards: credentials
    /// and the keyring resolve against it and every agent shares them.
    pub fn config(mut self, f: impl FnOnce(&mut Config) + Send + 'static) -> Self {
        self.config_fn = Some(Box::new(f));
        self
    }

    /// The agent's own in-process tools, built fresh for every turn.
    ///
    /// Until this existed, an embedder's tools could only reach an agent over
    /// the `mcp` builder (with the `mcp` feature enabled): a spec is data, and
    /// the session behind it is rebuilt from that data on every turn, so a
    /// `Box<dyn Tool>` had nowhere to live
    /// in between. The cost was paid by the model — a discovery call to learn
    /// what the server offers, an `mcp_call_tool` envelope whose inner
    /// `arguments` object no provider can validate or constrain decoding
    /// against, and a prompt section explaining the indirection.
    ///
    /// A tool named here is a real tool: its own schema on the wire, called by
    /// its own name.
    ///
    /// # A factory, not a belt
    ///
    /// `f` runs once per session build, and this path builds a session for
    /// every turn, so in practice it runs per turn. That is forced —
    /// [`Agent`](super::Agent) is `Clone` and `Box<dyn Tool>` is not, so a
    /// stored belt could not survive the rebuild — but it is also useful: a
    /// host whose tools are bound to
    /// something shorter-lived than the agent (one episode, one room, one
    /// assignment) can return a different belt each turn rather than
    /// registering a second agent for it.
    ///
    /// The prompt's tool catalogue is rendered from the same belt in the same
    /// build, so a belt that changes stays consistent with its description on
    /// any turn that composes a prompt. A **resumed** session reuses its
    /// persisted system messages, so a belt that moves under a long-lived
    /// thread will be described by the prompt that thread opened with. Vary a
    /// belt only on turns that run on a session of their own.
    ///
    /// # What the factory is told
    ///
    /// `f` receives a [`TurnContext`](openhuman_core::agent::TurnContext): the
    /// agent, and the conversation the turn runs in when the caller named one
    /// with [`Turn::session`](crate::Turn::session). A belt that varies has to
    /// vary on something, and without this the factory would have to infer its
    /// own occasion from state it closed over -- which holds only while the
    /// agent serves one conversation at a time. The moment it serves two, a
    /// belt bound to "the current episode" is a race rather than a decision.
    ///
    /// ```no_run
    /// use openhuman_embed::{AgentSpec, HostTurnTools, Tool};
    ///
    /// # fn belt_for(_chat: Option<&str>) -> Vec<Box<dyn Tool>> { Vec::new() }
    /// let spec = AgentSpec::new("assistant")
    ///     .tools(|turn| HostTurnTools::advertised(belt_for(turn.session_id())));
    /// ```
    #[must_use]
    pub fn tools(
        mut self,
        f: impl for<'a> Fn(
                openhuman_core::agent::TurnContext<'a>,
            ) -> openhuman_core::agent::HostTurnTools
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.host_tools = Some(std::sync::Arc::new(f));
        self
    }

    /// [`Self::tools`] for a caller that already holds the factory.
    ///
    /// `HarnessBuilder` forwards its own `tools` through here rather than
    /// wrapping the factory in a second closure.
    #[must_use]
    pub(crate) fn host_tools(mut self, factory: openhuman_core::agent::HostTools) -> Self {
        self.host_tools = Some(factory);
        self
    }

    // ── accessors for the build step ─────────────────────────────────────

    pub(crate) fn into_parts(self) -> AgentSpecParts {
        AgentSpecParts {
            id: self.id,
            post_turn_hooks: self.post_turn_hooks,
            tool_hooks: self.tool_hooks,
            session_store: self.session_store,
            approval_handler: self.approval_handler,
            definition: self.definition,
            template: self.template,
            model_defaults: self.model_defaults,
            provider: self.provider,
            inherit_provider_route: self.inherit_provider_route,
            access: self.access,
            tool_groups: self.tool_groups,
            domains: self.domains,
            #[cfg(feature = "mcp")]
            mcp_servers: self.mcp_servers,
            #[cfg(feature = "skills")]
            skills_dir: self.skills_dir,
            #[cfg(feature = "skills")]
            skills_dest: self.skills_dest,
            include_user_skills: self.include_user_skills,
            action_dir: self.action_dir,
            trusted: self.trusted,
            composio: self.composio,
            config_fn: self.config_fn,
            host_tools: self.host_tools,
            hooks: self.hooks,
            memory: self.memory,
            subagents: self.subagents,
        }
    }
}

/// The spec's fields, destructured for [`super::build::instantiate`].
pub(crate) struct AgentSpecParts {
    pub(crate) id: String,
    pub(crate) post_turn_hooks: Vec<std::sync::Arc<dyn crate::seams::PostTurnHook>>,
    pub(crate) tool_hooks: Vec<std::sync::Arc<dyn crate::seams::ToolHook>>,
    pub(crate) session_store: Option<std::sync::Arc<dyn crate::SessionStoreProvider>>,
    pub(crate) approval_handler: Option<std::sync::Arc<dyn crate::ApprovalHandler>>,
    pub(crate) definition: AgentDefinitionSpec,
    pub(crate) template: Option<String>,
    pub(crate) model_defaults: crate::ModelDefaults,
    pub(crate) provider: Option<Provider>,
    pub(crate) inherit_provider_route: bool,
    pub(crate) access: Option<Access>,
    pub(crate) tool_groups: Option<ToolGroups>,
    pub(crate) domains: Option<DomainSet>,
    #[cfg(feature = "mcp")]
    pub(crate) mcp_servers: Vec<crate::harness::McpServer>,
    #[cfg(feature = "skills")]
    pub(crate) skills_dir: Option<PathBuf>,
    #[cfg(feature = "skills")]
    pub(crate) skills_dest: SkillsDest,
    pub(crate) include_user_skills: Option<bool>,
    pub(crate) action_dir: Option<PathBuf>,
    pub(crate) trusted: Vec<(String, TrustedAccess)>,
    pub(crate) composio: Option<ComposioHostCredential>,
    pub(crate) config_fn: Option<ConfigEdit>,
    pub(crate) host_tools: Option<openhuman_core::agent::HostTools>,
    pub(crate) hooks: openhuman_core::agent::hooks::HookScope,
    pub(crate) memory: Option<MemoryBinding>,
    pub(crate) subagents: Vec<(String, AgentDefinitionSpec)>,
}

impl std::fmt::Debug for AgentSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The provider may carry a bearer; the config closure is opaque.
        f.debug_struct("AgentSpec")
            .field("id", &self.id)
            .field("access", &self.access)
            .field("action_dir", &self.action_dir)
            .field("composio", &self.composio)
            .field("memory", &self.memory)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "spec_tests.rs"]
mod tests;
