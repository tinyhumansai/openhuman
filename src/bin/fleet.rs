//! `openhuman-fleet` — a process-per-user supervisor + reverse proxy.
//!
//! Hosts one `openhuman-core` process per user/workspace and fronts them behind
//! a single endpoint so a team server can manage many members' assistants while
//! every existing client (`CloudHttpTransport`) keeps working unchanged. This is
//! Phase 4 of the pluggable-core plan (`docs/plans/pluggable-core/phase-4-fleet-host.md`).
//!
//! Design (process-per-user, not in-process multi-tenancy):
//! - Each tenant gets its own OS process (`openhuman-core run --headless-api`),
//!   its own workspace volume (`OPENHUMAN_WORKSPACE`), and its own core bearer
//!   (`OPENHUMAN_CORE_TOKEN`). This MVP does not yet run tenants under
//!   distinct OS users or containers, so it is not a production multi-tenant
//!   security boundary for arbitrary agent tools.
//! - The supervisor mints a distinct **edge token** per tenant for clients; it
//!   is the only holder of the tenants' **core bearers**. `EdgeToken` and
//!   `CoreBearer` are kept deliberately distinct so they cannot be confused.
//! - The reverse proxy forwards `POST /{user_id}/rpc` verbatim to that tenant's
//!   core `http://127.0.0.1:<port>/rpc`, so the JSON-RPC wire contract is
//!   unchanged end to end.
//!
//! MVP scope: explicit sequential port assignment with an authenticated JSON-RPC
//! readiness probe before registration (a production supervisor would read each
//! core's bound port from a ready file / `EmbeddedReadySignal` and reconcile
//! membership against `tinyhumansai/backend`). Limitations are logged, never
//! silently swallowed.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use axum::{
    body::Bytes,
    extract::Request,
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use clap::Parser;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Edge auth — maps opaque client-facing tokens to a user id.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeToken(String);

impl EdgeToken {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoreBearer(String);

impl CoreBearer {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

mod edge_auth {
    use super::EdgeToken;
    use std::collections::HashMap;

    /// Maps opaque, client-facing **edge tokens** to the user id they authorize.
    /// The fleet never hands a tenant's core bearer to a client — clients only
    /// ever see an edge token, which the proxy exchanges for the core bearer.
    #[derive(Default)]
    pub struct EdgeAuth {
        tokens: HashMap<EdgeToken, String>,
    }

    impl EdgeAuth {
        pub fn new() -> Self {
            Self::default()
        }

        /// Mint an edge token authorizing `user_id`. Deterministic prefix +
        /// caller-supplied unique suffix (a UUID at the call site) so this stays
        /// pure and unit-testable.
        pub fn insert(&mut self, token: EdgeToken, user_id: impl Into<String>) {
            self.tokens.insert(token, user_id.into());
        }

        /// The user id an edge token authorizes, if any.
        pub fn user_for(&self, token: &EdgeToken) -> Option<&str> {
            self.tokens.get(token).map(String::as_str)
        }

        pub fn remove_user(&mut self, user_id: &str) {
            self.tokens.retain(|_, mapped_user| mapped_user != user_id);
        }

        #[cfg(test)]
        pub fn len(&self) -> usize {
            self.tokens.len()
        }
    }
}

use edge_auth::EdgeAuth;

/// Keep the fleet proxy's `/rpc` request-body contract aligned with the core
/// server. Chat image attachments are base64-inlined into JSON-RPC bodies, and
/// direct core `/rpc` accepts up to 64 MiB for that path.
const MAX_RPC_BODY_BYTES: usize = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Tenant registry — pure derivation of per-user port / workspace / rpc url.
// ---------------------------------------------------------------------------

/// A provisioned tenant core: where it listens and the bearer to reach it.
#[derive(Debug, Clone)]
struct CoreInstance {
    user_id: String,
    port: u16,
    core_bearer: CoreBearer,
    workspace_dir: PathBuf,
    action_dir: PathBuf,
}

impl CoreInstance {
    /// The loopback RPC URL the proxy forwards to.
    fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}/rpc", self.port)
    }
}

/// Pure port assignment: tenant `index` (0-based) maps to `base_port + index`.
/// Kept a free function so it is trivially unit-testable and the policy is
/// obvious at the call site.
fn port_for_index(base_port: u16, index: usize) -> Option<u16> {
    u16::try_from(base_port as usize + index).ok()
}

