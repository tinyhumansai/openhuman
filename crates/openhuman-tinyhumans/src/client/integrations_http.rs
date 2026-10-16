//! [`IntegrationsHttp`]: the transport half of the `/agent-integrations/*`
//! client: route and credential guards, and the JSON / multipart sends over
//! [`TransportProfile::Integrations`].
//!
//! This is what the core's `IntegrationClient` does between "decide to call"
//! and "parse the envelope". The pieces that need core policy (egress
//! disclosure through `__host::egress`, the budget gate, pricing, envelope
//! parsing and 401 recovery) stay with the integrations domain that moves onto
//! this helper. Errors come back as the raw
//! [`BackendTransportError`], unclassified, so that domain can map them.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::transport::BackendRequest;
use crate::transport::{BackendTransport, BackendTransportError, TransportProfile};
use anyhow::Result;
use openhuman_embed::__host::security::credentials::session_support::BackendCredential;
use openhuman_embed::__host::util::redact_url_for_log;
use openhuman_embed::__host::util::tls::tls_client_builder;
use openhuman_embed::__host::util::url::{join_url, normalize_backend_api_base_url};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Method;
use serde_json::Value;

use super::backend_client::shared_transport;
use super::endpoint::validate_credential_endpoint;

const LOG_PREFIX: &str = "[tinyhumans-integrations-http]";
/// The header a TinyHumans API key rides on.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Strip any inference-style path that snuck into a backend URL (issue #2075,
/// Sentry `OPENHUMAN-TAURI-H6`/`-HN`): every route is joined onto the field, so
/// `.../openai/v1/chat/completions` would 404 every integration call.
/// Idempotent.
pub fn sanitize_backend_url(backend_url: &str) -> String {
    let cleaned = normalize_backend_api_base_url(backend_url);
    let trimmed = backend_url.trim().trim_end_matches('/');
    if !cleaned.is_empty() && cleaned != trimmed {
        tracing::warn!(
            input = %redact_url_for_log(trimmed),
            cleaned = %redact_url_for_log(&cleaned),
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

/// Refuse routes the SDK intentionally does not expose to this client
/// (`webhooks`, `admin`).
pub fn reject_privileged_backend_path(method: &str, path: &str) -> Result<()> {
    let route = path.split('?').next().unwrap_or(path);
    if route.split('/').any(|segment| {
        segment.eq_ignore_ascii_case("webhooks") || segment.eq_ignore_ascii_case("admin")
    }) {
        anyhow::bail!(
            "route is intentionally not exposed by the SDK: {} {}",
            method,
            route
        );
    }
    Ok(())
}

/// `Authorization: Bearer` for a session, `x-api-key` for an API key. For
/// requests built outside the transport (binary download, raw DELETE).
pub fn credential_headers(credential: &BackendCredential) -> Result<HeaderMap> {
    let secret = credential.secret().trim();
    let mut headers = HeaderMap::new();
    match credential {
        BackendCredential::Session(_) => {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {secret}"))
                    .map_err(|e| anyhow::anyhow!("invalid backend credential header: {e}"))?,
            );
        }
        BackendCredential::ApiKey(_) => {
            headers.insert(
                API_KEY_HEADER,
                HeaderValue::from_str(secret)
                    .map_err(|e| anyhow::anyhow!("invalid backend credential header: {e}"))?,
            );
        }
    }
    Ok(headers)
}

fn download_clients() -> &'static (reqwest::Client, reqwest::Client) {
    static CLIENTS: OnceLock<(reqwest::Client, reqwest::Client)> = OnceLock::new();
    CLIENTS.get_or_init(|| {
        // Neither client carries the product identity: their one caller
        // fetches a route that can redirect to presigned storage. The first
        // stops at the redirect, the second follows it without any backend
        // credential or header.
        let build = || {
            tls_client_builder()
                .http1_only()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15 * 60))
                .connect_timeout(Duration::from_secs(15))
                .build()
                .expect("failed to build integration download HTTP client")
        };
        (build(), build())
    })
}

/// Authenticated sends to the hosted `/agent-integrations/*` routes.
#[derive(Clone)]
pub struct IntegrationsHttp {
    backend_url: String,
    credential: BackendCredential,
    transport: Option<Arc<dyn BackendTransport>>,
}

