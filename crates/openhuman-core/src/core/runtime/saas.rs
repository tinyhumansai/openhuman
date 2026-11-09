//! Booting a core in [`Mode::Saas`](super::mode::Mode::Saas).
//!
//! A SaaS core serves many users from one process, each as their own profile,
//! behind a trusted gateway that authenticates them. This module holds the
//! **operator** side of that: [`SaasConfig`] (read from the operator's file,
//! never from any user's `config.toml`), the SaaS presets for the three
//! narrowing axes, and [`build`], which refuses to boot unless
//! [`boot_guard`](super::boot_guard) finds nothing unsafe.
//!
//! [`DomainSet::saas`] enables the operator plane (`profiles.*`) and the
//! user families whose per-user isolation has landed (threads, channels for
//! web chat, memory). The operator scope reaches only its own plane, and a
//! user only the reviewed `profiles::surface::USER_METHODS`. [`build`]
//! seeds the built-in agent definitions and installs the process's
//! [`ProfileHost`](crate::profiles::ProfileHost).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use super::boot_guard::{self, BootInputs, ServiceToken};
use super::mode::{self, Mode};
use super::{CoreBuilder, CoreContext, CoreRuntime, DomainSet, ServiceSet, TokenSource};
use crate::core::types::HostKind;
use crate::tools::toolpacks::ToolGroups;

/// The operator's SaaS deployment settings.
///
/// `Debug` redacts `storage_url`, which can carry database credentials.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaasConfig {
    /// Root of every user's state and the operator's own. Must be an absolute,
    /// existing directory that is not world-writable.
    pub root: PathBuf,
    /// Validated direct inference defaults shared by profile chat, relay, and cron.
    #[serde(default)]
    pub chat_provider: Option<super::direct_profile_provider::DirectProfileProvider>,
    /// Operator-owned direct Exa/Gemini policy; never a user settings RPC.
    #[serde(default)]
    pub search: Option<crate::config::SearchConfig>,
    /// Advertised names for the three search roles. Missing entries retain canonical names.
    #[serde(default)]
    pub search_role_names: std::collections::BTreeMap<tinysearch_bus::Role, String>,
    /// The gateway's bearer, one line in a `0600` file. Defaults to
    /// `<root>/service.token`.
    #[serde(default)]
    pub service_token_file: Option<PathBuf>,
    /// Host tool groups the operator permits an embedding host to supply
    /// (`profiles::tools::SaasToolGroup`: `host_files`, `host_shell`).
    /// This allowlist does not register tools. The standalone SaaS CLI has no
    /// host-tool adapter, so these groups expose no file or shell tools there.
    /// Empty by default: users get no tool that reaches the host.
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    /// The container every user shell command runs in.
    #[serde(default)]
    pub sandbox: SaasSandboxConfig,
    /// Extra RPC methods the operator exposes. Refused by the boot guard: the
    /// per-user RPC surface is the reviewed `profiles::surface` list.
    #[serde(default)]
    pub rpc_allowlist_extra: Vec<String>,
    /// Most profiles kept open at once.
    #[serde(default = "default_max_profiles_open", alias = "max_agents_open")]
    pub max_profiles_open: usize,
    /// How a gateway user id becomes a profile id: `"raw"` (the default) keeps
    /// an id that already fits `^[a-z0-9][a-z0-9_-]{0,63}$` and is not
    /// reserved, hashing anything else; `"hashed"` hashes every id
    /// (`profiles::ProfileIdMode`). Changing it may re-map users onto different profiles.
    #[serde(default)]
    pub profile_ids: crate::profiles::ProfileIdMode,
    /// Seconds an idle profile stays open.
    #[serde(default = "default_idle_evict_secs")]
    pub idle_evict_secs: u64,
    /// Let every user ride the operator's backend API key.
    #[serde(default)]
    pub shared_backend_api_key: bool,
    /// Let users store their own agent definitions.
    #[serde(default)]
    pub custom_definitions: bool,
    /// Require `X-OpenHuman-User-Sig` on every request made for a user
    /// (see `profiles::gateway`).
    #[serde(default = "default_true")]
    pub require_user_signature: bool,
    /// The storage backend every node of a deployment shares (`sqlite:…`,
    /// `mongodb://…`): the profile registry, the profile leases and the
    /// session store. `OPENHUMAN_STORAGE_URL` wins over it. Unset keeps the
    /// on-disk layout, `profile.toml` files and file-lock leases.
    #[serde(default)]
    pub storage_url: Option<String>,
    /// This node's id in the profile leases. Defaults to `OPENHUMAN_NODE_ID`,
    /// else a random id per process ([`Self::resolve_node_id`]). Must be
    /// unique among live nodes; a stable one lets a restarted node take its
    /// own profiles back at once instead of waiting out their leases.
    #[serde(default)]
    pub node_id: Option<String>,
    /// Where the gateway reaches this node, named to callers that hit another
    /// node while this one holds their profile. Setting it marks the node as
    /// one of a cluster, which needs a backend whose compare-and-swap holds
    /// across processes (the boot guard checks).
    #[serde(default)]
    pub advertise_url: Option<String>,
    /// Seconds a profile lease lasts without renewal. The holder renews every
    /// third of it; a node that stops renewing loses its profiles to others
    /// after this long.
    #[serde(default = "default_lease_ttl_secs")]
    pub lease_ttl_secs: u64,
    /// The operator's own state directory. Defaults to `<root>/operator`;
    /// nodes sharing one root each set their own.
    #[serde(default)]
    pub operator_dir: Option<PathBuf>,
    /// Cron limits forced onto every profile; a user's own config cannot
    /// loosen them.
    #[serde(default)]
    pub cron: SaasCronConfig,
    /// Most tool-call rounds one profile turn may take (chat, relay and cron
    /// alike). Forced onto every profile as `agent.max_tool_iterations_override`,
    /// so it wins over any agent definition's own cap. `None` leaves the
    /// definition in charge.
    #[serde(default)]
    pub max_tool_iterations: Option<usize>,
}

