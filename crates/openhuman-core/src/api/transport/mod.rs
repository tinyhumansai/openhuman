//! The backend port: how the core reaches the hosted TinyHumans backend
//! without depending on the SDK that speaks to it.
//!
//! The core owns the *routes* (`/payments/summary`,
//! `/agent-integrations/...`, `channels/{c}/messages`, ...) and the error
//! *classification* ([`crate::api::rest`], `integrations/client/errors.rs`).
//! What it does not own is the HTTP primitive that carries a request with the
//! right credential header shape and route policy. That is a
//! [`BackendTransport`], and the only production implementation lives in the
//! `openhuman-tinyhumans` crate on top of the vendored `tinyhumans-sdk`. A core
//! built without that crate has no transport and every backend-touching call
//! degrades to [`BackendTransportError::Unavailable`] — the core keeps running
//! agents, memory, tools and RPC without any TinyHumans connection.
//!
//! See [`install`] for how a transport reaches the core at runtime.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Method;
use serde_json::Value;

use crate::security::credentials::session_support::BackendCredential;

pub mod error;
pub mod install;
#[cfg(test)]
pub mod plain;

pub use error::BackendTransportError;
pub use install::{
    clear_backend_transport, install_backend_transport, installed_backend_transport, is_installed,
    resolve_backend_transport,
};

/// One `(name, value)` query pair; a `None` value omits the pair.
pub type QueryParam = (&'static str, Option<String>);

/// Which pre-built HTTP client profile a request rides. The two profiles
/// preserve the historically distinct `reqwest` configurations of
/// [`crate::api::rest::BackendOAuthClient`] and
/// `integrations::client::IntegrationClient`; see
/// [`crate::api::headers::backend_client_builder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportProfile {
    /// Control-plane REST (`/auth/*`, `/payments/*`, `/channels/*`, ...).
    Api,
    /// Agent tool traffic (`/agent-integrations/*`).
    Integrations,
}

/// A single backend round-trip, fully described by the caller.
#[derive(Debug)]
pub struct BackendRequest<'a> {
    /// Client profile to send with.
    pub profile: TransportProfile,
    /// Origin of the backend. Already sanitised by the caller
    /// (`BackendOAuthClient::new`, `IntegrationClient::new`): absolute
    /// `http(s)` URL, no path, no query.
    pub base_url: &'a str,
    /// HTTP verb.
    pub method: Method,
    /// Root-relative route, with or without a leading `/`.
    pub path: &'a str,
    /// Query pairs; `None` values are omitted.
    pub query: &'a [QueryParam],
    /// JSON body, if any.
    pub body: Option<&'a Value>,
    /// The credential to present. A session JWT rides as `Authorization:
    /// Bearer`, an API key as `x-api-key`; `None` sends an anonymous request.
    pub credential: Option<&'a BackendCredential>,
    /// Whether a `{success, data}` envelope is unwrapped before the value is
    /// returned (`success: false` then becomes
    /// [`BackendTransportError::Envelope`]).
    pub unwrap_envelope: bool,
}

impl<'a> BackendRequest<'a> {
    /// A request with no query, no body and no credential.
    pub fn new(
        profile: TransportProfile,
        base_url: &'a str,
        method: Method,
        path: &'a str,
    ) -> Self {
        Self {
            profile,
            base_url,
            method,
            path,
            query: &[],
            body: None,
            credential: None,
            unwrap_envelope: true,
        }
    }
}

/// The HTTP primitive every backend-bound call in the core goes through.
///
/// Implementations are process-wide singletons (`Arc<dyn BackendTransport>`)
/// and must be cheap to call concurrently. They own TLS, timeouts, redirect
/// and route policy; the core supplies routes, credentials and classification.
#[async_trait]
pub trait BackendTransport: Send + Sync + 'static {
    /// Send `req` and return the (optionally envelope-unwrapped) JSON body.
    async fn send_json(&self, req: BackendRequest<'_>) -> Result<Value, BackendTransportError>;

    /// `POST` a `multipart/form-data` body to `req.path` and return the
    /// envelope-unwrapped JSON body. `req.body` is ignored.
    async fn send_multipart(
        &self,
        req: BackendRequest<'_>,
        form: reqwest::multipart::Form,
    ) -> Result<Value, BackendTransportError>;

    /// The configured `reqwest::Client` for `profile`, for callers that drive a
    /// non-JSON request shape themselves (multipart STT upload). Carries the
    /// profile's TLS settings and attribution headers, but **no** credential.
    fn http_client(&self, profile: TransportProfile) -> reqwest::Client;

    /// Short stable name for logs (`"tinyhumans-sdk"`, `"plain-test"`).
    fn name(&self) -> &'static str;
}

/// Header the backend expects a TinyHumans API key on.
pub const API_KEY_HEADER: &str = "x-api-key";

/// The header a [`BackendCredential`] rides on: session JWT as
/// `Authorization: Bearer`, API key as `x-api-key`. Shared by transport
/// implementations that shape headers themselves.
pub fn credential_headers(
    credential: &BackendCredential,
) -> Result<HeaderMap, BackendTransportError> {
    let secret = credential.secret().trim();
    let mut headers = HeaderMap::new();
    match credential {
        BackendCredential::Session(_) => {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {secret}"))
                    .map_err(|e| BackendTransportError::Header(e.to_string()))?,
            );
        }
        BackendCredential::ApiKey(_) => {
            log::trace!("[backend-transport] authenticating request with x-api-key");
            headers.insert(
                API_KEY_HEADER,
                HeaderValue::from_str(secret)
                    .map_err(|e| BackendTransportError::Header(e.to_string()))?,
            );
        }
    }
    Ok(headers)
}

/// Parse a response body the way the backend transports do: empty → `Null`,
/// JSON → the value, anything else → the raw text as a JSON string.
pub fn parse_body_text(text: String) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }
}

/// Unwrap the hosted-backend `{success, data}` envelope.
///
/// `success: false` becomes [`BackendTransportError::Envelope`]: the backend
/// pairs a failed operation with an unsuccessful envelope that is not always
/// accompanied by a non-2xx status. A successful envelope with no `data` key
/// yields the remaining fields with `success` removed, so envelopes that
/// inline their payload (`{success, jwt}`) do not leak the flag.
pub fn unwrap_envelope(body: Value) -> Result<Value, BackendTransportError> {
    let Value::Object(mut map) = body else {
        return Ok(body);
    };
    match map.get("success").and_then(Value::as_bool) {
        Some(true) => {
            if let Some(data) = map.remove("data") {
                return Ok(data);
            }
            map.remove("success");
            Ok(Value::Object(map))
        }
        Some(false) => Err(BackendTransportError::Envelope {
            error: map
                .get("error")
                .or_else(|| map.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("request unsuccessful")
                .to_owned(),
            error_code: map
                .get("errorCode")
                .and_then(Value::as_str)
                .map(str::to_owned),
            details: map.get("details").cloned().unwrap_or(Value::Null),
        }),
        None => Ok(Value::Object(map)),
    }
}

/// Compose `base_url` + `path` + `query` into the request URL.
pub fn compose_url(
    base_url: &str,
    path: &str,
    query: &[QueryParam],
) -> Result<reqwest::Url, BackendTransportError> {
    let base = base_url.trim().trim_end_matches('/');
    let normalized = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let mut url = reqwest::Url::parse(&format!("{base}{normalized}"))
        .map_err(|e| BackendTransportError::Url(e.to_string()))?;
    let pairs: Vec<(&str, String)> = query
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| (*k, v.clone())))
        .collect();
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    Ok(url)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
