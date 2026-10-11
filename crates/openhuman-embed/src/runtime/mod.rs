//! The two-step library API: initialise one [`Runtime`], then instantiate
//! any number of independently configured [`Agent`](crate::Agent)s on it.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use openhuman_embed::{Access, AgentSpec, Provider, Runtime, Workspace};
//!
//! // Step 1 — the runtime: features, services, backend, the API key.
//! let runtime = Runtime::builder()
//!     .workspace(Workspace::dir("/var/lib/my-product/openhuman"))
//!     .api_key("th_live_…")
//!     .build()
//!     .await?;
//!
//! // Step 2 — agents, each fully described. Nothing about one leaks into
//! // another: their own MCP servers, skills, working directory, access tier.
//! let analyst = runtime.agent(
//!     AgentSpec::new("analyst")
//!         .system_prompt("You summarize documents.")
//!         .access(Access::readonly())
//!         .action_dir("/srv/documents"),
//! )?;
//! let writer = runtime.agent(
//!     AgentSpec::new("writer")
//!         .provider(Provider::openai_compatible("https://api.example/v1", "sk-…").model("gpt-5"))
//!         .access(Access::full())
//!         .action_dir("/srv/documents"),
//! )?;
//!
//! let analysis = analyst.run("Summarise this document.").await?;
//! let draft = writer.turn(format!("Explain: {}", analysis.reply)).send().await?;
//! println!("{}", draft.reply);
//! # Ok(())
//! # }
//! ```
//!
//! # What is runtime-wide and what is per-agent
//!
//! The runtime owns everything that is process-scoped in the core: the event
//! bus, the keyring and credential store, the RPC bearer, the background
//! [`ServiceSet`], the compiled-and-registered [`DomainSet`] and the API key.
//! An agent owns everything the core reads through its ambient context:
//! its `Config` (provider route and model, MCP servers, autonomy tier,
//! `action_dir`), its [`AgentDefinition`](openhuman_core::agent::harness::definition::AgentDefinition)
//! (system prompt, tool scope, sandbox mode), its profile (allowlists,
//! dedicated memory and transcripts), its skills root, its narrowed
//! `DomainSet` and [`ToolGroups`], its approval settings, sub-agents, MCP
//! host, cron jobs and per-turn state.
//!
//! The crate README's "Still process-owned" list names what agents share.
//!
//! # One runtime per process
//!
//! The process-scoped state above is seeded once, by
//! [`CoreContext::init`](openhuman_core::core::runtime::context::CoreContext::init).
//! [`RuntimeBuilder::build`] returns [`RuntimeError::AlreadyRunning`] for a
//! second runtime rather than letting two share a keyring and a bus while
//! believing they had separate workspaces. Agents are the unit of
//! multiplicity, not runtimes.
//!
//! # The tokio runtime is yours
//!
//! Use [`crate::process::tokio_runtime`] or
//! [`crate::process::tokio_runtime_builder`]. These helpers set the worker
//! stack size and blocking-thread limit for deep agent turns; Tokio's default
//! 2 MiB worker stack is too small. Create one [`Runtime`] and share it with
//! `Arc<Runtime>` across server requests, with independent agent/session scopes.

mod api_key;
mod build;
pub(crate) mod builder;
mod defaults;
mod host_agents;
pub(crate) use host_agents::AgentMap;
mod info;
mod lifecycle;
mod live_hooks;
mod presets;
mod run;
mod seams;
mod selection;
mod summary;

pub use api_key::ApiKey;
pub use builder::{ConfigSource, RuntimeBuilder};
pub use defaults::{AgentDefaults, LearningSettings, ModelDefaults, RuntimeDefaults, SkillsPolicy};
pub use info::{ConfigurationInfo, DefaultsInfo, RuntimeInfo, StorageInfo};
pub use lifecycle::RemoveAgent;
pub use run::run_from_args;
#[doc(hidden)]
pub use seams::StorageSource;
use selection::ModuleSelection;
pub use selection::{RuntimeModule, WeightClass};
pub use summary::BuilderSummary;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{CoreRuntime, DomainSet, ServiceSet};
use openhuman_core::tools::toolpacks::ToolGroups;

