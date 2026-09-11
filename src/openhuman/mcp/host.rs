//! The `tinymcp` service this process holds, and the conversion into it.
//!
//! The MCP client lives in `tinymcp` now. What is left here is the host half:
//! this holder, the RPC surface over it, the agent-facing tools, and the
//! prompt-injection scan over remote tool definitions.
//!
//! # Why a holder exists at all
//!
//! Before the extraction the connection map, the pending-authorization map and
//! the secret vault were process globals, so every caller reached them by
//! calling a free function. `tinymcp` owns them instead — a library that
//! installed globals would make two hosts in one process share connections, and
//! would make its own tests order-dependent. So one instance lives here, and
//! the free functions callers used become methods on it.
//!
//! # It is initialised once, at boot
//!
//! [`init`] is called from the core startup path, before anything that might
//! reach for a server. A caller that arrives before then gets `None` rather
//! than a lazily-built service on a configuration nobody chose.
//!
//! # The proxy decision is made here
//!
//! `tinymcp` takes an already-resolved proxy rather than a policy: whether a
//! proxy applies to a given service is decided by this application's scope
//! setting, its per-service list and its no-proxy list, and all three live
//! here. [`proxy_for_mcp`] consults them and hands over the answer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tinymcp::{
    AuditStore, McpClientConfig, McpClientIdentityConfig, McpProxyConfig, McpRegistry,
    McpServerConfig, McpServerRegistry, Store,
};

use crate::openhuman::config::Config;

/// The service key MCP traffic is scoped under for proxy decisions.
const PROXY_SERVICE_KEY: &str = "tool.mcp_client";

/// The port the core serves on when nothing says otherwise.
const DEFAULT_CORE_PORT: u16 = 7788;

/// One service per workspace, opened on first use.
///
/// Not a single `OnceLock`. Every entry point into this domain is addressed by
/// configuration — an RPC handler loads a `Config` and acts on *that* workspace
/// — and one process serves more than one over its life: the workspace can be
/// switched in place, and the test suite runs each case against its own. A
/// single service bound to whichever configuration booted first would answer
/// every one of those from the wrong store, silently.
///
/// The map is what keeps that cheap. Opening a service runs the store's
/// migrations and builds an HTTP client, which no request path should pay for.
/// Each entry carries the identity and proxy its host was built with, so the
/// reconnect supervisor can dial with the same identity the host's connections
/// use — `McpRegistry` keeps those private.
static HOSTS: OnceLock<Mutex<HashMap<PathBuf, HostEntry>>> = OnceLock::new();

/// One open host, with the identity and proxy it was built with.
///
/// The reconnect supervisor needs those to dial a reconnecting server with the
/// same identity the host's connections use, and `McpRegistry` keeps them
/// private. Storing them here — from the same `client_config` the host was
/// opened from — is what lets one supervisor task cover every workspace: it
/// rebuilds each host's `tinymcp::Supervisor` from the stored values rather
/// than recomputing them from a configuration it does not have.
#[derive(Debug)]
struct HostEntry {
    host: Arc<McpHost>,
    identity: McpClientIdentityConfig,
    proxy: Option<McpProxyConfig>,
}

/// The workspace [`init`] opened, for the callers that have no `Config`.
///
/// A handful of paths — dropping a connection, listing a connected server's
/// tools — are addressed by server id alone, because the connection map they
/// read used to be a process global. They resolve through here.
static DEFAULT_WORKSPACE: OnceLock<PathBuf> = OnceLock::new();

/// The three pieces of `tinymcp` this application drives.
///
/// Held together rather than separately because they are initialised from one
/// configuration and share a lifetime. This is the same shape `tinymcp`'s own
/// bus adapter assembles; that adapter is not used here, because a host calling
/// the library directly has no use for a bus inside it.
#[derive(Debug)]
pub struct McpHost {
    dynamic: McpRegistry,
    static_servers: McpServerRegistry,
    audit: AuditStore,
}

