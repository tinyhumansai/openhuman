//! Phase 4 embedding layer (#710).
//!
//! Produces a fixed-dimension vector per chunk / summary so retrieval can
//! rerank candidates by semantic similarity. Phase 4's default backend is a
//! local [Ollama](https://ollama.com) endpoint running `bge-m3`;
//! tests use the deterministic [`InertEmbedder`] so no network is required.
//!
//! Dimension is hard-coded at [`EMBEDDING_DIM`] (1024) — matches the
//! bge-m3 output and keeps the blob layout on `mem_tree_chunks` /
//! `mem_tree_summaries` consistent across providers. Mixing dimensions
//! mid-run would corrupt cosine comparisons; we catch that at the trait
//! level rather than deferring to retrieval-time diagnostics.
//!
//! NOTE: bge-m3 replaces the prior `nomic-embed-text` (768-dim, 2048
//! token context). Migration was driven by nomic's hard 2048-token
//! context cap causing long-chunk embed failures (chunker estimates
//! undercount BERT-WordPiece tokens by ~1.5-2× for HTML-derived
//! markdown, so 1500 chunker-tokens routinely exceed nomic's cap).
//! bge-m3 has a native 8192-token context. Existing `embedding` blobs
//! from the 768-dim era are invalid against the new dimension and
//! must be wiped or re-embedded.
//!
//! Write-time semantics: ingest + seal call [`Embedder::embed`] **before**
//! persisting the new row, so a provider error cascades into "don't write
//! this row". Legacy rows from Phases 1-3 predate embeddings and read back
//! with `Option::None`; retrieval tolerates that by dropping legacy rows
//! to the bottom of a semantic rerank.

use anyhow::{Context, Result};
use async_trait::async_trait;

pub mod cloud;
pub mod factory;
pub mod inert;
pub mod ollama;
pub mod openai_compat;

pub use cloud::CloudEmbedder;
pub use factory::{build_embedder_from_config, build_write_embedder};
pub use inert::InertEmbedder;
pub use ollama::OllamaEmbedder;
pub use openai_compat::OpenAiCompatEmbedder;

/// Embedding dimensionality used across the memory tree.
///
/// Hard-coded to match `bge-m3`; swapping providers requires a matching
/// dimension or the trait's post-call validation will bail. Any change
/// to this constant breaks on-disk compatibility with existing
/// `mem_tree_chunks.embedding` / `mem_tree_summaries.embedding` blobs.
pub const EMBEDDING_DIM: usize = 1024;

/// Trait backing all Phase 4 embedders. Implementations MUST produce
/// exactly [`EMBEDDING_DIM`] floats per call — callers that persist the
/// result rely on the fixed layout.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Stable short name, used in debug logs and provider diagnostics.
    fn name(&self) -> &'static str;

    /// Embed one text. Must return a `Vec<f32>` of length
    /// [`EMBEDDING_DIM`]. Hard failure — ingest / seal treat `Err` as
    /// "don't persist the row" so retries stay idempotent on `chunk_id`.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed many texts, returning **one [`Result`] per input position**
    /// aligned by index. A single failing text does not strand the rest of
    /// the batch — its slot carries the `Err` while the others succeed —
    /// which lets bulk callers (e.g. the re-embed backfill) attribute and
    /// skip individual rows exactly as a per-text loop would.
    ///
    /// The default implementation issues one sequential [`Embedder::embed`]
    /// call per text: correct for any provider, but with no batching win.
    /// Providers whose backend accepts many texts in a single request
    /// (cloud / OpenAI-compatible) override this to collapse N network
    /// round-trips into one — see [`embed_batch_via_provider`].
    ///
    /// The returned vector always has `texts.len()` elements.
    async fn embed_batch(&self, texts: &[&str]) -> Vec<Result<Vec<f32>>> {
        log::debug!(
            "[memory_tree::embed::{}] embed_batch:enter sequential texts={}",
            self.name(),
            texts.len()
        );
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(self.embed(text).await);
        }
        out
    }
}