impl std::fmt::Debug for SaasConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaasConfig")
            .field("root", &self.root)
            .field("chat_provider", &self.chat_provider)
            .field("search", &self.search)
            .field("search_role_names", &self.search_role_names)
            .field("service_token_file", &self.service_token_file)
            .field("tool_allowlist", &self.tool_allowlist)
            .field("sandbox", &self.sandbox)
            .field("rpc_allowlist_extra", &self.rpc_allowlist_extra)
            .field("max_profiles_open", &self.max_profiles_open)
            .field("profile_ids", &self.profile_ids)
            .field("idle_evict_secs", &self.idle_evict_secs)
            .field("shared_backend_api_key", &self.shared_backend_api_key)
            .field("custom_definitions", &self.custom_definitions)
            .field("require_user_signature", &self.require_user_signature)
            .field(
                "storage_url",
                &self.storage_url.as_ref().map(|_| "<redacted>"),
            )
            .field("node_id", &self.node_id)
            .field("advertise_url", &self.advertise_url)
            .field("lease_ttl_secs", &self.lease_ttl_secs)
            .field("operator_dir", &self.operator_dir)
            .field("cron", &self.cron)
            .field("max_tool_iterations", &self.max_tool_iterations)
            .finish()
    }
}

/// `[cron]`: the operator's cron limits for hosted profiles (see
/// [`crate::cron::limits`]).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaasCronConfig {
    /// Allow agent jobs bound to their creating conversation and the memory
    /// domain's two code-owned maintenance jobs. Shell and user-owned flow
    /// jobs remain disabled; delivery is limited to `origin` and `none`, the
    /// recurrence interval still applies, and memory jobs consume the cap.
    #[serde(default)]
    pub agent_jobs_only: bool,
    /// With `agent_jobs_only`: seconds between runs of a recurring job, at least.
    #[serde(default = "default_cron_min_interval_secs")]
    pub min_interval_secs: u64,
    /// With `agent_jobs_only`: most enabled jobs one profile may hold.
    #[serde(default = "default_cron_max_active_jobs")]
    pub max_active_jobs: u32,
}

impl Default for SaasCronConfig {
    fn default() -> Self {
        Self {
            agent_jobs_only: false,
            min_interval_secs: default_cron_min_interval_secs(),
            max_active_jobs: default_cron_max_active_jobs(),
        }
    }
}

