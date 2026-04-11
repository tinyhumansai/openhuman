//! Shared HTTP client for all integration tools.

use super::types::{BackendResponse, IntegrationPricing};
use std::error::Error as _;
use std::sync::Arc;
use std::time::Duration;

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
        let url = format!("{}{}", self.backend_url, path);
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
                tracing::warn!("[integrations] POST {} failed: {}", url, chain);
                anyhow::anyhow!("POST {} failed: {}", url, chain)
            })?;

        let status = resp.status();
        if !status.is_success() {
            let _body_text = resp.text().await.unwrap_or_default();
            tracing::debug!(
                "[integrations] POST {} → {} <redacted-response>",
                url,
                status
            );
            anyhow::bail!("Backend returned {} for POST {}", status, url);
        }

        let envelope: BackendResponse<T> = resp.json().await?;
        if !envelope.success {
            let msg = envelope
                .error
                .unwrap_or_else(|| "unknown backend error".into());
            anyhow::bail!("Backend error for POST {}: {}", url, msg);
        }
        envelope
            .data
            .ok_or_else(|| anyhow::anyhow!("Backend returned success but no data for POST {}", url))
    }

    /// GET from a backend endpoint and parse the response `data` field.
    pub async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = format!("{}{}", self.backend_url, path);
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
                tracing::warn!("[integrations] GET {} failed: {}", url, chain);
                anyhow::anyhow!("GET {} failed: {}", url, chain)
            })?;

        let status = resp.status();
        if !status.is_success() {
            let _body_text = resp.text().await.unwrap_or_default();
            tracing::debug!(
                "[integrations] GET {} → {} <redacted-response>",
                url,
                status
            );
            anyhow::bail!("Backend returned {} for GET {}", status, url);
        }

        let envelope: BackendResponse<T> = resp.json().await?;
        if !envelope.success {
            let msg = envelope
                .error
                .unwrap_or_else(|| "unknown backend error".into());
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
/// - backend URL → [`crate::api::config::effective_api_url`] applied to
///   `config.api_url` (which itself falls back to the `BACKEND_URL` /
///   `VITE_BACKEND_URL` env vars and finally the hosted default).
/// - auth token → [`crate::api::jwt::get_session_token`], i.e. the
///   app-session JWT written by `auth_store_session` — the same token
///   that billing, team, webhooks, referral, memory, etc. all use.
///   As a last-ditch fallback we also honour `config.api_key` so
///   non-interactive sidecar deployments that set `OPENHUMAN_API_KEY`
///   still work.
///
/// There are no per-feature toggles for the shared client itself —
/// callers that need a kill switch (e.g. twilio, google_places,
/// parallel) gate tool registration at their own level.
pub fn build_client(
    config: &crate::openhuman::config::Config,
) -> Option<Arc<IntegrationClient>> {
    let backend_url = crate::api::config::effective_api_url(&config.api_url);

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

    // Fallback: config.api_key for headless / CI deployments.
    let auth_token = session_token.or_else(|| {
        config
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    });

    match auth_token {
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
                 (no app-session JWT and no config.api_key fallback)"
            );
            None
        }
    }
}
