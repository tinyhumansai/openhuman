//! The SaaS gateway layer: which context each request runs under.
//!
//! Replaces the single-context layer when the process runs in SaaS mode. For
//! every request it:
//!
//! 1. answers `404` for routes a SaaS core never serves — the OpenAI-compatible
//!    `/v1`, the `/events/*` debug streams, the WebSockets, `/dev/connect` and
//!    the MCP OAuth callback — and for the `/events` chat stream outside a
//!    user's scope;
//! 2. with no `X-OpenHuman-User`, runs it on the operator plane (the bearer
//!    check downstream still applies);
//! 3. with one, checks the service bearer **first** — so an unauthenticated
//!    caller learns nothing about which users exist and cannot open profiles —
//!    then the signature, then runs the request under that user's profile.
//!
//! A profile another node hosts is refused with `409` and
//! `{"error":"profile_held","owner","endpoint","retry_after_ms"}`, plus the
//! `X-OpenHuman-Profile-Owner` header naming the holder: the gateway routes
//! the user there, or retries after `retry_after_ms`.
//!
//! # The gateway protocol
//!
//! The gateway authenticates users and calls the core with the service bearer
//! plus, for work on a user's behalf, the user's id:
//!
//! ```text
//! Authorization: Bearer <service token>
//! X-OpenHuman-User: <gateway user id>
//! X-OpenHuman-User-Sig: t=<unix secs>,v1=<hex hmac-sha256(service token, "<t>.<user id>")>
//! ```
//!
//! With a user header the request runs under that user's profile — which
//! must already be provisioned — and nothing else. The signature, required
//! unless the operator turns it off, binds the user id to the bearer holder
//! and a ±60 s window: a gateway that forwards a client's headers by mistake
//! cannot be talked into acting as another user. Mapping the user to a
//! profile and opening it is the core's
//! (`ProfileHost::resolve_user`); the headers, the signature and the
//! refusal statuses are this module's.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core_host::core::runtime::{CoreContext, DispatchCtx};
use crate::core_host::profiles::gateway::ResolveUserError;
use crate::core_host::profiles::{OpenError, Profile, ProfileHost};
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The gateway's user header.
pub const USER_HEADER: &str = "x-openhuman-user";
/// The gateway's signature over the user header.
pub const USER_SIG_HEADER: &str = "x-openhuman-user-sig";
/// How far a signature's timestamp may be from the core's clock.
pub const SIGNATURE_WINDOW_SECS: u64 = 60;

type HmacSha256 = Hmac<Sha256>;

fn mac(secret: &str, user_id: &str, ts: u64) -> HmacSha256 {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(format!("{ts}.{user_id}").as_bytes());
    mac
}

/// The `X-OpenHuman-User-Sig` value for `user_id` at `ts` (unix seconds).
pub fn sign(secret: &str, user_id: &str, ts: u64) -> String {
    let tag = mac(secret, user_id, ts).finalize().into_bytes();
    format!("t={ts},v1={}", hex::encode(tag))
}

/// Check a signature header for `user_id` against `now` (unix seconds).
pub fn verify(secret: &str, user_id: &str, header: &str, now: u64) -> Result<(), String> {
    let mut ts = None;
    let mut tag = None;
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", value)) => ts = value.parse::<u64>().ok(),
            Some(("v1", value)) => tag = hex::decode(value).ok(),
            _ => {}
        }
    }
    let (Some(ts), Some(tag)) = (ts, tag) else {
        return Err("malformed user signature".to_string());
    };
    if ts.abs_diff(now) > SIGNATURE_WINDOW_SECS {
        return Err("user signature is outside the allowed clock window".to_string());
    }
    mac(secret, user_id, ts)
        .verify_slice(&tag)
        .map_err(|_| "user signature does not match".to_string())
}

/// The `error` code of a refusal for a profile another node hosts.
pub const PROFILE_HELD: &str = "profile_held";

/// The response header naming the node that hosts a refused profile.
pub const PROFILE_OWNER_HEADER: &str = "x-openhuman-profile-owner";

/// The node that hosts a profile this one was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldBy {
    /// The holder's node id.
    pub owner: String,
    /// Where the holder can be reached, when it advertises an endpoint.
    pub endpoint: Option<String>,
    /// A polling hint in milliseconds: the time until its lease lapses unless
    /// renewed, capped at 60 seconds (a lease that never expires, such as a
    /// file lock, reports the cap). Retry after this, not necessarily after
    /// expiry.
    pub retry_after_ms: u64,
}

/// Why a gateway request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRefusal {
    /// The HTTP status to answer with.
    pub status: u16,
    pub message: String,
    /// Set on a `409`: who hosts the profile instead.
    pub held_by: Option<HeldBy>,
}

