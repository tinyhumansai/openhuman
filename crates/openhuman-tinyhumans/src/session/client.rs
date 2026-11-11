//! The backend calls a session owner makes: login-token exchange and
//! `GET /auth/me`, plus the store-time validation policy (budget, one retry,
//! transient classification) the core used to apply before persisting a JWT.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tinyhumans_sdk::api::types::LoginTokenRequest;
use tinyhumans_sdk::TinyHumansClient;

use crate::session::credential::{Credential, CredentialKind};

const LOG_PREFIX: &str = "[session][client]";

/// Header the backend reads to attribute a request to a product.
pub const PRODUCT_IDENTITY_HEADER: &str = "x-sdk-name";
const CORE_VERSION_HEADER: &str = "x-core-version";
const TAURI_VERSION_HEADER: &str = "x-tauri-version";
const HEADER_VALUE_MAX_LEN: usize = 64;

/// Transport-failure phrases treated as transient (same list as the core's
/// `observability::TRANSIENT_TRANSPORT_PHRASES`, kept in sync by hand because
/// this crate must not depend on the core).
const TRANSIENT_TRANSPORT_PHRASES: &[&str] = &[
    "timeout",
    "operation timed out",
    "connection forcibly closed",
    "connection reset",
    "tls handshake eof",
    "error sending request",
];

/// Wall-clock budget for store-time `GET /auth/me` validation. Kept well under
/// the desktop sign-in flow's patience so a slow backend fails *fast* into the
/// deferred-validation path instead of bouncing an authenticated user (#5166).
pub const VALIDATION_BUDGET: Duration = Duration::from_secs(12);
/// Operator override for [`VALIDATION_BUDGET`], in milliseconds.
pub const VALIDATION_BUDGET_ENV: &str = "OPENHUMAN_AUTH_ME_TIMEOUT_MS";
/// The name the core used before this crate existed; still honoured.
pub const LEGACY_VALIDATION_BUDGET_ENV: &str = "OPENHUMAN_AUTH_ME_STORE_TIMEOUT_MS";
const VALIDATION_RETRY_DELAY: Duration = Duration::from_millis(150);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Attribution headers every backend request carries. `sdk_name` is the
/// product identity (`x-sdk-name`), the versions are optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHeaders {
    pub sdk_name: String,
    pub core_version: Option<String>,
    pub tauri_version: Option<String>,
}

impl ClientHeaders {
    pub fn new(sdk_name: &str) -> Self {
        Self {
            sdk_name: sanitize_identity(sdk_name).unwrap_or_else(|| "openhuman".to_string()),
            core_version: None,
            tauri_version: None,
        }
    }

    pub fn with_core_version(mut self, version: &str) -> Self {
        self.core_version = sanitize_version(version);
        self
    }

    pub fn with_tauri_version(mut self, version: &str) -> Self {
        self.tauri_version = sanitize_version(version);
        self
    }

    fn header_map(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let mut put = |name: &'static str, value: &str| {
            if let Ok(value) = HeaderValue::from_str(value) {
                headers.insert(HeaderName::from_static(name), value);
            }
        };
        put(PRODUCT_IDENTITY_HEADER, &self.sdk_name);
        if let Some(v) = &self.core_version {
            put(CORE_VERSION_HEADER, v);
        }
        if let Some(v) = &self.tauri_version {
            put(TAURI_VERSION_HEADER, v);
        }
        headers
    }
}

/// Lower-cased ASCII alphanumerics plus `.`, `_`, `-`, capped at 64 — the
/// same allowlist as the core's `ProductIdentity::new`.
fn sanitize_identity(raw: &str) -> Option<String> {
    let s: String = raw
        .trim()
        .chars()
        .filter(|c| matches!(c, '0'..='9' | 'A'..='Z' | 'a'..='z' | '.' | '_' | '-'))
        .take(HEADER_VALUE_MAX_LEN)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    (!s.is_empty()).then_some(s)
}

fn sanitize_version(raw: &str) -> Option<String> {
    let s: String = raw
        .trim()
        .chars()
        .filter(|c| matches!(c, '0'..='9' | 'A'..='Z' | 'a'..='z' | '.' | '_' | '+' | '-'))
        .take(HEADER_VALUE_MAX_LEN)
        .collect();
    (!s.is_empty()).then_some(s)
}

