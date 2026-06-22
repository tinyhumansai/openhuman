//! Shared HTTP client for all integration tools.

use super::types::{BackendResponse, IntegrationPricing};
use std::error::Error as _;
use std::sync::Arc;
use std::time::Duration;

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
                return crate::openhuman::util::truncate_at_byte_boundary(trimmed, max_bytes);
            }
        }
    }
    crate::openhuman::util::truncate_at_byte_boundary(body, max_bytes)
}

fn managed_budget_applies_to_path(path: &str) -> bool {
    path != "/agent-integrations/pricing" && path.starts_with("/agent-integrations/")
}

/// Handle a `401 Unauthorized` from the OpenHuman backend's
/// `/agent-integrations/*` routes.
///
/// **Why this 401 is unambiguously a session-JWT rejection.** Every request
/// from [`IntegrationClient`] attaches the *app-session JWT* as its
/// `Authorization: Bearer` — [`build_client`] resolves the token via
/// [`crate::api::jwt::get_session_token`], the same token billing / team /
/// webhooks / memory all use. The backend's auth middleware
/// (`backend-openhuman`) is what answers `401 {"error":"Invalid token"}` when
/// that JWT is expired / revoked / rotated server-side — see the identical
/// envelope pinned in `inference/provider/config_rejection.rs` and the socket
/// reconnect loop's `"Invalid token"` handling in
/// `openhuman::socket::ws_loop`. A *third-party* integration's auth failure
/// never reaches this arm:
///
/// - **Composio backend mode** (the default that routes through this client):
///   provider-side failures come back as `2xx` envelope `success:false`
///   (`"Toolkit X is not enabled"`, `"Missing required fields: …"`) or a
///   descriptive non-401 4xx/5xx — handled by
///   [`crate::core::observability::is_provider_user_state_message`] /
///   [`crate::core::observability::is_backend_user_error_message`], NOT a bare
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
///    [`crate::core::observability::ExpectedErrorKind::SessionExpired`] and
///    stays demoted from Sentry, and (b) is recognised by
///    `core::jsonrpc::is_session_expired_error` *if* it ever propagates up to
///    the RPC boundary.
/// 2. **Publish `DomainEvent::SessionExpired` directly.** The autonomous agent
///    loop (`agent/harness/engine/tools.rs::run_one_tool`) swallows tool errors
///    into a `role:tool` result string fed back to the model — the `Err` never
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

    tracing::warn!(
        path = %path,
        method = %method,
        "[integrations] backend rejected session JWT (401) — publishing SessionExpired to drive re-login"
    );

    // Demote from Sentry (SESSION_EXPIRED classifies as expected) — keeps the
    // noise suppression the prior fix established.
    crate::core::observability::report_error_or_expected(
        message.as_str(),
        "integrations",
        "session_expired",
        &[
            ("path", path),
            ("status", "401"),
            ("failure", "session_jwt"),
        ],
    );

    // Drive recovery: publish SessionExpired so the credentials subscriber
    // clears the stale token and the UI prompts re-sign-in. The reason string
    // is already free of secrets (it names the path + sanitized backend
    // `error` detail), but re-scrub for defense-in-depth before it reaches the
    // subscriber's logs.
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::SessionExpired {
        source: format!("integrations.{method}:{path}"),
        reason: crate::openhuman::inference::provider::ops::sanitize_api_error(&message),
    });

    message
}

/// Strip any inference-style path that snuck into a backend URL before
/// it becomes the [`IntegrationClient::backend_url`] field. Idempotent —
/// returns the input unchanged when already clean.
///
/// See issue #2075 / Sentry `OPENHUMAN-TAURI-H6`, `-HN`: a misconfigured
/// `BACKEND_URL` env (e.g. `https://api.tinyhumans.ai/openai/v1/chat/completions`)
/// baked into a build silently produced 404 URLs like
/// `…/openai/v1/chat/completions/agent-integrations/composio/connections`
/// because every `IntegrationClient` method joins paths onto this field
/// via [`crate::api::config::api_url`].
fn sanitize_backend_url(backend_url: &str) -> String {
    let cleaned = crate::api::config::normalize_backend_api_base_url(backend_url);
    let trimmed = backend_url.trim().trim_end_matches('/');
    if !cleaned.is_empty() && cleaned != trimmed {
        // Redact userinfo (username/password) before logging — a
        // misconfigured URL could carry credentials in the authority
        // segment. The helper preserves host/path for diagnosability
        // while scrubbing secrets.
        tracing::warn!(
            input = %crate::api::config::redact_url_for_log(trimmed),
            cleaned = %crate::api::config::redact_url_for_log(&cleaned),
            "[integrations] backend_url carried an inference / non-root path; \
             stripping before use (issue #2075)"
        );
    }
    if cleaned.is_empty() {
        backend_url.to_string()
    } else {
        cleaned
    }
}