impl GatewayRefusal {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            held_by: None,
        }
    }

    /// The refusal for a profile's [`OpenError`] at `now_ms`.
    pub fn from_open_error(error: OpenError, now_ms: u64) -> Self {
        match error {
            OpenError::NotProvisioned(_) => Self::new(403, error.to_string()),
            OpenError::Full { .. } | OpenError::Storage(_) => Self::new(503, error.to_string()),
            OpenError::HeldElsewhere(record) => {
                log::debug!(
                    "[profiles][gateway] the profile is held by node={} epoch={}",
                    record.owner,
                    record.epoch
                );
                // A file-lock lease never expires (`u64::MAX`), so the raw
                // value is no retry hint (and overflows a JSON consumer's
                // safe integers); cap it.
                Self {
                    status: 409,
                    message: PROFILE_HELD.to_string(),
                    held_by: Some(HeldBy {
                        retry_after_ms: record.retry_after_ms(now_ms).min(MAX_RETRY_AFTER_MS),
                        owner: record.owner,
                        endpoint: record.endpoint,
                    }),
                }
            }
        }
    }

    /// The refusal for the core's [`ResolveUserError`] at `now_ms`.
    pub fn from_resolve_error(error: ResolveUserError, now_ms: u64) -> Self {
        match error {
            ResolveUserError::InvalidUser(why) => Self::new(400, why),
            ResolveUserError::Signature(why) => Self::new(401, why),
            ResolveUserError::Open(error) => Self::from_open_error(error, now_ms),
        }
    }
}

/// The longest `retry_after_ms` a `409` reports.
const MAX_RETRY_AFTER_MS: u64 = 60_000;

/// Milliseconds since the epoch, the clock the profile leases run on.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// Where a gateway request runs.
#[derive(Debug)]
pub enum GatewayScope {
    /// No user header: the operator plane.
    Operator,
    /// One user's profile.
    User(Arc<Profile>),
}

/// Pick the scope for a request that already presented the service bearer.
///
/// `secret` is the service bearer; `now` is unix seconds.
pub async fn resolve_scope(
    user_id: Option<&str>,
    signature: Option<&str>,
    secret: &str,
    now: u64,
) -> Result<GatewayScope, GatewayRefusal> {
    let Some(user_id) = user_id else {
        return Ok(GatewayScope::Operator);
    };
    let host = crate::core_host::profiles::host::host()
        .ok_or_else(|| GatewayRefusal::new(503, "this core serves no users"))?;
    resolve_user_on(&host, user_id, signature, secret, now)
        .await
        .map(GatewayScope::User)
}

/// [`resolve_scope`] for a request that names `user_id`, on an explicit
/// `host` rather than the process's: the profile that serves that user, or
/// the refusal.
pub async fn resolve_user_on(
    host: &ProfileHost,
    user_id: &str,
    signature: Option<&str>,
    secret: &str,
    now: u64,
) -> Result<Arc<Profile>, GatewayRefusal> {
    host.resolve_user(user_id, || {
        let signature = signature.ok_or_else(|| format!("missing {USER_SIG_HEADER}"))?;
        verify(secret, user_id, signature, now).inspect_err(|e| {
            log::warn!("[profiles][gateway] refused a scoped request: {e}");
        })
    })
    .await
    .map_err(|error| GatewayRefusal::from_resolve_error(error, now_ms()))
}

/// Route prefixes a SaaS core never serves.
pub(crate) const CLOSED_IN_SAAS: &[&str] = &[
    "/v1",
    "/events/",
    "/ws/",
    "/socket.io",
    "/dev/connect",
    "/oauth/",
];

/// A prefix ending in `/` closes only what lies beneath it; any other prefix
/// closes the path itself and everything beneath it.
pub(crate) fn is_closed_in_saas(path: &str) -> bool {
    CLOSED_IN_SAAS.iter().any(|prefix| {
        if prefix.ends_with('/') {
            path.starts_with(prefix)
        } else {
            path == *prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        }
    })
}

fn refuse(status: u16, message: &str) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::FORBIDDEN);
    (status, axum::Json(serde_json::json!({ "error": message }))).into_response()
}

/// The response for a refusal from the profile resolver: a plain error, or
/// for a profile another node hosts the `409` that names it.
pub(crate) fn refusal_response(refusal: GatewayRefusal) -> Response {
    let Some(held) = refusal.held_by else {
        return refuse(refusal.status, &refusal.message);
    };
    let body = serde_json::json!({
        "error": refusal.message,
        "owner": held.owner,
        "endpoint": held.endpoint,
        "retry_after_ms": held.retry_after_ms,
    });
    let mut response = (StatusCode::CONFLICT, axum::Json(body)).into_response();
    match axum::http::HeaderValue::from_str(&held.owner) {
        Ok(owner) => {
            response.headers_mut().insert(PROFILE_OWNER_HEADER, owner);
        }
        Err(_) => log::warn!("[rpc:saas] the profile owner is not a valid header value"),
    }
    response
}

