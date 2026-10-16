//! The SDK-backed [`BackendTransport`]: every hosted-backend request the core
//! makes rides the vendored `tinyhumans-sdk`'s HTTP primitive, which owns
//! route policy (the unexposed-route registry) and the credential header
//! shapes the backend expects.

use std::sync::Arc;

use async_trait::async_trait;
use openhuman_embed::__host::security::credentials::session_support::BackendCredential;
use reqwest::Method;
use serde_json::Value;
use tinyhumans_sdk::TinyHumansClient;

mod error;
mod port;

pub use error::map_sdk_error;
pub use port::{
    BackendRequest, BackendTransport, BackendTransportError, BaseUrlPurpose, QueryParam,
    TransportProfile,
};

/// One `reqwest::Client` per [`TransportProfile`], each built from
/// [`backend_client_builder`](crate::backend::headers::backend_client_builder)
/// so TLS, timeouts and the attribution headers (`x-core-version`,
/// `x-tauri-version`, `x-sdk-name`) ride every request. Also answers the
/// core's base-URL and identity questions from [`crate::backend`].
pub struct SdkBackendTransport {
    api: reqwest::Client,
    integrations: reqwest::Client,
}

impl SdkBackendTransport {
    /// Build the transport. Fails only if a `reqwest::Client` cannot be
    /// constructed (TLS backend unavailable, malformed attribution header).
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            api: crate::backend::headers::build_backend_client(TransportProfile::Api)?,
            integrations: crate::backend::headers::build_backend_client(
                TransportProfile::Integrations,
            )?,
        })
    }

    /// [`Self::new`] as a shared handle ready for
    /// the process-wide transport (see [`crate::client::shared_transport`]).
    pub fn shared() -> anyhow::Result<Arc<dyn BackendTransport>> {
        Ok(Arc::new(Self::new()?))
    }

    fn client(&self, profile: TransportProfile) -> &reqwest::Client {
        match profile {
            TransportProfile::Api => &self.api,
            TransportProfile::Integrations => &self.integrations,
        }
    }

    fn sdk(&self, req: &BackendRequest<'_>) -> TinyHumansClient {
        // The product identity also rides on the SDK's own default headers,
        // not just the transport's, so it survives if the SDK is ever given a
        // client this crate did not build. The SDK applies its own headers
        // after these, so it cannot be clobbered by `x-sdk-client`.
        let sdk = TinyHumansClient::new(req.base_url)
            .with_http_client(self.client(req.profile).clone())
            .with_default_headers(crate::backend::product::product_identity_headers());
        match req.credential {
            Some(BackendCredential::Session(secret)) => {
                sdk.with_token(Some(secret.trim().to_string()))
            }
            Some(BackendCredential::ApiKey(secret)) => {
                log::trace!("[tinyhumans-transport] authenticating request with x-api-key");
                sdk.with_api_key(Some(secret.trim().to_string()))
            }
            None => sdk,
        }
    }
}

#[async_trait]
impl BackendTransport for SdkBackendTransport {
    async fn send_json(&self, req: BackendRequest<'_>) -> Result<Value, BackendTransportError> {
        log::trace!(
            "[tinyhumans-transport] {} {} profile={:?} unwrap={}",
            req.method,
            req.path,
            req.profile,
            req.unwrap_envelope
        );
        let sdk = self.sdk(&req);
        sdk.raw()
            .send(
                req.method.clone(),
                req.path,
                req.query,
                req.body,
                req.unwrap_envelope,
            )
            .await
            .map_err(|error| map_sdk_error(error, &req.method, req.path))
    }

    async fn send_multipart(
        &self,
        req: BackendRequest<'_>,
        form: reqwest::multipart::Form,
    ) -> Result<Value, BackendTransportError> {
        log::trace!(
            "[tinyhumans-transport] POST(multipart) {} profile={:?}",
            req.path,
            req.profile
        );
        let sdk = self.sdk(&req);
        sdk.raw()
            .post_multipart(req.path, form)
            .await
            .map_err(|error| map_sdk_error(error, &Method::POST, req.path))
    }

    fn http_client(&self, profile: TransportProfile) -> reqwest::Client {
        self.client(profile).clone()
    }

    fn base_url(&self, configured: Option<&str>, purpose: BaseUrlPurpose) -> String {
        let configured = configured.map(str::to_owned);
        match purpose {
            BaseUrlPurpose::ControlPlane => {
                crate::backend::url::effective_backend_api_url(&configured)
            }
            BaseUrlPurpose::Inference => crate::backend::url::effective_api_url(&configured),
        }
    }

    fn product_identity(&self) -> String {
        crate::backend::product::product_identity()
            .as_str()
            .to_owned()
    }

    fn attribution_headers(&self) -> reqwest::header::HeaderMap {
        // Built fresh so a late product-identity or shell-version change is
        // reflected; a malformed header value degrades to identity only.
        crate::backend::headers::attribution_headers()
            .unwrap_or_else(|_| crate::backend::product::product_identity_headers())
    }

    fn name(&self) -> &'static str {
        "tinyhumans-sdk"
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "channel_404_tests.rs"]
mod channel_404_tests;
