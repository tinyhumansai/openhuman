//! OpenAI-compatible embedding provider.
//!
//! Works with OpenAI, LocalAI, Ollama, and any endpoint that implements the
//! `POST /v1/embeddings` contract.

use async_trait::async_trait;

use super::retry_after::{backoff_ms_for_attempt, MAX_429_RETRIES};
use super::EmbeddingProvider;

/// Embedding provider for OpenAI and compatible APIs (e.g., LocalAI, Ollama).
pub struct OpenAiEmbedding {
    base_url: String,
    api_key: String,
    model: String,
    dims: usize,
    /// When true, send `"dimensions": dims` in the request body. OpenAI's
    /// `text-embedding-3-*` models honour this (Matryoshka — e.g. 3-large can
    /// return 1024 instead of its native 3072). Off by default so providers
    /// that don't accept the field — Voyage (uses `output_dimension`), Cohere,
    /// LocalAI/Ollama — keep working unchanged. Set via
    /// [`Self::with_send_dimensions`] for the OpenAI / custom-OpenAI paths.
    send_dimensions: bool,
    /// When true, this provider points at a hosted cloud endpoint that always
    /// requires a bearer token (genuine OpenAI `api.openai.com`, Voyage), so an
    /// empty `api_key` must fail fast instead of POSTing an unauthenticated
    /// request. Off by default so the OpenAI-compatible provider keeps serving
    /// keyless local/custom endpoints (LocalAI, Ollama-via-OpenAI). Set via
    /// [`Self::with_required_api_key`]. See the guard in [`Self::embed`].
    requires_api_key: bool,
}