/// A request the gateway checks turned away, before any profile was
/// resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Refused {
    pub(crate) status: u16,
    pub(crate) message: &'static str,
}

impl Refused {
    fn new(status: u16, message: &'static str) -> Self {
        Self { status, message }
    }
}

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        refuse(self.status, self.message)
    }
}

/// A request the gateway checks let through.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admitted {
    /// No user header: the operator plane.
    Operator,
    /// A user request that presented the service bearer; its profile is
    /// resolved next.
    User {
        user: String,
        signature: Option<String>,
    },
}

fn header_str<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers().get(name).and_then(|v| v.to_str().ok())
}

fn bearer(req: &Request) -> Option<&str> {
    header_str(req, header::AUTHORIZATION.as_str())?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Runs the request under `ctx`: attaches the [`DispatchCtx`] that
/// `rpc_handler` hands to the dispatcher, and (until every reader has moved off
/// the ambient context, #7801) scopes the same context onto the request task.
pub(crate) async fn dispatch_in(ctx: Arc<CoreContext>, mut req: Request, next: Next) -> Response {
    req.extensions_mut()
        .insert(DispatchCtx::new(Arc::clone(&ctx)));
    CoreContext::scope(ctx, next.run(req)).await
}

/// The SaaS request layer; `operator` is the runtime's own context.
pub(crate) async fn saas_gateway(operator: Arc<CoreContext>, req: Request, next: Next) -> Response {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let secret = crate::core_host::core::auth::get_rpc_token();
    let (user, signature) = match decide(&req, secret) {
        Err(refused) => return refused.into_response(),
        Ok(Admitted::Operator) => return dispatch_in(operator, req, next).await,
        Ok(Admitted::User { user, signature }) => (user, signature),
    };
    let secret = secret.expect("decide admits a user only once the core has a token");
    match resolve_scope(Some(&user), signature.as_deref(), secret, now).await {
        Err(refusal) => refusal_response(refusal),
        Ok(GatewayScope::User(profile)) => {
            let ctx = Arc::clone(profile.context());
            // Holding the state for the request keeps the profile from being
            // evicted under it.
            let response = dispatch_in(ctx, req, next).await;
            drop(profile);
            response
        }
        Ok(GatewayScope::Operator) => dispatch_in(operator, req, next).await,
    }
}

/// Whether a request gets past the gateway checks, or the response that
/// refuses it. A user request that passes still has its signature and
/// profile resolved ([`resolve_scope`]).
///
/// `secret` is the service bearer (`None` before the core has one). Taking it
/// as an argument keeps the decision free of process-wide state.
pub(crate) fn decide(req: &Request, secret: Option<&str>) -> Result<Admitted, Refused> {
    let path = req.uri().path();
    if is_closed_in_saas(path) {
        log::debug!("[rpc:saas] {path} is not served in SaaS mode");
        return Err(Refused::new(404, "not found"));
    }

    // Absent means the operator plane. Present but unreadable, or present
    // twice, is refused: falling back to the operator would skip the user
    // signature check.
    let mut user_headers = req.headers().get_all(USER_HEADER).iter();
    let Some(first) = user_headers.next() else {
        // The chat event stream is a user's; the operator has none.
        if path == "/events" {
            return Err(Refused::new(404, "not found"));
        }
        return Ok(Admitted::Operator);
    };
    if user_headers.next().is_some() {
        log::debug!("[rpc:saas] refusing a request with more than one {USER_HEADER}");
        return Err(Refused::new(400, "more than one user header"));
    }
    let Ok(user) = first.to_str() else {
        log::debug!("[rpc:saas] refusing an unreadable {USER_HEADER}");
        return Err(Refused::new(400, "unreadable user header"));
    };

    let Some(secret) = secret else {
        return Err(Refused::new(503, "the core is not ready"));
    };
    if !bearer(req)
        .is_some_and(|supplied| crate::core_host::core::auth::bearer_matches(supplied, secret))
    {
        return Err(Refused::new(401, "unauthorized"));
    }
    // Checked after the bearer, so an unauthenticated caller learns nothing.
    // A second signature must not be ignored in favour of the first.
    let mut sig_headers = req.headers().get_all(USER_SIG_HEADER).iter();
    let signature = sig_headers.next();
    if sig_headers.next().is_some() {
        log::debug!("[rpc:saas] refusing a request with more than one {USER_SIG_HEADER}");
        return Err(Refused::new(400, "more than one user signature header"));
    }
    let signature = signature
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    Ok(Admitted::User {
        user: user.to_owned(),
        signature,
    })
}

#[cfg(test)]
#[path = "saas_gateway_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "saas_gateway_proptest_tests.rs"]
mod proptest_tests;

#[cfg(test)]
#[path = "saas_gateway_signature_proptest_tests.rs"]
mod signature_proptest_tests;
