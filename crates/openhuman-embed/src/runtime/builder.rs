//! Assembling a [`Runtime`](super::Runtime): the builder and its knobs.
//!
//! The builder turns typed inputs into one in-memory base [`Config`] plus a
//! [`DomainSet`]/[`ServiceSet`]/[`ToolGroups`] triple, installs the API key
//! into the credential store, installs the host's process-global seams, and
//! hands everything to [`CoreBuilder`](openhuman_core::core::runtime::CoreBuilder).
//! Nothing here mutates the process environment.
//!
//! The pieces live beside this file: [`presets`](super::presets) (the
//! per-host starting points), [`seams`](super::seams) (process-global
//! installers restored on drop), `build.rs` (the boot sequence) and `run.rs`
//! (the CLI entry).

use std::path::PathBuf;
use std::sync::Arc;

use openhuman_core::agent::session_store::SessionStoreProvider;
use openhuman_core::backend::BackendTransport;
use openhuman_core::config::Config;
use openhuman_core::core::runtime::{DomainSet, ServiceSet, TokenSource};
use openhuman_core::core::types::HostKind;
use openhuman_core::tools::toolpacks::ToolGroups;

use super::seams::HostSeams;
use super::ApiKey;
use crate::harness::{Access, Provider, Workspace};
use crate::Session;

// Re-exported for the sibling test module and older call sites that reached
// the default triples through this module.
pub(crate) use super::build::apply_provider;

/// Whether `services` asks for any background work.
pub(crate) fn requests_background_services(services: ServiceSet) -> bool {
    services != ServiceSet::none()
}

#[cfg(test)]
pub(crate) use super::build::{effective_host_kind, routed_provider_effective};
#[cfg(test)]
pub(crate) use super::presets::{default_domains, default_services};
#[cfg(test)]
pub(crate) use super::{RuntimeError, RUNTIME_LIVE};

/// Who produces the [`Config`] the core boots with.
///
/// The choice is not cosmetic. A config the core is *handed* makes it a
/// scoped embedder: credentials live under that config's root, the operator's
/// `active_user.toml` is never read or written, and handlers see that exact
/// config for the life of the process. A config the core *discovers* is the
/// operator's install, re-read from `config.toml` and the environment, with
/// per-user activation — what the desktop app, the CLI and the TUI run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigSource {
    /// The builder resolves the config — from the [`Workspace`], a supplied
    /// [`RuntimeBuilder::config`], and the other knobs — and hands it to the
    /// core verbatim. The library default.
    #[default]
    Resolved,
    /// The core discovers its config itself, exactly as a desktop, CLI or TUI
    /// host does. Requires [`Workspace::Inherit`]; refuses the knobs that
    /// would edit a config (`config`, `backend_url`, `workspace_dir`,
    /// `action_dir`, `api_key`), because there is no config in hand to edit.
    /// [`RuntimeBuilder::access`] and [`RuntimeBuilder::provider`] then only
    /// set the defaults of agents created with [`crate::Runtime::agent`](super::Runtime::agent).
    Discovered,
}

/// Builder for a [`Runtime`](super::Runtime). Obtain with
/// [`crate::Runtime::builder`](super::Runtime::builder) or one of the host presets
/// ([`RuntimeBuilder::library`], [`desktop`](RuntimeBuilder::desktop),
/// [`cli`](RuntimeBuilder::cli), [`tui`](RuntimeBuilder::tui)).
pub struct RuntimeBuilder {
    pub(super) workspace: Workspace,
    pub(super) workspace_dir: Option<PathBuf>,
    pub(super) action_dir: Option<PathBuf>,
    pub(super) config_source: ConfigSource,
    pub(super) provider: Provider,
    pub(super) access: Access,
    pub(super) services: Option<ServiceSet>,
    pub(super) domains: Option<DomainSet>,
    pub(super) tool_groups: Option<ToolGroups>,
    pub(super) host_kind: HostKind,
    pub(super) token: TokenSource,
    pub(super) listen_host: Option<String>,
    pub(super) listen_port: Option<u16>,
    pub(super) config: Option<Config>,
    pub(super) session: Option<Session>,
    pub(super) backend_url: Option<String>,
    pub(super) api_key: Option<ApiKey>,
    pub(super) backend_transport: Option<Arc<dyn BackendTransport>>,
    pub(super) memory_engine: Option<Arc<dyn tinymemory_api::MemoryEngine>>,
    pub(super) session_store: Option<Arc<dyn SessionStoreProvider>>,
    pub(super) seams: HostSeams,
    pub(super) max_agents: usize,
    pub(super) agent_defaults: super::AgentDefaults,
    pub(super) config_knobs: super::info::ConfigKnobs,
    pub(super) selection: Option<super::ModuleSelection>,
}

