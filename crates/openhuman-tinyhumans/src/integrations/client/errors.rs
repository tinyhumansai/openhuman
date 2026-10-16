//! Backend error classification for [`IntegrationClient`]: extracting a
//! readable detail from an error body, mapping transport errors to `anyhow`, and
//! handling the session-JWT 401 → session-expiry recovery path (and the
//! API-key 401, which must never trigger it).

use std::error::Error as _;

use crate::BackendTransportError;

use crate::integrations::types::BackendResponse;

use super::construct::IntegrationClient;

/// Maximum length (in bytes) of backend error body included in propagated
/// errors. Keep this bounded — error messages flow through tracing/Sentry and
/// are surfaced in user-facing toasts, neither of which want a 100KB blob.
pub(crate) const MAX_ERROR_BODY_LEN: usize = 500;

/// Extract a human-readable failure detail from a backend error response body.
///
/// The backend wraps every error response in
/// `{ "success": false, "error": "<msg>" }` (see
/// `backend-openhuman/src/middlewares/errorHandler.ts`). When the body parses
/// as that envelope, return the inner `error` string verbatim — it is the
/// authoritative failure message (e.g. `"Insufficient balance"`,
/// `"Toolkit \"X\" is not enabled"`).
///
/// Otherwise (non-JSON body, missing `error` field) fall back to the raw
/// text truncated to `max_bytes` at a UTF-8 char boundary so callers always
/// get *something* to grep for, without unbounded memory in error paths.
pub(crate) fn extract_error_detail(body: &str, max_bytes: usize) -> String {
    if body.is_empty() {
        return "<empty body>".to_string();
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = v.get("error").and_then(|e| e.as_str()) {
            let trimmed = msg.trim();
            if !trimmed.is_empty() {
                return openhuman_embed::__host::util::truncate_at_byte_boundary(
                    trimmed, max_bytes,
                );
            }
        }
    }
    openhuman_embed::__host::util::truncate_at_byte_boundary(body, max_bytes)
}

/// Handle a `401 Unauthorized` from the OpenHuman backend's
/// `/agent-integrations/*` routes.
///
/// **Why this 401 is unambiguously a session-JWT rejection.** Only reached
/// when the client authenticates with the *app-session JWT* (an API-key client
/// takes [`handle_api_key_unauthorized`] instead); that JWT rides
/// `Authorization: Bearer`, resolved by
/// `session_support::resolve_backend_credential`, the same resolver billing /
/// team / webhooks / memory all use. The backend's auth middleware
/// (`backend-openhuman`) is what answers `401 {"error":"Invalid token"}` when
/// that JWT is expired / revoked / rotated server-side — see the identical
/// envelope pinned in `inference/provider/config_rejection.rs` and the socket
/// reconnect loop's `"Invalid token"` handling in
/// `openhuman::platform::socket::ws_loop`. A *third-party* integration's auth failure
/// never reaches this arm:
///
/// - **Composio backend mode** (the default that routes through this client):
///   provider-side failures come back as `2xx` envelope `success:false`
///   (`"Toolkit X is not enabled"`, `"Missing required fields: …"`) or a
///   descriptive non-401 4xx/5xx — handled by
///   [`openhuman_embed::__host::core::observability::is_provider_user_state_message`] /
///   [`openhuman_embed::__host::core::observability::is_backend_user_error_message`], NOT a bare
///   401. The backend's auth wall is the only thing that returns 401 here.
/// - **Composio direct mode**: bypasses this client entirely (it talks to
///   `backend.composio.dev` with `x-api-key` via `ComposioTool`), so a
///   direct-mode key 401 carries the distinct `"[composio-direct] … HTTP 401:
///   Invalid API key"` shape and never lands here.
///
/// So narrowing to `status == 401` (and *only* 401 — 403 stays generic, it can
/// be an authz/scope rejection on a backend-mediated resource rather than a
/// dead session) targets exactly the session-JWT rejection with zero risk of
/// logging the user out for an unrelated integration problem.
///
/// What this does, mirroring `api/rest.rs::flatten_authed_error` (typed-401 →
/// `SESSION_EXPIRED:` sentinel) and
/// `inference/provider/ops/http_error.rs::publish_backend_session_expired`
/// (direct publish for paths whose error is consumed inline / swallowed):
///
/// 1. Build a `SESSION_EXPIRED:`-prefixed message so it (a) classifies as
///    [`openhuman_embed::__host::core::observability::ExpectedErrorKind::SessionExpired`] and
///    stays demoted from Sentry, and (b) is recognised by
///    `core::session_expiry::is_session_expired_error` *if* it ever propagates up to
///    the RPC boundary.
/// 2. **Publish `DomainEvent::SessionExpired` directly.** The autonomous agent
///    tool path converts tool errors into a `role:tool` result string fed back
///    to the model — the `Err` never
///    reaches `jsonrpc::invoke_method`, so relying on propagation alone would
///    leave re-login un-triggered (this is the root-cause gap behind
///    TAURI-RUST-84E: the prior fix demoted the noise but never drove
///    recovery). Publishing here makes the credentials subscriber clear the
///    session and the UI prompt re-sign-in regardless of which call site
///    surfaced the 401.
fn handle_session_jwt_unauthorized(method: &str, path: &str, url: &str, detail: &str) -> String {
    let message = format!(
        "SESSION_EXPIRED: backend rejected session token on {method} {path} \
         (401 for {url}: {detail}) — sign in again to resume"
    );

    let soft = is_composio_soft_auth_path(method, path);

    tracing::warn!(
        path = %path,
        method = %method,
        soft_auth = soft,
        "[integrations] backend rejected session JWT (401)"
    );

    // Demote from Sentry (SESSION_EXPIRED classifies as expected) — keeps the
    // noise suppression the prior fix established. Applies to both paths.
    openhuman_embed::__host::core::observability::report_error_or_expected(
        message.as_str(),
        "integrations",
        "session_expired",
        &[
            ("path", path),
            ("status", "401"),
            ("failure", "session_jwt"),
        ],
    );

    // Soft path: surface the sentinel to the caller (→ in-place CTA) WITHOUT
    // the global sign-out. See `is_composio_soft_auth_path`.
    if soft {
        tracing::debug!(
            path = %path,
            "[integrations] soft composio auth path — returning SESSION_EXPIRED to the panel without publishing global SessionExpired (#4281)"
        );
        return message;
    }

    // Drive recovery: publish SessionExpired so the credentials subscriber
    // clears the stale token and the UI prompts re-sign-in. The reason string
    // is already free of secrets (it names the path + sanitized backend
    // `error` detail), but re-scrub for defense-in-depth before it reaches the
    // subscriber's logs.
    openhuman_embed::__host::core::bus::BUS.publish(
        openhuman_embed::__host::core::events::DomainEvent::SessionExpired {
            source: format!("integrations.{method}:{path}"),
            reason: tinyinference_core::sanitize::sanitize_api_error(&message),
        },
    );

    message
}

