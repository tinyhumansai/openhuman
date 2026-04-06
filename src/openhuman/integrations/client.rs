//! Shared HTTP client for all integration tools.

use super::types::{BackendResponse, IntegrationPricing};
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
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
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
            .await?;

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
            .await?;

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

/// Helper: build an `Arc<IntegrationClient>` from config, or `None` if
/// integrations are disabled or misconfigured.
pub fn build_client(
    config: &crate::openhuman::config::IntegrationsConfig,
) -> Option<Arc<IntegrationClient>> {
    if !config.enabled {
        return None;
    }
    match (
        config.backend_url.as_deref().map(str::trim),
        config.auth_token.as_deref().map(str::trim),
    ) {
        (Some(url), Some(token)) if !url.is_empty() && !token.is_empty() => Some(Arc::new(
            IntegrationClient::new(url.to_owned(), token.to_owned()),
        )),
        _ => {
            tracing::warn!(
                "[integrations] enabled but backend_url or auth_token missing — skipping"
            );
            None
        }
    }
}
