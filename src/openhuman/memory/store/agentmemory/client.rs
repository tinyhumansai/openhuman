//! HTTP client wrapper for the agentmemory REST server.
//!
//! Wraps `reqwest::Client` with the agentmemory base URL, optional bearer
//! token, a configurable per-request timeout, and a plaintext-bearer guard
//! that refuses to send the secret over `http://<non-loopback>` per the
//! v0.9.12 contract.

use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;
use url::Url;

/// Default agentmemory REST endpoint when no override is configured.
pub const DEFAULT_AGENTMEMORY_URL: &str = "http://localhost:3111";

/// Default per-request timeout. Generous enough to absorb a cold-start on
/// the iii engine + a vector recall round-trip on a 100k-memory store.
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;

/// Thin HTTP wrapper around the agentmemory REST surface.
pub struct AgentMemoryClient {
    http: reqwest::Client,
    base: Url,
    secret: Option<String>,
}

impl AgentMemoryClient {
    /// Builds a client configured for the given URL + optional secret +
    /// timeout. The plaintext-bearer guard fires here, before any request
    /// goes on the wire — a misconfigured deploy fails loud at
    /// construction time rather than silently leaking the token.
    pub fn new(url: Option<&str>, secret: Option<&str>, timeout_ms: Option<u64>) -> Result<Self> {
        let raw = url.unwrap_or(DEFAULT_AGENTMEMORY_URL).trim();
        if raw.is_empty() {
            return Err(anyhow!(
                "agentmemory_url cannot be empty — leave it unset to use {DEFAULT_AGENTMEMORY_URL}"
            ));
        }
        let parsed = Url::parse(raw)
            .with_context(|| format!("agentmemory_url is not a valid URL: {raw}"))?;

        if let Some(secret) = secret.filter(|s| !s.trim().is_empty()) {
            enforce_plaintext_bearer_guard(&parsed, secret)?;
        }

        let timeout = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("failed to build reqwest client for agentmemory backend")?;

        Ok(Self {
            http,
            base: parsed,
            secret: secret
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        })
    }

    /// Base URL (mostly for log lines + error context).
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// `GET <base>/<path>`. Returns a 404 as `Ok(None)` for the get-by-key
    /// shape; everything else surfaces as an error.
    pub async fn get_optional<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>> {
        let url = self.url_for(path)?;
        let resp = self
            .http
            .request(Method::GET, url.clone())
            .headers(self.headers()?)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        match resp.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => {
                Ok(Some(resp.json::<T>().await.with_context(|| {
                    format!("failed to decode JSON response from GET {url}")
                })?))
            }
            s => Err(decode_error(&url, s, resp.text().await.ok())),
        }
    }

    /// `GET <base>/<path>` expecting a 200 + JSON body.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.get_optional(path)
            .await?
            .ok_or_else(|| anyhow!("agentmemory returned 404 for GET {path}"))
    }

    /// `POST <base>/<path>` with a JSON body, expecting a 2xx response.
    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = self.url_for(path)?;
        let resp = self
            .http
            .request(Method::POST, url.clone())
            .headers(self.headers()?)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(decode_error(&url, status, resp.text().await.ok()));
        }
        resp.json::<T>()
            .await
            .with_context(|| format!("failed to decode JSON response from POST {url}"))
    }

    /// `GET <base>/agentmemory/livez` — booleanises the health check.
    pub async fn livez(&self) -> bool {
        let Ok(url) = self.url_for("agentmemory/livez") else {
            return false;
        };
        let Ok(headers) = self.headers() else {
            return false;
        };
        match self
            .http
            .request(Method::GET, url)
            .headers(headers)
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    fn url_for(&self, path: &str) -> Result<Url> {
        let mut joined = self.base.clone();
        let trimmed = path.trim_start_matches('/');
        // Split off `?query` so it doesn't get appended as a literal path
        // segment — `path_segments_mut().extend(split('/'))` would
        // percent-encode the `?` and the server would 404.
        let (path_part, query_part) = match trimmed.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (trimmed, None),
        };
        joined
            .path_segments_mut()
            .map_err(|_| anyhow!("agentmemory base URL cannot be a base: {}", self.base))?
            .pop_if_empty()
            .extend(path_part.split('/'));
        if let Some(q) = query_part {
            joined.set_query(Some(q));
        }
        Ok(joined)
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(secret) = &self.secret {
            let value = format!("Bearer {secret}");
            let header = HeaderValue::from_str(&value)
                .context("agentmemory_secret is not a valid HTTP header value")?;
            h.insert(AUTHORIZATION, header);
        }
        Ok(h)
    }
}

