//! [`BackendClient`]: authenticated JSON calls to hosted-backend routes,
//! sent through this crate's own [`SdkBackendTransport`].

use std::sync::{Arc, Mutex};

use crate::transport::BackendRequest;
use crate::transport::{BackendTransport, TransportProfile};
use anyhow::{Context, Result};
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::credentials::session_support::BackendCredential;
use reqwest::{Client, Method, Url};
use serde_json::Value;

use super::endpoint::validate_credential_endpoint;
use super::error::HostedApiError;
use crate::backend::url::effective_backend_api_url;
use crate::transport::SdkBackendTransport;

const LOG_PREFIX: &str = "[tinyhumans-client]";

fn shared_slot() -> &'static Mutex<Option<Arc<SdkBackendTransport>>> {
    static SLOT: Mutex<Option<Arc<SdkBackendTransport>>> = Mutex::new(None);
    &SLOT
}

/// The process-wide [`SdkBackendTransport`] the hosted client sends through,
/// built on first use. It is this crate's own instance: it does not read the
/// core's installed-transport slot, so the client works the same with or
/// without [`install`](crate::install).
pub fn shared_transport() -> Result<Arc<SdkBackendTransport>> {
    let mut slot = shared_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(transport) = slot.as_ref() {
        return Ok(transport.clone());
    }
    let transport = Arc::new(SdkBackendTransport::new().context("build tinyhumans transport")?);
    *slot = Some(transport.clone());
    Ok(transport)
}

/// Drop the cached [`shared_transport`] so the next use rebuilds it. A
/// transport captures its attribution headers once, so a product-identity
/// change (see [`install`](crate::install)) must discard it.
pub(crate) fn reset_shared_transport() {
    *shared_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// A client for authenticated hosted-backend routes.
///
/// Owns the routes and the error classification; the HTTP round-trip rides a
/// [`BackendTransport`], by default the crate's [`shared_transport`], which
/// carries TLS, timeouts, attribution headers and the credential header shape.
#[derive(Clone)]
pub struct BackendClient {
    pub(super) base: Url,
    transport: Option<Arc<dyn BackendTransport>>,
}

impl BackendClient {
    /// Creates a client for the given API base URL.
    ///
    /// Any path, query, or fragment in `api_base` is stripped so that
    /// `Url::join` always resolves root-relative REST paths correctly, even
    /// when a caller passes a full completions URL
    /// (`https://host/v1/chat/completions`).
    pub fn new(api_base: &str) -> Result<Self> {
        let mut base = Url::parse(api_base.trim()).context("Invalid API base URL")?;
        anyhow::ensure!(
            matches!(base.scheme(), "http" | "https") && base.host_str().is_some(),
            "API base URL must be an absolute http(s) URL with host"
        );
        base.set_path("");
        base.set_query(None);
        base.set_fragment(None);
        Ok(Self {
            base,
            transport: None,
        })
    }

    /// A client for the control-plane origin resolved from `config.api_url`
    /// ([`effective_backend_api_url`]).
    pub fn from_config(config: &Config) -> Result<Self> {
        Self::new(&effective_backend_api_url(&config.api_url))
    }

    /// Send through `transport` instead of the shared SDK transport.
    pub fn with_transport(mut self, transport: Arc<dyn BackendTransport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// The backend origin this client resolves routes against.
    pub fn base_url(&self) -> &str {
        self.base.as_str()
    }

    fn unavailable(&self, method: &str, path: &str) -> anyhow::Error {
        let route = self.url_for(path).map(|u| u.path().to_string());
        anyhow::Error::new(HostedApiError::BackendUnavailable {
            method: method.to_string(),
            path: route.unwrap_or_else(|_| path.to_string()),
        })
    }

    fn transport(&self, method: &Method, path: &str) -> Result<Arc<dyn BackendTransport>> {
        if let Some(transport) = &self.transport {
            return Ok(transport.clone());
        }
        match shared_transport() {
            Ok(transport) => Ok(transport),
            Err(error) => {
                log::debug!(
                    "{LOG_PREFIX} transport unavailable for {} {path}: {error:#}",
                    method.as_str()
                );
                Err(self.unavailable(method.as_str(), path))
            }
        }
    }

    /// The transport's `reqwest::Client`, for callers that drive a non-JSON
    /// request shape (e.g. `multipart/form-data` STT uploads) without
    /// re-implementing TLS/proxy plumbing. Carries the attribution headers;
    /// the caller adds the credential.
    pub fn raw_client(&self) -> Result<Client> {
        let transport = match &self.transport {
            Some(transport) => transport.clone(),
            None => shared_transport()
                .map(|t| t as Arc<dyn BackendTransport>)
                .map_err(|_| self.unavailable("RAW", ""))?,
        };
        Ok(transport.http_client(TransportProfile::Api))
    }

    /// Resolve a backend-relative path against the base URL, as
    /// [`authed_json`](Self::authed_json) does internally.
    pub fn url_for(&self, path: &str) -> Result<Url> {
        self.base
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("build URL for {path}"))
    }

    /// Generic authenticated JSON request for backend API routes.
    ///
    /// `credential` accepts a [`BackendCredential`] or a bare session token.
    /// The transport sends API keys as `x-api-key` and sessions as Bearer JWTs.
    pub async fn authed_json(
        &self,
        credential: impl Into<BackendCredential>,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let credential = credential.into();
        let is_api_key = credential.is_api_key();
        validate_credential_endpoint(self.base.as_str(), is_api_key)?;
        let transport = self.transport(&method, path)?;
        log::trace!("{LOG_PREFIX} {} {path}", method.as_str());
        let response = transport
            .send_json(BackendRequest {
                profile: TransportProfile::Api,
                base_url: self.base.as_str(),
                method: method.clone(),
                path,
                query: &[],
                body: body.as_ref(),
                credential: Some(&credential),
                unwrap_envelope: true,
            })
            .await;
        self.finish_authed_json(method, path, response, is_api_key)
    }
}

#[cfg(test)]
#[path = "backend_client_tests.rs"]
mod tests;