/// Pure workspace derivation: `<root>/<user_id>`. The caller is responsible for
/// having validated `user_id` (see [`is_valid_user_id`]).
fn workspace_for(root: &Path, user_id: &str) -> PathBuf {
    root.join(user_id)
}

/// Pure action sandbox derivation: `<root>/.tenant-action-dirs/<user_id>`.
/// Keep this outside `workspace_dir`; core policy treats the workspace tree as
/// internal state and blocks acting tools there.
fn action_dir_for(root: &Path, user_id: &str) -> PathBuf {
    root.join(".tenant-action-dirs").join(user_id)
}

/// A user id must be a single safe path segment — no separators, no `..`, non
/// empty — so it cannot escape the workspaces root or the proxy route.
fn is_valid_user_id(user_id: &str) -> bool {
    !user_id.is_empty()
        && user_id.len() <= 128
        && user_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---------------------------------------------------------------------------
// Fleet state
// ---------------------------------------------------------------------------

struct Fleet {
    instances: HashMap<String, CoreInstance>,
    edge_auth: EdgeAuth,
    http: reqwest::Client,
}

impl Fleet {
    fn user_for_bearer(&self, headers: &HeaderMap) -> Option<String> {
        let token = bearer_from_headers(headers)?;
        self.edge_auth.user_for(&token).map(str::to_string)
    }
}

/// Extract the bearer token from an `Authorization: Bearer <t>` header.
fn bearer_from_headers(headers: &HeaderMap) -> Option<EdgeToken> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let mut parts = v.trim().splitn(2, char::is_whitespace);
            let scheme = parts.next()?;
            let token = parts.next()?.trim();
            if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
                Some(EdgeToken::new(token))
            } else {
                None
            }
        })
}

// ---------------------------------------------------------------------------
// Proxy handler
// ---------------------------------------------------------------------------

async fn rpc_proxy(
    State(fleet): State<Arc<RwLock<Fleet>>>,
    AxumPath(user_id): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (http, rpc_url, core_bearer) = {
        let fleet = fleet.read().await;

        // Edge auth: the bearer must map to a user, and that user must match the
        // path segment — so tenant A's token cannot reach tenant B's core.
        let authorized = fleet.user_for_bearer(&headers);
        match authorized {
            Some(u) if u == user_id => {}
            Some(_) => {
                log::warn!(
                    "[fleet] reject: edge token authorized a different user than /{user_id}"
                );
                return (StatusCode::FORBIDDEN, "token/user mismatch").into_response();
            }
            None => {
                return (StatusCode::UNAUTHORIZED, "missing or unknown edge token").into_response();
            }
        }

        let Some(instance) = fleet.instances.get(&user_id) else {
            return (StatusCode::NOT_FOUND, "no such tenant").into_response();
        };
        (
            fleet.http.clone(),
            instance.rpc_url(),
            instance.core_bearer.clone(),
        )
    };

    // Forward verbatim to the tenant core, swapping the edge token for the
    // tenant's core bearer. The JSON-RPC body is passed through untouched.
    let upstream = http
        .post(rpc_url)
        .header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", core_bearer.as_str()),
        )
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;

    match upstream {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok());
            match resp.bytes().await {
                Ok(bytes) => {
                    let mut response = (status, bytes).into_response();
                    if let Some(content_type) = content_type {
                        response
                            .headers_mut()
                            .insert(axum::http::header::CONTENT_TYPE, content_type);
                    }
                    response
                }
                Err(e) => {
                    log::error!("[fleet] upstream body read failed for /{user_id}: {e}");
                    (StatusCode::BAD_GATEWAY, "upstream body error").into_response()
                }
            }
        }
        Err(e) => {
            log::error!("[fleet] upstream request to tenant {user_id} failed: {e}");
            (StatusCode::BAD_GATEWAY, "tenant core unreachable").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Core process lifecycle
// ---------------------------------------------------------------------------

/// Spawn one `openhuman-core run --headless-api` child bound to `instance.port`,
/// scoped to the tenant's workspace and core bearer. Returns the child handle.
async fn spawn_core(
    core_bin: &Path,
    instance: &CoreInstance,
) -> anyhow::Result<tokio::process::Child> {
    std::fs::create_dir_all(&instance.workspace_dir).with_context(|| {
        format!(
            "creating workspace dir {} for tenant {}",
            instance.workspace_dir.display(),
            instance.user_id
        )
    })?;
    std::fs::create_dir_all(&instance.action_dir).with_context(|| {
        format!(
            "creating action dir {} for tenant {}",
            instance.action_dir.display(),
            instance.user_id
        )
    })?;

    log::info!(
        "[fleet] spawning core for tenant={} port={} workspace={} action_dir={}",
        instance.user_id,
        instance.port,
        instance.workspace_dir.display(),
        instance.action_dir.display()
    );

    let child = tokio::process::Command::new(core_bin)
        .arg("run")
        .arg("--headless-api")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(instance.port.to_string())
        .env("OPENHUMAN_WORKSPACE", &instance.workspace_dir)
        .env("OPENHUMAN_ACTION_DIR", &instance.action_dir)
        .env("OPENHUMAN_CORE_TOKEN", instance.core_bearer.as_str())
        // Each tenant is a headless single-core; keep channel listeners off so a
        // fleet host doesn't poll every member's messaging integrations.
        .env("OPENHUMAN_DISABLE_CHANNEL_LISTENERS", "1")
        .kill_on_drop(true)
        .spawn()
        .with_context(|| {
            format!(
                "spawning {} for tenant {}",
                core_bin.display(),
                instance.user_id
            )
        })?;

    Ok(child)
}

async fn ensure_loopback_port_available(port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("tenant port {port} is not available on 127.0.0.1"))?;
    drop(listener);
    Ok(())
}

