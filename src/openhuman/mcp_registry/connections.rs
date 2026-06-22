//! Global in-process registry of active MCP client connections.
//!
//! Keyed by `server_id` (UUID). Connections are established by [`connect`]
//! and removed by [`disconnect`]. The actual transport
//! ([`McpStdioClient`] for local subprocess installs,
//! [`McpHttpClient`] for HTTP-remote installs hosted by Smithery /
//! similar) lives in [`crate::openhuman::mcp_client`] — this module just
//! owns the per-server lifecycle, the transport dispatch, and a global
//! handle map.
//!
//! Dispatch is driven by the `transport` field on each [`InstalledServer`],
//! which is set at install time by `mcp_setup_install_and_connect` and
//! persisted in the `mcp_servers.transport` column.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use serde_json::Value;
use tokio::sync::RwLock;

use crate::openhuman::config::{Config, HttpHeader, McpAuthConfig};
use crate::openhuman::mcp_client::{McpHttpClient, McpRemoteTool, McpStdioClient};

use super::store;
use super::types::{ConnStatus, InstalledServer, McpTool, ServerStatus, Transport};

/// Build a static HTTP auth config from an installed HTTP-remote server's
/// stored env values. Each non-empty entry is treated as a request header
/// (key = header name, value = the user-supplied secret) per the registry's
/// declared `remotes[].headers`. ALL such headers are applied — a server that
/// authenticates with more than one header (e.g. a client key + client secret)
/// gets every header on the dial, not just the first. `__`-prefixed keys are
/// internal bookkeeping (e.g. the OAuth refresh bundle) and are never sent.
/// Returns [`McpAuthConfig::None`] when nothing usable is stored — e.g.
/// OAuth-only servers, which then surface their 401 challenge at `initialize`.
fn build_http_auth(env: &[(String, String)]) -> McpAuthConfig {
    let headers: Vec<HttpHeader> = env
        .iter()
        .filter(|(k, v)| !k.starts_with("__") && !v.trim().is_empty())
        .map(|(name, value)| HttpHeader {
            name: name.clone(),
            value: value.clone(),
        })
        .collect();
    match headers.len() {
        0 => McpAuthConfig::None,
        // A single header keeps the simple `Header` variant (back-compat).
        1 => {
            let h = headers.into_iter().next().expect("len checked == 1");
            McpAuthConfig::Header {
                name: h.name,
                value: h.value,
            }
        }
        // Multiple headers are ALL sent (multi-header remote auth).
        _ => McpAuthConfig::Headers { headers },
    }
}

/// Follow redirects on `url` (unauthenticated) and return the final resolved
/// URL, so the authenticated MCP dial can target it directly.
///
/// HTTP clients strip the `Authorization` header across a **cross-origin**
/// redirect (a security default), so a server published behind a redirecting
/// vanity host (e.g. `sh.inference.ac` -> `api.inference.sh/mcp`) would never
/// receive its token. Resolving the final URL here means the authenticated
/// request has no redirect to strip. The final status (often 401/405 from the
/// real endpoint to an unauthenticated GET) is irrelevant — we only read
/// `resp.url()`. Returns `None` on any error; the caller falls back to the
/// original URL, and non-redirecting servers resolve to themselves (no-op).
async fn resolve_final_url(url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;
    match client.get(url).send().await {
        Ok(resp) => Some(resp.url().to_string()),
        Err(e) => {
            tracing::debug!("[mcp-registry] redirect resolution failed for {url}: {e}");
            None
        }
    }
}