/// Shared client for all integration tools. Holds backend URL, auth token,
/// a reusable `reqwest::Client`, and a lazily-fetched pricing cache.
pub struct IntegrationClient {
    pub backend_url: String,
    pub auth_token: String,
    budget_config: Option<Arc<crate::openhuman::config::Config>>,
    http_client: reqwest::Client,
    pricing: tokio::sync::OnceCell<IntegrationPricing>,
}

impl IntegrationClient {
    pub fn new(backend_url: String, auth_token: String) -> Self {
        Self::new_inner(backend_url, auth_token, None)
    }

    pub fn new_with_budget_config(
        backend_url: String,
        auth_token: String,
        config: Arc<crate::openhuman::config::Config>,
    ) -> Self {
        Self::new_inner(backend_url, auth_token, Some(config))
    }

    fn new_inner(
        backend_url: String,
        auth_token: String,
        budget_config: Option<Arc<crate::openhuman::config::Config>>,
    ) -> Self {
        // Defense-in-depth (issue #2075 / Sentry OPENHUMAN-TAURI-H6, -HN):
        // every prod call site routes `backend_url` through
        // `effective_backend_api_url` which strips inference-style paths,
        // but any future caller that forgets that step would silently
        // produce 404 URLs like
        //   https://api.tinyhumans.ai/openai/v1/chat/completions/agent-integrations/composio/connections
        // (the inference path concatenated with every domain path). We
        // re-strip here so the field invariant — "backend_url has no
        // inference path" — holds locally, and `warn!` once when we have
        // to fix up the input so the regression is observable in logs.
        let backend_url = sanitize_backend_url(&backend_url);

        // Platform-appropriate TLS backend — see [`crate::openhuman::tls`].
        // Windows uses schannel (native-tls) to honor the OS cert store;
        // macOS / Linux keep rustls which avoids the OpenSSL runtime dep and
        // has historically been more reliable on staging TLS handshakes.
        let http_client = crate::openhuman::tls::tls_client_builder()
            .http1_only()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build integration HTTP client");

        Self {
            backend_url,
            auth_token,
            budget_config,
            http_client,
            pricing: tokio::sync::OnceCell::new(),
        }
    }

    async fn ensure_budget_available(&self, path: &str) -> anyhow::Result<()> {
        if !managed_budget_applies_to_path(path) {
            return Ok(());
        }
        if let Some(config) = &self.budget_config {
            if crate::openhuman::team::managed_tool_budget_exhausted(config).await {
                anyhow::bail!(
                    "Managed cloud tools are disabled because your OpenHuman AI credits are exhausted. Add credits or route the task to user-supplied providers."
                );
            }
        }
        Ok(())
    }