impl IntegrationsHttp {
    /// A helper for `backend_url` (sanitised) authenticating with `credential`.
    pub fn new(backend_url: &str, credential: BackendCredential) -> Self {
        Self {
            backend_url: sanitize_backend_url(backend_url),
            credential,
            transport: None,
        }
    }

    /// Send through `transport` instead of the shared SDK transport.
    pub fn with_transport(mut self, transport: Arc<dyn BackendTransport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// The (sanitised) backend origin routes are joined onto.
    pub fn backend_url(&self) -> &str {
        &self.backend_url
    }

    /// The credential requests carry.
    pub fn credential(&self) -> &BackendCredential {
        &self.credential
    }

    /// Whether the credential is a TinyHumans API key rather than a session.
    pub fn uses_api_key(&self) -> bool {
        self.credential.is_api_key()
    }

    /// The absolute URL for `path` (for logs and error text).
    pub fn url_for(&self, path: &str) -> String {
        join_url(&self.backend_url, path)
    }

    /// Check the credential may be sent to this origin (managed host or
    /// loopback for a key; HTTPS or loopback for any credential).
    pub fn validate_credential_endpoint(&self) -> Result<()> {
        validate_credential_endpoint(&self.backend_url, self.uses_api_key())
    }

    /// The auth headers for a request built outside the transport.
    pub fn auth_headers(&self) -> Result<HeaderMap> {
        self.validate_credential_endpoint()?;
        credential_headers(&self.credential)
    }

    /// The no-redirect client for binary downloads (no attribution headers).
    pub fn download_client(&self) -> reqwest::Client {
        download_clients().0.clone()
    }

    /// The client that follows a presigned-storage redirect without any
    /// backend credential.
    pub fn presigned_client(&self) -> reqwest::Client {
        download_clients().1.clone()
    }

    fn resolve_transport(&self) -> Result<Arc<dyn BackendTransport>, BackendTransportError> {
        match &self.transport {
            Some(transport) => Ok(transport.clone()),
            None => shared_transport()
                .map(|t| t as Arc<dyn BackendTransport>)
                .map_err(|error| {
                    log::debug!("{LOG_PREFIX} transport unavailable: {error:#}");
                    BackendTransportError::Unavailable
                }),
        }
    }

    fn request<'a>(
        &'a self,
        method: Method,
        path: &'a str,
        body: Option<&'a Value>,
    ) -> BackendRequest<'a> {
        BackendRequest {
            profile: TransportProfile::Integrations,
            base_url: &self.backend_url,
            method,
            path,
            query: &[],
            body,
            credential: Some(&self.credential),
            // The integrations domain parses the `{success,data}` envelope
            // itself so its error classification sees the raw shape.
            unwrap_envelope: false,
        }
    }

    /// Send one JSON round-trip and return the raw (envelope-intact) body.
    /// Runs the credential and route guards first; guard failures are `Other`.
    pub async fn send_json(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, BackendTransportError> {
        self.validate_credential_endpoint()
            .map_err(|e| BackendTransportError::Other(e.to_string()))?;
        reject_privileged_backend_path(method.as_str(), path)
            .map_err(|e| BackendTransportError::Other(e.to_string()))?;
        let transport = self.resolve_transport()?;
        log::trace!("{LOG_PREFIX} {} {path}", method.as_str());
        transport.send_json(self.request(method, path, body)).await
    }

    /// Send a `multipart/form-data` upload and return the response body.
    pub async fn send_multipart(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<Value, BackendTransportError> {
        self.validate_credential_endpoint()
            .map_err(|e| BackendTransportError::Other(e.to_string()))?;
        reject_privileged_backend_path("POST", path)
            .map_err(|e| BackendTransportError::Other(e.to_string()))?;
        let transport = self.resolve_transport()?;
        log::trace!("{LOG_PREFIX} POST(multipart) {path}");
        transport
            .send_multipart(self.request(Method::POST, path, None), form)
            .await
    }
}

#[cfg(test)]
#[path = "integrations_http_tests.rs"]
mod tests;
