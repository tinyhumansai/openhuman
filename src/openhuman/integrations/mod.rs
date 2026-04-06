//! Agent integration tools that proxy through the backend API.
//!
//! Each tool calls a backend endpoint (authenticated via JWT Bearer token) which
//! handles external API calls, billing, rate limiting, and markup. The client
//! never talks to external services directly.

pub mod google_places;
pub mod parallel;
pub mod twilio;

pub use google_places::{GooglePlacesDetailsTool, GooglePlacesSearchTool};
pub use parallel::{ParallelExtractTool, ParallelSearchTool};
pub use twilio::TwilioCallTool;

use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

// ── Tool scope ───────────────��──────────────────────────────────────

/// Controls where an integration tool is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolScope {
    /// Available in agent loop, CLI, and RPC.
    All,
    /// Only available in the autonomous agent loop.
    #[allow(dead_code)]
    AgentOnly,
    /// Only available via explicit CLI/RPC invocation (not autonomous agent).
    CliRpcOnly,
}

// ── Pricing types (fetched from backend) ────────────────────────────

/// Per-integration pricing returned by `GET /agent-integrations/pricing`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IntegrationPricing {
    #[serde(default)]
    pub integrations: PricingIntegrations,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PricingIntegrations {
    #[serde(default)]
    pub twilio: Option<IntegrationPricingEntry>,
    #[serde(default)]
    pub google_places: Option<IntegrationPricingEntry>,
    #[serde(default)]
    pub parallel: Option<IntegrationPricingEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntegrationPricingEntry {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub pricing: serde_json::Value,
}

// ── Backend response envelope ───────────────��───────────────────────

/// Standard `{ success, data, error }` envelope from the backend.
#[derive(Debug, Deserialize)]
pub struct BackendResponse<T> {
    #[allow(dead_code)]
    pub success: bool,
    pub data: T,
    #[serde(default)]
    #[allow(dead_code)]
    pub error: Option<String>,
}

// ── Shared HTTP client ─────────────────────────────────��────────────

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
            let body_text = resp.text().await.unwrap_or_default();
            tracing::debug!("[integrations] POST {} → {} {}", url, status, body_text);
            anyhow::bail!("Backend returned {}: {}", status, body_text);
        }

        let envelope: BackendResponse<T> = resp.json().await?;
        Ok(envelope.data)
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
            let body_text = resp.text().await.unwrap_or_default();
            tracing::debug!("[integrations] GET {} → {} {}", url, status, body_text);
            anyhow::bail!("Backend returned {}: {}", status, body_text);
        }

        let envelope: BackendResponse<T> = resp.json().await?;
        Ok(envelope.data)
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
    match (config.backend_url.as_deref(), config.auth_token.as_deref()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_scope_equality() {
        assert_eq!(ToolScope::All, ToolScope::All);
        assert_ne!(ToolScope::All, ToolScope::CliRpcOnly);
        assert_ne!(ToolScope::AgentOnly, ToolScope::CliRpcOnly);
    }

    #[test]
    fn backend_response_deserializes() {
        let json = r#"{"success": true, "data": {"foo": 42}}"#;
        let resp: BackendResponse<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert_eq!(resp.data["foo"], 42);
    }

    #[test]
    fn integration_pricing_defaults_on_missing_fields() {
        let json = r#"{"integrations": {}}"#;
        let pricing: IntegrationPricing = serde_json::from_str(json).unwrap();
        assert!(pricing.integrations.twilio.is_none());
        assert!(pricing.integrations.google_places.is_none());
        assert!(pricing.integrations.parallel.is_none());
    }

    #[test]
    fn build_client_returns_none_when_disabled() {
        let config = crate::openhuman::config::IntegrationsConfig::default();
        assert!(build_client(&config).is_none());
    }

    #[test]
    fn build_client_returns_none_when_url_missing() {
        let config = crate::openhuman::config::IntegrationsConfig {
            enabled: true,
            backend_url: None,
            auth_token: Some("tok".into()),
            ..Default::default()
        };
        assert!(build_client(&config).is_none());
    }
}