use crate::agent::{Agent, AgentError, AgentSpec};
use crate::harness::workspace::ResolvedWorkspace;
use crate::harness::HarnessCore;
use crate::{Core, CoreError};

/// Guards the process-scoped core state described in the module docs.
pub(crate) static RUNTIME_LIVE: AtomicBool = AtomicBool::new(false);

/// Error from building a [`Runtime`].
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// An RPC made during the build (storing a session) failed.
    #[error(transparent)]
    Call(#[from] CoreError),

    /// The core itself failed to initialize.
    #[error("failed to build the embedded core: {0:#}")]
    Build(#[source] anyhow::Error),

    /// A filesystem operation laying out the workspace failed.
    #[error("failed to {what}")]
    Workspace {
        /// What was being attempted, phrased to complete "failed to …".
        what: &'static str,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A second [`Runtime`] was built in this process. See the module docs.
    #[error(
        "an OpenHuman runtime is already running in this process; \
         core state (keyring, event bus, domain subscribers) is process-scoped, \
         so a second one would share it. Create agents on the existing runtime."
    )]
    AlreadyRunning,

    /// A builder input could not be honoured.
    #[error("{0}")]
    Invalid(String),

    /// [`RuntimeBuilder::api_key`] was given a blank key.
    #[error("the TinyHumans API key is blank")]
    BlankApiKey,

    /// A requested module was excluded from this core artifact.
    #[error("module {module:?} requires Cargo feature `{feature}`")]
    MissingFeature {
        /// Requested module family.
        module: RuntimeModule,
        /// Cargo feature required by this selection.
        feature: &'static str,
    },

    /// The requested storage driver was excluded from this core artifact.
    #[error("storage driver {driver} requires Cargo feature `{feature}`")]
    MissingStorageFeature {
        /// Requested storage driver.
        driver: String,
        /// Cargo feature required by this selection.
        feature: &'static str,
    },

    /// [`Workspace::Stateless`](crate::Workspace::Stateless) was asked for
    /// without a [`RuntimeBuilder::session_store`] to keep conversations in.
    #[error(
        "a stateless workspace keeps no conversations on disk;          give the runtime a session store"
    )]
    NoSessionStore,
}

/// The process-scoped state a [`Runtime`] and every [`Agent`](crate::Agent)
/// built on it share ownership of: the core itself and, for
/// [`crate::Workspace::Ephemeral`], the workspace it lives in.
///
/// Held as `Arc<CoreGuard>` by both `Runtime` and `AgentInner` so its `Drop`
/// — releasing [`RUNTIME_LIVE`] and removing an ephemeral workspace — runs
/// only once the *last* of them goes away. `Agent`s are owned rather than
/// borrowed from `Runtime`, so a caller can drop the `Runtime` handle while
/// an `Agent` (or a `Turn` in flight) is still alive; if `Runtime` tore this
/// state down unconditionally on its own drop, the surviving agent would run
/// turns against a removed workspace while a second `Runtime::builder().build()`
/// call reinitialized the same process-global keyring, event bus and
/// subscribers underneath it.
pub(crate) struct CoreGuard {
    pub(crate) events: Arc<crate::events::EventHub>,
    core: Option<Core>,
    /// Held for its `Drop`: an ephemeral workspace lives exactly as long as
    /// the last owner of this guard.
    workspace: ResolvedWorkspace,
    /// The provider this runtime installed, if any. It is removed only while
    /// it still owns the process slot.
    session_store: Option<Arc<dyn openhuman_core::agent::session_store::SessionStoreProvider>>,
    previous_session_store:
        Option<Arc<dyn openhuman_core::agent::session_store::SessionStoreProvider>>,
    /// The resolver through which the core's cron and workflow drivers find
    /// this runtime's agents; removed with the runtime.
    host_agents: Arc<dyn openhuman_core::agent::host_agents::HostAgentResolver>,
    /// Handlers registered with [`Runtime::on_system_job`], by job name.
    system_jobs:
        Mutex<HashMap<String, openhuman_core::cron::system_job_handlers::SystemJobRegistration>>,
    /// Process-global seams this runtime installed; dropping restores the
    /// restorable ones (see [`seams`]).
    seams: Mutex<Option<seams::InstalledSeams>>,
}