/// Handle a `401` on a request authenticated with a TinyHumans API key.
///
/// There is no session to expire: the key is revoked, mistyped or blocked by
/// its IP allowlist. So this never publishes `DomainEvent::SessionExpired`
/// (that would sign a desktop user out of a session the request never used)
/// and uses the same `API_KEY_REJECTED:` sentinel as
/// `api::rest::flatten_authed_error`. A key that lacks a scope answers `403`,
/// which stays on the generic non-2xx path.
fn handle_api_key_unauthorized(method: &str, path: &str, url: &str, detail: &str) -> String {
    let message = format!(
        "API_KEY_REJECTED: backend rejected api key on {method} {path} \
         (401 for {url}: {detail}) — check the configured TinyHumans API key"
    );
    tracing::warn!(
        path = %path,
        method = %method,
        "[integrations] backend rejected api key (401)"
    );
    openhuman_embed::__host::core::observability::report_error_or_expected(
        message.as_str(),
        "integrations",
        "api_key_rejected",
        &[("path", path), ("status", "401"), ("failure", "api_key")],
    );
    message
}

/// Composio **trigger-catalog reads** (`GET /agent-integrations/composio/triggers…`)
/// where a 401 is a single recoverable read failure rather than whole-session
/// death. The connection itself is still active — `list_connections` uses the
/// *same* session JWT and succeeds, so signing the user out on a triggers-only
/// 401 over-reacts and (per #2286) must not happen.
///
/// For these reads [`handle_session_jwt_unauthorized`] still builds the
/// `SESSION_EXPIRED:` sentinel (so the trigger panel can classify the error and
/// render an in-place "Sign in again" CTA) and still demotes from Sentry, but
/// it does **not** publish [`DomainEvent::SessionExpired`] — that global
/// teardown would unmount the panel before the CTA is usable (#4281). A
/// genuinely dead session is still caught by the authoritative paths
/// (app-state snapshot, connections poll), which keep driving re-login.
///
/// Scoped to `GET` deliberately: trigger **writes** (`POST` enable / disable /
/// create) keep the standard global-sign-out on a 401 — they are not the
/// "catalog won't load" surface this issue addresses, and a write that 401s on
/// a dead session has no in-place CTA to fall back to (its error renders as a
/// per-row toggle failure, not the panel banner).
pub(super) fn is_composio_soft_auth_path(method: &str, path: &str) -> bool {
    // Match on a real path boundary, not a bare prefix: `…/triggers` exact,
    // `…/triggers/…` (the `available` catalog), or `…/triggers?…` (the active
    // list with a `toolkit` query). A bare `starts_with` would also match an
    // unrelated `…/triggersXYZ` route and wrongly suppress the global sign-out.
    const BASE: &str = "/agent-integrations/composio/triggers";
    method.eq_ignore_ascii_case("GET")
        && path
            .strip_prefix(BASE)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || rest.starts_with('?'))
}

