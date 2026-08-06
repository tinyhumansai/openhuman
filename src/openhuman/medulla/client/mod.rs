//! HTTP/SSE client for the Medulla orchestration backend.
//!
//! Surfaces: auth (`/auth`), durable sessions (`/medulla/v1`), SSE event
//! streaming, one-shot orchestration (`/orchestration/v1`), and the public
//! feedback board (`/feedback`, in [`feedback`]).
//!
//! Every response is wrapped in a `{ "success": true, "data": ... }` envelope;
//! errors arrive as `{ "success": false, "error": ..., "errorCode": ... }` and
//! are surfaced as [`ClientError::Api`], preserving the `errorCode`.

mod account;
pub mod error;
pub mod feedback;
mod orchestration;
#[cfg(test)]
pub(crate) use orchestration::parse_run_result;
mod program;
mod routing;
mod sessions;
pub mod sse;
pub mod types;

pub use error::{ClientError, Result};
pub use feedback::{
    FeedbackComment, FeedbackDetail, FeedbackGithub, FeedbackItem, FeedbackPage, FeedbackQuery,
    FeedbackSort, FeedbackStatus, FeedbackSubmission, FeedbackType,
};
pub use program::*;
pub use routing::RoutingStrategy;
pub use types::*;

use serde::de::DeserializeOwned;
use serde_json::Value;

/// Default backend base URL.
pub const DEFAULT_BASE_URL: &str = "http://localhost:5000";

impl MedullaClientBuilder {
    /// Set the backend base URL (default [`DEFAULT_BASE_URL`]).
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Set the bearer JWT sent with every request.
    pub fn jwt(mut self, jwt: impl Into<String>) -> Self {
        self.jwt = Some(jwt.into());
        self
    }

    /// Supply a preconfigured `reqwest::Client`.
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Build the client.
    pub fn build(self) -> MedullaClient {
        let base_url = self
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        MedullaClient {
            base_url,
            jwt: self.jwt.unwrap_or_default(),
            http: self.http.unwrap_or_default(),
        }
    }
}

impl MedullaClient {
    /// Start building a client.
    pub fn builder() -> MedullaClientBuilder {
        MedullaClientBuilder::default()
    }

    /// Construct a client from a base URL and JWT.
    pub fn new(base_url: impl Into<String>, jwt: impl Into<String>) -> Self {
        Self::builder().base_url(base_url).jwt(jwt).build()
    }

    /// The configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The configured JWT.
    pub fn jwt(&self) -> &str {
        &self.jwt
    }

    /// Replace the JWT (e.g. after a token refresh).
    pub fn set_jwt(&mut self, jwt: impl Into<String>) {
        self.jwt = jwt.into();
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", self.jwt),
        )
    }

    /// Send a request and unwrap the `{success, data}` envelope into `T`.
    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> Result<T> {
        let resp = req.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        unwrap_envelope(status.as_u16(), &bytes)
    }
}

/// Unwrap a `{success, data}` envelope into `T`, mapping failures and non-2xx
/// responses into [`ClientError::Api`].
pub(crate) fn unwrap_envelope<T: DeserializeOwned>(status: u16, body: &[u8]) -> Result<T> {
    let env: RawEnvelope = match serde_json::from_slice(body) {
        Ok(env) => env,
        Err(e) => {
            // Body was not a recognizable envelope. If the HTTP status already
            // signals failure, report that; otherwise it's a decode error.
            if !(200..300).contains(&status) {
                return Err(ClientError::Api {
                    status: Some(status),
                    message: String::from_utf8_lossy(body).trim().to_string(),
                    error_code: None,
                    details: None,
                });
            }
            return Err(ClientError::Decode(e.to_string()));
        }
    };

    if env.success && (200..300).contains(&status) {
        let data = env.data.unwrap_or(Value::Null);
        serde_json::from_value(data).map_err(|e| ClientError::Decode(e.to_string()))
    } else {
        Err(ClientError::Api {
            status: Some(status),
            message: env
                .error
                .unwrap_or_else(|| format!("request failed with status {status}")),
            error_code: env.error_code,
            details: env.details,
        })
    }
}

/// Minimal percent-encoding for the JWT query parameter and for untrusted path
/// segments (ids interpolated into a URL). Encodes everything outside the
/// unreserved set, so a `/` in an id cannot escape its segment.
pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests;
use types::RawEnvelope;