impl Drop for CoreGuard {
    fn drop(&mut self) {
        // Stop handing this runtime's agents and handlers to the core's
        // drivers before the core goes.
        openhuman_core::agent::host_agents::clear_if(&self.host_agents);
        self.system_jobs
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        // Drop the core while the process slot is still claimed. Releasing it
        // first lets another builder initialize process-scoped state while
        // this runtime's keyring, bearer, event bus and subscribers are live.
        // Dropping it also stops the background services it started.
        drop(self.core.take());
        if let Some(installed) = self.session_store.take() {
            if openhuman_core::agent::session_store::clear_if(&installed) {
                openhuman_core::agent::session_store::restore(self.previous_session_store.take());
            }
        }
        drop(
            self.seams
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
        // For an ephemeral workspace, take ownership of the temp path and
        // remove it with a short retry. The core's memory/session writers keep
        // running a moment after a turn returns and can recreate workspace
        // subdirectories while `TempDir`'s own drop-time removal is racing
        // them, leaving an empty directory behind. A bounded retry lets those
        // writes settle before giving up.
        if let Some(temp) = self.workspace._temp.take() {
            let root = temp.keep();
            let mut quiet_passes = 0;
            for _ in 0..20 {
                match std::fs::remove_dir_all(&root) {
                    Ok(()) => quiet_passes += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        quiet_passes += 1;
                    }
                    Err(_) => quiet_passes = 0,
                }
                if quiet_passes >= 5 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        RUNTIME_LIVE.store(false, std::sync::atomic::Ordering::Release);
        log::debug!("[embed][runtime] released");
    }
}

/// An initialised OpenHuman core, ready to host agents.
///
/// Build once with [`Runtime::builder`]; share as `Arc<Runtime>` when several
/// parts of the host create agents. Dropping the last of this `Runtime` and
/// every [`Agent`](crate::Agent) built on it tears the core down and, for
/// [`crate::Workspace::Ephemeral`], removes the workspace — see `CoreGuard`.
pub struct Runtime {
    id: String,
    guard: Arc<CoreGuard>,
    /// The config every agent starts from. Already carries the runtime-wide
    /// defaults (backend URL, access, provider model, supplied overrides).
    base_config: Config,
    /// Why the discovered config could not be loaded, when it could not:
    /// `base_config` is then a placeholder and agents are refused.
    config_unavailable: Option<String>,
    /// Where `Workspace::Inherit` resolved to, for the per-agent layout rule.
    inherited: bool,
    domains: DomainSet,
    tool_groups: ToolGroups,
    agents: Arc<host_agents::AgentMap>,
    max_agents: usize,
    defaults: Mutex<AgentDefaults>,
    effective_host_kind: openhuman_core::core::types::HostKind,
    selection: Option<ModuleSelection>,
    storage_info: StorageInfo,
    storage_backend: Option<Arc<dyn openhuman_core::storage::StorageBackend>>,
}

impl Runtime {
    /// Subscribe to content-free runtime metadata; slow consumers receive a lag error.
    pub fn events(&self) -> crate::RuntimeEvents {
        self.guard.events.subscribe()
    }

    /// Backend installed for this runtime; absent for the classic workspace layout.
    pub fn storage(&self) -> Option<Arc<dyn openhuman_core::storage::StorageBackend>> {
        self.storage_backend.clone()
    }

    /// Snapshot of defaults used by newly created agents. Existing agents retain theirs.
    pub fn defaults(&self) -> RuntimeDefaults {
        self.defaults
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Register a reusable definition template. Agents resolve it once, at creation.
    pub fn define_template(
        &self,
        id: impl Into<String>,
        spec: crate::AgentDefinitionSpec,
    ) -> Result<(), AgentError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(AgentError::Invalid("template id must not be blank".into()));
        }
        spec.clone()
            .inherit(self.defaults().definition)?
            .into_core("template-validation")?;
        self.defaults
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .templates
            .insert(id, spec);
        Ok(())
    }

    /// Opaque identity of this instantiated runtime.
    pub fn runtime_id(&self) -> &str {
        &self.id
    }

    /// Start configuring a runtime.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    /// Instantiate an agent on this runtime.
    ///
    /// Lays out the agent's directories, copies its skills, assembles its
    /// `Config` from the runtime's base plus the spec, and derives its
    /// [`CoreContext`](openhuman_core::core::runtime::CoreContext). No turn
    /// runs and no RPC is made. Ids are unique per runtime for as long as the
    /// agent (any clone of it) is alive.
    pub fn agent(&self, spec: AgentSpec) -> Result<Agent, AgentError> {
        let id = spec.id().to_string();
        if let Some(error) = &self.config_unavailable {
            log::warn!("[embed][runtime] agent refused id={id}: config unavailable");
            return Err(AgentError::Invalid(format!(
                "the runtime's config failed to load ({error}); refusing to start an agent \
                 on a default workspace"
            )));
        }
        // Held across `instantiate` (fs layout only, no turn, no await) so a
        // concurrent `agent()` call for the same id cannot pass the duplicate
        // check while this one is still being built. Releasing the lock
        // between the check and the insert let two callers both observe the
        // id as free and both instantiate, with the second `insert`
        // silently overwriting the first agent's registry entry.
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        if agents.contains_key(&id) {
            return Err(AgentError::DuplicateId(id));
        }
        if agents.len() >= self.max_agents {
            log::warn!(
                "[embed][runtime] agent refused id={id}: {} live agents is the limit",
                self.max_agents
            );
            return Err(AgentError::AgentLimit {
                limit: self.max_agents,
            });
        }
        let inner = Arc::new(crate::agent::build::instantiate(self, spec)?);
        agents.insert(id.clone(), Arc::downgrade(&inner));
        drop(agents);
        self.guard
            .events
            .emit(Some(id.clone()), None, crate::RuntimeEventKind::AgentAdded);
        log::debug!("[embed][runtime] agent registered id={id}");
        Ok(Agent::from_inner(inner))
    }

    /// The runtime's messaging channels. See [`crate::channels`].
    #[cfg(feature = "channels")]
    pub fn channels(&self) -> crate::channels::Channels<'_> {
        crate::channels::Channels::new(self)
    }