/// Decide which URL to dial WITH the user's stored credentials, given the
/// original install URL and the redirect-resolved final URL.
///
/// `resolve_final_url` follows redirects unauthenticated, so a vanity host can
/// legitimately resolve cross-origin to its real API (e.g. `sh.inference.ac`
/// -> `api.inference.sh`). But blindly replaying stored auth headers to *any*
/// redirect target also lets a redirecting / compromised host retarget the
/// token to a different origin. As a guard we only honor a **cross-origin**
/// redirect for the authenticated dial when the final origin is **HTTPS** (TLS
/// authenticates the host and prevents a cleartext/downgrade leak); otherwise
/// we fall back to the original URL, where the HTTP client's own cross-origin
/// `Authorization` stripping protects the token. Same-origin redirects are
/// always honored. (Pinning the resolved origin at install time would harden
/// this further against a same-scheme HTTPS retarget — tracked as follow-up.)
fn credential_safe_dial_url(original: &str, resolved: String) -> String {
    let (Ok(o), Ok(r)) = (
        reqwest::Url::parse(original),
        reqwest::Url::parse(&resolved),
    ) else {
        return original.to_string();
    };
    let same_origin = o.scheme() == r.scheme()
        && o.host_str() == r.host_str()
        && o.port_or_known_default() == r.port_or_known_default();
    if same_origin || r.scheme() == "https" {
        resolved
    } else {
        tracing::warn!(
            "[mcp-registry] refusing to replay credentials to a non-HTTPS cross-origin redirect target \
             ({original} -> {resolved}); dialing the original url instead"
        );
        original.to_string()
    }
}

// ── Connection record ────────────────────────────────────────────────────────

/// Active transport for one connected MCP install. Mirrors
/// [`crate::openhuman::mcp_client::registry::McpTransportClient`] but lives
/// here so `mcp_registry` doesn't have to depend on the static-config
/// registry's specific wrapping. Both variants expose the same surface
/// (`initialize` / `list_tools` / `call_tool` / `close_session`) so callers
/// don't have to branch.
enum ActiveClient {
    Stdio(Arc<McpStdioClient>),
    Http(Arc<McpHttpClient>),
}

impl ActiveClient {
    async fn list_tools(&self) -> anyhow::Result<Vec<McpRemoteTool>> {
        match self {
            Self::Stdio(c) => c.list_tools().await,
            Self::Http(c) => c.list_tools().await,
        }
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> anyhow::Result<crate::openhuman::mcp_client::McpServerToolResult> {
        match self {
            Self::Stdio(c) => c.call_tool(name, arguments).await,
            Self::Http(c) => c.call_tool(name, arguments).await,
        }
    }

    async fn close_session(&self) -> anyhow::Result<()> {
        match self {
            Self::Stdio(c) => c.close_session().await,
            Self::Http(c) => c.close_session().await,
        }
    }
}

/// One live MCP client (stdio subprocess OR HTTP-remote dial) plus the
/// tool list cached after `initialize`.
struct Connection {
    client: ActiveClient,
    tools: RwLock<Vec<McpTool>>,
    /// Stable registry identity, stamped at connect time so a connected
    /// overview can name + describe servers without re-reading the install
    /// store (which needs `&Config`). Lets sync, config-free callers — e.g.
    /// the orchestrator prompt builder — list connected servers by name and
    /// description.
    qualified_name: String,
    display_name: String,
    description: Option<String>,
}

impl Connection {
    async fn tools_snapshot(&self) -> Vec<McpTool> {
        self.tools.read().await.clone()
    }
}

/// One connected server's identity + advertised tools, for prompt-surface
/// discovery (the orchestrator's "## Connected MCP Servers" block). Sourced
/// entirely from the live connection map — no `Config`, no store read.
#[derive(Debug, Clone)]
pub struct ConnectedServerOverview {
    pub server_id: String,
    pub qualified_name: String,
    pub display_name: String,
    /// Short registry description — the primary capability hint surfaced in
    /// the orchestrator prompt (mirrors Composio's per-toolkit description).
    pub description: Option<String>,
    /// Advertised tools — retained for a tool-count fallback when a server
    /// has no description, and for any caller that wants the full list.
    pub tools: Vec<McpTool>,
}

// ── Global registry ──────────────────────────────────────────────────────────

static CONNECTIONS: OnceLock<RwLock<HashMap<String, Arc<Connection>>>> = OnceLock::new();

fn connections() -> &'static RwLock<HashMap<String, Arc<Connection>>> {
    CONNECTIONS.get_or_init(|| RwLock::new(HashMap::new()))
}