/// Validate that a freshly-produced embedding has exactly [`EMBEDDING_DIM`]
/// floats, returning a labelled error otherwise. Shared by the per-text and
/// batched provider adapters so the "wrong dims" diagnostic is identical
/// regardless of path.
pub(crate) fn check_embed_dim(v: Vec<f32>, label: &str) -> Result<Vec<f32>> {
    if v.len() != EMBEDDING_DIM {
        anyhow::bail!(
            "{label} embedder returned {} dims, expected {}",
            v.len(),
            EMBEDDING_DIM
        );
    }
    Ok(v)
}

/// Batch-embed `texts` through a unified [`EmbeddingProvider`] in a single
/// request and adapt the result to the [`Embedder::embed_batch`] contract:
/// one dimension-checked [`Result`] per input position.
///
/// On a wholesale batch failure (network / auth) **or** a length-contract
/// violation from the provider, this falls back to per-text
/// [`EmbeddingProvider::embed_one`] so a single transient blip cannot fail —
/// and, in the backfill, *tombstone* — every row in the batch. That preserves
/// the resilience of the original per-item loop while keeping the happy-path
/// single-round-trip win.
pub(crate) async fn embed_batch_via_provider(
    inner: &dyn crate::openhuman::embeddings::EmbeddingProvider,
    label: &str,
    texts: &[&str],
) -> Vec<Result<Vec<f32>>> {
    if texts.is_empty() {
        return Vec::new();
    }
    log::debug!(
        "[memory_tree::embed::{label}] embed_batch:enter texts={}",
        texts.len()
    );
    match inner.embed(texts).await {
        Ok(vectors) if vectors.len() == texts.len() => {
            log::debug!(
                "[memory_tree::embed::{label}] embed_batch:success collapsed {} texts into one provider call",
                texts.len()
            );
            vectors
                .into_iter()
                .map(|v| check_embed_dim(v, label))
                .collect()
        }
        Ok(vectors) => {
            log::warn!(
                "[memory_tree::embed::{label}] embed_batch:fallback batch returned {} vectors for {} texts; \
                 falling back to per-text embedding",
                vectors.len(),
                texts.len()
            );
            embed_each_via_provider(inner, label, texts).await
        }
        Err(e) => {
            log::warn!(
                "[memory_tree::embed::{label}] embed_batch:fallback batch embed failed ({e:#}); \
                 falling back to per-text embedding"
            );
            embed_each_via_provider(inner, label, texts).await
        }
    }
}

/// Sequential per-text fallback used when a provider's native batch call is
/// unavailable or fails wholesale. Each slot is dimension-checked so the
/// result is interchangeable with the happy-path mapping in
/// [`embed_batch_via_provider`].
async fn embed_each_via_provider(
    inner: &dyn crate::openhuman::embeddings::EmbeddingProvider,
    label: &str,
    texts: &[&str],
) -> Vec<Result<Vec<f32>>> {
    let mut out = Vec::with_capacity(texts.len());
    for text in texts {
        let result = inner
            .embed_one(text)
            .await
            .with_context(|| format!("{label} embeddings failed"))
            .and_then(|v| check_embed_dim(v, label));
        out.push(result);
    }
    out
}

/// Cosine similarity between two equal-length vectors.
///
/// Returns `0.0` when either vector has zero magnitude (including empty
/// vectors) to keep the rerank sort stable instead of surfacing `NaN`.
/// Length mismatch also returns `0.0` — callers upstream of the
/// comparison should normalise to [`EMBEDDING_DIM`] before calling.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Pack a `Vec<f32>` into little-endian bytes for SQLite BLOB storage.
///
/// Output length is `v.len() * 4`. The inverse is [`unpack_embedding`].
pub fn pack_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Unpack little-endian bytes into a `Vec<f32>`.
///
/// Errors when the byte length isn't a multiple of 4 or doesn't match
/// [`EMBEDDING_DIM`] (after decoding). The latter guards against rows
/// written with a mismatched-provider blob silently passing as valid.
pub fn unpack_embedding(b: &[u8]) -> Result<Vec<f32>> {
    if !b.len().is_multiple_of(4) {
        anyhow::bail!(
            "embedding blob length {} not a multiple of 4 — corrupt row",
            b.len()
        );
    }
    let floats: Vec<f32> = b
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if floats.len() != EMBEDDING_DIM {
        anyhow::bail!(
            "embedding blob length {} floats, expected {}",
            floats.len(),
            EMBEDDING_DIM
        );
    }
    Ok(floats)
}