    /// POST JSON to a backend endpoint and parse the response `data` field.
    pub async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<T> {
        self.ensure_budget_available(path).await?;
        let url = crate::api::config::api_url(&self.backend_url, path);
        tracing::debug!("[integrations] POST {}", url);

        let resp = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| {
                // Log the full error source chain so the caller gets
                // something useful instead of reqwest's top-level
                // "error sending request for url (…)" which hides the
                // real cause (DNS / TLS / connect / timeout).
                let mut chain = format!("{e}");
                let mut src: Option<&(dyn std::error::Error + 'static)> = e.source();
                while let Some(s) = src {
                    chain.push_str(" → ");
                    chain.push_str(&s.to_string());
                    src = s.source();
                }
                // Use `report_error_or_expected` so transport-level shapes
                // ("error sending request for url", "tls handshake eof",
                // "connection refused/reset", …) are classified as
                // `NetworkUnreachable` and skip Sentry — user-environment
                // problems (VPN drop, captive portal, ISP block, TLS MITM)
                // that no retry on our side can resolve (OPENHUMAN-TAURI-2G).
                crate::core::observability::report_error_or_expected(
                    chain.as_str(),
                    "integrations",
                    "post",
                    &[("path", path), ("failure", "transport")],
                );
                anyhow::anyhow!("POST {} failed: {}", url, chain)
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let detail = extract_error_detail(&body_text, MAX_ERROR_BODY_LEN);
            let status_str = status.as_u16().to_string();
            // A 401 is the backend's auth middleware rejecting our app-session
            // JWT (not a third-party provider 401 — see
            // `handle_session_jwt_unauthorized` for the full
            // session-JWT-vs-provider argument). Route it into the
            // session-expiry recovery flow: demote from Sentry AND publish
            // `SessionExpired` so re-login fires even though the autonomous
            // agent loop swallows the propagated error (TAURI-RUST-84E). This
            // must come BEFORE the generic non-2xx branch so the 401 doesn't
            // fall through to a plain `BackendUserError` that only demotes.
            if status == reqwest::StatusCode::UNAUTHORIZED {
                let message = handle_session_jwt_unauthorized("POST", path, &url, &detail);
                anyhow::bail!("{message}");
            }
            // Route through `report_error_or_expected` so 4xx user-input /
            // auth-state failures (e.g. OPENHUMAN-TAURI-BC: SharePoint
            // authorize 400 because the user didn't fill in the required
            // Tenant Name field) demote to a warn breadcrumb instead of
            // firing a Sentry event. 5xx and non-transient 4xx still
            // surface — see `is_backend_user_error_message` for the exact
            // status set classified as expected.
            crate::core::observability::report_error_or_expected(
                format!("Backend returned {status} for POST {url}: {detail}").as_str(),
                "integrations",
                "post",
                &[
                    ("path", path),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
            anyhow::bail!("Backend returned {status} for POST {url}: {detail}");
        }

        let envelope: BackendResponse<T> = resp.json().await?;
        if !envelope.success {
            let msg = envelope
                .error
                .unwrap_or_else(|| "unknown backend error".into());
            // Route through `report_error_or_expected` so user-state envelope
            // failures the backend wraps as 2xx + `success: false` (composio
            // "Toolkit X is not enabled", "Trigger type … not found",
            // "Missing required fields: …" — OPENHUMAN-TAURI-3R / -3S / -34 /
            // -97) demote to an info breadcrumb instead of firing a Sentry
            // event. Genuine backend bugs (unknown envelope shapes, internal
            // panics) still surface.
            crate::core::observability::report_error_or_expected(
                msg.as_str(),
                "integrations",
                "post",
                &[("path", path), ("failure", "envelope_error")],
            );
            anyhow::bail!("Backend error for POST {}: {}", url, msg);
        }
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("Backend returned success but no data for POST {}", url))
    }

    /// GET from a backend endpoint and parse the response `data` field.
    pub async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        self.ensure_budget_available(path).await?;
        let url = crate::api::config::api_url(&self.backend_url, path);
        tracing::debug!("[integrations] GET {}", url);

        let resp = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await
            .map_err(|e| {
                let mut chain = format!("{e}");
                let mut src: Option<&(dyn std::error::Error + 'static)> = e.source();
                while let Some(s) = src {
                    chain.push_str(" → ");
                    chain.push_str(&s.to_string());
                    src = s.source();
                }
                // Mirrors the post() transport site — classify reqwest
                // transport-level failures as NetworkUnreachable so they
                // skip Sentry. OPENHUMAN-TAURI-2G: TLS handshake EOF
                // against api.tinyhumans.ai from a SG user.
                crate::core::observability::report_error_or_expected(
                    chain.as_str(),
                    "integrations",
                    "get",
                    &[("path", path), ("failure", "transport")],
                );
                anyhow::anyhow!("GET {} failed: {}", url, chain)
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let detail = extract_error_detail(&body_text, MAX_ERROR_BODY_LEN);
            let status_str = status.as_u16().to_string();
            // Mirrors the post() session-JWT 401 arm — a 401 here is the
            // backend rejecting our session JWT, so drive re-login (publish
            // SessionExpired) and demote from Sentry, before the generic
            // non-2xx branch. See `handle_session_jwt_unauthorized`.
            if status == reqwest::StatusCode::UNAUTHORIZED {
                let message = handle_session_jwt_unauthorized("GET", path, &url, &detail);
                anyhow::bail!("{message}");
            }
            // Mirrors the post() site — see OPENHUMAN-TAURI-BC. 4xx
            // user-input / auth-state shapes demote to a warn breadcrumb
            // via the observability classifier; 5xx and non-transient 4xx
            // still surface.
            crate::core::observability::report_error_or_expected(
                format!("Backend returned {status} for GET {url}: {detail}").as_str(),
                "integrations",
                "get",
                &[
                    ("path", path),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
            anyhow::bail!("Backend returned {status} for GET {url}: {detail}");
        }

        let envelope: BackendResponse<T> = resp.json().await?;
        if !envelope.success {
            let msg = envelope
                .error
                .unwrap_or_else(|| "unknown backend error".into());
            // Mirrors the post() envelope-error site — see the comment there
            // for OPENHUMAN-TAURI-3R/-3S/-34/-97 rationale. User-state
            // envelope failures demote; genuine backend bugs still surface.
            crate::core::observability::report_error_or_expected(
                msg.as_str(),
                "integrations",
                "get",
                &[("path", path), ("failure", "envelope_error")],
            );
            anyhow::bail!("Backend error for GET {}: {}", url, msg);
        }
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("Backend returned success but no data for GET {}", url))
    }

    /// Fetch and cache pricing info from the backend. Returns a default
    /// (empty) pricing struct on network errors so tool registration never fails.
    pub async fn pricing(&self) -> &IntegrationPricing {
        self.pricing
            .get_or_init(|| async {
                match self
                    .get::<IntegrationPricing>("/agent-integrations/pricing")
                    .await
                {
                    Ok(p) => {
                        tracing::debug!("[integrations] pricing fetched successfully");
                        p
                    }
                    Err(e) => {
                        tracing::warn!("[integrations] failed to fetch pricing: {e}");
                        IntegrationPricing::default()
                    }
                }
            })
            .await
    }
}

/// Fetch pricing for the integrations module, honouring the
/// Composio routing mode.
///
/// When `config.composio.mode == "direct"`, the user is running with
/// their own Composio API key and there is **no backend session** that
/// could serve `/agent-integrations/pricing` — the backend route is
/// what mediates the margin between Composio's raw price and what the
/// hosted product charges. In direct mode, margins do not apply
/// (the user pays Composio directly) and the backend may not even be
/// reachable (sovereign / offline-friendly deployments). We
/// short-circuit to the default empty pricing struct and emit a
/// `[composio-direct]` log line so this branch is easy to grep.
///
/// In backend mode we fall through to the live cache on
/// [`IntegrationClient::pricing`], preserving the existing behavior
/// for every caller. The empty default struct is identical to what
/// [`IntegrationClient::pricing`] returns on a network error, so
/// downstream consumers don't need a separate code path.
pub async fn pricing_for_config(
    client: &IntegrationClient,
    config: &crate::openhuman::config::Config,
) -> IntegrationPricing {
    use crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT;

    if config.composio.mode.trim() == COMPOSIO_MODE_DIRECT {
        tracing::debug!(
            "[composio-direct] pricing short-circuit: backend `/agent-integrations/pricing` \
             is unreachable in direct mode — returning default (empty) pricing"
        );
        return IntegrationPricing::default();
    }
    client.pricing().await.clone()
}

/// Helper: build an `Arc<IntegrationClient>` from the root config, or
/// `None` if the user isn't signed in yet.
///
/// Both the backend URL and the auth token come from **core defaults**:
///
/// - backend URL → [`crate::api::config::effective_backend_api_url`]
///   applied to `config.api_url`. Unlike the plain
///   [`crate::api::config::effective_api_url`] resolver (which honours a
///   user-set local-AI endpoint so chat completions still work), the
///   backend resolver detects local-AI URLs and falls back to the
///   `BACKEND_URL` / `VITE_BACKEND_URL` env vars (and finally the hosted
///   default) so backend paths don't get concatenated onto a local
///   Ollama/vLLM endpoint and 404.
/// - auth token → [`crate::api::jwt::get_session_token`], i.e. the
///   app-session JWT written by `auth_store_session` — the same token
///   that billing, team, webhooks, referral, memory, etc. all use.
///
/// There are no per-feature toggles for the shared client itself —
/// callers that need a kill switch (e.g. twilio, google_places,
/// parallel) gate tool registration at their own level.
pub fn build_client(config: &crate::openhuman::config::Config) -> Option<Arc<IntegrationClient>> {
    // Use the integrations-specific resolver: when `config.api_url` is set
    // to a local-AI endpoint (Ollama, vLLM, …), it would still be perfect
    // for `/v1/chat/completions`, but reusing it as the base for backend
    // integration paths produces URLs like
    //   http://127.0.0.1:11434/v1/agent-integrations/composio/toolkits
    // which 404 against the local LLM and flooded Sentry
    // (OPENHUMAN-TAURI-51 / -80 / -7Z). The helper falls through to env /
    // default backend in that case so integrations actually work.
    let backend_url = crate::api::config::effective_backend_api_url(&config.api_url);

    // Primary: app-session JWT from the auth profile store.
    let session_token = match crate::api::jwt::get_session_token(config) {
        Ok(Some(tok)) => {
            let trimmed = tok.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("[integrations] failed to read session token: {e}");
            None
        }
    };

    match session_token {
        Some(token) => {
            tracing::debug!(
                backend_url = %backend_url,
                "[integrations] client built (session token resolved)"
            );
            Some(Arc::new(IntegrationClient::new_with_budget_config(
                backend_url,
                token,
                Arc::new(config.clone()),
            )))
        }
        None => {
            tracing::warn!(
                "[integrations] no auth token available — user is not signed in \
                 (no app-session JWT)"
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