// ── Per-server last connect error ────────────────────────────────────────────

/// The most recent connect failure for one server: the raw diagnostic message
/// (for logs/debugging) plus whether it was specifically an HTTP 401 (auth
/// required). Both live in ONE record under ONE lock so a status read can never
/// observe a torn snapshot — e.g. the message updated but the auth flag stale,
/// which a two-map design would allow if `all_status` interleaved between the
/// two writes (#3719).
#[derive(Clone)]
struct ConnectFailure {
    message: String,
    /// `true` when the failure was an MCP HTTP 401 → drives
    /// `ServerStatus::Unauthorized` so the UI offers a re-auth path instead of
    /// a raw error blob.
    unauthorized: bool,
}

static LAST_ERRORS: OnceLock<RwLock<HashMap<String, ConnectFailure>>> = OnceLock::new();

fn last_errors() -> &'static RwLock<HashMap<String, ConnectFailure>> {
    LAST_ERRORS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Whether a connect failure's root cause was an MCP HTTP 401 (auth required).
/// Uses a typed `downcast` on the `anyhow` chain — not string matching — so the
/// classification can't drift from the message wording.
fn is_unauthorized_error(err: &anyhow::Error) -> bool {
    // Walk the whole source chain (not just the outermost error) so the
    // classification survives any `?`/`.context()` wrapping a caller may add.
    err.chain()
        .any(|cause| cause.is::<crate::openhuman::mcp_client::McpUnauthorizedError>())
}

/// Read the most recent connect-failure message for `server_id`. `None` when
/// the server has never failed, or when the most recent connect succeeded.
pub async fn last_error_for(server_id: &str) -> Option<String> {
    last_errors()
        .read()
        .await
        .get(server_id)
        .map(|f| f.message.clone())
}

/// Whether `server_id`'s most recent connect failed due to HTTP 401.
pub async fn needs_auth(server_id: &str) -> bool {
    last_errors()
        .read()
        .await
        .get(server_id)
        .is_some_and(|f| f.unauthorized)
}

