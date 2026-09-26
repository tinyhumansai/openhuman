//! The one way a hosted domain reaches the TinyHumans backend: a typed
//! [`TinyHumansClient`] authenticated with the core's resolved credential,
//! plus the single mapping from [`tinyhumans_sdk::Error`] onto the error
//! sentinels the core's JSON-RPC layer classifies.
//!
//! # Credential first, no network
//!
//! [`HostedClient::from_config`] asks the core for the credential
//! ([`resolve_backend_credential`]) before anything touches the network. The
//! offline local session, a missing token and a locally-expired token all come
//! back as the core's own error string, which already carries the right
//! sentinel (`BACKEND_UNAVAILABLE:` / `SESSION_EXPIRED:`), so a user without a
//! TinyHumans account never produces a doomed request or a Sentry event
//! (Sentry 36649).
//!
//! # Error mapping
//!
//! | SDK error                      | RPC error string                           |
//! | ------------------------------ | ------------------------------------------ |
//! | `Status{401}`, session         | `SESSION_EXPIRED: …`                       |
//! | `Status{401}`, API key         | `API_KEY_REJECTED: …`                      |
//! | `Status{..}` other             | `{op} failed ({status}): {body}`           |
//! | `Http`                         | `backend request {op}: {source chain}`     |
//! | `Envelope` / `Decode` / others | `backend request {op}: {error}`            |
//! | `RouteNotExposed`              | same, logged at `error` (a bug here)       |
//!
//! The non-sentinel shapes are exactly what `BackendOAuthClient::authed_json`
//! produced before, so the JSON-RPC classifiers (transient status, transport
//! phrases, budget exhaustion) keep matching them.

use openhuman_core::api::config::effective_backend_api_url;
use openhuman_core::api::transport::{resolve_backend_transport, TransportProfile};
use openhuman_core::config::Config;
use openhuman_core::core::observability::{
    contains_transient_transport_phrase, is_transient_http_status_code, API_KEY_REJECTED_PREFIX,
};
use openhuman_core::security::credentials::session_support::{
    resolve_backend_credential, BackendCredential,
};
use serde_json::Value;
use tinyhumans_sdk::{Error as SdkError, TinyHumansClient};

const LOG_PREFIX: &str = "[hosted][client]";

/// Which kind of credential authenticated a [`HostedClient`]. Decides the
/// recovery a 401 maps to: a session expires, an API key is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    Session,
    ApiKey,
}

/// A typed SDK client bound to the caller's backend credential.
pub struct HostedClient {
    sdk: TinyHumansClient,
    kind: CredentialKind,
}

impl HostedClient {
    /// Resolve the credential for `config` and build the client.
    ///
    /// Errors without any network I/O when there is no usable credential; the
    /// error is the core's own string, returned unchanged (it already carries
    /// its sentinel).
    pub fn from_config(config: &Config) -> Result<Self, String> {
        let credential = resolve_backend_credential(config).inspect_err(|err| {
            log::debug!("{LOG_PREFIX} no usable backend credential; skipping request: {err}");
        })?;
        let base_url = backend_origin(&effective_backend_api_url(&config.api_url))?;
        Ok(Self::with_credential(&base_url, credential))
    }

    /// Build the client against `base_url` for an already-resolved credential.
    pub fn with_credential(base_url: &str, credential: BackendCredential) -> Self {
        let sdk = TinyHumansClient::new(base_url)
            .with_http_client(http_client())
            // The product identity (`x-sdk-name`) also rides the SDK's own
            // default headers, exactly as the SDK transport does, so it holds
            // even for a client this crate did not build.
            .with_default_headers(openhuman_core::api::product::product_identity_headers());
        match credential {
            BackendCredential::Session(secret) => Self {
                sdk: sdk.with_token(Some(secret.trim().to_string())),
                kind: CredentialKind::Session,
            },
            BackendCredential::ApiKey(secret) => Self {
                sdk: sdk.with_api_key(Some(secret.trim().to_string())),
                kind: CredentialKind::ApiKey,
            },
        }
    }

    /// The typed SDK client.
    pub fn sdk(&self) -> &TinyHumansClient {
        &self.sdk
    }

