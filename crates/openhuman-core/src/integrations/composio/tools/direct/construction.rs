//! Constructors and the small per-request setup helpers (`client`,
//! `ensure_request_url`) that every other `ComposioTool` impl block relies on.

use super::types::{
    ensure_https, is_loopback_http_url, ComposioTool, COMPOSIO_API_BASE_V2, COMPOSIO_API_BASE_V3,
};
use crate::security::SecurityPolicy;
use reqwest::Client;
use std::sync::Arc;

use super::types::is_loopback_http_base;

pub(super) fn normalize_entity_id(entity_id: &str) -> String {
    let trimmed = entity_id.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

impl ComposioTool {
    pub fn new(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        // Production always pins the real HTTPS endpoints.
        Self::new_internal(
            api_key,
            default_entity_id,
            security,
            COMPOSIO_API_BASE_V2.to_string(),
            COMPOSIO_API_BASE_V3.to_string(),
            false,
        )
    }

    pub(crate) fn auth_key_fingerprint(&self) -> u64 {
        crate::integrations::composio::direct_auth::fingerprint_api_key(&self.api_key)
    }

    /// Debug-test seam for raw integration coverage: construct a direct
    /// Composio tool against explicit v2/v3 base URLs. Non-HTTPS URLs are
    /// accepted only for loopback hosts and only in debug builds.
    #[cfg(debug_assertions)]
    pub fn new_with_base_urls_for_loopback(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
        base_v2: String,
        base_v3: String,
    ) -> anyhow::Result<Self> {
        for base in [&base_v2, &base_v3] {
            if !base.starts_with("https://") && !is_loopback_http_base(base) {
                anyhow::bail!("debug Composio base URL must be HTTPS or loopback HTTP");
            }
        }
        Ok(Self::new_internal(
            api_key,
            default_entity_id,
            security,
            base_v2,
            base_v3,
            true,
        ))
    }

    /// Construct against explicit v2/v3 API roots. Both must be HTTPS;
    /// loopback HTTP is accepted only in debug builds.
    pub fn new_with_base_urls(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
        base_v2: String,
        base_v3: String,
    ) -> anyhow::Result<Self> {
        let allow_loopback = cfg!(debug_assertions);
        for base in [&base_v2, &base_v3] {
            let accepted =
                base.starts_with("https://") || (allow_loopback && is_loopback_http_base(base));
            if !accepted {
                anyhow::bail!("Composio base URL must be HTTPS");
            }
        }
        Ok(Self::new_internal(
            api_key,
            default_entity_id,
            security,
            base_v2,
            base_v3,
            allow_loopback,
        ))
    }

    /// Test-only seam: construct with an explicit Composio v3 base URL so
    /// unit tests can point the direct `/tools` request — including the
    /// `tags` filter — at a local mock instead of `backend.composio.dev`.
    ///
    /// `#[cfg(test)]`-gated on purpose: `list_tool_schemas_v3` attaches the
    /// `x-api-key` header to whatever `base_v3` holds, so the only way to
    /// reach the v3 endpoint in production is [`Self::new`], which always
    /// uses the HTTPS [`COMPOSIO_API_BASE_V3`] const. An injectable base must
    /// never carry a non-HTTPS URL outside tests.
    #[cfg(test)]
    pub(crate) fn new_with_v3_base(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
        base_v3: String,
    ) -> Self {
        Self::new_internal(
            api_key,
            default_entity_id,
            security,
            COMPOSIO_API_BASE_V2.to_string(),
            base_v3,
            true,
        )
    }

    /// Shared constructor body. Private so the injectable `base_v3` cannot be
    /// supplied by production callers — they go through [`Self::new`] (real
    /// HTTPS const) and tests through the `#[cfg(test)]` `new_with_v3_base`.
    fn new_internal(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
        base_v2: String,
        base_v3: String,
        allow_insecure_loopback: bool,
    ) -> Self {
        let trimmed = api_key.trim();
        if trimmed.len() != api_key.len() {
            // The key carried leading/trailing whitespace that would otherwise
            // reach Composio's `x-api-key` header verbatim and trip the
            // server-side "Invalid API key format" 401 (Sentry TAURI-RUST-D3).
            // We trim here so the request succeeds; logging the length delta
            // (never the key itself) helps trace which credential source
            // produced a dirty value without leaking the secret.
            tracing::debug!(
                original_len = api_key.len(),
                trimmed_len = trimmed.len(),
                "[composio] trimmed leading/trailing whitespace from api_key"
            );
        }
        Self {
            api_key: trimmed.to_string(),
            default_entity_id: normalize_entity_id(default_entity_id.unwrap_or("default")),
            security,
            base_v2,
            base_v3,
            allow_insecure_loopback,
        }
    }

    pub(super) fn client(&self) -> Client {
        crate::config::build_runtime_proxy_client_with_timeouts("tool.composio", 60, 10)
    }

    pub(super) fn ensure_request_url(&self, url: &str) -> anyhow::Result<()> {
        if self.allow_insecure_loopback && is_loopback_http_url(url) {
            return Ok(());
        }
        ensure_https(url)
    }
}
