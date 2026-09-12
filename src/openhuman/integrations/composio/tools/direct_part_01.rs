// Composio Tool Provider — optional managed tool surface with 1000+ OAuth integrations.
//
// When enabled, OpenHuman can execute actions on Gmail, Notion, GitHub, Slack, etc.
// through Composio's API without storing raw OAuth tokens locally.
//
// This is opt-in. Users who prefer sovereign/local-only mode skip this entirely.
// The Composio API key is stored in the encrypted secret store.

use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use anyhow::Context;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

const COMPOSIO_API_BASE_V2: &str = "https://backend.composio.dev/api/v2";
const COMPOSIO_API_BASE_V3: &str = "https://backend.composio.dev/api/v3";

fn ensure_https(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("https://") {
        anyhow::bail!(
            "Refusing to transmit sensitive data over non-HTTPS URL: URL scheme must be https"
        );
    }
    Ok(())
}

fn is_loopback_http_url(url: &str) -> bool {
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

#[cfg(debug_assertions)]
fn is_loopback_http_base(url: &str) -> bool {
    is_loopback_http_url(&format!("{}/", url.trim_end_matches('/')))
}

/// A tool that proxies actions to the Composio managed tool platform.
pub struct ComposioTool {
    api_key: String,
    default_entity_id: String,
    security: Arc<SecurityPolicy>,
    base_v2: String,
    /// Base URL for Composio v3 endpoints (`{base}/tools`). Production
    /// always uses [`COMPOSIO_API_BASE_V3`] via [`Self::new`]; the
    /// `#[cfg(test)]` `new_with_v3_base` constructor lets unit tests point
    /// the direct-mode `/tools` listing at a local axum mock — the same
    /// base-URL injection the backend `ComposioClient` gets through
    /// `IntegrationClient::new` in `client_tests.rs`.
    base_v3: String,
    allow_insecure_loopback: bool,
}