    /// The runtime's scheduled jobs. See [`crate::cron`].
    pub fn cron(&self) -> crate::cron::Cron<'_> {
        crate::cron::Cron::new(&self.base_config)
    }

    /// Run `handler` whenever system job `name` comes due (see
    /// [`JobSpec::system`](crate::JobSpec::system)), and record its result as
    /// the run's: `Err` is a failed run, retried like any other. A later
    /// registration under the same name replaces this one; handlers live as
    /// long as the runtime.
    pub fn on_system_job<F, Fut>(
        &self,
        name: impl Into<String>,
        handler: F,
    ) -> Result<(), crate::cron::CronError>
    where
        F: Fn(crate::cron::SystemJobContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let name = name.into();
        if name.trim().is_empty() || name.contains(':') {
            return Err(crate::cron::CronError::Invalid(format!(
                "system job name {name:?} must be non-blank and contain no ':'"
            )));
        }
        let registration = openhuman_core::cron::system_job_handlers::register(
            &name,
            crate::cron::boxed_handler(handler),
        );
        log::debug!("[embed][runtime] system job handler registered name={name}");
        self.guard
            .system_jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name, registration);
        Ok(())
    }

    /// Start the background services this runtime's [`ServiceSet`] selects
    /// (the cron scheduler, channel listeners, …). [`RuntimeBuilder::build`]
    /// already does this when the set asks for background work; call
    /// it after [`stop_services`](Self::stop_services) to restart them.
    /// Idempotent while they run.
    pub async fn start_services(&self) {
        log::debug!("[embed][runtime] start_services");
        self.core_runtime().start_services().await;
        self.guard.events.emit(
            None,
            None,
            crate::RuntimeEventKind::ServicesChanged { running: true },
        );
    }

    /// Stop the background services. They also stop when the last owner of
    /// the runtime's core (this runtime or one of its agents) drops.
    pub fn stop_services(&self) {
        log::debug!("[embed][runtime] stop_services");
        self.core_runtime().stop_services();
        self.guard.events.emit(
            None,
            None,
            crate::RuntimeEventKind::ServicesChanged { running: false },
        );
    }

    /// Ids reserved by live agents or their teardown in progress.
    pub fn agent_ids(&self) -> Vec<String> {
        let agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids: Vec<String> = agents.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Typed access to non-turn core domains (config, auth).
    ///
    /// Turns belong to agents: this facade deliberately exposes neither the
    /// orchestrator agent nor the raw runtime, so no turn can bypass an
    /// agent's provider route and access tier.
    pub fn core(&self) -> HarnessCore<'_> {
        HarnessCore::new(self.core_ref())
    }

    /// One tenant's memory: the agents, items, learnings and brain below the
    /// layout root `root` (`team:acme`), every call confined to that subtree.
    /// Bind agents to the same root ([`MemoryBinding::root`](crate::MemoryBinding::root))
    /// and this is their memory as the operator sees it. See [`crate::memory`].
    ///
    /// # Errors
    ///
    /// [`MemoryError::InvalidRequest`](crate::memory::MemoryError::InvalidRequest)
    /// when `root` is not a valid layout root, or is the store root itself.
    pub fn memory(&self, root: &str) -> crate::memory::MemoryResult<crate::memory::Memory> {
        if let Some(error) = &self.config_unavailable {
            return Err(crate::memory::MemoryError::InvalidRequest(format!(
                "the runtime's config failed to load ({error})"
            )));
        }
        crate::memory::Memory::bind(self.base_config.clone(), root)
    }

    /// The directory holding `config.toml`, the credential store and, for
    /// runtime-owned workspaces, the `workspace/` and `agents/` trees.
    pub fn root_dir(&self) -> &Path {
        self.base_config
            .config_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
    }

    /// The runtime-wide workspace: session database, shared memory, every
    /// agent's `agents/<id>/` home, and the canonical `session_raw/` transcripts.
    pub fn workspace_dir(&self) -> &Path {
        &self.base_config.workspace_dir
    }

    /// Domain families registered at build time. Agents may only narrow this.
    pub fn domains(&self) -> DomainSet {
        self.domains
    }

    /// Tool-group disclosure agents inherit unless they narrow it.
    pub fn tool_groups(&self) -> &ToolGroups {
        &self.tool_groups
    }

    /// Background services selected at build time.
    pub fn services(&self) -> ServiceSet {
        self.core_ref().raw().services()
    }

    /// Whether a TinyHumans API key is currently installed.
    ///
    /// Reads the credential store live (a cheap local file read, not an
    /// RPC) rather than a construction-time snapshot: a host that calls
    /// [`HarnessCore::auth`]'s [`Auth::store_api_key`](crate::Auth::store_api_key)
    /// or [`Auth::clear_api_key`](crate::Auth::clear_api_key) on a running
    /// runtime — both exposed via [`Runtime::core`] — must see this reflect
    /// that change immediately. A cached bool would otherwise report `false`
    /// right after a key was stored, or `true` right after it was cleared.
    pub fn has_api_key(&self) -> bool {
        openhuman_core::security::credentials::api_key::has_api_key(&self.base_config)
    }

    pub(crate) fn core_ref(&self) -> &Core {
        self.guard
            .core
            .as_ref()
            .expect("runtime core is present until the last guard owner drops")
    }

    /// The core runtime under this handle: the controller registry a host
    /// dispatches JSON-RPC methods through in-process
    /// ([`CoreRuntime::invoke`]).
    ///
    /// This is the operator-host escape hatch. The JSON-RPC server serves it,
    /// and the terminal UI drives its threads, config and auth screens with
    /// it. Library embedders should prefer [`Runtime::agent`] for turns and
    /// [`Runtime::core`] for typed config/auth access, because a raw invoke
    /// carries no agent's provider route or access tier.
    ///
    /// The handle stays valid while this `Runtime` is alive. Keep the
    /// `Runtime` for the whole session: dropping it tears the core down even
    /// if a clone of this `Arc` is still held.
    pub fn core_runtime(&self) -> &Arc<CoreRuntime> {
        self.core_ref().raw()
    }

    /// The shared teardown guard, cloned into every [`AgentInner`] so the
    /// core and an ephemeral workspace outlive whichever of `Runtime` or its
    /// agents is dropped last. See `CoreGuard`.
    pub(crate) fn guard(&self) -> Arc<CoreGuard> {
        Arc::clone(&self.guard)
    }

    /// Keeps agent ids reserved through the last owner's teardown.
    pub(crate) fn agent_registry(&self) -> Arc<AgentMap> {
        Arc::clone(&self.agents)
    }

    pub(crate) fn base_config(&self) -> &Config {
        &self.base_config
    }

    pub(crate) fn inherited_workspace(&self) -> bool {
        self.inherited
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        core: Core,
        workspace: ResolvedWorkspace,
        session_store: Option<Arc<dyn openhuman_core::agent::session_store::SessionStoreProvider>>,
        previous_session_store: Option<
            Arc<dyn openhuman_core::agent::session_store::SessionStoreProvider>,
        >,
        seams: Option<seams::InstalledSeams>,
        base_config: Config,
        config_unavailable: Option<String>,
        inherited: bool,
        domains: DomainSet,
        tool_groups: ToolGroups,
        max_agents: usize,
        defaults: AgentDefaults,
        effective_host_kind: openhuman_core::core::types::HostKind,
        selection: Option<ModuleSelection>,
        storage_info: StorageInfo,
        storage_backend: Option<Arc<dyn openhuman_core::storage::StorageBackend>>,
    ) -> Self {
        let agents: Arc<host_agents::AgentMap> = Arc::new(Mutex::new(HashMap::new()));
        let resolver: Arc<dyn openhuman_core::agent::host_agents::HostAgentResolver> =
            Arc::new(host_agents::RuntimeAgents(Arc::clone(&agents)));
        openhuman_core::agent::host_agents::install(Arc::clone(&resolver));
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            guard: Arc::new(CoreGuard {
                events: crate::events::EventHub::new(256),
                core: Some(core),
                workspace,
                session_store,
                previous_session_store,
                host_agents: resolver,
                system_jobs: Mutex::new(HashMap::new()),
                seams: Mutex::new(seams),
            }),
            base_config,
            config_unavailable,
            inherited,
            domains,
            tool_groups,
            agents,
            max_agents,
            defaults: Mutex::new(defaults),
            effective_host_kind,
            selection,
            storage_info,
            storage_backend,
        }
    }
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Resolved paths and a provider bearer are both in here.
        f.debug_struct("Runtime")
            .field("has_api_key", &self.has_api_key())
            .field("domains", &self.domains)
            .finish_non_exhaustive()
    }
}
