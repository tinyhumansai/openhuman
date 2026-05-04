//! Phase 4 embedding layer (#710).
//!
//! Produces a fixed-dimension vector per chunk / summary so retrieval can
//! rerank candidates by semantic similarity. Phase 4's default backend is a
//! local [Ollama](https://ollama.com) endpoint running `nomic-embed-text`;
//! tests use the deterministic [`InertEmbedder`] so no network is required.
//!
//! Dimension is hard-coded at [`EMBEDDING_DIM`] (768) — matches the
//! nomic-embed-text output and keeps the blob layout on `mem_tree_chunks` /
//! `mem_tree_summaries` consistent across providers. Mixing dimensions
//! mid-run would corrupt cosine comparisons; we catch that at the trait
//! level rather than deferring to retrieval-time diagnostics.
//!
//! Write-time semantics: ingest + seal call [`Embedder::embed`] **before**
//! persisting the new row, so a provider error cascades into "don't write
//! this row". Legacy rows from Phases 1-3 predate embeddings and read back
//! with `Option::None`; retrieval tolerates that by dropping legacy rows
//! to the bottom of a semantic rerank.

use anyhow::{Context, Result};
use async_trait::async_trait;

pub mod factory;
pub mod inert;
pub mod ollama;

pub use factory::build_embedder_from_config;
pub use inert::InertEmbedder;
pub use ollama::OllamaEmbedder;

/// Embedding dimensionality used across the memory tree.
///
/// Hard-coded to match `nomic-embed-text`; swapping providers requires a
/// matching dimension or the trait's post-call validation will bail. Any
/// change to this constant breaks on-disk compatibility with existing
/// `mem_tree_chunks.embedding` / `mem_tree_summaries.embedding` blobs.
pub const EMBEDDING_DIM: usize = 768;

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
        let bad = vec![0u8; 16]; // 4 floats, expected 768
        let err = unpack_embedding(&bad).unwrap_err().to_string();
        assert!(err.contains("expected 768"), "got {err}");
    }

    #[test]
    fn pack_checked_rejects_wrong_dim() {
        let too_short = vec![0.0_f32; 5];
        assert!(pack_checked(&too_short).is_err());
        let correct = vec![0.0_f32; EMBEDDING_DIM];
        assert!(pack_checked(&correct).is_ok());
    }
}