impl OpenAiEmbedding {
    /// Creates a new OpenAI-style provider.
    pub fn new(base_url: &str, api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dims,
            send_dimensions: false,
            requires_api_key: false,
        }
    }

    /// Opt into sending the OpenAI `dimensions` request parameter so a
    /// reducible model (`text-embedding-3-large` / `-3-small`) returns exactly
    /// `dims` floats instead of its native size. Only call this for genuine
    /// OpenAI / OpenAI-compatible endpoints that implement the parameter —
    /// see [`Self::send_dimensions`]. Returns `self` for builder chaining.
    pub fn with_send_dimensions(mut self, send: bool) -> Self {
        self.send_dimensions = send;
        self
    }

    /// Mark this provider as a keyed cloud endpoint that must have an API key.
    /// When set, [`Self::embed`] fails fast (before any HTTP round-trip) if the
    /// resolved `api_key` is empty, instead of silently omitting the
    /// `Authorization` header. Use for genuine OpenAI (`api.openai.com`) and
    /// Voyage; leave off for keyless local/custom OpenAI-compatible endpoints.
    /// Returns `self` for builder chaining.
    pub fn with_required_api_key(mut self, required: bool) -> Self {
        self.requires_api_key = required;
        self
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the configured model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Internal helper to build an HTTP client with proxy support.
    fn http_client(&self) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client("memory.embeddings")
    }

    /// Checks if the base URL includes a specific path (e.g., /api/v1).
    fn has_explicit_api_path(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        let path = url.path().trim_end_matches('/');
        !path.is_empty() && path != "/"
    }

    /// Checks if the URL already ends with /embeddings.
    fn has_embeddings_endpoint(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        url.path().trim_end_matches('/').ends_with("/embeddings")
    }

    /// Constructs the final URL for the embeddings endpoint.
    pub fn embeddings_url(&self) -> String {
        if self.has_embeddings_endpoint() {
            return self.base_url.clone();
        }

        if self.has_explicit_api_path() {
            format!("{}/embeddings", self.base_url)
        } else {
            format!("{}/v1/embeddings", self.base_url)
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbedding {
    fn name(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    /// Sends a POST request to the embedding API.
    ///
    /// On 429 (Too Many Requests) or 503 (Service Unavailable) the call is
    /// retried up to `MAX_429_RETRIES` times with exponential backoff.  When
    /// the server supplies a `Retry-After` header its value (delta-seconds) is
    /// preferred over the computed backoff.  After all retries are exhausted the
    /// canonical error message is returned so the `TransientUpstreamHttp`
    /// classifier in `core::observability` demotes it to a warning breadcrumb
    /// instead of a Sentry error event.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Pre-flight: empty / whitespace-only entries are guaranteed 400s from
        // the upstream (OpenAI: `"input must be a non-empty string"`; OpenHuman
        // cloud backend: `"input must be a non-empty string or array of
        // non-empty strings"`). Bailing here keeps the round-trip and quota
        // out of the picture and — crucially — bypasses the `report_error_or_
        // expected` Sentry route below, so a caller passing an empty summary
        // stops manifesting as a server fault (#13021).
        if let Some(idx) = texts.iter().position(|t| t.trim().is_empty()) {
            tracing::warn!(
                target: "openai::embed",
                "[openai] refusing embed: input[{idx}] is empty/whitespace \
                 (count={}, model={}). Caller must filter empty strings.",
                texts.len(),
                self.model,
            );
            anyhow::bail!(
                "openai embed: refusing empty/whitespace input at index {idx} of {} (model={})",
                texts.len(),
                self.model,
            );
        }

        // Fast-fail when this is a keyed cloud provider (OpenAI / Voyage) but the
        // resolved key is empty. The key collapses to "" when the stored BYO
        // credential can't be read — the OS-keychain consent is `none`/declined
        // (`cached_consent=none`) or the cred fails to decrypt — because
        // `resolve_api_key` swallows every such failure into "". Without this
        // guard the request goes out with NO `Authorization` header at all and
        // OpenAI 401s "You didn't provide an API key" on every embed; the memory
        // pipeline re-embeds per document and floods Sentry (TAURI-RUST-4TZ:
        // 3.9k events). Bailing here skips the wasted request, and the "API key
        // not set" wording is demoted by the `ApiKeyMissing` classifier in
        // `core::observability` to a single low-cardinality breadcrumb. The
        // remediation surfaces the keychain-consent / re-enter-key path so the
        // stored key can actually be read. Scoped via `requires_api_key`: the
        // OpenAI-compatible provider legitimately supports keyless local/custom
        // endpoints (LocalAI, Ollama-via-OpenAI), which keep omitting the header
        // rather than bailing — mirroring the Cohere guard (TAURI-RUST-52S).
        if self.requires_api_key && self.api_key.trim().is_empty() {
            let message = format!(
                "Embedding API key not set (model={}) — re-enter your key or grant \
                 keychain access in Settings → Memory",
                self.model,
            );
            crate::core::observability::report_error_or_expected(
                message.as_str(),
                "embeddings",
                "openai_embed",
                &[("model", self.model.as_str()), ("failure", "missing_key")],
            );
            anyhow::bail!(message);
        }

        let url = self.embeddings_url();

        tracing::debug!(
            target: "openai::embed",
            "[openai] embed: model={}, count={}, url={}",
            self.model, texts.len(), url
        );

        let mut body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });
        // Request a specific output size on OpenAI 3-* models (Matryoshka) so
        // the vector matches `dims` (e.g. 3-large → 1024 for the memory tree's
        // fixed EMBEDDING_DIM). Gated by `send_dimensions` because Voyage /
        // Cohere / LocalAI don't accept this exact field.
        if self.send_dimensions && self.dims > 0 {
            body["dimensions"] = serde_json::json!(self.dims);
        }

        // Retry loop: handles 429 Too Many Requests and 503 Service Unavailable
        // with Retry-After–aware exponential backoff.
        for attempt in 0..=MAX_429_RETRIES {
            let mut req = self
                .http_client()
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);

            // Only set Authorization header when an API key is configured.
            if !self.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", self.api_key));
            }

            // Proactively gate every outbound attempt (initial + retries) against
            // the per-endpoint rate budget so cloud backends (OpenHuman/Voyage,
            // OpenAI, custom remote endpoints) stay under their account quota
            // instead of tripping 429s. The chokepoint must sit inside the loop:
            // a single pre-loop acquire would let retried 429/503 attempts bypass
            // token consumption and let concurrent callers blow past the cap,
            // ironically triggering more 429s. Token consumption tracks the number
            // of HTTP attempts (1 + retries actually executed). Loopback endpoints
            // are exempt (see `rate_limit`).
            super::rate_limit::acquire_embedding_slot(&self.base_url).await;

            let resp = req.send().await?;

            let status = resp.status();

            // Retry on 429 and 503 — both can carry a Retry-After header.
            let is_retryable = status.as_u16() == 429 || status.as_u16() == 503;

            if is_retryable && attempt < MAX_429_RETRIES {
                // Read Retry-After before consuming the body.
                let retry_after_val = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_owned());

                let body_text = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    target: "openai::embed",
                    "[embeddings] openai {} body on retry: {body_text}",
                    status.as_u16()
                );

                let delay_ms = backoff_ms_for_attempt(attempt, retry_after_val.as_deref());

                tracing::debug!(
                    target: "openai::embed",
                    "[embeddings] openai {}, retrying in {}ms (attempt {}/{})",
                    status.as_u16(), delay_ms, attempt + 1, MAX_429_RETRIES
                );

                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            if !status.is_success() {
                let status_str = status.as_u16().to_string();
                let text = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    target: "openai::embed",
                    "[openai] embed error: status={status}, body={text}"
                );
                let mut message = format!("Embedding API error ({status}): {text}");
                // A 404/405 means the base URL responded but exposes no
                // embeddings route — the user pointed the Custom
                // (OpenAI-compatible) provider at a chat-only endpoint (e.g.
                // DeepSeek). Append an actionable remediation while PRESERVING
                // the `Embedding API error (404…)` prefix that
                // `observability::is_embedding_endpoint_absent` keys on, so the
                // event is still demoted from Sentry. Host-agnostic text (no
                // URL/credential echo). TAURI-RUST-5JR.
                if matches!(status.as_u16(), 404 | 405) {
                    message.push_str(
                        " — this endpoint has no embeddings API; pick an \
                         embeddings-capable provider in Settings → Memory",
                    );
                }
                // A 400 "… does not exist" / "does not support embeddings" body
                // means the endpoint IS an embeddings API but the configured
                // model id is not an embeddings model — the user pasted a chat
                // model (e.g. an OpenRouter `…:free` id) into the embeddings
                // model field. Append an actionable remediation while PRESERVING
                // the `(400` + body text that `observability::
                // is_embedding_model_rejected` keys on, so the event is demoted
                // from a per-embed Sentry flood to a breadcrumb. Scoped to the
                // model-rejection body shape so a genuine 400 (oversized input,
                // real server fault) still reaches Sentry. TAURI-RUST-9SK.
                else if status.as_u16() == 400
                    && (text.contains("does not exist")
                        || text.contains("does not support embeddings"))
                {
                    message.push_str(
                        " — this model isn't an embeddings model; pick an \
                         embeddings-capable model in Settings → Memory",
                    );
                }
                // Use `report_error_or_expected` so transient upstream HTTP
                // failures (e.g. 429 Too Many Requests after retry cap) log a
                // warning breadcrumb instead of firing a Sentry error event.
                crate::core::observability::report_error_or_expected(
                    message.as_str(),
                    "embeddings",
                    "openai_embed",
                    &[
                        ("model", self.model.as_str()),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
                anyhow::bail!(message);
            }

            let json: serde_json::Value = resp.json().await?;
            let data = json
                .get("data")
                .and_then(|d| d.as_array())
                .ok_or_else(|| anyhow::anyhow!("Invalid embedding response: missing 'data'"))?;

            // Validate that the response count matches the input count.
            if data.len() != texts.len() {
                anyhow::bail!(
                    "openai embed count mismatch: sent {} texts, got {} items in 'data'",
                    texts.len(),
                    data.len()
                );
            }

            let mut embeddings = Vec::with_capacity(data.len());
            for (i, item) in data.iter().enumerate() {
                let embedding = item
                    .get("embedding")
                    .and_then(|e| e.as_array())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Invalid embedding item at index {i}: missing 'embedding'")
                    })?;

                let mut vec = Vec::with_capacity(embedding.len());
                for (j, v) in embedding.iter().enumerate() {
                    #[allow(clippy::cast_possible_truncation)]
                    let f = v.as_f64().ok_or_else(|| {
                        anyhow::anyhow!("non-numeric value at data[{i}].embedding[{j}]: {v}")
                    })? as f32;
                    vec.push(f);
                }

                // Validate dimensions.
                if self.dims > 0 && vec.len() != self.dims {
                    anyhow::bail!(
                        "openai embed dimension mismatch at index {i}: expected {}, got {}",
                        self.dims,
                        vec.len()
                    );
                }

                embeddings.push(vec);
            }

            tracing::debug!(
                target: "openai::embed",
                "[openai] embed success: model={}, count={}, dims={}",
                self.model, embeddings.len(),
                embeddings.first().map(|v| v.len()).unwrap_or(0)
            );

            return Ok(embeddings);
        }

        // The loop always exits via `return Ok(...)`, `bail!(...)`, or
        // `continue`; this point is structurally unreachable.  On the final
        // attempt (`attempt == MAX_429_RETRIES`) the retryable guard is false
        // and execution falls into the non-2xx branch above, which bails with
        // the body-bearing format "Embedding API error (429 ...): <body>" —
        // that format preserves the "(429 " substring required by the
        // TransientUpstreamHttp classifier in core::observability.
        unreachable!("embed retry loop must exit via return or bail")
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