impl McpHost {
    /// Builds a host from `config`, without installing it as the process one.
    ///
    /// Public so a test can drive a host over its own workspace: the
    /// process-wide one is a `OnceLock`, and a suite that needs a fresh store
    /// per case cannot share it. The startup path and [`for_config`] build
    /// through [`from_client_config`] instead, so a host and the entry stored
    /// beside it share one conversion.
    ///
    /// # Errors
    ///
    /// Returns an error when either store cannot be opened, or when an HTTP
    /// client cannot be built.
    pub fn open(config: &Config) -> anyhow::Result<Self> {
        let client = client_config(config);
        Self::from_client_config(config.workspace_dir.as_path(), &client)
    }

    /// Builds a host over `workspace` from an already-converted client
    /// configuration.
    ///
    /// [`open`] and [`for_config`] share this so both derive from the one
    /// conversion: a host and the entry stored beside it can never disagree
    /// about which identity and proxy they were built with.
    fn from_client_config(workspace: &Path, client: &McpClientConfig) -> anyhow::Result<Self> {
        Ok(Self {
            dynamic: McpRegistry::new(
                Store::open(workspace)
                    .map_err(|error| anyhow::anyhow!("failed to open the mcp store: {error}"))?,
                client.registry_auth.clone(),
                client.client_identity.clone(),
                client.proxy.clone(),
            )
            .map_err(|error| anyhow::anyhow!("failed to start the mcp registry: {error}"))?,
            static_servers: McpServerRegistry::from_config(client).map_err(|error| {
                anyhow::anyhow!("failed to build the static server set: {error}")
            })?,
            audit: AuditStore::open(workspace)
                .map_err(|error| anyhow::anyhow!("failed to open the mcp audit log: {error}"))?,
        })
    }

    /// The user-installed servers, their connections, and the catalogs.
    #[must_use]
    pub fn dynamic(&self) -> &McpRegistry {
        &self.dynamic
    }

    /// The servers declared in this application's own configuration.
    #[must_use]
    pub fn static_servers(&self) -> &McpServerRegistry {
        &self.static_servers
    }

    /// The write-audit log.
    #[must_use]
    pub fn audit(&self) -> &AuditStore {
        &self.audit
    }
}

/// The service for `config`'s workspace, opening it on first use.
///
/// This is the entry point for every caller that has a `Config`, which is
/// almost all of them. Two calls with the same workspace get the same service,
/// so a connection opened through one is visible through the other.
///
/// # Errors
///
/// Returns an error when the stores cannot be opened or an HTTP client cannot
/// be built.
///
/// Opening is not done under the process-wide lock: building a host runs the
/// store's migrations and constructs an HTTP client, which no request path
/// should pay for and no lock should be held across. The host is built first,
/// and only then is the map locked to see whether another initializer won the
/// race; if one did, its service is returned and the freshly-built one is
/// dropped.
pub fn for_config(config: &Config) -> anyhow::Result<Arc<McpHost>> {
    let workspace = config.workspace_dir.clone();

    // A host already under this workspace needs no re-opening: it would be
    // wasteful (and, on the common path, a plain Mutex of the shared map held
    // only for a lookup) to build a second one before noticing the first.
    let hosts = HOSTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(existing) = hosts
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&workspace)
    {
        return Ok(Arc::clone(&existing.host));
    }

    // Build outside the lock: opening runs the store's migrations and builds
    // an HTTP client, neither of which should serialize or block a Tokio
    // worker behind the process-wide map. The client configuration is
    // converted once, so the host and the entry stored beside it share it.
    let client = client_config(config);
    let service = Arc::new(McpHost::from_client_config(&workspace, &client)?);

    // Recheck before inserting — a concurrent initializer may have won the
    // race while the service was being built. Its Arc wins; ours is dropped.
    let mut hosts = hosts
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(existing) = hosts.get(&workspace) {
        return Ok(Arc::clone(&existing.host));
    }

    hosts.insert(
        workspace,
        HostEntry {
            host: Arc::clone(&service),
            identity: client.client_identity,
            proxy: client.proxy,
        },
    );

    Ok(service)
}