/// Live agents a runtime hosts unless [`RuntimeBuilder::max_agents`] says
/// otherwise.
pub const DEFAULT_MAX_AGENTS: usize = 1024;

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    /// A builder with safe defaults: an ephemeral workspace, the machine's
    /// configured inference, the supervised access tier, no background
    /// services, and every domain a library agent can use. Same as
    /// [`RuntimeBuilder::library`].
    pub fn new() -> Self {
        Self {
            workspace: Workspace::default(),
            workspace_dir: None,
            action_dir: None,
            config_source: ConfigSource::Resolved,
            provider: Provider::inherit(),
            access: Access::default(),
            services: None,
            domains: None,
            tool_groups: None,
            host_kind: HostKind::Library,
            token: TokenSource::EnvOrFile,
            listen_host: None,
            listen_port: None,
            config: None,
            session: None,
            backend_url: None,
            api_key: None,
            backend_transport: None,
            memory_engine: None,
            session_store: None,
            seams: HostSeams::default(),
            max_agents: DEFAULT_MAX_AGENTS,
            agent_defaults: super::AgentDefaults::default(),
            config_knobs: super::info::ConfigKnobs::default(),
            selection: None,
        }
    }

    /// Replace the runtime-wide agent defaults. Explicit individual setters still win.
    pub fn agent_defaults(mut self, defaults: super::AgentDefaults) -> Self {
        self.provider = defaults.provider.clone();
        self.access = defaults.access.clone();
        self.domains = Some(defaults.domains);
        self.tool_groups = Some(defaults.tool_groups.clone());
        self.agent_defaults = defaults;
        self
    }
    /// Sampling defaults inherited by agents and then overridden per turn.
    pub fn model_defaults(mut self, model: super::ModelDefaults) -> Self {
        self.agent_defaults.model = model;
        self
    }
    /// Default confinement for new agents.
    pub fn sandbox(mut self, mode: crate::SandboxModeSpec) -> Self {
        self.agent_defaults.sandbox = mode;
        self
    }
    /// Default definition extended by each agent.
    pub fn definition_base(mut self, definition: crate::AgentDefinitionSpec) -> Self {
        self.agent_defaults.definition = definition;
        self
    }
    /// Runtime-wide skill installation and discovery policy.
    pub fn skills(mut self, policy: super::SkillsPolicy) -> Self {
        self.agent_defaults.skills = policy;
        self
    }
    /// Servers inherited by every agent, with agent declarations added afterwards.
    #[cfg(feature = "mcp")]
    pub fn mcp_baseline(mut self, servers: impl IntoIterator<Item = crate::McpServer>) -> Self {
        self.agent_defaults.mcp_baseline = servers.into_iter().collect();
        self
    }
    /// Detailed autonomy settings; access tiers are subsequently applied to each agent.
    pub fn autonomy(mut self, value: openhuman_core::config::schema::AutonomyConfig) -> Self {
        self.config_knobs.autonomy = Some(value);
        self
    }
    /// Tool rules applied to every agent, which agents may only narrow.
    pub fn tool_rules(mut self, value: tinytools::ToolRules) -> Self {
        self.config_knobs.tool_rules = Some(value);
        self
    }
    /// Data egress policy for this runtime.
    pub fn privacy(mut self, value: openhuman_core::config::schema::PrivacyConfig) -> Self {
        self.config_knobs.privacy = Some(value);
        self
    }
    /// Credential encryption policy.
    pub fn secrets(mut self, value: openhuman_core::config::schema::SecretsConfig) -> Self {
        self.config_knobs.secrets = Some(value);
        self
    }
    /// Background memory learning defaults.
    pub fn learning(mut self, value: super::LearningSettings) -> Self {
        self.config_knobs.learning = Some(value);
        self
    }
    /// Cron scheduler configuration; `services` separately controls its lifecycle.
    pub fn cron(mut self, value: openhuman_core::config::schema::CronConfig) -> Self {
        self.config_knobs.cron = Some(value);
        self
    }

    /// The transport the runtime reaches the hosted TinyHumans backend
    /// through (see [`BackendTransport`]).
    ///
    /// The core carries no backend client of its own: without a transport
    /// every hosted-backend surface (billing, integrations tools, channel
    /// relay, cloud voice) answers with a typed "backend unavailable" error
    /// while agents, memory, skills and RPC work as normal. The
    /// `openhuman-tinyhumans` crate supplies the SDK-backed implementation
    /// and a builder that installs it for you.
    pub fn backend_transport(mut self, transport: Arc<dyn BackendTransport>) -> Self {
        self.backend_transport = Some(transport);
        self
    }

    /// The memory engine every agent and [`crate::Runtime::memory`](super::Runtime::memory)
    /// use, in place of the configured `[memory]` engine (TinyHumans over the
    /// backend credential, or CortexDB with a stored key).
    ///
    /// For a host that owns its memory store, or a test that wants
    /// TinyMemory's in-memory reference engine: memory then runs without a
    /// TinyHumans credential. The engine's writes are scrubbed like any
    /// other's. It is process-wide, as the runtime is, and the core has no
    /// uninstall for it: it stays installed for the life of the process.
    pub fn memory_engine(mut self, engine: Arc<dyn tinymemory_api::MemoryEngine>) -> Self {
        self.memory_engine = Some(engine);
        self
    }

    /// Where every agent's conversations are kept, in place of files under
    /// the workspace: transcripts, the turn journal and run status, goals and
    /// todos, each agent's apart from every other's (the provider is asked for
    /// the stores of the agent's id).
    ///
    /// For a host serving many users from one process out of its own
    /// database; pair it with [`Workspace::Stateless`] so nothing durable is
    /// left on disk. [`InMemorySessionStores`](crate::InMemorySessionStores)
    /// keeps everything in memory. It is process-wide, as the runtime is, and
    /// is removed with the runtime.
    pub fn session_store(mut self, provider: Arc<dyn SessionStoreProvider>) -> Self {
        self.session_store = Some(provider);
        self
    }

    /// The most agents this runtime hosts at once; [`crate::Runtime::agent`]
    /// returns [`AgentError::AgentLimit`](crate::AgentError::AgentLimit)
    /// beyond it. Removed and dropped agents do not count. Defaults to
    /// [`DEFAULT_MAX_AGENTS`].
    pub fn max_agents(mut self, limit: usize) -> Self {
        self.max_agents = limit;
        self
    }

    /// Where the runtime keeps its credential store, session database and
    /// every agent's memory, transcripts and skills.
    ///
    /// [`workspace_dir`](Self::workspace_dir) and
    /// [`action_dir`](Self::action_dir) refine the paths this resolves to.
    pub fn workspace(mut self, workspace: Workspace) -> Self {
        self.workspace = workspace;
        self
    }

    /// Put the core's internal state (`config.workspace_dir`: session
    /// database, memory, attachments, every agent's `agents/<id>/` home) at
    /// exactly `dir`, created if absent.
    ///
    /// Applied **after** the [`Workspace`] is resolved and only to that one
    /// field: the credential root (`config_path`'s parent) stays where the
    /// `Workspace` put it, so this splits state from credentials rather than
    /// moving both. Use [`Workspace::Dir`] to move everything together.
    /// Not available with [`ConfigSource::Discovered`].
    pub fn workspace_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.workspace_dir = Some(dir.into());
        self
    }

    /// The runtime-wide read/write root for acting tools (`config.action_dir`),
    /// created if absent. Agents may still name their own.
    ///
    /// Overrides the sibling `action/` directory an ephemeral or `Dir`
    /// workspace would otherwise create, and the operator's configured
    /// `action_dir` under [`Workspace::Inherit`]. Keep it outside the
    /// workspace: acting tools are refused inside it. Not available with
    /// [`ConfigSource::Discovered`].
    pub fn action_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.action_dir = Some(dir.into());
        self
    }

    /// Who produces the boot [`Config`]; see [`ConfigSource`]. The host
    /// presets other than [`library`](Self::library) pick
    /// [`ConfigSource::Discovered`].
    pub fn config_source(mut self, source: ConfigSource) -> Self {
        self.config_source = source;
        self
    }

    /// The TinyHumans API key — the runtime's only credential.
    ///
    /// Installed into the runtime's credential store before the core boots.
    /// Managed inference then sends it as a bearer to the TinyHumans
    /// OpenAI-compatible endpoint and backend REST calls send it as
    /// `x-api-key`; no user session is involved. Agents that name their own
    /// [`Provider`] never use it.
    pub fn api_key(mut self, key: impl Into<ApiKey>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Point the core's backend calls at `url`.
    ///
    /// Unset means whatever [`Config`] resolves to — for a fresh config the
    /// hosted TinyHumans backend. Set it to a stub or a self-hosted backend
    /// when the runtime should not reach the real one.
    pub fn backend_url(mut self, url: impl Into<String>) -> Self {
        self.backend_url = Some(url.into());
        self
    }

    /// Default provider for agents that do not name one.
    pub fn provider(mut self, provider: Provider) -> Self {
        self.provider = provider;
        self
    }

    /// Default access tier for agents that do not name one. Defaults to
    /// [`Access::supervised`].
    pub fn access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// Override which background services run.
    ///
    /// The library default is deliberately minimal (no services): cron, the
    /// login-gated services and the memory queue each write
    /// to the workspace on their own schedule, turning a library call into a
    /// background process the caller did not ask for.
    ///
    /// A set that selects background work (`cron: true` to let
    /// [`crate::Runtime::cron`] jobs fire on their own, say) is started by
    /// [`build`](Self::build) and stopped when the runtime drops; see
    /// [`crate::Runtime::start_services`] / [`crate::Runtime::stop_services`]. Starting is
    /// idempotent, so a transport that serves the runtime
    /// (`openhuman-rpc`) and calls `start_services` once its listener is
    /// bound does not start them twice.
    pub fn services(mut self, services: ServiceSet) -> Self {
        self.services = Some(services);
        self
    }

    /// Override which domain families exist at runtime.
    ///
    /// Families are registered once, at boot, so an agent can only *narrow*
    /// this set. The library default is [`DomainSet::embedded`] plus `mcp`
    /// and `skills` when those features are compiled in — the runtime cannot
    /// know yet which agents will declare servers or skills, and an agent
    /// that declares none narrows them back off for itself.
    pub fn domains(mut self, domains: DomainSet) -> Self {
        self.domains = Some(domains);
        self
    }

    /// Default tool-group disclosure. Agents may narrow it.
    ///
    /// Defaults to every group withheld behind `use_skill`, matching the
    /// desktop app. See [`ToolGroups::advertised`] and [`ToolGroups::none`].
    pub fn tool_groups(mut self, tool_groups: ToolGroups) -> Self {
        self.tool_groups = Some(tool_groups);
        self
    }

    /// Identify the host to the core. Defaults to [`HostKind::Library`],
    /// which accepts caller-supplied provider credentials without an
    /// OpenHuman app login.
    pub fn host_kind(mut self, host_kind: HostKind) -> Self {
        self.host_kind = host_kind;
        self
    }

    /// How the per-process RPC bearer is seeded. Defaults to
    /// [`TokenSource::EnvOrFile`]: `OPENHUMAN_CORE_TOKEN` when set, otherwise
    /// a fresh token written to `core.token` beside the config. A host that
    /// already holds the bearer in memory (the desktop shell) passes
    /// [`TokenSource::Fixed`] so it never crosses the environment.
    pub fn token(mut self, token: TokenSource) -> Self {
        self.token = token;
        self
    }

    /// The address a transport serving this runtime binds.
    ///
    /// Recorded on the core runtime only: the runtime binds nothing itself.
    /// A piece left unset falls back, at serve time, to `OPENHUMAN_CORE_HOST`
    /// / `OPENHUMAN_CORE_PORT` and then `127.0.0.1:7788`. See also
    /// [`listen_host`](Self::listen_host) and [`listen_port`](Self::listen_port)
    /// for setting one half.
    pub fn listen(self, host: impl Into<String>, port: u16) -> Self {
        self.listen_host(host).listen_port(port)
    }

    /// The bind host alone; see [`listen`](Self::listen).
    pub fn listen_host(mut self, host: impl Into<String>) -> Self {
        self.listen_host = Some(host.into());
        self
    }

    /// The bind port alone; see [`listen`](Self::listen).
    pub fn listen_port(mut self, port: u16) -> Self {
        self.listen_port = Some(port);
        self
    }

    /// Install an app session before the first turn.
    ///
    /// Kept for hosts that drive authenticated backend features on behalf of
    /// a signed-in user. A runtime with an [`api_key`](Self::api_key) does not
    /// need one.
    pub fn session(mut self, session: Session) -> Self {
        self.session = Some(session);
        self
    }

    /// Start from a caller-supplied [`Config`] instead of the default.
    ///
    /// Every other builder method is applied **on top** of it, and every
    /// agent starts from the result, so this is the escape hatch for the
    /// config fields the builder does not model — not a way to bypass them.
    pub fn config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