/// Pack helper that also validates the input dimension before storing.
/// Used by write-time call sites where we want a loud error if a provider
/// misbehaves rather than writing a differently-shaped blob.
pub fn pack_checked(v: &[f32]) -> Result<Vec<u8>> {
    if v.len() != EMBEDDING_DIM {
        anyhow::bail!(
            "embedding vector has {} dims, expected {}",
            v.len(),
            EMBEDDING_DIM
        );
    }
    Ok(pack_embedding(v))
}

/// Decode a possibly-NULL embedding blob straight from a query row.
/// Returns `Ok(None)` for NULL (legacy rows predating Phase 4) and
/// surfaces decoding errors with context so the caller sees which row
/// was malformed.
pub fn decode_optional_blob(
    blob: Option<Vec<u8>>,
    context_label: &str,
) -> Result<Option<Vec<f32>>> {
    match blob {
        None => Ok(None),
        Some(bytes) => {
            let v = unpack_embedding(&bytes)
                .with_context(|| format!("decode embedding for {context_label}"))?;
            Ok(Some(v))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors_is_one() {
        let a = vec![0.1_f32, 0.2, 0.3, 0.4];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors_is_zero() {
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![0.0_f32, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_vectors_is_minus_one() {
        let a = vec![1.0_f32, 2.0, 3.0];
        let b = vec![-1.0_f32, -2.0, -3.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_returns_zero_not_nan() {
        let a = vec![0.0_f32; 4];
        let b = vec![1.0_f32, 2.0, 3.0, 4.0];
        let s = cosine_similarity(&a, &b);
        assert_eq!(s, 0.0, "expected 0.0, got {s}");
        assert!(!s.is_nan());
    }

    #[test]
    fn cosine_empty_returns_zero() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn cosine_length_mismatch_returns_zero() {
        let a = vec![1.0_f32, 2.0];
        let b = vec![1.0_f32, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn pack_unpack_round_trip() {
        let v: Vec<f32> = (0..EMBEDDING_DIM).map(|i| (i as f32) / 100.0).collect();
        let packed = pack_embedding(&v);
        assert_eq!(packed.len(), EMBEDDING_DIM * 4);
        let back = unpack_embedding(&packed).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn unpack_wrong_byte_count_errors() {
        let bad = vec![0u8, 0, 0]; // not multiple of 4
        assert!(unpack_embedding(&bad).is_err());
    }

    #[test]
    fn unpack_wrong_dim_errors() {
        // Correct byte multiple, but wrong float count.
        let bad = vec![0u8; 16]; // 4 floats, expected EMBEDDING_DIM (1024)
        let err = unpack_embedding(&bad).unwrap_err().to_string();
        assert!(
            err.contains(&format!("expected {EMBEDDING_DIM}")),
            "got {err}"
        );
    }

    #[test]
    fn pack_checked_rejects_wrong_dim() {
        let too_short = vec![0.0_f32; 5];
        assert!(pack_checked(&too_short).is_err());
        let correct = vec![0.0_f32; EMBEDDING_DIM];
        assert!(pack_checked(&correct).is_ok());
    }

    // --- batch-embedding (variant B) scaffolding + tests ---

    use crate::openhuman::embeddings::EmbeddingProvider;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn ok_vec() -> Vec<f32> {
        vec![0.5_f32; EMBEDDING_DIM]
    }

    #[derive(Clone)]
    enum ProviderMode {
        /// One correct-dim vector per text (single batch call succeeds).
        Ok,
        /// Batch (`len > 1`) call errors, per-text (`len == 1`) succeeds —
        /// exercises the whole-batch-error fallback path.
        BatchFailsPerTextOk,
        /// Batch (`len > 1`) returns one extra vector, per-text is fine —
        /// exercises the length-mismatch fallback path.
        WrongCount,
        /// Returns `len` vectors but the one at `idx` has the wrong dim —
        /// length matches so no fallback; that position must map to `Err`.
        OneWrongDim(usize),
    }

    struct FakeProvider {
        calls: Arc<AtomicUsize>,
        mode: ProviderMode,
    }

    #[async_trait::async_trait]
    impl EmbeddingProvider for FakeProvider {
        fn name(&self) -> &str {
            "fake"
        }
        fn model_id(&self) -> &str {
            "fake-model"
        }
        fn dimensions(&self) -> usize {
            EMBEDDING_DIM
        }
        async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                ProviderMode::Ok => Ok(texts.iter().map(|_| ok_vec()).collect()),
                ProviderMode::BatchFailsPerTextOk => {
                    if texts.len() > 1 {
                        anyhow::bail!("simulated batch endpoint failure")
                    } else {
                        Ok(texts.iter().map(|_| ok_vec()).collect())
                    }
                }
                ProviderMode::WrongCount => {
                    if texts.len() > 1 {
                        Ok((0..texts.len() + 1).map(|_| ok_vec()).collect())
                    } else {
                        Ok(texts.iter().map(|_| ok_vec()).collect())
                    }
                }
                ProviderMode::OneWrongDim(idx) => Ok(texts
                    .iter()
                    .enumerate()
                    .map(|(i, _)| if i == idx { vec![0.0_f32; 3] } else { ok_vec() })
                    .collect()),
            }
        }
    }

    #[tokio::test]
    async fn embed_batch_via_provider_happy_is_single_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let p = FakeProvider {
            calls: calls.clone(),
            mode: ProviderMode::Ok,
        };
        let out = embed_batch_via_provider(&p, "test", &["a", "b", "c"]).await;
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|r| r.is_ok()));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "happy path must collapse to exactly one batch call"
        );
    }

    #[tokio::test]
    async fn embed_batch_via_provider_empty_makes_no_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let p = FakeProvider {
            calls: calls.clone(),
            mode: ProviderMode::Ok,
        };
        let texts: [&str; 0] = [];
        let out = embed_batch_via_provider(&p, "test", &texts).await;
        assert!(out.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn embed_batch_via_provider_falls_back_on_batch_error() {
        let calls = Arc::new(AtomicUsize::new(0));
        let p = FakeProvider {
            calls: calls.clone(),
            mode: ProviderMode::BatchFailsPerTextOk,
        };
        let out = embed_batch_via_provider(&p, "test", &["a", "b", "c"]).await;
        assert_eq!(out.len(), 3);
        assert!(
            out.iter().all(|r| r.is_ok()),
            "per-text fallback should still produce all vectors"
        );
        // 1 failed batch call + 3 per-text calls.
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn embed_batch_via_provider_falls_back_on_length_mismatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let p = FakeProvider {
            calls: calls.clone(),
            mode: ProviderMode::WrongCount,
        };
        let out = embed_batch_via_provider(&p, "test", &["a", "b"]).await;
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|r| r.is_ok()));
        // 1 mismatched batch call + 2 per-text calls.
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn embed_batch_via_provider_maps_wrong_dim_per_position() {
        let calls = Arc::new(AtomicUsize::new(0));
        let p = FakeProvider {
            calls: calls.clone(),
            mode: ProviderMode::OneWrongDim(1),
        };
        let out = embed_batch_via_provider(&p, "test", &["a", "b", "c"]).await;
        assert_eq!(out.len(), 3);
        assert!(out[0].is_ok());
        assert!(out[1].is_err(), "wrong-dim vector maps to Err at its slot");
        assert!(out[2].is_ok());
        // Length matched, so no fallback — a single batch call.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    struct SeqEmbedder {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Embedder for SeqEmbedder {
        fn name(&self) -> &'static str {
            "seq"
        }
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if text == "bad" {
                anyhow::bail!("simulated per-text failure")
            }
            Ok(ok_vec())
        }
        // Uses the default `embed_batch`.
    }

    #[tokio::test]
    async fn default_embed_batch_calls_embed_per_text() {
        let calls = Arc::new(AtomicUsize::new(0));
        let e = SeqEmbedder {
            calls: calls.clone(),
        };
        let out = e.embed_batch(&["a", "b", "c"]).await;
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|r| r.is_ok()));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn default_embed_batch_preserves_per_position_errors() {
        let calls = Arc::new(AtomicUsize::new(0));
        let e = SeqEmbedder {
            calls: calls.clone(),
        };
        let out = e.embed_batch(&["ok", "bad", "ok"]).await;
        assert_eq!(out.len(), 3);
        assert!(out[0].is_ok());
        assert!(out[1].is_err());
        assert!(out[2].is_ok());
    }
}