/// Opens the service for `config` and marks its workspace the default.
///
/// Called once, from the core startup path. A second call opens nothing new and
/// leaves the default alone, so a service already holding live connections is
/// never displaced.
///
/// # Errors
///
/// Returns an error when the stores cannot be opened or an HTTP client cannot
/// be built. A caller should log and continue: MCP being unavailable must not
/// stop the core coming up.
pub fn init(config: &Config) -> anyhow::Result<()> {
    for_config(config)?;

    if DEFAULT_WORKSPACE.set(config.workspace_dir.clone()).is_err() {
        tracing::debug!("[mcp] a default workspace was already set; leaving it alone");
        return Ok(());
    }

    tracing::info!(workspace = ?config.workspace_dir, "[mcp] service initialised");
    Ok(())
}

/// The service for the paths that have no `Config` to resolve by.
///
/// Anything holding one should call [`for_config`] instead, which cannot answer
/// `None` and cannot answer from the wrong workspace.
///
/// Resolves in two steps. The workspace [`init`] opened wins, because that is
/// the one this process booted against. Failing that — nothing has booted, but
/// something opened a service anyway — a *single* open workspace is
/// unambiguously the one meant, so it answers that. With several open and no
/// default, there is no honest answer and it says so.
///
/// The second step is what makes the connection map readable from a caller that
/// never booted: the agent's tool registry asks which MCP tools are live, and it
/// asks by server id alone. Without it, a process that opened a service through
/// [`for_config`] would still report nothing connected.
#[must_use]
pub fn try_service() -> Option<Arc<McpHost>> {
    let hosts = HOSTS
        .get()?
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    resolve(DEFAULT_WORKSPACE.get().map(PathBuf::as_path), &hosts)
}

/// The rule [`try_service`] applies, as a function of its inputs.
///
/// Separated from the process state so it can be tested. The state it reads is
/// two `OnceLock`s and a process-wide map that every case in this suite shares,
/// so a test driving `try_service` directly could only assert whatever the rest
/// of the suite happened to leave behind — which is to say, nothing.
fn resolve(default: Option<&Path>, hosts: &HashMap<PathBuf, HostEntry>) -> Option<Arc<McpHost>> {
    if let Some(workspace) = default {
        if let Some(entry) = hosts.get(workspace) {
            return Some(Arc::clone(&entry.host));
        }
    }

    match hosts.len() {
        1 => hosts.values().next().map(|entry| Arc::clone(&entry.host)),
        _ => None,
    }
}

/// The default workspace's service, erroring when it is not up yet.
///
/// # Errors
///
/// Returns a message naming the state, so an RPC caller learns the core is
/// still starting rather than that their server does not exist.
pub fn service() -> Result<Arc<McpHost>, String> {
    try_service().ok_or_else(|| "the mcp service is not initialised yet".to_string())
}

/// Every host currently open, with the identity and proxy each was built with.
///
/// The reconnect supervisor walks this each tick: one task supervises every
/// workspace's installed servers, including a host opened after boot (a
/// workspace switch), not just the one booted against. The identity and proxy
/// ride along because a reconnect must dial with the same identity the host's
/// connections use, and `McpRegistry` keeps those private.
#[must_use]
pub fn all_hosts() -> Vec<(
    PathBuf,
    Arc<McpHost>,
    McpClientIdentityConfig,
    Option<McpProxyConfig>,
)> {
    let Some(hosts) = HOSTS.get() else {
        return Vec::new();
    };

    hosts
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|(workspace, entry)| {
            (
                workspace.clone(),
                Arc::clone(&entry.host),
                entry.identity.clone(),
                entry.proxy.clone(),
            )
        })
        .collect()
}

