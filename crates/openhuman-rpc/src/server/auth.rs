//! Bearer-token authentication for the core's HTTP API.
//!
//! The token itself is owned by `openhuman::core::auth`; this module is
//! the route policy on top of it.
//!
//! Endpoints exempt from auth (checked by [`rpc_auth_middleware`]):
//! - `GET /`              — public info page
//! - `GET /health`        — liveness probe
//! - `GET /schema`        — read-only schema discovery
//! - `GET /dev/connect`   — dev-only core handoff; see `server::dev_connect`
//! - `GET /events`        — SSE stream; browser `EventSource` cannot set
//!   headers, so the handler enforces a bind-token / bearer credential itself
//! - `OPTIONS *`          — CORS preflight (handled by outer CORS middleware)
//!
//! Endpoints that accept the bearer either via header **or** `?token=…` query
//! param (see [`QUERY_TOKEN_PATHS`]):
//! - `GET /events/webhooks` — webhook SSE; browser `EventSource` cannot set
//!   headers, so the FE forwards the bearer as a query param. Validated
//!   against the same in-process RPC token — no separate secret.
//!
//! Executable surfaces:
//! - `POST /rpc` requires the per-launch core bearer token.
//! - `GET /v1/models` and `POST /v1/chat/completions` accept either that
//!   internal bearer or a stable user-managed external API key stored under
//!   `openhuman::inference::http::EXTERNAL_OPENAI_COMPAT_PROVIDER`.

use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Request gate shared with the two-phase desktop boot. While active, only the
/// preflight HTTP/RPC surface is admitted; the same bound server opens its
/// normal router after config secrets are decrypted.
#[derive(Clone, Debug)]
pub(crate) struct PreflightGate(Arc<AtomicBool>);

impl PreflightGate {
    pub(crate) fn new_active() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub(crate) fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn open(&self) {
        self.0.store(false, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn close_for_test(&self) {
        self.0.store(true, Ordering::Release);
    }
}

const PREFLIGHT_RPC_METHODS: &[&str] = &[
    "core.ping",
    "core.version",
    "openhuman.service_status",
    "openhuman.update_version",
    "openhuman.approval_list_pending",
    "openhuman.approval_decide",
];

pub(crate) fn preflight_rpc_method_allowed(method: &str) -> bool {
    PREFLIGHT_RPC_METHODS.contains(&method)
}

pub(crate) fn preflight_http_path_allowed(method: &Method, path: &str) -> bool {
    *method == Method::OPTIONS
        || (*method == Method::GET && path == "/health")
        || (*method == Method::POST && path == "/rpc")
}

/// While the desktop has only loaded sealed config, keep every route except
/// health and JSON-RPC behind a closed gate. JSON-RPC has its own method
/// allowlist in `preflight_rpc_handler`.
pub(crate) async fn preflight_surface_middleware(
    gate: axum::extract::State<PreflightGate>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    if !gate.is_active() || preflight_http_path_allowed(req.method(), req.uri().path()) {
        return next.run(req).await;
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "ok": false,
            "error": "startup_preflight",
            "message": "Core startup approval preflight is active"
        })),
    )
        .into_response()
}

/// Stop Engine.IO polling before its own layer can allocate a session. CORS is
/// kept outside this middleware by the router builder.
pub(crate) async fn preflight_socketio_middleware(
    gate: axum::extract::State<PreflightGate>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if gate.is_active() && (path == "/socket.io" || path.starts_with("/socket.io/")) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ok": false,
                "error": "startup_preflight",
                "message": "Core startup approval preflight is active"
            })),
        )
            .into_response();
    }
    next.run(req).await
}

use crate::core_host::config::Config;
use crate::core_host::credentials::AuthService;
use crate::core_host::inference::http::EXTERNAL_OPENAI_COMPAT_PROVIDER;

/// Paths that bypass bearer-token authentication.
///
/// `/rpc` and `/v1/*` carry executable surfaces and must be protected. The
/// other routes are read-only, or are streaming / WebSocket upgrades whose
/// clients (browser `EventSource`, browser `WebSocket`) cannot set
/// `Authorization` headers via standard APIs. `/events` is not unauthenticated
/// — it is exempt from the *middleware* header check but enforces its own
/// bind-token credential inside the handler.
const PUBLIC_PATHS: &[&str] = &[
    "/",
    "/health",
    // External browser OAuth redirect for HTTP-remote MCP servers — the
    // authorization server posts back here with `?code=…&state=…` and no
    // bearer; the one-time `state` (minted in `oauth_begin`) is the guard.
    "/oauth/mcp/callback",
    // Dev-only handoff to a loopback Vite renderer. Guards live in the handler
    // (`server::dev_connect`): debug build or explicit opt-in, loopback `app`
    // origin and `Host`, no cross-site navigation.
    "/dev/connect",
    "/schema",
    "/events",
];

/// Public path prefixes — match when the request path begins with any entry.
///
/// Use this only when the suffix is dynamic (path params). For exact paths,
/// add to [`PUBLIC_PATHS`] instead.
///
/// Intentionally empty: the only entry was AgentBox's `/jobs/{job_id}`, which
/// left with that domain. The mechanism is kept for the next dynamic-suffix
/// public route rather than re-derived when one appears.
const PUBLIC_PATH_PREFIXES: &[&str] = &[];

