//! Ollama-based embedding provider.
//!
//! Calls the local Ollama server's `/api/embed` endpoint for embeddings.
//! Default model: `bge-m3` (1024 dimensions).

use async_trait::async_trait;

use crate::EmbeddingProvider;

/// Default Ollama base URL.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Default embedding model for Ollama.
pub const DEFAULT_OLLAMA_MODEL: &str = "bge-m3";

/// Default dimensions for `bge-m3`.
pub const DEFAULT_OLLAMA_DIMENSIONS: usize = 1024;

/// Embedding provider backed by a local Ollama instance.
#[derive(Debug)]
pub struct OllamaEmbedding {
    base_url: String,
    model: String,
    dims: usize,
    client: reqwest::Client,
}

impl OllamaEmbedding {
    /// Creates a new Ollama embedding provider.
    pub fn try_new(base_url: &str, model: &str, dims: usize) -> anyhow::Result<Self> {
        let base_url = Self::normalize_base_url(base_url)?;
        let model = Self::normalize_model(model)?;
        let dims = if dims == 0 { DEFAULT_OLLAMA_DIMENSIONS } else { dims };

        tracing::debug!(
            target: "embeddings.ollama",
            "[embeddings] OllamaEmbedding created: url={base_url}, model={model}, dims={dims}"
        );

        Ok(Self { base_url, model, dims, client: reqwest::Client::new() })
    }

    /// Creates a new Ollama embedding provider, panicking if invalid.
    pub fn new(base_url: &str, model: &str, dims: usize) -> Self {
        Self::try_new(base_url, model, dims).expect("invalid Ollama embedding configuration")
    }

    /// Set a custom HTTP client (e.g. with proxy configuration).
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str { &self.base_url }

    /// Returns the configured model name.
    pub fn model(&self) -> &str { &self.model }

