use super::*;
use serde_json::json;

/// A test embedding provider that returns deterministic vectors.
struct FakeEmbedding {
    dims: usize,
}

#[async_trait::async_trait]
impl EmbeddingProvider for FakeEmbedding {
    fn name(&self) -> &str {
        "fake"
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| text_to_vec(t, self.dims)).collect())
    }
}

fn text_to_vec(text: &str, dims: usize) -> Vec<f32> {
    let mut vec = vec![0.0_f32; dims];
    for (i, byte) in text.bytes().enumerate() {
        vec[i % dims] += byte as f32 / 255.0;
    }
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut vec {
            *x /= norm;
        }
    }
    vec
}

struct MismatchEmbedding;

#[async_trait::async_trait]
impl EmbeddingProvider for MismatchEmbedding {
    fn name(&self) -> &str {
        "mismatch"
    }
    fn dimensions(&self) -> usize {
        2
    }
    async fn embed(&self, _texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(vec![vec![1.0, 0.0]])
    }
}

fn fake_store(dims: usize) -> VectorStore {
    VectorStore::open_in_memory(Arc::new(FakeEmbedding { dims })).unwrap()
}

// ── vec_to_bytes / bytes_to_vec ─────────────────────────

#[test]
fn roundtrip_vec_bytes() {
    let original = vec![1.0_f32, -2.5, 3.14, 0.0, f32::MAX, f32::MIN];
    let bytes = vec_to_bytes(&original);
    assert_eq!(bytes.len(), original.len() * 4);
    assert_eq!(original, bytes_to_vec(&bytes));
}

#[test]
fn empty_vec_roundtrip() {
    assert!(bytes_to_vec(&vec_to_bytes(&[])).is_empty());
}

#[test]
fn bytes_to_vec_truncates_partial_bytes() {
    assert_eq!(bytes_to_vec(&[0u8; 5]).len(), 1);
}

// ── cosine_similarity ───────────────────────────────────

#[test]
fn cosine_identical() {
    let v = vec![1.0_f32, 2.0, 3.0];
    assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
}

#[test]
fn cosine_orthogonal() {
    assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
}

#[test]
fn cosine_opposite() {
    assert!(cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]).abs() < 1e-6);
}

#[test]
fn cosine_mismatched_lengths() {
    assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0, 2.0, 3.0]), 0.0);
}

#[test]
fn cosine_empty() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}

#[test]
fn cosine_zero_vector() {
    assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
}

#[test]
fn cosine_similar_high() {
    assert!(cosine_similarity(&[1.0, 2.0, 3.0], &[1.1, 2.1, 3.1]) > 0.99);
}

// ── VectorStore: open / metadata ────────────────────────

#[test]
fn open_in_memory_succeeds() {
    let store = fake_store(3);
    assert_eq!(store.count(None).unwrap(), 0);
}

#[test]
fn open_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sub/dir/vectors.db");
    let store = VectorStore::open(&db_path, Arc::new(FakeEmbedding { dims: 3 })).unwrap();
    assert_eq!(store.count(None).unwrap(), 0);
    assert!(db_path.exists());
}

#[test]
fn open_reopen_same_dims_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v.db");
    VectorStore::open(&db_path, Arc::new(FakeEmbedding { dims: 4 })).unwrap();
    // Reopen with same dims — should work.
    VectorStore::open(&db_path, Arc::new(FakeEmbedding { dims: 4 })).unwrap();
}

#[test]
fn open_reopen_different_dims_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v.db");
    VectorStore::open(&db_path, Arc::new(FakeEmbedding { dims: 4 })).unwrap();
    let result = VectorStore::open(&db_path, Arc::new(FakeEmbedding { dims: 8 }));
    let msg = result.err().expect("should be an error").to_string();
    assert!(msg.contains("dimension mismatch"), "msg: {msg}");
    assert!(msg.contains("4"), "should mention stored dims: {msg}");
    assert!(msg.contains("8"), "should mention runtime dims: {msg}");
}

#[test]
fn embedder_accessor() {
    let store = fake_store(3);
    assert_eq!(store.embedder().name(), "fake");
    assert_eq!(store.embedder().dimensions(), 3);
}