/// Returns `true` when `path` bypasses bearer-token authentication.
///
/// A path is public when it appears in [`PUBLIC_PATHS`] (exact match) or
/// begins with any entry in [`PUBLIC_PATH_PREFIXES`] (prefix match).
fn is_public_path(path: &str) -> bool {
    PUBLIC_PATHS.contains(&path)
        || PUBLIC_PATH_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

/// Paths that may authenticate via `?token=…` in the URL when no
/// `Authorization` header is present.
///
/// Browser `EventSource` cannot attach custom headers, so an SSE route that
/// returns sensitive data (webhook deliveries, registration changes) is
/// otherwise indistinguishable from a public endpoint — any local process on
/// `127.0.0.1` can subscribe. Allowing the bearer in the query string lets
/// the FE attach it explicitly while keeping a single token of truth
/// (validated by [`bearer_matches`] against the same in-process RPC token).
///
/// Add new entries here only for SSE / WebSocket routes whose clients cannot
/// send headers and that carry per-user data. The follow-up approvals stream
/// (#1339) is the next planned addition.
const QUERY_TOKEN_PATHS: &[&str] = &["/events/webhooks", "/ws/live-voice"];

/// Axum middleware: enforce `Authorization: Bearer <token>` on all protected
/// endpoints.
///
/// Public paths (see [`PUBLIC_PATHS`]) and CORS preflight `OPTIONS` requests
/// bypass this check. `/rpc` requires the exact per-launch bearer token that
/// was written to `core.token` at startup; `/v1/*` additionally accepts a
/// stable user-managed external API key.
pub async fn rpc_auth_middleware(req: axum::extract::Request, next: Next) -> Response {
    let path = req.uri().path().to_string();

    // CORS preflight and public utility paths bypass auth.
    if req.method() == Method::OPTIONS || is_public_path(&path) {
        return next.run(req).await;
    }

    let Some(expected) = crate::core_host::core::auth::get_rpc_token() else {
        // Shouldn't happen in production — token is always initialized before
        // the router starts serving. Deny to be safe.
        log::error!("[auth] RPC token not initialized — denying request to {path}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "error": "server_error",
                "message": "Auth subsystem not initialized"
            })),
        )
            .into_response();
    };

    let header_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if crate::core_host::core::auth::bearer_matches(header_token, expected) {
        log::trace!("[auth] authorized request to {path} (header)");
        return next.run(req).await;
    }

    if is_external_inference_path(&path) && verify_external_inference_bearer(header_token).await {
        log::trace!("[auth] authorized request to {path} (external inference bearer)");
        return next.run(req).await;
    }

    // Header path failed — fall back to `?token=…` for SSE/WS routes whose
    // browser clients cannot set headers. The query token is validated
    // against the same in-process RPC bearer (single source of truth), so
    // this is not a separate credential — only a transport workaround.
    if QUERY_TOKEN_PATHS.contains(&path.as_str()) {
        if let Some(query_token) = extract_query_token(req.uri().query()) {
            if crate::core_host::core::auth::bearer_matches(&query_token, expected) {
                log::trace!("[auth] authorized request to {path} (query token)");
                return next.run(req).await;
            }
        }
    }

    log::warn!("[auth] unauthorized request to {path} — missing or wrong bearer token");
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "ok": false,
            "error": "unauthorized",
            "message": "Missing or invalid Authorization header. Supply 'Authorization: Bearer <token>'."
        })),
    )
        .into_response()
}

fn is_external_inference_path(path: &str) -> bool {
    path == "/v1" || path.starts_with("/v1/")
}

async fn verify_external_inference_bearer_for_config(config: &Config, supplied: &str) -> bool {
    if supplied.trim().is_empty() {
        return false;
    }

    let auth = AuthService::from_config(config);
    match auth
        .get_provider_bearer_token(EXTERNAL_OPENAI_COMPAT_PROVIDER, None)
        .await
    {
        Ok(expected) => external_inference_bearer_matches(supplied, expected.as_deref()),
        Err(err) => {
            log::warn!("[auth] failed to read external inference bearer: {err}");
            false
        }
    }
}

fn external_inference_bearer_matches(supplied: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected
        .map(str::trim)
        .filter(|expected| !expected.is_empty())
    else {
        return false;
    };
    crate::core_host::core::auth::bearer_matches(supplied, expected)
}

async fn verify_external_inference_bearer(supplied: &str) -> bool {
    if supplied.trim().is_empty() {
        return false;
    }

    let config = match Config::load_or_init().await {
        Ok(config) => config,
        Err(err) => {
            log::warn!("[auth] failed to load config for external inference bearer: {err}");
            return false;
        }
    };

    verify_external_inference_bearer_for_config(&config, supplied).await
}

/// Pull the first `token` query parameter out of a URL query string.
///
/// Returns `None` when the query is absent, the key is missing, or the
/// value is empty after trimming. URL decoding is delegated to
/// [`url::form_urlencoded`] so percent-encoded tokens decode the same way
/// they were encoded by the FE via `encodeURIComponent`.
fn extract_query_token(query: Option<&str>) -> Option<String> {
    let query = query?;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "token" {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
