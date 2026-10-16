//! The hosted-backend HTTP port: the request shape, the [`BackendTransport`]
//! trait and its typed error. Owned by this crate; the core has no backend
//! port. [`SdkBackendTransport`](crate::SdkBackendTransport) implements it.

use async_trait::async_trait;
use openhuman_embed::__host::security::credentials::session_support::BackendCredential;
use reqwest::header::HeaderMap;
use reqwest::Method;
use serde_json::Value;

/// One `(name, value)` query pair; a `None` value omits the pair.
pub type QueryParam = (&'static str, Option<String>);

/// Which pre-built HTTP client profile a request rides. The two profiles
/// preserve the historically distinct `reqwest` configurations of
/// [`crate::backend::BackendClient`] and
/// `integrations::client::IntegrationClient`; the transport builds one
/// `reqwest::Client` per profile.
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
    /// (`BackendClient::new`, `IntegrationClient::new`): absolute
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

    /// The backend origin for `purpose`, given the operator's configured
    /// override (`config.api_url`, already trimmed; `None` when unset).
    ///
    /// The transport owns defaults, environment overrides and any guard
    /// against an override that is really an inference endpoint — the core
    /// knows none of the hosted backend's URLs.
    fn base_url(&self, configured: Option<&str>, purpose: BaseUrlPurpose) -> String;

    /// The product identity this host attributes backend traffic to (the
    /// `x-sdk-name` value), for callers that hand it to a module or header
    /// the transport does not build itself.
    fn product_identity(&self) -> String;

    /// The attribution headers this host stamps on backend traffic
    /// (product identity, client versions), for a caller that must reach the
    /// backend with a client the transport did not build (the managed model
    /// catalog rides the generic provider client).
    fn attribution_headers(&self) -> HeaderMap;

    /// Short stable name for logs (`"tinyhumans-sdk"`, `"plain-test"`).
    fn name(&self) -> &'static str;
}

/// What a backend base URL is for. The two differ only in how an operator
/// override is treated: control-plane calls must never land on an inference
/// endpoint the user pointed `api_url` at, while managed inference honours it
/// as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseUrlPurpose {
    /// Account, integrations, channels, voice, sockets, telemetry.
    ControlPlane,
    /// The OpenAI-compatible inference proxy (chat, embeddings, models).
    Inference,
}

/// Failure of a single backend round-trip, as seen by the core.
#[derive(Debug, thiserror::Error)]
pub enum BackendTransportError {
    /// No transport is installed in this process, so the core cannot reach the
    /// hosted backend at all. This is the normal state of a core built and
    /// run without `openhuman-tinyhumans`; callers surface it as a typed
    /// "backend unavailable" condition, never as a bug.
    #[error("no backend transport installed")]
    Unavailable,
    /// The base URL and path did not compose into a valid URL.
    #[error("invalid url: {0}")]
    Url(String),
    /// The HTTP client failed before a response arrived (DNS, TLS, connect,
    /// timeout, body read). Kept as the `reqwest` error so the source chain
    /// walk in the classifiers still sees nested causes.
    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),
    /// The backend answered with a non-2xx status. `body` is the parsed JSON
    /// body when it parsed, otherwise the raw text as a JSON string.
    #[error("http {status}: {body}")]
    Status { status: u16, body: Value },
    /// `404` on a channel-message route answered by a backend handler: that
    /// message no longer exists (deleted provider-side or garbage-collected).
    #[error("http 404: channel message {provider}/{message_id} not found")]
    ChannelMessageNotFound {
        /// Channel provider path segment (`telegram`, `discord`, …).
        provider: String,
        /// Provider message id from the path.
        message_id: String,
    },
    /// `404` on a channel-message route because the backend has **no such
    /// route** (no handler matched), not because the message is gone. Today
    /// this is every `PATCH` edit (#5230): the message still exists and the
    /// caller must keep its id.
    #[error("http 404: backend implements no route for channel message {provider}/{message_id}")]
    ChannelMessageRouteMissing {
        /// Channel provider path segment.
        provider: String,
        /// Provider message id from the path.
        message_id: String,
    },
    /// The response carried a `{success:false, ...}` envelope on a 2xx status.
    #[error("backend reported failure: {error}")]
    Envelope {
        error: String,
        error_code: Option<String>,
        details: Value,
    },
    /// A header value was not representable on the wire.
    #[error("invalid header value: {0}")]
    Header(String),
    /// The response body could not be decoded into the expected shape.
    #[error("response decoding failed: {0}")]
    Decode(String),
    /// The transport refuses this route by policy (the SDK's unexposed-route
    /// registry, for example). `(method, path)`.
    #[error("route is intentionally not exposed by the backend transport: {0} {1}")]
    RouteNotExposed(String, String),
    /// Anything else the transport implementation reports.
    #[error("{0}")]
    Other(String),
}

impl BackendTransportError {
    /// The HTTP status when this error is a [`Self::Status`].
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::ChannelMessageNotFound { .. } | Self::ChannelMessageRouteMissing { .. } => {
                Some(404)
            }
            _ => None,
        }
    }
}