/// Poll a tenant core through authenticated JSON-RPC until it responds or the
/// attempt budget is exhausted. This intentionally avoids unauthenticated
/// `/health`: a stale OpenHuman process on the assigned port could look healthy
/// but reject this tenant's core bearer, so authenticated readiness fails closed.
async fn wait_authenticated_ready(
    http: &reqwest::Client,
    instance: &CoreInstance,
    attempts: u32,
) -> bool {
    let url = instance.rpc_url();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "fleet-ready",
        "method": "openhuman.security_policy_info",
        "params": {}
    });
    for attempt in 1..=attempts {
        match http
            .post(&url)
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", instance.core_bearer.as_str()),
            )
            .json(&body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(value) if readiness_body_succeeded(&value) => return true,
                    Ok(value) => {
                        log::debug!(
                            "[fleet] readiness probe tenant={} port={} returned JSON-RPC failure has_result={} has_error={}",
                            instance.user_id,
                            instance.port,
                            value.get("result").is_some(),
                            value.get("error").is_some()
                        );
                    }
                    Err(e) => {
                        log::debug!(
                            "[fleet] readiness probe tenant={} port={} returned invalid JSON-RPC body: {e}",
                            instance.user_id,
                            instance.port
                        );
                    }
                }
            }
            Ok(resp) => log::debug!(
                "[fleet] readiness probe tenant={} port={} returned status={}",
                instance.user_id,
                instance.port,
                resp.status()
            ),
            Err(e) => log::trace!(
                "[fleet] readiness probe tenant={} port={} failed attempt={attempt}: {e}",
                instance.user_id,
                instance.port
            ),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250 * attempt as u64)).await;
    }
    false
}

fn readiness_body_succeeded(value: &serde_json::Value) -> bool {
    value.get("error").is_none() && value.get("result").is_some()
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "openhuman-fleet",
    about = "Process-per-user OpenHuman core supervisor + reverse proxy"
)]
struct Args {
    /// Address the reverse proxy listens on.
    #[arg(long, default_value = "127.0.0.1:8899")]
    listen: String,
    /// Root directory under which each tenant's workspace is created.
    #[arg(long, default_value = "./fleet-workspaces")]
    workspaces_root: PathBuf,
    /// Path to the `openhuman-core` binary to spawn per tenant.
    #[arg(long, default_value = "openhuman-core")]
    core_bin: PathBuf,
    /// First tenant core port; tenant N listens on `base_core_port + N`.
    #[arg(long, default_value_t = 7900)]
    base_core_port: u16,
    /// Comma-separated user ids to provision at boot.
    #[arg(long, value_delimiter = ',')]
    users: Vec<String>,
    /// Restricted file that receives minted edge tokens.
    #[arg(long)]
    edge_token_output: PathBuf,
}

type ProvisionedFleet = (
    HashMap<String, CoreInstance>,
    EdgeAuth,
    Vec<(String, EdgeToken)>,
);

