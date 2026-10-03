//! [`EmbeddingProvider`] — text → vector.
//!
//! Consumers (tool discovery ranking, voice, the embeddings RPC) take an
//! `Arc<dyn EmbeddingProvider>` and never decide which provider, credentials,
//! rate limit or fallback back it: that is the factory's policy.

use async_trait::async_trait;

/// Formats the canonical embedding-space signature string.
///
/// This is the **single source of truth** for the signature format. Both the
/// live-provider [`EmbeddingProvider::signature`] and any config-derived
/// signature must route through here, so a signature computed from
/// configuration is byte-identical to one computed from an instantiated
/// provider. Drift between the two silently splits one embedding space into
/// two, and every vector written on the wrong side of the split becomes
/// unsearchable without a re-embed.
/// # Delimiters in a component
///
/// A component containing `;`, `=` or `%` is percent-encoded, because without
/// that the format is ambiguous: `("a;model=b", "c")` and `("a", "b;model=c")`
/// are different embedding spaces that would otherwise produce one identical
/// key, and vectors from both would then be compared as though they came from
/// the same model.
///
/// Encoding only those three characters is what keeps this from being a
/// migration. Every provider and model identifier actually in use is
/// alphanumeric plus `-`, `_`, `.`, `/` or `:`, and each of those passes
/// through untouched — so every signature already on disk still formats to the
/// same bytes. Only a name that could have collided changes, and such a name
/// has never been written.
#[must_use]
pub fn format_embedding_signature(name: &str, model_id: &str, dims: usize) -> String {
    let name = escape_component(name);
    let model_id = escape_component(model_id);
    format!("provider={name};model={model_id};dims={dims}")
}

/// Percent-encode the three characters that carry structure in a signature.
///
/// `%` goes first and must: encoding it afterwards would re-encode the `%` this
/// function just introduced, and `a;b` would arrive as `a%3Bb` from one path
/// and `a%253Bb` from another.
fn escape_component(value: &str) -> String {
    if !value.contains(['%', ';', '=']) {
        // The overwhelmingly common path, and the one that guarantees existing
        // keys are untouched: no allocation beyond the copy, no rewriting.
        return value.to_string();
    }
    value
        .replace('%', "%25")
        .replace(';', "%3B")
        .replace('=', "%3D")
}

#[cfg(test)]
#[path = "embeddings_embedding_signature_tests.rs"]
mod embedding_signature_tests;

/// Converts text into numerical vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Provider name, e.g. `"ollama"`, `"openai"`.
    fn name(&self) -> &str;

    /// Stable model identifier used to generate embeddings.
    fn model_id(&self) -> &str;

    /// Number of dimensions in the generated embeddings.
    fn dimensions(&self) -> usize;

    /// Stable signature for the embedding space.
    ///
    /// Changing any component means existing vectors are no longer comparable
    /// with newly generated ones and must be stored and queried separately
    /// until a migration re-embeds them.
    fn signature(&self) -> String {
        format_embedding_signature(self.name(), self.model_id(), self.dimensions())
    }

    /// Generates embeddings for a batch of strings.
    ///
    /// # Errors
    /// Propagates transport, authentication and quota failures from the
    /// underlying provider.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;

    /// Generates an embedding for a single string.
    ///
    /// # Errors
    /// As [`Self::embed`], plus an error when the provider returns no vector.
    async fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }
}

/// The inert provider bound when semantic search is switched off or no
/// embedding backend is configured. Reports zero dimensions and returns one
/// empty vector per input, so keyword-only retrieval keeps working while
/// vector rerank degrades to a no-op rather than an error.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopEmbedding;

#[async_trait]
impl EmbeddingProvider for NoopEmbedding {
    fn name(&self) -> &str {
        "none"
    }

    fn model_id(&self) -> &str {
        "none"
    }

    fn dimensions(&self) -> usize {
        0
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(vec![Vec::new(); texts.len()])
    }
}