/// Why a `GET /auth/me` did not yield a user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchMeError {
    /// The backend answered 401: the session credential is dead.
    Rejected(String),
    /// The backend answered with any other non-2xx status or a `success:false`
    /// envelope. 402/403/404 are refusals (plan, WAF, proxy), not an expired
    /// session, so the session is kept and the caller backs off.
    Transient(String),
    /// The request never got an answer (connect/timeout/TLS/read failure).
    Transport(String),
    /// A refresh completed after its credential was replaced or cleared.
    /// Callers must read the current core credential and retry instead of
    /// applying this response to the prior session.
    Superseded,
    /// Replayed from the current-user cache's backoff window instead of going
    /// to the network. Carries the failure that opened the window.
    Suppressed {
        message: String,
        consecutive: u32,
        retry_in: Duration,
    },
}

impl FetchMeError {
    pub fn message(&self) -> &str {
        match self {
            Self::Rejected(m) | Self::Transient(m) | Self::Transport(m) => m,
            Self::Superseded => "refresh superseded by a credential change",
            Self::Suppressed { message, .. } => message,
        }
    }

    /// "The backend was unreachable or unhealthy", as opposed to "the
    /// credential is bad". Only these are worth backing off or deferring.
    pub fn is_availability_failure(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Transport(_))
    }

    fn from_sdk(error: tinyhumans_sdk::Error) -> Self {
        match error {
            tinyhumans_sdk::Error::Status { status, body } => {
                // Error bodies are controlled by the backend or an intervening
                // proxy.  Do not carry them into the manager's logs.
                let message = format!("http {status}: {}", error_body_shape(&body));
                // `/auth/me` always carries the session credential.
                if openhuman_embed::__host::core::session_expiry::backend_status_means_session_expired(
                    status, false,
                ) {
                    Self::Rejected(message)
                } else {
                    Self::Transient(message)
                }
            }
            tinyhumans_sdk::Error::Envelope { error, .. } => Self::Transient(error),
            other => {
                let message = format!("{other:#}");
                if contains_transient_transport_phrase(&message) {
                    Self::Transport(message)
                } else {
                    // A decoding or SDK failure provides no authoritative
                    // evidence that the credential was refused.
                    Self::Transport("unexpected backend client failure".to_string())
                }
            }
        }
    }
}

fn error_body_shape(body: &Value) -> &'static str {
    match body {
        Value::Null => "empty response body",
        Value::String(value) if value.trim().is_empty() => "empty response body",
        _ => "non-empty response body",
    }
}

impl std::fmt::Display for FetchMeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(m) => write!(f, "backend rejected the credential: {m}"),
            Self::Transient(m) => write!(f, "backend temporarily unavailable: {m}"),
            Self::Transport(m) => write!(f, "backend unreachable: {m}"),
            Self::Superseded => write!(f, "refresh superseded by a credential change"),
            Self::Suppressed {
                message,
                consecutive,
                retry_in,
            } => write!(
                f,
                "backend failed {consecutive}x, retrying in {}ms: {message}",
                retry_in.as_millis()
            ),
        }
    }
}