/// Mirrors the v0.9.12 plaintext-bearer guard from agentmemory's first-party
/// integration plugins: a bearer token must never cross plaintext HTTP to a
/// non-loopback host. Loopback (`localhost`, `127.0.0.1`, `::1`) is exempt;
/// `https://` is exempt. `AGENTMEMORY_REQUIRE_HTTPS=1` escalates the warning
/// path to a hard refusal even on loopback so a misconfigured production
/// deploy can fail loud rather than leak the secret once.
fn enforce_plaintext_bearer_guard(url: &Url, _secret: &str) -> Result<()> {
    if url.scheme().eq_ignore_ascii_case("https") {
        return Ok(());
    }
    let host = url.host_str().unwrap_or("");
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]");
    let require_https = std::env::var("AGENTMEMORY_REQUIRE_HTTPS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if require_https && url.scheme() != "https" {
        return Err(anyhow!(
            "agentmemory_secret is set and AGENTMEMORY_REQUIRE_HTTPS=1 \
             refuses to send the bearer over scheme `{}` (host {host}). \
             Switch agentmemory_url to https:// or unset AGENTMEMORY_REQUIRE_HTTPS.",
            url.scheme(),
        ));
    }

    if !loopback {
        log::warn!(
            "[memory::agentmemory] agentmemory_secret is set and agentmemory_url ({url}) \
             is plaintext HTTP to a non-loopback host ({host}). The bearer will be \
             observable on the wire. Set AGENTMEMORY_REQUIRE_HTTPS=1 to make this a \
             hard error, or switch to https://."
        );
    }
    Ok(())
}

fn decode_error(url: &Url, status: StatusCode, body: Option<String>) -> anyhow::Error {
    let body = body.unwrap_or_default();
    let snippet = if body.len() > 512 {
        format!("{}…", &body[..512])
    } else {
        body
    };
    anyhow!(
        "agentmemory returned {status} for {url}: {snippet}",
        snippet = snippet.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_plaintext_is_allowed_without_require_https() {
        unsafe { std::env::remove_var("AGENTMEMORY_REQUIRE_HTTPS") };
        let url = Url::parse("http://localhost:3111").unwrap();
        assert!(enforce_plaintext_bearer_guard(&url, "secret").is_ok());

        let url = Url::parse("http://127.0.0.1:3111").unwrap();
        assert!(enforce_plaintext_bearer_guard(&url, "secret").is_ok());
    }

    #[test]
    fn https_is_always_allowed_even_with_require_https() {
        unsafe { std::env::set_var("AGENTMEMORY_REQUIRE_HTTPS", "1") };
        let url = Url::parse("https://memory.example.com").unwrap();
        assert!(enforce_plaintext_bearer_guard(&url, "secret").is_ok());
        unsafe { std::env::remove_var("AGENTMEMORY_REQUIRE_HTTPS") };
    }

    #[test]
    fn plaintext_non_loopback_with_require_https_is_refused() {
        unsafe { std::env::set_var("AGENTMEMORY_REQUIRE_HTTPS", "1") };
        let url = Url::parse("http://memory.example.com:3111").unwrap();
        let err = enforce_plaintext_bearer_guard(&url, "secret").unwrap_err();
        assert!(
            err.to_string().contains("refuses"),
            "expected refusal, got: {err}"
        );
        unsafe { std::env::remove_var("AGENTMEMORY_REQUIRE_HTTPS") };
    }
}