    fn normalize_base_url(base_url: &str) -> anyhow::Result<String> {
        let raw = if base_url.trim().is_empty() { DEFAULT_OLLAMA_URL } else { base_url.trim() };
        let url = reqwest::Url::parse(raw)
            .map_err(|e| anyhow::anyhow!("invalid Ollama base_url `{raw}`: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            anyhow::bail!("invalid Ollama base_url `{raw}`: expected http:// or https://");
        }
        if !url.username().is_empty() || url.password().is_some() {
            anyhow::bail!("invalid Ollama base_url `{raw}`: no credentials in URL");
        }
        if url.query().is_some() || url.fragment().is_some() {
            anyhow::bail!("invalid Ollama base_url `{raw}`: query/fragment not supported");
        }
        let segments: Vec<String> = url
            .path_segments()
            .map(|parts| parts.filter(|p| !p.is_empty()).map(|p| p.to_ascii_lowercase()).collect())
            .unwrap_or_default();
        let has_api_suffix = segments.iter().any(|p| p == "api" || p == "v1");
        let is_chat = segments.len() >= 2
            && segments[segments.len() - 2] == "chat"
            && segments[segments.len() - 1] == "completions";
        if has_api_suffix || is_chat {
            anyhow::bail!(
                "invalid Ollama base_url `{raw}`: use the server root (e.g. {DEFAULT_OLLAMA_URL})"
            );
        }
        Ok(url.as_str().trim_end_matches('/').to_string())
    }

    fn normalize_model(model: &str) -> anyhow::Result<String> {
        let model = if model.trim().is_empty() { DEFAULT_OLLAMA_MODEL.to_string() } else { model.trim().to_string() };
        if model.to_ascii_lowercase().starts_with("local-") {
            anyhow::bail!("invalid Ollama embedding model `{model}`: `local-*` IDs are routing aliases");
        }
        Ok(model)
    }

    fn embed_url(&self) -> anyhow::Result<String> {
        let _ = reqwest::Url::parse(&self.base_url)
            .map_err(|e| anyhow::anyhow!("invalid Ollama base_url `{}`: {e}", self.base_url))?;
        Ok(format!("{}/api/embed", self.base_url))
    }

    async fn embed_one_with_nan_recovery(&self, text: &str) -> anyhow::Result<Option<Vec<f32>>> {
        let resp = self.client
            .post(self.embed_url()?)
            .json(&EmbedReq { model: self.model.clone(), input: vec![text.to_string()] })
            .send().await
            .map_err(|e| anyhow::anyhow!("ollama embed failed (running at {}?): {e}", self.base_url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 500 && is_nan_encode_error(&body) {
                tracing::warn!(target: "embeddings.ollama", "ollama NaN for single text (model={})", self.model);
                return Ok(None);
            }
            anyhow::bail!("ollama embed failed: {status}: {}", body.trim());
        }

        let payload: EmbedResp = resp.json().await?;
        if payload.embeddings.len() != 1 {
            anyhow::bail!("ollama count mismatch: sent 1, got {}", payload.embeddings.len());
        }
        let v = payload.embeddings.into_iter().next().unwrap();
        if v.len() != self.dims {
            anyhow::bail!("ollama dims mismatch: expected {}, got {}", self.dims, v.len());
        }
        Ok(Some(v))
    }

    async fn embed_per_text_fallback(&self, total_len: usize, live: &[(usize, String)]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut result = vec![Vec::new(); total_len];
        let mut nan_count = 0usize;
        for (idx, text) in live {
            match self.embed_one_with_nan_recovery(text).await? {
                Some(v) => result[*idx] = v,
                None => nan_count += 1,
            }
        }
        tracing::warn!(target: "embeddings.ollama", "per-text fallback: {nan_count}/{} NaN substituted", live.len());
        Ok(result)
    }
}

impl Default for OllamaEmbedding {
    fn default() -> Self {
        Self::try_new(DEFAULT_OLLAMA_URL, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_DIMENSIONS)
            .expect("default Ollama config must be valid")
    }
}

#[derive(serde::Serialize)]
struct EmbedReq { model: String, input: Vec<String> }

#[derive(serde::Deserialize)]
struct EmbedResp { #[serde(default)] embeddings: Vec<Vec<f32>> }

fn is_nan_encode_error(body: &str) -> bool {
    body.to_ascii_lowercase().contains("unsupported value: nan")
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedding {
    fn name(&self) -> &str { "ollama" }
    fn model_id(&self) -> &str { &self.model }
    fn dimensions(&self) -> usize { self.dims }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() { return Ok(Vec::new()); }

        let live: Vec<(usize, String)> = texts.iter().enumerate()
            .filter_map(|(i, t)| { let s = t.trim().to_string(); if s.is_empty() { None } else { Some((i, s)) } })
            .collect();

        if live.is_empty() { return Ok(vec![Vec::new(); texts.len()]); }

        let input: Vec<String> = live.iter().map(|(_, t)| t.clone()).collect();
        tracing::debug!(target: "embeddings.ollama", "sending {} texts to ollama model={}", input.len(), self.model);

        let resp = self.client
            .post(self.embed_url()?)
            .json(&EmbedReq { model: self.model.clone(), input: input.clone() })
            .send().await
            .map_err(|e| {
                tracing::warn!(target: "embeddings", model = %self.model, "ollama transport error: {e}");
                anyhow::anyhow!("ollama embed failed (running at {}?): {e}", self.base_url)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let message = format!("ollama embed failed: {status}: {}", body.trim());

            if status.as_u16() == 500 && is_nan_encode_error(&body) {
                tracing::warn!(target: "embeddings.ollama", "NaN batch error (model={}), per-text fallback", self.model);
                if live.len() == 1 { return Ok(vec![Vec::new(); texts.len()]); }
                return self.embed_per_text_fallback(texts.len(), &live).await;
            }

            tracing::warn!(target: "embeddings", model = %self.model, status = %status.as_u16(), "{message}");
            anyhow::bail!(message);
        }

        let payload: EmbedResp = resp.json().await
            .map_err(|e| anyhow::anyhow!("ollama response parse failed: {e}"))?;

        if payload.embeddings.len() != input.len() {
            anyhow::bail!("ollama count mismatch: sent {}, got {}", input.len(), payload.embeddings.len());
        }
        for (i, vec) in payload.embeddings.iter().enumerate() {
            if vec.len() != self.dims {
                anyhow::bail!("ollama dims mismatch at {i}: expected {}, got {}", self.dims, vec.len());
            }
        }

        let mut result = vec![Vec::new(); texts.len()];
        for ((idx, _), emb) in live.iter().zip(payload.embeddings.into_iter()) {
            result[*idx] = emb;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_valid() {
        assert_eq!(OllamaEmbedding::normalize_base_url("http://localhost:11434").unwrap(), "http://localhost:11434");
    }

    #[test]
    fn normalize_url_rejects_api() {
        assert!(OllamaEmbedding::normalize_base_url("http://localhost:11434/api").is_err());
    }

    #[test]
    fn default_works() {
        let e = OllamaEmbedding::default();
        assert_eq!(e.model(), DEFAULT_OLLAMA_MODEL);
    }
}