// ── insert + count ──────────────────────────────────────

#[tokio::test]
async fn insert_and_count() {
    let store = fake_store(4);
    store.insert("a", "ns1", "hello", json!({})).await.unwrap();
    store.insert("b", "ns1", "world", json!({})).await.unwrap();
    store.insert("c", "ns2", "other", json!({})).await.unwrap();
    assert_eq!(store.count(Some("ns1")).unwrap(), 2);
    assert_eq!(store.count(Some("ns2")).unwrap(), 1);
    assert_eq!(store.count(None).unwrap(), 3);
}

#[tokio::test]
async fn insert_upsert_replaces() {
    let store = fake_store(4);
    store
        .insert("a", "ns", "original", json!({"v": 1}))
        .await
        .unwrap();
    store
        .insert("a", "ns", "updated", json!({"v": 2}))
        .await
        .unwrap();
    assert_eq!(store.count(Some("ns")).unwrap(), 1);
    let results = store
        .search_by_vector("ns", &text_to_vec("updated", 4), 10)
        .unwrap();
    assert_eq!(results[0].text, "updated");
    assert_eq!(results[0].metadata["v"], 2);
}

#[test]
fn insert_with_vector_sync() {
    let store = fake_store(3);
    store
        .insert_with_vector("id1", "ns", "text", &[1.0, 0.0, 0.0], json!({"k": "v"}))
        .unwrap();
    assert_eq!(store.count(Some("ns")).unwrap(), 1);
}

// ── insert_batch ────────────────────────────────────────

#[tokio::test]
async fn insert_batch_multiple() {
    let store = fake_store(4);
    let entries = vec![
        ("a", "alpha", json!({})),
        ("b", "beta", json!({})),
        ("c", "gamma", json!({})),
    ];
    store.insert_batch("ns", &entries).await.unwrap();
    assert_eq!(store.count(Some("ns")).unwrap(), 3);
}

#[tokio::test]
async fn insert_batch_empty() {
    let store = fake_store(4);
    store.insert_batch("ns", &[]).await.unwrap();
    assert_eq!(store.count(None).unwrap(), 0);
}

#[tokio::test]
async fn insert_batch_mismatch_error() {
    let store = VectorStore::open_in_memory(Arc::new(MismatchEmbedding)).unwrap();
    let entries = vec![("a", "alpha", json!({})), ("b", "beta", json!({}))];
    let err = store.insert_batch("ns", &entries).await.unwrap_err();
    assert!(err.to_string().contains("mismatch"));
}

// ── search ──────────────────────────────────────────────

#[tokio::test]
async fn search_returns_ranked_results() {
    let store = fake_store(8);
    store
        .insert("a", "ns", "the quick brown fox", json!({}))
        .await
        .unwrap();
    store
        .insert("b", "ns", "a lazy dog sleeps", json!({}))
        .await
        .unwrap();
    store
        .insert("c", "ns", "the quick brown fox jumps", json!({}))
        .await
        .unwrap();
    let results = store.search("ns", "the quick brown fox", 2).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].score >= results[1].score);
}

#[tokio::test]
async fn search_respects_limit() {
    let store = fake_store(4);
    for i in 0..10 {
        store
            .insert(&format!("id-{i}"), "ns", &format!("text {i}"), json!({}))
            .await
            .unwrap();
    }
    assert_eq!(store.search("ns", "text", 3).await.unwrap().len(), 3);
}