impl IntegrationClient {
    /// Render a reqwest transport error with its full source chain (mirrors
    /// the inline closures in [`Self::post`] / [`Self::get`]) and route it
    /// through the observability classifier so network-environment failures
    /// skip Sentry.
    pub(super) fn report_transport_error(
        e: reqwest::Error,
        method: &str,
        path: &str,
        url: &str,
    ) -> anyhow::Error {
        let mut chain = format!("{e}");
        let mut src: Option<&(dyn std::error::Error + 'static)> = e.source();
        while let Some(s) = src {
            chain.push_str(" → ");
            chain.push_str(&s.to_string());
            src = s.source();
        }
        openhuman_embed::__host::core::observability::report_error_or_expected(
            chain.as_str(),
            "integrations",
            method,
            &[("path", path), ("failure", "transport")],
        );
        anyhow::anyhow!("{} {} failed: {}", method.to_uppercase(), url, chain)
    }

    pub(super) fn map_transport_error(
        &self,
        error: BackendTransportError,
        method: &str,
        path: &str,
        url: &str,
    ) -> anyhow::Error {
        let method_upper = method.to_uppercase();
        match error {
            // A core with no backend transport installed (built without
            // `openhuman-tinyhumans`): expected build state, not a defect.
            // `report_error_or_expected` keys off the `BACKEND_UNAVAILABLE:`
            // prefix to keep it out of Sentry.
            BackendTransportError::Unavailable => {
                let msg = format!(
                    "{}{method_upper} {url}: managed backend is not available in this build",
                    openhuman_embed::__host::core::observability::BACKEND_UNAVAILABLE_PREFIX
                );
                openhuman_embed::__host::core::observability::report_error_or_expected(
                    msg.as_str(),
                    "integrations",
                    method,
                    &[("path", path), ("failure", "backend_unavailable")],
                );
                anyhow::anyhow!(msg)
            }
            BackendTransportError::Http(error) => {
                Self::report_transport_error(error, method, path, url)
            }
            BackendTransportError::Status { status, body } => {
                let body_text = match body {
                    serde_json::Value::String(text) => text,
                    value => value.to_string(),
                };
                let detail = extract_error_detail(&body_text, MAX_ERROR_BODY_LEN);
                if status == reqwest::StatusCode::UNAUTHORIZED.as_u16()
                    && self.credential.is_api_key()
                {
                    return anyhow::anyhow!(handle_api_key_unauthorized(
                        &method_upper,
                        path,
                        url,
                        &detail
                    ));
                }
                if status == reqwest::StatusCode::UNAUTHORIZED.as_u16() {
                    return anyhow::anyhow!(handle_session_jwt_unauthorized(
                        &method_upper,
                        path,
                        url,
                        &detail
                    ));
                }
                let status_text = reqwest::StatusCode::from_u16(status)
                    .map(|status| status.to_string())
                    .unwrap_or_else(|_| status.to_string());
                let status_code = status.to_string();
                openhuman_embed::__host::core::observability::report_error_or_expected(
                    format!("Backend returned {status_text} for {method_upper} {url}: {detail}")
                        .as_str(),
                    "integrations",
                    method,
                    &[
                        ("path", path),
                        ("status", status_code.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
                anyhow::anyhow!("Backend returned {status_text} for {method_upper} {url}: {detail}")
            }
            other => anyhow::anyhow!("{method_upper} {url} failed: {other}"),
        }
    }

    pub(super) fn parse_envelope<T: serde::de::DeserializeOwned>(
        method: &str,
        path: &str,
        url: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<T> {
        let method_upper = method.to_uppercase();
        let envelope: BackendResponse<T> = serde_json::from_value(value)?;
        if !envelope.success {
            let msg = envelope
                .error
                .unwrap_or_else(|| "unknown backend error".into());
            openhuman_embed::__host::core::observability::report_error_or_expected(
                msg.as_str(),
                "integrations",
                method,
                &[("path", path), ("failure", "envelope_error")],
            );
            anyhow::bail!("Backend error for {} {}: {}", method_upper, url, msg);
        }
        envelope.data.ok_or_else(|| {
            anyhow::anyhow!(
                "Backend returned success but no data for {} {}",
                method_upper,
                url
            )
        })
    }
}