/// Converts the MCP section of this application's configuration.
pub fn client_config(config: &Config) -> McpClientConfig {
    let mut client = McpClientConfig {
        enabled: config.mcp_client.enabled,
        servers: config
            .mcp_client
            .servers
            .iter()
            .filter(|server| credentialed_endpoint_transport_allowed(server))
            .map(server_config)
            .collect(),
        proxy: proxy_for_mcp(),
        ..McpClientConfig::default()
    };

    client.client_identity.name = config.mcp_client.client_identity.name.clone();
    client.client_identity.title = config.mcp_client.client_identity.title.clone();
    client.client_identity.version = config.mcp_client.client_identity.version.clone();

    client
        .registry_auth
        .smithery_api_key
        .clone_from(&config.mcp_client.registry_auth.smithery_api_key);
    client
        .registry_auth
        .mcp_official_base
        .clone_from(&config.mcp_client.registry_auth.mcp_official_base);
    client
        .registry_auth
        .mcp_official_token
        .clone_from(&config.mcp_client.registry_auth.mcp_official_token);

    // The documentation server is seeded here rather than by the module: it is
    // this application's own, and `tinymcp` has no business knowing about it.
    if config.gitbooks.enabled
        && !client
            .servers
            .iter()
            .any(|server| server.name == GITBOOKS_SERVER_NAME)
    {
        client.servers.push(McpServerConfig {
            name: GITBOOKS_SERVER_NAME.to_string(),
            endpoint: config.gitbooks.endpoint.clone(),
            description: Some("OpenHuman GitBook documentation MCP server.".to_string()),
            timeout_secs: config.gitbooks.timeout_secs,
            ..McpServerConfig::default()
        });
    }

    client
}

/// The name the documentation server is registered under.
pub const GITBOOKS_SERVER_NAME: &str = "gitbooks";

/// Whether a declared server's transport may be dialed with its configured
/// credentials attached.
///
/// Attaching a credential to an outbound request only makes sense when the
/// request cannot be read by a third party in transit — i.e. when the endpoint
/// is HTTPS, or is on the loopback interface where the bytes never leave the
/// machine. A credentialed plaintext-HTTP endpoint on any other host would put
/// the credential on the wire in the clear, so it is refused: the server is
/// dropped from the configured set with a warning rather than silently sent.
///
/// stdio servers (no `endpoint`) and credential-less servers are always fine.
fn credentialed_endpoint_transport_allowed(
    server: &crate::openhuman::config::McpServerConfig,
) -> bool {
    use crate::openhuman::config::McpAuthConfig;

    if matches!(server.auth, McpAuthConfig::None) {
        return true;
    }
    let endpoint = server.endpoint.trim();
    if endpoint.is_empty() {
        // stdio subprocess — credentials are passed via env, not the wire.
        return true;
    }
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        // Unparsable endpoints fail later with a clearer "could not build the
        // static server set" message; nothing is attached on this path.
        return true;
    };
    if url.scheme() == "https" {
        return true;
    }
    if url.scheme() != "http" {
        // Unknown scheme — let the transport decide rather than guessing.
        return true;
    }
    let loopback = url
        .host_str()
        .map(|host| host == "localhost" || host == "127.0.0.1" || host == "::1")
        .unwrap_or(false);
    if loopback {
        return true;
    }
    tracing::warn!(
        "[mcp] refusing credentialed server `{}`: plaintext HTTP endpoint {endpoint}          is not loopback; use https:// instead",
        server.name
    );
    false
}

/// Converts one declared server.
fn server_config(server: &crate::openhuman::config::McpServerConfig) -> McpServerConfig {
    McpServerConfig {
        name: server.name.clone(),
        endpoint: server.endpoint.clone(),
        command: server.command.clone(),
        args: server.args.clone(),
        // Ordered on the way across, so the serialized form does not depend on
        // hash iteration order.
        env: server
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        cwd: server.cwd.clone(),
        description: server.description.clone(),
        enabled: server.enabled,
        allowed_tools: server.allowed_tools.clone(),
        disallowed_tools: server.disallowed_tools.clone(),
        timeout_secs: server.timeout_secs,
        auth: auth_config(&server.auth),
    }
}