/// Drop any recorded error (generic or auth-required) for `server_id`. Called on
/// successful connect, explicit disconnect, uninstall, and enable→disable
/// transitions.
pub async fn clear_last_error(server_id: &str) {
    last_errors().write().await.remove(server_id);
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Bring up a new MCP client for `server`, run `initialize`, cache the
/// tool list, and store the connection in the global registry.
///
/// On success the prior error (if any) for this `server_id` is cleared.
/// On failure the error message is recorded in [`LAST_ERRORS`] so callers
/// can surface it in status polling without re-attempting the connect.
///
/// Dispatches on `server.transport`:
/// - [`Transport::Stdio`] — spawn `command` + `args` as a subprocess and
///   speak JSON-RPC over stdin/stdout (the original behaviour).
/// - [`Transport::HttpRemote`] — dial the published HTTPS endpoint
///   directly with [`McpHttpClient`]. No subprocess. Needed for the
///   `~99%` of Smithery listings that are HTTP-remote.
pub async fn connect(config: &Config, server: &InstalledServer) -> anyhow::Result<Vec<McpTool>> {
    let result = connect_inner(config, server).await;
    match &result {
        Ok(_) => {
            last_errors().write().await.remove(&server.server_id);
            tracing::debug!(
                "[mcp-registry] last_error cleared server_id={}",
                server.server_id
            );
        }
        Err(err) => {
            // Record the raw diagnostic AND the 401 classification together in a
            // single record under one lock, so a concurrent `all_status` can't
            // observe a torn snapshot (message set but auth flag stale).
            let unauthorized = is_unauthorized_error(err);
            last_errors().write().await.insert(
                server.server_id.clone(),
                ConnectFailure {
                    message: err.to_string(),
                    unauthorized,
                },
            );
            tracing::debug!(
                "[mcp-registry] last_error recorded server_id={} unauthorized={unauthorized} err={err}",
                server.server_id
            );
        }
    }
    result
}

async fn connect_inner(config: &Config, server: &InstalledServer) -> anyhow::Result<Vec<McpTool>> {
    tracing::debug!(
        "[mcp-registry] connect server_id={} qualified_name={} transport={}",
        server.server_id,
        server.qualified_name,
        server.transport.dispatch_kind()
    );

    let env_map = store::load_env_values(config, &server.server_id).unwrap_or_default();
    let env: Vec<(String, String)> = env_map.into_iter().collect();

    tracing::debug!(
        "[mcp-registry] connect server_id={} env_keys={:?}",
        server.server_id,
        env.iter().map(|(k, _)| k).collect::<Vec<_>>()
    );

    let identity = config.mcp_client.client_identity.clone();

    // Branch on transport variant. Both branches end with `initialize` +
    // `list_tools` so a misconfigured server fails loudly at connect
    // instead of silently at first `call_tool`.
    let client = match &server.transport {
        Transport::Stdio => {
            let stdio = Arc::new(McpStdioClient::new(
                server.command.clone(),
                server.args.clone(),
                env,
                None,
                identity,
            ));
            stdio.initialize().await?;
            ActiveClient::Stdio(stdio)
        }
        Transport::HttpRemote { url } => {
            if url.is_empty() {
                anyhow::bail!(
                    "[mcp-registry] http_remote server_id={} has empty deployment_url",
                    server.server_id
                );
            }
            // Refresh an expired OAuth access token before dialing so the agent
            // never connects with a stale token (silent refresh-token grant; a
            // no-op for static-token / no-auth servers).
            if let Err(e) = super::oauth::refresh_if_expired(config, &server.server_id).await {
                tracing::warn!(
                    "[mcp-registry] oauth refresh failed for server_id={} (using existing token): {e}",
                    server.server_id
                );
            }
            // Build static auth from the (possibly just-refreshed) stored env:
            // each entry is a request header (key = header name, value = the
            // secret, e.g. `Authorization` -> `Bearer <token>`), from the install
            // form's declared `remotes[].headers` or a captured OAuth token.
            let env_now: Vec<(String, String)> = store::load_env_values(config, &server.server_id)
                .unwrap_or_default()
                .into_iter()
                .collect();
            let auth = build_http_auth(&env_now);
            // Resolve redirects up-front and dial the FINAL url directly. A
            // server published behind a redirecting vanity host (e.g.
            // `sh.inference.ac` -> `api.inference.sh/mcp`) would otherwise lose
            // its `Authorization` header: HTTP clients strip auth across a
            // cross-origin redirect, so the token never reaches the real
            // endpoint. Resolving here means the authenticated request goes
            // straight to the final URL with no redirect to strip it.
            let resolved = resolve_final_url(url).await.unwrap_or_else(|| url.clone());
            let dial_url = credential_safe_dial_url(url, resolved);
            if dial_url != *url {
                tracing::info!(
                    "[mcp-registry] resolved redirecting url {url} -> {dial_url} for authenticated dial"
                );
            }
            // 30s timeout matches setup_ops::test_connection so install
            // and runtime see the same connect-failure deadlines.
            let http = Arc::new(McpHttpClient::with_options(dial_url, 30, auth, identity));
            http.initialize().await?;
            ActiveClient::Http(http)
        }
    };

    let remote_tools = client.list_tools().await?;
    let safe_remote_tools =
        crate::openhuman::mcp_client::apply_safety_filter(&server.server_id, remote_tools);
    let tools: Vec<McpTool> = safe_remote_tools
        .into_iter()
        .map(into_registry_tool)
        .collect();

    let conn = Arc::new(Connection {
        client,
        tools: RwLock::new(tools.clone()),
        qualified_name: server.qualified_name.clone(),
        display_name: server.display_name.clone(),
        description: server.description.clone(),
    });

    {
        let mut map = connections().write().await;
        map.insert(server.server_id.clone(), conn);
    }

    let _ = store::update_last_connected(config, &server.server_id);

    tracing::debug!(
        "[mcp-registry] connect ok server_id={} tools={}",
        server.server_id,
        tools.len()
    );

    Ok(tools)
}

/// Whether `server_id` currently has a live entry in the connection
/// registry. Note this only reflects map membership — a silently-dropped
/// transport stays in the map until something probes it. Use
/// [`probe_alive`] for an actual liveness check.
pub async fn is_connected(server_id: &str) -> bool {
    connections().read().await.contains_key(server_id)
}

/// Actively probe a connected server's transport by issuing a `tools/list`
/// round-trip under `timeout`. Returns `true` only when the call succeeds.
///
/// This is the detection mechanism the reconnect supervisor (#3312) relies
/// on: MCP transports can drop silently (subprocess exits, HTTP session
/// expires) while their `Connection` stays in the registry, so "is it in the
/// map" (`is_connected`) is not enough — a dead transport only surfaces on the
/// next actual call. A periodic lightweight probe converts that latent failure
/// into an observable one so the supervisor can disconnect + reconnect.
///
/// Returns `false` (rather than erroring) for a missing connection, a transport
/// error, or a timeout — all of which mean "not usable, reconnect".
pub async fn probe_alive(server_id: &str, timeout: std::time::Duration) -> bool {
    tracing::trace!("[mcp-registry] probe_alive server_id={server_id} timeout={timeout:?}");
    let conn = {
        let map = connections().read().await;
        map.get(server_id).cloned()
    };
    let Some(conn) = conn else {
        return false;
    };
    match tokio::time::timeout(timeout, conn.client.list_tools()).await {
        Ok(Ok(_)) => {
            tracing::trace!("[mcp-registry] probe_alive server_id={server_id} alive");
            true
        }
        Ok(Err(err)) => {
            tracing::debug!(
                "[mcp-registry] probe_alive server_id={server_id} transport error: {err}"
            );
            false
        }
        Err(_) => {
            tracing::debug!(
                "[mcp-registry] probe_alive server_id={server_id} timed out after {timeout:?}"
            );
            false
        }
    }
}

/// Disconnect and remove from the registry. Also clears any recorded
/// connect error so the next status poll starts from a clean slate.
pub async fn disconnect(server_id: &str) -> bool {
    tracing::debug!("[mcp-registry] disconnect server_id={server_id}");
    let conn = {
        let mut map = connections().write().await;
        map.remove(server_id)
    };
    last_errors().write().await.remove(server_id);
    if let Some(c) = conn {
        let _ = c.client.close_session().await;
        tracing::debug!("[mcp-registry] disconnected server_id={server_id}");
        true
    } else {
        tracing::debug!("[mcp-registry] disconnect noop server_id={server_id}");
        false
    }
}

/// Invoke `tools/call` on a connected server. The MCP `CallToolResult` is
/// returned as the raw JSON value (matches the prior wire contract used by
/// `tool_call`).
pub async fn call_tool(
    server_id: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let conn = {
        let map = connections().read().await;
        map.get(server_id).cloned()
    }
    .ok_or_else(|| format!("[mcp-registry] server_id={server_id} not connected"))?;

    conn.client
        .call_tool(tool_name, arguments)
        .await
        .map(|r| r.raw_result)
        .map_err(|e| e.to_string())
}

/// Return status summaries for all installed servers.
///
/// Priority order: `Disabled` > `Connected` > `Unauthorized` > `Error` >
/// `Disconnected`.
/// - `!s.enabled` → `Disabled` (suppresses tool count and last_error).
/// - connected (id in live registry) → `Connected` + tool count.
/// - connect failed with HTTP 401 (`AUTH_REQUIRED`) → `Unauthorized`. The raw
///   error string is intentionally NOT surfaced (it leaks an internal OAuth
///   metadata URL, #3719) — the UI renders a localized "needs sign-in" message
///   and the re-auth affordance keyed off the status alone.
/// - other recorded connect failure in `LAST_ERRORS` → `Error` + message.
/// - otherwise → `Disconnected`.
/// Pure status decision for one installed server, factored out of
/// [`all_status`] so the priority order is unit-testable without a live
/// connection registry or store. Inputs:
/// - `enabled` — the install's enabled flag.
/// - `connected_tool_count` — `Some(n)` when the server is in the live map
///   (with its advertised tool count), `None` otherwise.
/// - `auth_required` — most recent connect failed with HTTP 401.
/// - `generic_error` — most recent (non-401) connect error message, if any.
///
/// Priority: `Disabled` > `Connected` > `Unauthorized` > `Error` >
/// `Disconnected`. The raw error is surfaced ONLY for the generic `Error` case;
/// `Unauthorized` deliberately carries no message (the UI localizes it and
/// avoids leaking the OAuth metadata URL, #3719).
fn classify_server_status(
    enabled: bool,
    connected_tool_count: Option<u32>,
    auth_required: bool,
    generic_error: Option<String>,
) -> (ServerStatus, u32, Option<String>) {
    if !enabled {
        (ServerStatus::Disabled, 0, None)
    } else if let Some(n) = connected_tool_count {
        (ServerStatus::Connected, n, None)
    } else if auth_required {
        (ServerStatus::Unauthorized, 0, None)
    } else if let Some(err) = generic_error {
        (ServerStatus::Error, 0, Some(err))
    } else {
        (ServerStatus::Disconnected, 0, None)
    }
}

pub async fn all_status(config: &Config) -> Vec<ConnStatus> {
    let installed = store::list_servers(config).unwrap_or_default();
    let connected_ids: Vec<String> = {
        let map = connections().read().await;
        map.keys().cloned().collect()
    };

    // One snapshot of the unified failure map — message + auth flag are read
    // together, so a server's status can't be classified from a torn pair.
    let failures_snapshot = last_errors().read().await.clone();

    let mut out = Vec::with_capacity(installed.len());
    for s in installed {
        let is_connected = connected_ids.iter().any(|id| id == &s.server_id);

        // Resolve the live tool count up front (the only async input), then let
        // the pure classifier pick the status — keeps the priority logic
        // testable without a live registry / DB.
        let connected_tool_count = if is_connected {
            let map = connections().read().await;
            Some(match map.get(&s.server_id) {
                Some(c) => c.tools_snapshot().await.len() as u32,
                None => 0,
            })
        } else {
            None
        };

        let failure = failures_snapshot.get(&s.server_id);
        let (status, tool_count, last_error) = classify_server_status(
            s.enabled,
            connected_tool_count,
            failure.is_some_and(|f| f.unauthorized),
            // Only a generic (non-401) failure carries a surfaced message; the
            // 401 message is withheld (it leaks the OAuth metadata URL).
            failure
                .filter(|f| !f.unauthorized)
                .map(|f| f.message.clone()),
        );

        out.push(ConnStatus {
            server_id: s.server_id,
            qualified_name: s.qualified_name,
            display_name: s.display_name,
            status,
            tool_count,
            last_error,
        });
    }
    out
}

/// Collect tools from all currently-connected servers for tool_registry integration.
/// Returns `(server_id, qualified_name, tool)` triples. `qualified_name` is
/// best-effort sourced from the connection's `server_id` here — callers that
/// need the real qualified name should re-join against `store::list_servers`.
pub async fn all_connected_tools() -> Vec<(String, String, McpTool)> {
    let snapshot: Vec<(String, Arc<Connection>)> = {
        let map = connections().read().await;
        map.iter()
            .map(|(id, c)| (id.clone(), Arc::clone(c)))
            .collect()
    };

    let mut out: Vec<(String, String, McpTool)> = Vec::new();
    for (server_id, c) in snapshot {
        for tool in c.tools_snapshot().await {
            out.push((server_id.clone(), server_id.clone(), tool));
        }
    }
    out
}

/// Per-server overview of every currently-connected server: identity +
/// advertised tools. Used to surface connected MCP capabilities in the
/// orchestrator system prompt so it can route to `use_mcp_server` without
/// the user naming the server. Config-free (reads only the live map).
pub async fn connected_overview() -> Vec<ConnectedServerOverview> {
    let snapshot: Vec<(String, Arc<Connection>)> = {
        let map = connections().read().await;
        map.iter()
            .map(|(id, c)| (id.clone(), Arc::clone(c)))
            .collect()
    };

    let mut out = Vec::with_capacity(snapshot.len());
    for (server_id, c) in snapshot {
        out.push(ConnectedServerOverview {
            server_id,
            qualified_name: c.qualified_name.clone(),
            display_name: c.display_name.clone(),
            description: c.description.clone(),
            tools: c.tools_snapshot().await,
        });
    }
    // Stable order so the prompt (and its KV-cache prefix) doesn't churn
    // across turns purely from HashMap iteration order.
    out.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
    out
}

/// Snapshot the tools exposed by a single currently-connected server.
///
/// Returns `None` when `server_id` is not in the live connection map (the
/// caller should connect first); `Some(vec![])` when connected but the
/// server advertised no tools. This is the cheap discovery primitive the
/// agent uses to learn a connected server's tool names + input schemas
/// without forcing a reconnect/handshake (which [`connect`] would do).
pub async fn tools_for(server_id: &str) -> Option<Vec<McpTool>> {
    let conn = {
        let map = connections().read().await;
        map.get(server_id).cloned()
    }?;
    Some(conn.tools_snapshot().await)
}

// ── Boundary conversion ──────────────────────────────────────────────────────

fn into_registry_tool(remote: McpRemoteTool) -> McpTool {
    // Read through the sanitized display accessor so remote
    // description content is always bounded + scrubbed before reaching
    // the agent LLM context downstream.
    let description = remote.display_description();
    McpTool {
        name: remote.name,
        description,
        input_schema: remote.input_schema,
    }
}

#[cfg(test)]
mod tests {
    // Live-connection tests require a real MCP subprocess and live in
    // tests/json_rpc_e2e.rs. Keep this slot for sync helper tests.
    use super::{
        build_http_auth, classify_server_status, credential_safe_dial_url, is_unauthorized_error,
    };
    use crate::openhuman::config::McpAuthConfig;
    use crate::openhuman::mcp_client::McpUnauthorizedError;
    use crate::openhuman::mcp_registry::types::ServerStatus;

    #[test]
    fn classify_server_status_priority_order() {
        // Disabled wins over everything (even a live connection / 401 / error).
        assert_eq!(
            classify_server_status(false, Some(3), true, Some("boom".into())),
            (ServerStatus::Disabled, 0, None)
        );
        // Connected → tool count surfaced, no error.
        assert_eq!(
            classify_server_status(true, Some(5), true, Some("boom".into())),
            (ServerStatus::Connected, 5, None)
        );
        // Not connected + 401 → Unauthorized, and NO raw error is leaked even
        // when a generic error is also recorded.
        assert_eq!(
            classify_server_status(true, None, true, Some("MCP unauthorized … HTTP 401".into())),
            (ServerStatus::Unauthorized, 0, None)
        );
        // Not connected + generic error (no 401) → Error + message.
        assert_eq!(
            classify_server_status(true, None, false, Some("timed out".into())),
            (ServerStatus::Error, 0, Some("timed out".into()))
        );
        // Not connected, no error → Disconnected.
        assert_eq!(
            classify_server_status(true, None, false, None),
            (ServerStatus::Disconnected, 0, None)
        );
    }

    #[test]
    fn is_unauthorized_error_classifies_typed_401_only() {
        // A typed 401 from the transport → auth required (regardless of the
        // message wording, since we downcast rather than string-match).
        let unauth = anyhow::Error::new(McpUnauthorizedError {
            endpoint: "https://example.com".into(),
            resource_metadata: Some("https://example.com/.well-known/x".into()),
        });
        assert!(is_unauthorized_error(&unauth));

        // A 401 survives `?`-style context wrapping (anyhow keeps the root
        // downcastable), matching how the error reaches `connect`.
        let wrapped = unauth.context("connecting to MCP server");
        assert!(is_unauthorized_error(&wrapped));

        // Any other transport failure → NOT auth required (stays generic Error).
        let other = anyhow::anyhow!("MCP HTTP 500 — upstream exploded");
        assert!(!is_unauthorized_error(&other));
    }

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn build_http_auth_none_when_empty_or_blank() {
        assert!(matches!(build_http_auth(&[]), McpAuthConfig::None));
        assert!(matches!(
            build_http_auth(&kv(&[("Authorization", "   ")])),
            McpAuthConfig::None
        ));
    }

    #[test]
    fn build_http_auth_applies_all_headers_when_multiple() {
        // A server requiring more than one header (e.g. a client key + client
        // secret) must get EVERY header on the dial — not just the first.
        // Values are sent verbatim (e.g. an already-`Bearer ...` Authorization).
        let auth = build_http_auth(&kv(&[
            ("X-Client-Key", "abc"),
            ("authorization", "Bearer adv_sk_123"),
        ]));
        match auth {
            McpAuthConfig::Headers { headers } => {
                assert_eq!(
                    headers.len(),
                    2,
                    "both headers must be applied: {headers:?}"
                );
                let key = headers
                    .iter()
                    .find(|h| h.name == "X-Client-Key")
                    .expect("X-Client-Key present");
                assert_eq!(key.value, "abc");
                let auth = headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("authorization"))
                    .expect("authorization present");
                assert_eq!(auth.value, "Bearer adv_sk_123");
            }
            other => panic!("expected multi-header Headers auth, got {other:?}"),
        }
    }

    #[test]
    fn credential_safe_dial_url_guards_cross_origin_credential_replay() {
        // Same-origin redirect → honored (e.g. path rewrite).
        assert_eq!(
            credential_safe_dial_url("https://a.example/mcp", "https://a.example/v2/mcp".into()),
            "https://a.example/v2/mcp"
        );
        // Cross-origin but HTTPS (vanity host → real API) → honored: this is the
        // legitimate inference.sh-style flow.
        assert_eq!(
            credential_safe_dial_url(
                "https://sh.inference.ac/mcp",
                "https://api.inference.sh/mcp".into()
            ),
            "https://api.inference.sh/mcp"
        );
        // Cross-origin DOWNGRADE to http → refused: falls back to the original
        // url so creds are not replayed cleartext to a redirect-chosen origin.
        assert_eq!(
            credential_safe_dial_url("https://good.example/mcp", "http://evil.example/mcp".into()),
            "https://good.example/mcp"
        );
    }

    #[test]
    fn build_http_auth_single_header_uses_header_variant() {
        // Exactly one usable header keeps the simple `Header` variant.
        match build_http_auth(&kv(&[("authorization", "Bearer t")])) {
            McpAuthConfig::Header { name, value } => {
                assert!(name.eq_ignore_ascii_case("authorization"));
                assert_eq!(value, "Bearer t");
            }
            other => panic!("expected single Header auth, got {other:?}"),
        }
    }

    #[test]
    fn build_http_auth_skips_internal_underscore_keys() {
        // The OAuth refresh bundle (`__oauth__`) must never be sent as a header.
        assert!(matches!(
            build_http_auth(&kv(&[("__oauth__", "{\"refresh_token\":\"r\"}")])),
            McpAuthConfig::None
        ));
        // Authorization still applies alongside an internal key.
        match build_http_auth(&kv(&[("__oauth__", "{}"), ("Authorization", "Bearer t")])) {
            McpAuthConfig::Header { name, value } => {
                assert!(name.eq_ignore_ascii_case("authorization"));
                assert_eq!(value, "Bearer t");
            }
            other => panic!("expected Header auth, got {other:?}"),
        }
    }

    #[test]
    fn build_http_auth_single_custom_header() {
        let auth = build_http_auth(&kv(&[("X-API-Key", "secret")]));
        match auth {
            McpAuthConfig::Header { name, value } => {
                assert_eq!(name, "X-API-Key");
                assert_eq!(value, "secret");
            }
            other => panic!("expected Header auth, got {other:?}"),
        }
    }
}