/// Provision the in-memory tenant table + edge tokens for `users`. Pure w.r.t.
/// the filesystem/network so it is unit-testable; spawning happens separately.
fn provision(
    users: &[String],
    workspaces_root: &Path,
    base_core_port: u16,
) -> anyhow::Result<ProvisionedFleet> {
    let mut instances = HashMap::new();
    let mut edge_auth = EdgeAuth::new();
    let mut minted = Vec::new();

    for (index, user_id) in users.iter().enumerate() {
        if !is_valid_user_id(user_id) {
            anyhow::bail!("invalid user id {user_id:?}: must be a single [A-Za-z0-9_-] segment");
        }
        if instances.contains_key(user_id) {
            anyhow::bail!("duplicate user id {user_id:?}");
        }
        let port = port_for_index(base_core_port, index)
            .with_context(|| format!("port overflow assigning tenant #{index}"))?;
        let core_bearer = CoreBearer::new(format!("core-{}", uuid::Uuid::new_v4()));
        let edge_token = EdgeToken::new(format!("edge-{}", uuid::Uuid::new_v4()));
        edge_auth.insert(edge_token.clone(), user_id.clone());
        minted.push((user_id.clone(), edge_token));
        instances.insert(
            user_id.clone(),
            CoreInstance {
                user_id: user_id.clone(),
                port,
                core_bearer,
                workspace_dir: workspace_for(workspaces_root, user_id),
                action_dir: action_dir_for(workspaces_root, user_id),
            },
        );
    }

    Ok((instances, edge_auth, minted))
}

fn remove_tenant(
    instances: &mut HashMap<String, CoreInstance>,
    edge_auth: &mut EdgeAuth,
    minted: &mut Vec<(String, EdgeToken)>,
    user_id: &str,
) {
    instances.remove(user_id);
    edge_auth.remove_user(user_id);
    minted.retain(|(minted_user, _)| minted_user != user_id);
}

fn write_edge_tokens(path: &Path, minted: &[(String, EdgeToken)]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut output = String::new();
        for (user_id, token) in minted {
            output.push_str(user_id);
            output.push(' ');
            output.push_str(token.as_str());
            output.push('\n');
        }
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| format!("opening edge token output {}", path.display()))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("restricting edge token output {}", path.display()))?;
        use std::io::Write as _;
        file.write_all(output.as_bytes())
            .with_context(|| format!("writing edge token output {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let _ = minted;
        anyhow::bail!(
            "edge token output {} requires restrictive file permissions; unsupported on this platform",
            path.display()
        );
    }

    Ok(())
}

const FLEET_ALLOWED_ORIGINS_ENV: &str = "OPENHUMAN_CORE_ALLOWED_ORIGINS";

fn is_fleet_origin_allowed(origin: &str) -> bool {
    is_fleet_origin_allowed_with_extra(
        origin,
        std::env::var(FLEET_ALLOWED_ORIGINS_ENV).ok().as_deref(),
    )
}

fn is_fleet_origin_allowed_with_extra(origin: &str, extra_origins: Option<&str>) -> bool {
    if matches!(
        origin,
        "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
    ) {
        return true;
    }

    if let Some(rest) = origin.strip_prefix("http://") {
        let authority = rest.split('/').next().unwrap_or("");
        let host = if let Some(stripped) = authority.strip_prefix('[') {
            stripped.split(']').next().unwrap_or("")
        } else {
            authority.split(':').next().unwrap_or("")
        };
        if matches!(host, "127.0.0.1" | "localhost" | "::1") {
            return true;
        }
    }

    if let Some(extra) = extra_origins {
        for candidate in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if candidate == origin {
                return true;
            }
        }
    }

    false
}

async fn fleet_cors_middleware(req: Request, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    if req.method() == Method::OPTIONS {
        return with_fleet_cors_headers(StatusCode::NO_CONTENT.into_response(), origin.as_deref());
    }

    let response = next.run(req).await;
    with_fleet_cors_headers(response, origin.as_deref())
}