    /// The credential kind this client authenticates with.
    pub fn kind(&self) -> CredentialKind {
        self.kind
    }

    /// Map an SDK result onto the RPC `String` error channel. `op` names the
    /// route as `"METHOD /template"` (no ids) for logs and error text.
    pub fn finish<T>(&self, op: &str, result: Result<T, SdkError>) -> Result<T, String> {
        result.map_err(|err| map_error(self.kind, op, err))
    }

    /// [`Self::finish`] for the SDK's untyped `DynamicResponse` payloads.
    pub fn finish_value(
        &self,
        op: &str,
        result: Result<tinyhumans_sdk::api::types::DynamicResponse, SdkError>,
    ) -> Result<Value, String> {
        self.finish(op, result).map(|v| v.0)
    }
}

/// The backend origin with any path, query and fragment stripped — the same
/// normalisation `BackendOAuthClient::new` applies, so a completions-style
/// `api_url` (`https://host/v1/chat/completions`) still reaches `/teams/...`.
fn backend_origin(api_url: &str) -> Result<String, String> {
    let mut url =
        url::Url::parse(api_url.trim()).map_err(|e| format!("Invalid API base URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("API base URL must be an absolute http(s) URL with host".to_string());
    }
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// The `reqwest::Client` hosted calls ride: the active backend transport's
/// (TLS backend, timeouts and attribution headers the core specifies), or a
/// freshly built one with the same settings when none resolves.
fn http_client() -> reqwest::Client {
    if let Ok(transport) = resolve_backend_transport() {
        return transport.http_client(TransportProfile::Api);
    }
    openhuman_core::api::headers::build_backend_client(TransportProfile::Api).unwrap_or_else(
        |err| {
            log::warn!("{LOG_PREFIX} failed to build backend client, using default: {err}");
            reqwest::Client::new()
        },
    )
}

/// The single `tinyhumans_sdk::Error` → RPC error mapping.
pub fn map_error(kind: CredentialKind, op: &str, err: SdkError) -> String {
    match err {
        SdkError::Status { status: 401, .. } => {
            log::info!("{LOG_PREFIX} 401 on {op} — credential rejected ({kind:?})");
            match kind {
                // `SESSION_EXPIRED` makes the JSON-RPC layer skip Sentry and
                // publish `SessionExpired` so the auth domain re-signs in.
                CredentialKind::Session => {
                    format!("SESSION_EXPIRED: backend rejected session token on {op}")
                }
                // An API key has no session to expire: clearing the app session
                // would be the wrong recovery (see `BackendApiError::ApiKeyRejected`).
                CredentialKind::ApiKey => {
                    format!("{API_KEY_REJECTED_PREFIX} backend rejected api key on {op}")
                }
            }
        }
        SdkError::Status { status, body } => {
            let text = match body {
                Value::String(text) => text,
                Value::Null => String::new(),
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            let status_label = reqwest::StatusCode::from_u16(status)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| status.to_string());
            if is_transient_http_status_code(status) {
                log::warn!("{LOG_PREFIX} transient {status} on {op}");
            } else {
                log::debug!(
                    "{LOG_PREFIX} {status} on {op} (response_body_len={})",
                    text.len()
                );
            }
            format!("{op} failed ({status_label}): {text}")
        }
        SdkError::Http(e) => {
            // Walk the source chain so transient markers hidden in nested
            // causes (reqwest → hyper → rustls) still classify downstream.
            let mut message = e.to_string();
            let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&e);
            while let Some(s) = src {
                message.push_str(": ");
                message.push_str(&s.to_string());
                src = s.source();
            }
            if contains_transient_transport_phrase(&message) {
                log::warn!("{LOG_PREFIX} transient transport failure on {op}: {message}");
            } else {
                log::debug!("{LOG_PREFIX} transport failure on {op}: {message}");
            }
            format!("backend request {op}: {message}")
        }
        SdkError::RouteNotExposed(method, path) => {
            log::error!("{LOG_PREFIX} SDK refused unexposed route {method} {path} for {op}");
            format!("backend request {op}: route is not exposed by the SDK: {method} {path}")
        }
        other => {
            log::debug!("{LOG_PREFIX} {op} failed: {other}");
            format!("backend request {op}: {other}")
        }
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