fn default_cron_min_interval_secs() -> u64 {
    300
}

fn default_cron_max_active_jobs() -> u32 {
    20
}

/// `[sandbox]`: the one-shot container a user's shell command runs in. The
/// user's `sandbox/` directory is its only writable mount; the root filesystem
/// is read-only and every capability is dropped (`sandbox::docker`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaasSandboxConfig {
    /// Image to run in. Use one whose default user is not root.
    #[serde(default = "default_sandbox_image")]
    pub image: String,
    /// Docker network. `none` (the default) gives the container no network;
    /// `host` is refused at boot.
    #[serde(default = "default_sandbox_network")]
    pub network: String,
    #[serde(default = "default_sandbox_memory_mb")]
    pub memory_limit_mb: u64,
    #[serde(default = "default_sandbox_cpus")]
    pub cpu_limit: f64,
}

impl Default for SaasSandboxConfig {
    fn default() -> Self {
        Self {
            image: default_sandbox_image(),
            network: default_sandbox_network(),
            memory_limit_mb: default_sandbox_memory_mb(),
            cpu_limit: default_sandbox_cpus(),
        }
    }
}

fn default_sandbox_image() -> String {
    "alpine:3.20".to_string()
}

fn default_sandbox_network() -> String {
    "none".to_string()
}

fn default_sandbox_memory_mb() -> u64 {
    512
}

fn default_sandbox_cpus() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_max_profiles_open() -> usize {
    256
}

fn default_idle_evict_secs() -> u64 {
    30 * 60
}

fn default_lease_ttl_secs() -> u64 {
    30
}

/// The environment variable that names this node (see [`SaasConfig::node_id`]).
pub const NODE_ID_VAR: &str = "OPENHUMAN_NODE_ID";

impl SaasConfig {
    /// A config rooted at `root` with every default.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            chat_provider: None,
            search: None,
            search_role_names: std::collections::BTreeMap::new(),
            service_token_file: None,
            tool_allowlist: Vec::new(),
            sandbox: SaasSandboxConfig::default(),
            rpc_allowlist_extra: Vec::new(),
            max_profiles_open: default_max_profiles_open(),
            profile_ids: crate::profiles::ProfileIdMode::default(),
            idle_evict_secs: default_idle_evict_secs(),
            shared_backend_api_key: false,
            custom_definitions: false,
            require_user_signature: true,
            storage_url: None,
            node_id: None,
            advertise_url: None,
            lease_ttl_secs: default_lease_ttl_secs(),
            operator_dir: None,
            cron: SaasCronConfig::default(),
            max_tool_iterations: None,
        }
    }

    /// Read the operator's TOML file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading SaaS config {}: {e}", path.display()))?;
        let config: Self = toml::from_str(&raw).map_err(|e| {
            // `message()` omits the source excerpt, which could hold a key.
            anyhow::anyhow!("parsing SaaS config {}: {}", path.display(), e.message())
        })?;
        log::debug!(
            "[saas] loaded operator config from {} (root={})",
            path.display(),
            config.root.display()
        );
        Ok(config)
    }

    /// Where the gateway bearer is read from.
    pub fn service_token_path(&self) -> PathBuf {
        self.service_token_file
            .clone()
            .unwrap_or_else(|| self.root.join("service.token"))
    }

    /// The operator's own state directory: [`Self::operator_dir`] when set,
    /// else `<root>/operator`.
    pub fn operator_dir(&self) -> PathBuf {
        self.operator_dir
            .clone()
            .unwrap_or_else(|| self.root.join("operator"))
    }

    /// The storage URL in effect: `env` (`OPENHUMAN_STORAGE_URL`) when set,
    /// else [`Self::storage_url`]. Blank values count as unset.
    pub fn storage_url_with(&self, env: Option<&str>) -> Option<String> {
        env.into_iter()
            .chain(self.storage_url.as_deref())
            .map(str::trim)
            .find(|url| !url.is_empty())
            .map(str::to_owned)
    }

    /// [`Self::storage_url_with`] the process environment.
    pub fn resolved_storage_url(&self) -> Option<String> {
        let env = std::env::var(crate::storage::STORAGE_URL_VAR).ok();
        self.storage_url_with(env.as_deref())
    }

    /// Fill in [`Self::node_id`]: the configured one, else `env`
    /// (`OPENHUMAN_NODE_ID`), else a fresh random id. Called once at boot so
    /// every lease this process takes names the same node.
    pub fn resolve_node_id(&mut self, env: Option<&str>) -> &str {
        let id = self
            .node_id
            .as_deref()
            .into_iter()
            .chain(env)
            .map(str::trim)
            .find(|id| !id.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("node-{}", uuid::Uuid::new_v4().simple()));
        self.node_id.insert(id).as_str()
    }

    /// Whether this node is one of a cluster (it advertises an endpoint).
    pub fn is_clustered(&self) -> bool {
        self.advertise_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
    }

    /// How long a profile lease lasts without renewal (at least one second).
    pub fn lease_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.lease_ttl_secs.max(1))
    }

    /// The config the operator plane boots with. It roots every path under
    /// [`Self::operator_dir`], so booting never resolves the host's
    /// `~/.openhuman` or an `active_user.toml`.
    pub fn operator_config(&self) -> crate::config::Config {
        let dir = self.operator_dir();
        crate::config::Config {
            workspace_dir: dir.join("workspace"),
            config_path: dir.join("config.toml"),
            action_dir: dir.join("action"),
            ..crate::config::Config::default()
        }
    }
}