fn with_fleet_cors_headers(mut response: Response, origin: Option<&str>) -> Response {
    let headers = response.headers_mut();
    headers.append(header::VARY, HeaderValue::from_static("Origin"));

    if let Some(origin) = origin {
        if is_fleet_origin_allowed(origin) {
            if let Ok(value) = HeaderValue::from_str(origin) {
                headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
            }
        } else {
            log::warn!("[fleet][cors] rejected disallowed origin: {origin}");
        }
    }

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Authorization"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
    response
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = env_logger::builder().is_test(false).try_init();
    let args = Args::parse();

    if args.users.is_empty() {
        anyhow::bail!("no tenants: pass --users a,b,c");
    }

    let (mut instances, mut edge_auth, mut minted) =
        provision(&args.users, &args.workspaces_root, args.base_core_port)?;

    let proxy_http = reqwest::Client::builder()
        .build()
        .context("building fleet proxy HTTP client")?;
    let readiness_http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("building readiness HTTP client")?;

    // Spawn each tenant core. Children are monitored for the lifetime of the
    // process; aborting a monitor drops the child with kill_on_drop(true).
    let mut children = Vec::new();
    for instance in instances.values().cloned().collect::<Vec<_>>() {
        let spawn_result = match ensure_loopback_port_available(instance.port).await {
            Ok(()) => spawn_core(&args.core_bin, &instance).await,
            Err(e) => Err(e),
        };
        match spawn_result {
            Ok(mut child) => {
                let healthy = wait_authenticated_ready(&readiness_http, &instance, 20).await;
                if healthy {
                    log::info!("[fleet] tenant {} core ready", instance.user_id);
                    children.push((instance.user_id.clone(), child));
                } else {
                    log::error!("[fleet] tenant {} health probe timed out", instance.user_id);
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    remove_tenant(
                        &mut instances,
                        &mut edge_auth,
                        &mut minted,
                        &instance.user_id,
                    );
                }
            }
            Err(e) => {
                log::error!("[fleet] failed to spawn tenant {}: {e:#}", instance.user_id);
                remove_tenant(
                    &mut instances,
                    &mut edge_auth,
                    &mut minted,
                    &instance.user_id,
                );
            }
        }
    }

    if instances.is_empty() {
        anyhow::bail!("no tenant cores started successfully");
    }

    write_edge_tokens(&args.edge_token_output, &minted)?;
    log::info!(
        "[fleet] wrote {} edge token(s) to {}",
        minted.len(),
        args.edge_token_output.display()
    );

    let fleet = Arc::new(RwLock::new(Fleet {
        instances,
        edge_auth,
        http: proxy_http,
    }));

    let mut child_tasks = Vec::new();
    for (user_id, mut child) in children {
        let fleet_for_child = Arc::clone(&fleet);
        child_tasks.push(tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => {
                    log::warn!("[fleet] tenant {user_id} core exited with status {status}");
                }
                Err(e) => {
                    log::warn!("[fleet] tenant {user_id} core wait failed: {e}");
                }
            }
            fleet_for_child.write().await.instances.remove(&user_id);
        }));
    }

    let app = Router::new()
        .route(
            "/{user_id}/rpc",
            post(rpc_proxy).route_layer(DefaultBodyLimit::max(MAX_RPC_BODY_BYTES)),
        )
        .layer(middleware::from_fn(fleet_cors_middleware))
        .with_state(fleet);

    let addr: SocketAddr = args
        .listen
        .parse()
        .with_context(|| format!("invalid --listen address {:?}", args.listen))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding proxy on {addr}"))?;
    log::info!("[fleet] reverse proxy listening on http://{addr} — POST /{{user_id}}/rpc");

    axum::serve(listener, app).await.context("serving proxy")?;

    for task in child_tasks {
        task.abort();
        let _ = task.await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — pure logic (no child processes / network).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_assignment_is_sequential_from_base() {
        assert_eq!(port_for_index(7900, 0), Some(7900));
        assert_eq!(port_for_index(7900, 5), Some(7905));
    }

    #[test]
    fn port_assignment_detects_overflow() {
        assert_eq!(port_for_index(u16::MAX, 1), None);
    }

    #[test]
    fn workspace_is_user_scoped_under_root() {
        let ws = workspace_for(Path::new("/srv/fleet"), "alice");
        assert_eq!(ws, PathBuf::from("/srv/fleet/alice"));
    }

    #[test]
    fn action_dir_is_user_scoped_outside_workspace_tree() {
        let root = Path::new("/srv/fleet");
        let workspace = workspace_for(root, "alice");
        let action_dir = action_dir_for(root, "alice");

        assert_eq!(
            action_dir,
            PathBuf::from("/srv/fleet/.tenant-action-dirs/alice")
        );
        assert!(!action_dir.starts_with(&workspace));
    }

    #[test]
    fn user_id_validation_rejects_path_escapes() {
        assert!(is_valid_user_id("alice"));
        assert!(is_valid_user_id("user_42-x"));
        assert!(!is_valid_user_id(""));
        assert!(!is_valid_user_id("../etc"));
        assert!(!is_valid_user_id("a/b"));
        assert!(!is_valid_user_id("a.b"));
    }

    #[test]
    fn provision_assigns_distinct_ports_and_edge_tokens() {
        let root = PathBuf::from("/tmp/ws");
        let users = vec!["alice".to_string(), "bob".to_string()];
        let (instances, edge_auth, minted) = provision(&users, &root, 7900).unwrap();

        assert_eq!(instances.len(), 2);
        assert_eq!(instances["alice"].port, 7900);
        assert_eq!(instances["bob"].port, 7901);
        assert_eq!(instances["alice"].workspace_dir, root.join("alice"));
        assert_eq!(
            instances["alice"].action_dir,
            root.join(".tenant-action-dirs").join("alice")
        );
        assert_ne!(instances["alice"].action_dir, instances["bob"].action_dir);
        assert_ne!(instances["alice"].core_bearer, instances["bob"].core_bearer);
        assert_eq!(instances["alice"].rpc_url(), "http://127.0.0.1:7900/rpc");

        // Every minted edge token resolves back to exactly its user.
        assert_eq!(edge_auth.len(), 2);
        for (user_id, token) in &minted {
            assert_eq!(edge_auth.user_for(token), Some(user_id.as_str()));
        }
    }

    #[test]
    fn provision_rejects_duplicate_and_invalid_users() {
        let root = PathBuf::from("/tmp/ws");
        assert!(provision(&["a".into(), "a".into()], &root, 7900).is_err());
        assert!(provision(&["../x".into()], &root, 7900).is_err());
    }

    #[test]
    fn bearer_parsing_requires_bearer_prefix() {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer edge-123".parse().unwrap(),
        );
        assert_eq!(bearer_from_headers(&h), Some(EdgeToken::new("edge-123")));

        let mut lower = HeaderMap::new();
        lower.insert(
            axum::http::header::AUTHORIZATION,
            "bearer edge-456".parse().unwrap(),
        );
        assert_eq!(
            bearer_from_headers(&lower),
            Some(EdgeToken::new("edge-456"))
        );

        let mut h2 = HeaderMap::new();
        h2.insert(
            axum::http::header::AUTHORIZATION,
            "edge-123".parse().unwrap(),
        );
        assert_eq!(bearer_from_headers(&h2), None);
    }

    #[test]
    fn readiness_body_requires_jsonrpc_result() {
        assert!(readiness_body_succeeded(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "fleet-ready",
            "result": {"tier": "supervised"}
        })));

        assert!(!readiness_body_succeeded(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "fleet-ready",
            "error": {"code": -32000, "message": "config unavailable"}
        })));

        assert!(!readiness_body_succeeded(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "fleet-ready"
        })));
    }

    #[cfg(unix)]
    #[test]
    fn edge_token_output_is_written_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge-tokens.txt");
        write_edge_tokens(
            &path,
            &[("alice".to_string(), EdgeToken::new("edge-secret"))],
        )
        .unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "alice edge-secret\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn edge_token_output_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("attacker-readable.txt");
        std::fs::write(&target, "unchanged").unwrap();
        let output = dir.path().join("edge-tokens.txt");
        symlink(&target, &output).unwrap();

        assert!(write_edge_tokens(
            &output,
            &[("alice".to_string(), EdgeToken::new("edge-secret"))],
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn edge_token_output_rejects_preowned_files() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("edge-tokens.txt");
        std::fs::write(&output, "attacker keeps this inode").unwrap();

        assert!(write_edge_tokens(
            &output,
            &[("alice".to_string(), EdgeToken::new("edge-secret"))],
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            "attacker keeps this inode"
        );
    }

    #[test]
    fn fleet_cors_allows_tauri_loopback_and_extra_origins() {
        assert!(is_fleet_origin_allowed_with_extra(
            "tauri://localhost",
            None
        ));
        assert!(is_fleet_origin_allowed_with_extra(
            "http://127.0.0.1:1420",
            None
        ));
        assert!(is_fleet_origin_allowed_with_extra(
            "https://fleet.example",
            Some("https://fleet.example")
        ));
        assert!(!is_fleet_origin_allowed_with_extra(
            "https://evil.example",
            Some("https://fleet.example")
        ));
    }

    #[test]
    fn fleet_cors_headers_echo_allowed_origin_only() {
        let allowed = with_fleet_cors_headers(
            StatusCode::NO_CONTENT.into_response(),
            Some("tauri://localhost"),
        );
        assert_eq!(
            allowed
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("tauri://localhost")
        );
        assert_eq!(
            allowed
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
                .and_then(|value| value.to_str().ok()),
            Some("Content-Type, Authorization")
        );

        let rejected = with_fleet_cors_headers(
            StatusCode::NO_CONTENT.into_response(),
            Some("https://evil.example"),
        );
        assert!(rejected
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }
}