#[tokio::test]
async fn search_empty_namespace() {
    let store = fake_store(4);
    assert!(store.search("empty", "query", 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn search_namespace_isolation() {
    let store = fake_store(4);
    store.insert("a", "ns1", "hello", json!({})).await.unwrap();
    store.insert("b", "ns2", "hello", json!({})).await.unwrap();
    assert_eq!(store.search("ns1", "hello", 10).await.unwrap()[0].id, "a");
    assert_eq!(store.search("ns2", "hello", 10).await.unwrap()[0].id, "b");
}

// ── search_by_vector ────────────────────────────────────

#[test]
fn search_by_vector_limit_zero() {
    let store = fake_store(3);
    store
        .insert_with_vector("a", "ns", "t", &[1.0, 0.0, 0.0], json!({}))
        .unwrap();
    assert!(store
        .search_by_vector("ns", &[1.0, 0.0, 0.0], 0)
        .unwrap()
        .is_empty());
}

#[test]
fn search_by_vector_scores_correct() {
    let store = fake_store(3);
    store
        .insert_with_vector("x", "ns", "x", &[1.0, 0.0, 0.0], json!({}))
        .unwrap();
    store
        .insert_with_vector("y", "ns", "y", &[0.0, 1.0, 0.0], json!({}))
        .unwrap();
    let results = store.search_by_vector("ns", &[1.0, 0.0, 0.0], 2).unwrap();
    assert_eq!(results[0].id, "x");
    assert!((results[0].score - 1.0).abs() < 1e-6);
    assert!(results[1].score < 1e-6);
}

#[test]
fn search_by_vector_preserves_metadata() {
    let store = fake_store(2);
    store
        .insert_with_vector("a", "ns", "t", &[1.0, 0.0], json!({"key": "value"}))
        .unwrap();
    assert_eq!(
        store.search_by_vector("ns", &[1.0, 0.0], 1).unwrap()[0].metadata["key"],
        "value"
    );
}

#[test]
fn search_handles_invalid_metadata_json() {
    let store = fake_store(2);
    {
        let conn = store.conn.lock();
        conn.execute(
            "INSERT INTO vectors (id, namespace, text, embedding, metadata, created_at, updated_at)
             VALUES ('bad', 'ns', 'text', ?1, 'not-json', 0.0, 0.0)",
            rusqlite::params![vec_to_bytes(&[1.0, 0.0])],
        )
        .unwrap();
    }
    let results = store.search_by_vector("ns", &[1.0, 0.0], 1).unwrap();
    assert_eq!(results[0].id, "bad");
    assert!(results[0].metadata.is_null());
}

// ── delete ──────────────────────────────────────────────

#[tokio::test]
async fn delete_existing() {
    let store = fake_store(4);
    store.insert("a", "ns", "text", json!({})).await.unwrap();
    assert!(store.delete("ns", "a").unwrap());
    assert_eq!(store.count(Some("ns")).unwrap(), 0);
}

#[test]
fn delete_nonexistent() {
    assert!(!fake_store(3).delete("ns", "no-such-id").unwrap());
}

#[tokio::test]
async fn delete_wrong_namespace() {
    let store = fake_store(4);
    store.insert("a", "ns1", "text", json!({})).await.unwrap();
    assert!(!store.delete("ns2", "a").unwrap());
    assert_eq!(store.count(Some("ns1")).unwrap(), 1);
}

// ── clear_namespace ─────────────────────────────────────

#[tokio::test]
async fn clear_namespace_removes_all() {
    let store = fake_store(4);
    store.insert("a", "ns", "one", json!({})).await.unwrap();
    store.insert("b", "ns", "two", json!({})).await.unwrap();
    store
        .insert("c", "other", "three", json!({}))
        .await
        .unwrap();
    assert_eq!(store.clear_namespace("ns").unwrap(), 2);
    assert_eq!(store.count(Some("ns")).unwrap(), 0);
    assert_eq!(store.count(Some("other")).unwrap(), 1);
}

#[test]
fn clear_empty_namespace() {
    assert_eq!(fake_store(3).clear_namespace("empty").unwrap(), 0);
}

// ── list_namespaces ─────────────────────────────────────

#[tokio::test]
async fn list_namespaces_empty() {
    assert!(fake_store(3).list_namespaces().unwrap().is_empty());
}

#[tokio::test]
async fn list_namespaces_populated() {
    let store = fake_store(4);
    store.insert("a", "beta", "t", json!({})).await.unwrap();
    store.insert("b", "alpha", "t", json!({})).await.unwrap();
    store.insert("c", "beta", "t", json!({})).await.unwrap();
    assert_eq!(store.list_namespaces().unwrap(), vec!["alpha", "beta"]);
}

// ── count ───────────────────────────────────────────────

#[test]
fn count_empty() {
    let store = fake_store(3);
    assert_eq!(store.count(None).unwrap(), 0);
    assert_eq!(store.count(Some("ns")).unwrap(), 0);
}
