//! Core `ComposioTool` struct and the loopback/HTTPS URL-safety helpers
//! shared by every direct-mode Composio request path.

use crate::security::SecurityPolicy;
use std::sync::Arc;

pub(super) const COMPOSIO_API_BASE_V2: &str = "https://backend.composio.dev/api/v2";
pub(super) const COMPOSIO_API_BASE_V3: &str = "https://backend.composio.dev/api/v3";

pub(super) fn ensure_https(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("https://") {
        anyhow::bail!(
            "Refusing to transmit sensitive data over non-HTTPS URL: URL scheme must be https"
        );
    }
    Ok(())
}

pub(super) fn is_loopback_http_url(url: &str) -> bool {
    // Parse rather than prefix-match: a raw `starts_with("http://127.0.0.1:")`
    // is fooled by userinfo smuggling like
    // `http://127.0.0.1:8080@evil.com/api/v3/tools`, which reqwest routes to the
    // *parsed* host (`evil.com`). Verify the actual scheme + host and reject any
    // embedded credentials so the insecure-loopback path can never leak the
    // `x-api-key` header to a non-loopback host.
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

pub(super) fn is_loopback_http_base(url: &str) -> bool {
    is_loopback_http_url(&format!("{}/", url.trim_end_matches('/')))
}

/// A tool that proxies actions to the Composio managed tool platform.
pub struct ComposioTool {
    pub(super) api_key: String,
    pub(super) default_entity_id: String,
    pub(super) security: Arc<SecurityPolicy>,
    pub(super) base_v2: String,
    /// Base URL for Composio v3 endpoints (`{base}/tools`). Production
    /// always uses [`COMPOSIO_API_BASE_V3`] via [`ComposioTool::new`]; the
    /// `#[cfg(test)]` `new_with_v3_base` constructor lets unit tests point
    /// the direct-mode `/tools` listing at a local axum mock — the same
    /// base-URL injection the backend `ComposioClient` gets through
    /// `IntegrationClient::new` in `client_tests.rs`.
    pub(super) base_v3: String,
    pub(super) allow_insecure_loopback: bool,
}
