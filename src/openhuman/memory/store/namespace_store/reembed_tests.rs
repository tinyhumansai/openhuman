//! Tests for the `reembed` sweep — which `vector_chunks` rows are pending.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tempfile::TempDir;

use crate::openhuman::inference::embeddings::EmbeddingProvider;
use crate::openhuman::memory::store::UnifiedMemory;

/// Embedder whose only interesting property is a fixed dimensionality: the
/// sweep predicate keys entirely off `dimensions()`. `returns` is the width of
/// the vectors it actually hands back, which a degraded provider lets drift
/// away from the declared `dims`.
struct DimStub {
    dims: usize,
    returns: usize,
}

impl DimStub {
    /// A healthy provider: returns vectors at its declared dimensionality.
    fn healthy(dims: usize) -> Self {
        Self {
            dims,
            returns: dims,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for DimStub {
    fn name(&self) -> &str {
        "stub"
    }
    fn model_id(&self) -> &str {
        "stub-model"
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|_| vec![0.1_f32; self.returns.max(1)])
            .collect())
    }
}

/// Insert one `vector_chunks` row directly. `dim = Some(n)` writes a matching
/// non-null embedding blob (mirroring `documents.rs`, which sets `embedding`
/// and `dim` together); `dim = None` writes the text-only, vector-less row a
/// failed batch embed leaves behind.
fn insert_chunk(
    memory: &UnifiedMemory,
    namespace: &str,
    document_id: &str,
    idx: usize,
    text: &str,
    dim: Option<i64>,
) {
    let embedding: Option<Vec<u8>> = dim.map(|d| vec![0_u8; (d.max(0) as usize) * 4]);
    let model_signature: Option<String> =
        dim.map(|d| format!("provider=stub;model=stub-model;dims={d}"));
    let conn = memory.conn.lock();
    conn.execute(
        "INSERT OR REPLACE INTO vector_chunks
           (namespace, document_id, chunk_id, text, embedding, metadata_json, created_at, updated_at, model_signature, dim)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            namespace,
            document_id,
            format!("{document_id}:{idx}"),
            text,
            embedding,
            "{}",
            1000.0_f64,
            1000.0_f64,
            model_signature,
            dim,
        ],
    )
    .unwrap();
}

#[tokio::test]
async fn scan_flags_null_and_wrong_dimension_but_not_healthy_or_empty() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(DimStub::healthy(1024)), None).unwrap();

    // Healthy: dimension matches the active embedder (1024) — NOT pending.
    insert_chunk(
        &memory,
        "skill-gmail",
        "healthy",
        0,
        "colorado boulder",
        Some(1024),
    );
    // Degenerate dims=1 (a partial cloud failure) — pending.
    insert_chunk(
        &memory,
        "skill-gmail",
        "degenerate",
        0,
        "colorado springs",
        Some(1),
    );
    // Never embedded (NULL vector) — pending.
    insert_chunk(&memory, "skill-gmail", "failed", 0, "denver colorado", None);
    // Blank text — permanently un-embeddable, must never be pending.
    insert_chunk(&memory, "skill-gmail", "blank", 0, "   ", None);

    let pending = memory.scan_chunks_needing_reembed(100).unwrap();
    let ids: HashSet<&str> = pending.iter().map(|c| c.document_id.as_str()).collect();

    assert!(
        ids.contains("degenerate"),
        "dims=1 degenerate row must be pending"
    );
    assert!(ids.contains("failed"), "NULL-embedding row must be pending");
    assert!(
        !ids.contains("healthy"),
        "matching-dim row must not be pending"
    );
    assert!(
        !ids.contains("blank"),
        "blank-text row must never be pending"
    );
    assert_eq!(
        pending.len(),
        2,
        "exactly the two deficient rows are pending"
    );
}

#[tokio::test]
async fn scan_respects_limit() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(DimStub::healthy(1024)), None).unwrap();
    for i in 0..5 {
        insert_chunk(&memory, "skill-gmail", "doc", i, "needs embedding", None);
    }
    let pending = memory.scan_chunks_needing_reembed(3).unwrap();
    assert_eq!(pending.len(), 3, "limit caps the returned candidate count");
}

#[tokio::test]
async fn sweep_reembeds_pending_rows_and_clears_the_pending_set() {
    let tmp = TempDir::new().unwrap();
    // 8-dim active embedder: rows at dim=8 are healthy, dim=1 / NULL are not.
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(DimStub::healthy(8)), None).unwrap();
    insert_chunk(&memory, "skill-gmail", "failed", 0, "denver colorado", None);
    insert_chunk(
        &memory,
        "skill-gmail",
        "degenerate",
        0,
        "colorado springs",
        Some(1),
    );
    insert_chunk(&memory, "skill-gmail", "healthy", 0, "boulder", Some(8));

    let report = memory.reembed_pending(100).await;
    assert_eq!(
        report.reembedded, 2,
        "the NULL and dims=1 rows get fresh vectors"
    );
    assert_eq!(
        report.failed, 0,
        "no row is left pending after a clean sweep"
    );

    // Nothing is pending once the sweep has stamped every deficient row.
    let pending = memory.scan_chunks_needing_reembed(100).unwrap();
    assert!(pending.is_empty(), "sweep cleared the pending set");
}

#[tokio::test]
async fn a_degraded_provider_leaves_rows_pending_instead_of_stamping_short_vectors() {
    let tmp = TempDir::new().unwrap();
    // Declares 8 dims but hands back 1 — the partial cloud failure that put
    // `dims=1` vectors into flo's store in the first place.
    let memory = UnifiedMemory::new(
        tmp.path(),
        Arc::new(DimStub {
            dims: 8,
            returns: 1,
        }),
        None,
    )
    .unwrap();
    insert_chunk(&memory, "skill-gmail", "failed", 0, "denver colorado", None);

    let report = memory.reembed_pending(100).await;
    assert_eq!(report.reembedded, 0, "a short vector is not a repair");
    assert_eq!(report.failed, 1, "the row is reported as still pending");

    // Still pending — so a later sweep, under a working provider, fixes it.
    // Stamping the short vector instead would both hide the row from recall and
    // keep it pending forever, re-embedding it on every single pass.
    let pending = memory.scan_chunks_needing_reembed(100).unwrap();
    assert_eq!(pending.len(), 1, "the row must stay in the pending set");
}
