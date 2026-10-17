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
//! The decision itself lives in `crate::core_host::profiles::gateway`.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core_host::core::runtime::{CoreContext, DispatchCtx};
use crate::core_host::profiles::gateway::{
    resolve_scope, GatewayRefusal, GatewayScope, PROFILE_OWNER_HEADER, USER_HEADER, USER_SIG_HEADER,
};
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

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