impl ServiceSet {
    /// The services a SaaS core runs: the HTTP JSON-RPC transport and nothing
    /// that acts on a user's data from the background.
    pub fn saas() -> Self {
        Self::headless_api()
    }
}

impl DomainSet {
    /// The domain families a SaaS core registers: the operator plane and the
    /// user families whose per-user isolation has landed. Profiles derive
    /// their contexts from these; `profiles::surface` keeps the operator
    /// scope on its own plane and each user on the user allowlist.
    pub fn saas() -> Self {
        Self {
            operator: true,
            agent: true,
            threads: true,
            channels: true,
            memory: true,
            ..Self::none()
        }
    }
}

/// Boot a SaaS core: check the deployment, lock the process to SaaS, and
/// build the operator plane.
pub async fn build(
    config: SaasConfig,
    host: Option<String>,
    port: Option<u16>,
) -> anyhow::Result<CoreRuntime> {
    build_with_profile_capabilities(config, host, port, None).await
}

/// Boot with immutable operator adapters for authenticated profile turns.
///
/// # Errors
/// As [`build`].
pub async fn build_with_profile_capabilities(
    mut config: SaasConfig,
    host: Option<String>,
    port: Option<u16>,
    capabilities: Option<Arc<crate::profiles::capabilities::ProfileCapabilities>>,
) -> anyhow::Result<CoreRuntime> {
    crate::profiles::provider_settings::ProfileProviderSettings::from_saas(&config)?;
    if CoreContext::default_context().is_some() {
        anyhow::bail!(
            "[saas] a core is already running in this process; a SaaS core must be the only one"
        );
    }
    mode::reserve_saas_boot().map_err(|e| anyhow::anyhow!("[saas] {e}"))?;

    let services = ServiceSet::saas();
    let domains = DomainSet::saas();
    let token = ServiceToken::read(&config.service_token_path());
    let env: Vec<(String, String)> = std::env::vars().collect();
    let sandbox_available = if boot_guard::needs_sandbox(&config) {
        crate::sandbox::docker::is_docker_available(&crate::config::Config::default())
            .await
            .unwrap_or(false)
    } else {
        false
    };
    boot_guard::check(&BootInputs {
        host_kind: HostKind::Saas,
        services,
        domains,
        config: &config,
        token: &token,
        env: &env,
        home: dirs::home_dir(),
        sandbox_available,
    })?;
    let ServiceToken::Valid(bearer) = token else {
        unreachable!("boot guard accepts only a valid service token");
    };
    let node_env = std::env::var(NODE_ID_VAR).ok();
    let node_id = config.resolve_node_id(node_env.as_deref()).to_owned();

    mode::lock_mode(Mode::Saas).map_err(|e| anyhow::anyhow!("[saas] {e}"))?;

    let operator = config.operator_config();
    for dir in [&operator.workspace_dir, &operator.action_dir] {
        std::fs::create_dir_all(dir)
            .map_err(|e| anyhow::anyhow!("[saas] creating {}: {e}", dir.display()))?;
    }
    // Root the keyring (the master key and every stored credential) under the
    // operator directory, never the host's `~/.openhuman`.
    crate::security::secret_storage::init_workspace(&operator.workspace_dir);
    let keyring_dir = crate::security::secret_storage::store::workspace_dir_for_file_backend();
    if keyring_dir != operator.workspace_dir {
        anyhow::bail!(
            "[saas] the keyring is already rooted at {}; a SaaS core keeps it under {}",
            keyring_dir.display(),
            operator.workspace_dir.display()
        );
    }
    log::info!(
        "[saas] booting operator plane root={} node={node_id} clustered={} lease_ttl_secs={} \
         max_profiles_open={} profile_ids={:?} idle_evict_secs={}",
        config.root.display(),
        config.is_clustered(),
        config.lease_ttl_secs,
        config.max_profiles_open,
        config.profile_ids,
        config.idle_evict_secs
    );

    // Built-in agent definitions only. The registry is process-wide and the
    // first initialiser wins, so seed it before the core builds: a lazy init
    // during the build would otherwise load one user's workspace definitions
    // for everyone.
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()?;

    let mut builder = CoreBuilder::new(HostKind::Saas)
        .token(TokenSource::Fixed(Arc::new(bearer)))
        .services(services)
        .domains(domains)
        .tool_groups(ToolGroups::none())
        .config(operator);
    if let Some(host) = host {
        builder = builder.host(host);
    }
    if let Some(port) = port {
        builder = builder.port(port);
    }
    let runtime = builder.build().await?;
    // Whoever initialised first won; refuse to serve users from a registry
    // that holds anything but the built-ins.
    verify_builtin_definitions(
        crate::agent_registry::host_definition::AgentDefinitionRegistry::global(),
    )?;
    // A relayed platform message (`channel_relay_inbound`) runs through the
    // channel dispatch pipeline, which asks the native bus for an
    // `agent.run_turn`. The `Agent` family stays off in SaaS (no agent RPCs
    // on any surface), so its subscriber plan never registers that handler;
    // register it alone. The request carries the caller's own turn parts, and
    // the handler runs in the caller's (profile's) scope.
    crate::agent::bus::register_agent_handlers();
    log::debug!("[saas] registered the native agent.run_turn handler for relayed channel turns");
    // With a storage backend installed (the host opened the configured URL
    // before boot), the profile registry and leases live in it and every node
    // sharing it contends for the same profiles; without one they are files
    // under the root.
    let backend = crate::storage::installed();
    if config.resolved_storage_url().is_some() && backend.is_none() {
        anyhow::bail!(
            "[saas] a storage URL is configured but no backend is installed; \
             the host must open it before boot"
        );
    }
    let host = Arc::new(
        crate::profiles::ProfileHost::with_fenced_backend(
            config,
            runtime.context().clone(),
            backend,
        )
        .map_err(|e| anyhow::anyhow!("[saas] {e}"))?
        .with_capabilities(capabilities),
    );
    crate::profiles::host::install(Arc::clone(&host));
    crate::profiles::lease::heartbeat(Arc::clone(&host), runtime.context().clone());
    crate::profiles::background::spawn(
        &crate::core::runtime::DispatchCtx::new(runtime.context().clone()),
        host,
    );
    Ok(runtime)
}

/// Refuse boot unless the process-wide agent definition registry is seeded and
/// holds the built-ins only.
pub(crate) fn verify_builtin_definitions(
    registry: Option<&crate::agent_registry::host_definition::AgentDefinitionRegistry>,
) -> anyhow::Result<()> {
    let Some(registry) = registry else {
        anyhow::bail!("[saas] the agent definition registry was not seeded");
    };
    if !registry.holds_builtins_only() {
        anyhow::bail!(
            "[saas] the agent definition registry holds workspace or home definitions; a SaaS core serves the built-ins only"
        );
    }
    log::info!(
        "[saas] agent definitions: {} built-in(s), no workspace or home overrides",
        registry.len()
    );
    Ok(())
}

#[cfg(test)]
#[path = "saas_tests.rs"]
mod tests;