/// Converts one server's credentials.
fn auth_config(auth: &crate::openhuman::config::McpAuthConfig) -> tinymcp::McpAuthConfig {
    use crate::openhuman::config::McpAuthConfig as Host;
    use tinymcp::McpAuthConfig as Module;

    match auth {
        Host::None => Module::None,
        Host::BearerToken { token } => Module::BearerToken {
            token: token.clone(),
        },
        Host::Basic { username, password } => Module::Basic {
            username: username.clone(),
            password: password.clone(),
        },
        Host::Header { name, value } => Module::Header {
            name: name.clone(),
            value: value.clone(),
        },
        Host::Headers { headers } => Module::Headers {
            headers: headers
                .iter()
                .map(|header| tinymcp::HttpHeader::new(&header.name, &header.value))
                .collect(),
        },
        Host::QueryParam { name, value } => Module::QueryParam {
            name: name.clone(),
            value: value.clone(),
        },
    }
}

/// The statically declared server set, built from this application's
/// configuration.
///
/// A set that cannot be built — a malformed proxy, an unusable TLS setting — is
/// logged and reported as empty. Those conditions affect every server equally,
/// and taking the whole tool surface down over one of them helps nobody: the
/// agent loses its MCP tools either way, and an empty set says so without also
/// failing whatever else was being assembled.
#[must_use]
pub fn static_registry(config: &Config) -> McpServerRegistry {
    match McpServerRegistry::from_config(&client_config(config)) {
        Ok(registry) => registry,
        Err(error) => {
            tracing::warn!("[mcp] could not build the static server set: {error}");
            McpServerRegistry::default()
        }
    }
}

/// The loopback address a browser sign-in redirects back to.
///
/// `tinymcp` takes this rather than deriving it, and correctly: only this
/// application knows which port it actually bound, which may not be the one it
/// asked for. A wrong guess sends the browser to a dead listener, where sign-in
/// simply hangs with nothing to explain it.
///
/// In order: the address the core advertised after binding, then the requested
/// port, then the default.
pub fn oauth_redirect_uri() -> String {
    let port = advertised_port()
        .or_else(|| {
            std::env::var("OPENHUMAN_CORE_PORT")
                .ok()
                .and_then(|value| value.trim().parse::<u16>().ok())
        })
        .unwrap_or(DEFAULT_CORE_PORT);

    format!("http://127.0.0.1:{port}/oauth/mcp/callback")
}

/// The port the core actually bound, read from what it advertised.
///
/// Authoritative when present: it reflects any fallback the core took when its
/// preferred port was busy.
fn advertised_port() -> Option<u16> {
    let advertised = std::env::var("OPENHUMAN_CORE_RPC_URL").ok()?;
    explicit_port(&advertised)
}

/// The explicit port in a URL, or `None` when it names none.
///
/// Kept separate so it is testable without touching the process environment.
fn explicit_port(url: &str) -> Option<u16> {
    reqwest::Url::parse(url).ok().and_then(|url| url.port())
}

/// The proxy MCP traffic should use, or `None` to connect directly.
///
/// Resolved here because the *policy* lives here: the scope setting, the
/// per-service list and the no-proxy list are this application's, and sending
/// the decision rather than the policy keeps them in the one place that owns
/// them.
pub fn proxy_for_mcp() -> Option<McpProxyConfig> {
    let proxy = crate::openhuman::config::runtime_proxy_config();

    if !proxy.should_apply_to_service(PROXY_SERVICE_KEY) {
        return None;
    }

    Some(McpProxyConfig {
        http_proxy: proxy.http_proxy.clone(),
        https_proxy: proxy.https_proxy.clone(),
        all_proxy: proxy.all_proxy.clone(),
        no_proxy: proxy.no_proxy.clone(),
    })
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
