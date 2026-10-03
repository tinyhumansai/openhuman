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
//! let reviewer = runtime.agent(
//!     AgentSpec::new("reviewer")
//!         .system_prompt("You review pull requests.")
//!         .access(Access::readonly())
//!         .action_dir("/srv/checkouts/pr-42"),
//! )?;
//! let fixer = runtime.agent(
//!     AgentSpec::new("fixer")
//!         .provider(Provider::openai_compatible("https://api.example/v1", "sk-…").model("gpt-5"))
//!         .access(Access::full())
//!         .action_dir("/srv/checkouts/pr-42"),
//! )?;
//!
//! let review = reviewer.run("Summarise the risks in this change.").await?;
//! let fix = fixer.turn(format!("Address: {}", review.reply)).send().await?;
//! println!("{}", fix.reply);
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
//! `DomainSet` and [`ToolGroups`].
//!
//! Some settings are still read from the runtime's boot config by every
//! agent; see the crate README's "still runtime-wide" list. They are
//! documented rather than hidden.
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
//! Build it with
//! [`AGENT_WORKER_STACK_BYTES`](openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
//! and [`MAX_BLOCKING_THREADS`](openhuman_core::core::runtime::MAX_BLOCKING_THREADS);
//! an agent turn is a very deep async state machine and the default 2 MiB
//! worker stack overflows.

mod api_key;
pub(crate) mod builder;

pub use api_key::ApiKey;
pub use builder::RuntimeBuilder;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, Weak};

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{CoreRuntime, DomainSet, ServiceSet};
use openhuman_core::tools::toolpacks::ToolGroups;

use crate::agent::{Agent, AgentError, AgentInner, AgentSpec};
use crate::harness::workspace::ResolvedWorkspace;
use crate::harness::{Access, HarnessCore, Provider};
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
}

/// The process-scoped state a [`Runtime`] and every [`Agent`](crate::Agent)
/// built on it share ownership of: the core itself and, for
/// [`Workspace::Ephemeral`], the workspace it lives in.
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
    core: Option<Core>,
    /// Held for its `Drop`: an ephemeral workspace lives exactly as long as
    /// the last owner of this guard.
    workspace: ResolvedWorkspace,
}

impl Drop for CoreGuard {
    fn drop(&mut self) {
        // Drop the core while the process slot is still claimed. Releasing it
        // first lets another builder initialize process-scoped state while
        // this runtime's keyring, bearer, event bus and subscribers are live.
        drop(self.core.take());
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
/// [`Workspace::Ephemeral`], removes the workspace — see [`CoreGuard`].
pub struct Runtime {
    id: String,
    guard: Arc<CoreGuard>,
    /// The config every agent starts from. Already carries the runtime-wide
    /// defaults (backend URL, access, provider model, supplied overrides).
    base_config: Config,
    /// Where `Workspace::Inherit` resolved to, for the per-agent layout rule.
    inherited: bool,
    domains: DomainSet,
    tool_groups: ToolGroups,
    provider: Provider,
    access: Access,
    agents: Mutex<HashMap<String, Weak<AgentInner>>>,
}

impl Runtime {
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
        // Held across `instantiate` (fs layout only, no turn, no await) so a
        // concurrent `agent()` call for the same id cannot pass the duplicate
        // check while this one is still being built. Releasing the lock
        // between the check and the insert let two callers both observe the
        // id as free and both instantiate, with the second `insert`
        // silently overwriting the first agent's registry entry.
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        agents.retain(|_, weak| weak.strong_count() > 0);
        if agents.contains_key(&id) {
            return Err(AgentError::DuplicateId(id));
        }
        let inner = Arc::new(crate::agent::build::instantiate(self, spec)?);
        agents.insert(id.clone(), Arc::downgrade(&inner));
        drop(agents);
        log::debug!("[embed][runtime] agent registered id={id}");
        Ok(Agent::from_inner(inner))
    }

    /// Ids of the agents currently alive on this runtime.
    pub fn agent_ids(&self) -> Vec<String> {
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        agents.retain(|_, weak| weak.strong_count() > 0);
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

    pub(crate) fn core_runtime(&self) -> &Arc<CoreRuntime> {
        self.core_ref().raw()
    }

    /// The shared teardown guard, cloned into every [`AgentInner`] so the
    /// core and an ephemeral workspace outlive whichever of `Runtime` or its
    /// agents is dropped last. See [`CoreGuard`].
    pub(crate) fn guard(&self) -> Arc<CoreGuard> {
        Arc::clone(&self.guard)
    }

    pub(crate) fn base_config(&self) -> &Config {
        &self.base_config
    }

    pub(crate) fn inherited_workspace(&self) -> bool {
        self.inherited
    }

    pub(crate) fn default_provider(&self) -> &Provider {
        &self.provider
    }

    pub(crate) fn default_access(&self) -> &Access {
        &self.access
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        core: Core,
        workspace: ResolvedWorkspace,
        base_config: Config,
        inherited: bool,
        domains: DomainSet,
        tool_groups: ToolGroups,
        provider: Provider,
        access: Access,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            guard: Arc::new(CoreGuard {
                core: Some(core),
                workspace,
            }),
            base_config,
            inherited,
            domains,
            tool_groups,
            provider,
            access,
            agents: Mutex::new(HashMap::new()),
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
