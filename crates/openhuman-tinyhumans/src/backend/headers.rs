//! Attribution headers and `reqwest` client profiles for TinyHumans backend
//! traffic.
//!
//! Every request that reaches the hosted backend carries the same three
//! attribution headers regardless of which transport sends it: `x-core-version`
//! (the workspace version the core ships as), `x-tauri-version` (the desktop
//! shell, when it hosts the core) and `x-sdk-name` (the product identity from
//! [`crate::backend::product`]). This is TinyHumans header policy, so it lives
//! with the TinyHumans transport; the core only asks the installed
//! [`BackendTransport`](openhuman_embed::BackendTransport) for them.

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::time::Duration;

use crate::transport::TransportProfile;

/// Upper bound on the `x-core-version` / `x-tauri-version` header values. A
/// version string is short; anything longer is a misconfigured environment and
/// is clamped rather than propagated into every backend request.
pub(crate) const CLIENT_VERSION_HEADER_MAX_LEN: usize = 64;

/// Environment variable the Tauri shell sets to its own package version before
/// spawning the in-process core, so backend analytics can attribute
/// core-originated requests to the desktop build hosting them.
pub const TAURI_VERSION_ENV_VAR: &str = "OPENHUMAN_TAURI_VERSION";

/// Restrict a version string to header-safe characters and clamp its length.
/// Returns `None` when nothing safe remains.
pub(crate) fn sanitize_client_version(raw: &str) -> Option<String> {
    let sanitized: String = raw
        .trim()
        .chars()
        .filter(|c| matches!(c, '0'..='9' | 'A'..='Z' | 'a'..='z' | '.' | '_' | '+' | '-'))
        .take(CLIENT_VERSION_HEADER_MAX_LEN)
        .collect();

    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

/// The attribution headers every backend-bound request carries:
/// `x-core-version`, `x-tauri-version` (only when the shell exported it) and
/// `x-sdk-name`.
///
/// Set at the transport level rather than per request because some callers
/// drive a raw `reqwest::Client` themselves (multipart STT upload via
/// [`BackendClient::raw_client`](openhuman_embed::__host::backend::BackendClient::raw_client))
/// and that traffic needs attributing too.
pub fn attribution_headers() -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if let Some(version) = sanitize_client_version(env!("CARGO_PKG_VERSION")) {
        headers.insert(
            HeaderName::from_static("x-core-version"),
            HeaderValue::from_str(&version).context("invalid x-core-version header value")?,
        );
    }
    if let Ok(raw) = std::env::var(TAURI_VERSION_ENV_VAR) {
        if let Some(version) = sanitize_client_version(&raw) {
            headers.insert(
                HeaderName::from_static("x-tauri-version"),
                HeaderValue::from_str(&version).context("invalid x-tauri-version header value")?,
            );
        }
    }
    let (name, value) = crate::backend::product::product_identity_header();
    headers.insert(name, value);
    Ok(headers)
}

/// A `reqwest::ClientBuilder` carrying the platform TLS backend, HTTP/1-only
/// negotiation, the timeouts historically used for `profile`, and the
/// attribution headers as defaults.
///
/// Platform-appropriate TLS: Windows → schannel (honors the OS cert store,
/// required for corporate TLS-inspection proxies); macOS / Linux → rustls.
/// See [`openhuman_embed::__host::util::tls::tls_client_builder`].
pub fn backend_client_builder(profile: TransportProfile) -> Result<reqwest::ClientBuilder> {
    let timeout = match profile {
        // `BackendClient` historically allowed 120 s: it fronts slow
        // control-plane calls (billing summaries, integration token handoffs).
        TransportProfile::Api => Duration::from_secs(120),
        // `/agent-integrations/*` tool calls were always capped at 60 s.
        TransportProfile::Integrations => Duration::from_secs(60),
    };
    Ok(openhuman_embed::__host::util::tls::tls_client_builder()
        .default_headers(attribution_headers()?)
        .http1_only()
        // The SDK adds `x-api-key` per request. Reqwest does not strip this
        // custom header when following a cross-origin redirect.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15)))
}

/// Build the `reqwest::Client` for `profile` with
/// [`backend_client_builder`]'s settings.
pub fn build_backend_client(profile: TransportProfile) -> Result<reqwest::Client> {
    backend_client_builder(profile)?
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
}

#[cfg(test)]
#[path = "headers_tests.rs"]
mod tests;