pub fn contains_transient_transport_phrase(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    TRANSIENT_TRANSPORT_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

#[derive(Debug, thiserror::Error)]
pub enum SessionClientError {
    #[error("invalid backend API base URL: {0}")]
    InvalidBaseUrl(String),
    #[error("failed to build HTTP client: {0}")]
    Http(String),
    #[error("login token is required")]
    EmptyLoginToken,
    #[error("consume login token failed: {0}")]
    ConsumeFailed(String),
    #[error("consume login token response missing jwt")]
    MissingJwt,
}

/// A pooled client for one backend base URL. Cheap to clone; keep one per
/// base URL so the TCP+TLS connection survives between polls (#6180).
#[derive(Clone)]
pub struct SessionClient {
    base: String,
    sdk: TinyHumansClient,
}

/// Canonical non-secret backend origin shared by client construction and
/// callers that pin the expected issuer before accepting a callback.
pub(super) fn normalize_backend_origin(base_url: &str) -> Result<String, SessionClientError> {
    let mut base = url::Url::parse(base_url.trim())
        .map_err(|e| SessionClientError::InvalidBaseUrl(e.to_string()))?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
    {
        return Err(SessionClientError::InvalidBaseUrl(
            "must be an absolute http(s) URL with host".to_string(),
        ));
    }
    base.set_path("");
    base.set_query(None);
    base.set_fragment(None);
    Ok(base.as_str().trim_end_matches('/').to_string())
}

impl SessionClient {
    pub fn new(base_url: &str, headers: &ClientHeaders) -> Result<Self, SessionClientError> {
        let base = normalize_backend_origin(base_url)?;

        let http = openhuman_embed::__host::util::tls::tls_client_builder()
            .http1_only()
            // `sdk_with` adds `x-api-key` per request. Reqwest does not strip
            // this custom header when following a cross-origin redirect.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .default_headers(headers.header_map())
            .build()
            .map_err(|e| SessionClientError::Http(e.to_string()))?;
        let sdk = TinyHumansClient::new(base.as_str())
            .with_http_client(http)
            .with_default_headers(headers.header_map());
        Ok(Self { base, sdk })
    }

    /// The normalised base URL (no trailing slash) — the cache key for
    /// everything fetched through this client.
    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn sdk_with(&self, credential: &Credential) -> TinyHumansClient {
        let secret = credential.secret.trim().to_string();
        match credential.kind {
            CredentialKind::ApiKey => self.sdk.clone().with_api_key(Some(secret)),
            CredentialKind::Session | CredentialKind::Local => {
                self.sdk.clone().with_token(Some(secret))
            }
        }
    }

    /// Exchange a one-time login token for a session JWT
    /// (`POST /auth/login-token/consume`).
    pub async fn consume_login_token(
        &self,
        login_token: &str,
    ) -> Result<String, SessionClientError> {
        let token = login_token.trim();
        if token.is_empty() {
            return Err(SessionClientError::EmptyLoginToken);
        }
        log::debug!("{LOG_PREFIX} consuming login token on {}", self.base);
        let response = self
            .sdk
            .auth()
            .consume_login_token(&LoginTokenRequest {
                token: token.to_string(),
            })
            .await
            .map_err(|e| SessionClientError::ConsumeFailed(format!("{e:#}")))?;
        let jwt = response
            .get("jwt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if jwt.is_empty() {
            return Err(SessionClientError::MissingJwt);
        }
        Ok(jwt)
    }

    /// `GET /auth/me` with `credential`. The SDK unwraps the `{success,data}`
    /// envelope; a nested `user` object is unwrapped here the way the core's
    /// `parse_api_response_value` always did.
    pub async fn fetch_me(&self, credential: &Credential) -> Result<Value, FetchMeError> {
        let value = self
            .sdk_with(credential)
            .auth()
            .me()
            .await
            .map_err(FetchMeError::from_sdk)?;
        let value: Value = value.0;
        let user = match value.as_object().and_then(|obj| obj.get("user")) {
            Some(user) if !user.is_null() => user.clone(),
            _ => value,
        };
        log::debug!("{LOG_PREFIX} GET /auth/me ok on {}", self.base);
        Ok(user)
    }

    /// Store-time validation: `fetch_me` bounded by [`validation_budget`],
    /// retried once after a transient failure. A budget overrun is reported as
    /// [`FetchMeError::Transport`] so the caller can take the deferred path.
    pub async fn validate_for_store(&self, credential: &Credential) -> Result<Value, FetchMeError> {
        let budget = validation_budget();
        match tokio::time::timeout(budget, self.validate_once_with_retry(credential)).await {
            Ok(result) => result,
            Err(_elapsed) => {
                let reason = format!(
                    "GET /auth/me validation timeout after {}ms (store-time budget exceeded)",
                    budget.as_millis()
                );
                log::warn!("{LOG_PREFIX} {reason}");
                Err(FetchMeError::Transport(reason))
            }
        }
    }

    async fn validate_once_with_retry(
        &self,
        credential: &Credential,
    ) -> Result<Value, FetchMeError> {
        match self.fetch_me(credential).await {
            Ok(user) => Ok(user),
            Err(first) if first.is_availability_failure() => {
                log::debug!(
                    "{LOG_PREFIX} retrying GET /auth/me after transient failure: {}",
                    first.message()
                );
                tokio::time::sleep(VALIDATION_RETRY_DELAY).await;
                self.fetch_me(credential).await
            }
            Err(other) => Err(other),
        }
    }
}

/// The store-time validation budget: `OPENHUMAN_AUTH_ME_TIMEOUT_MS`, else the
/// legacy `OPENHUMAN_AUTH_ME_STORE_TIMEOUT_MS`, else [`VALIDATION_BUDGET`].
pub fn validation_budget() -> Duration {
    let parse = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis)
    };
    if let Some(d) = parse(VALIDATION_BUDGET_ENV) {
        return d;
    }
    if let Some(d) = parse(LEGACY_VALIDATION_BUDGET_ENV) {
        log::debug!(
            "{LOG_PREFIX} {LEGACY_VALIDATION_BUDGET_ENV} is deprecated; use {VALIDATION_BUDGET_ENV}"
        );
        return d;
    }
    VALIDATION_BUDGET
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
