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

/// Shared client for all integration tools. Holds backend URL, auth token,
/// a reusable `reqwest::Client`, and a lazily-fetched pricing cache.
pub struct IntegrationClient {
    pub backend_url: String,
    pub auth_token: String,
    http_client: reqwest::Client,
    pricing: tokio::sync::OnceCell<IntegrationPricing>,
}

impl IntegrationClient {
    pub fn new(backend_url: String, auth_token: String) -> Self {
        // Match the TLS config used by `BackendOAuthClient` in
        // `src/api/rest.rs`: force rustls + HTTP/1.1 so we get the same
        // consistent cross-platform behaviour every other backend-proxied
        // domain (billing, team, webhooks, referral, …) already relies
        // on. The default builder picks up native-tls on macOS, which
        // has historically failed on staging TLS handshakes while
        // rustls succeeds — so the integrations client was the odd one
        // out with raw "error sending request" failures.
        let http_client = reqwest::Client::builder()
            .use_rustls_tls()
            .http1_only()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build integration HTTP client");

        Self {
            backend_url,
            auth_token,
            http_client,
            pricing: tokio::sync::OnceCell::new(),
        }
    }

    /// POST JSON to a backend endpoint and parse the response `data` field.
    pub async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<T> {
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
            Some(Arc::new(IntegrationClient::new(backend_url, token)))
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
