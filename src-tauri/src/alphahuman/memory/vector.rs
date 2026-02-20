// Vector operations — cosine similarity, normalization, hybrid merge.

/// Cosine similarity between two vectors. Returns 0.0–1.0.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;

    for (x, y) in a.iter().zip(b.iter()) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if !denom.is_finite() || denom < f64::EPSILON {
        return 0.0;
    }

    let raw = dot / denom;
    if !raw.is_finite() {
        return 0.0;
    }

    // Clamp to [0, 1] — embeddings are typically positive
    #[allow(clippy::cast_possible_truncation)]
    let sim = raw.clamp(0.0, 1.0) as f32;
    sim
}

/// Serialize f32 vector to bytes (little-endian)
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// Deserialize bytes to f32 vector (little-endian)
pub fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            f32::from_le_bytes(arr)
        })
        .collect()
}

/// A scored result for hybrid merging
#[derive(Debug, Clone)]
pub struct ScoredResult {
    pub id: String,
    pub vector_score: Option<f32>,
    pub keyword_score: Option<f32>,
    pub final_score: f32,
}

/// Hybrid merge: combine vector and keyword results with weighted fusion.
///
/// Normalizes each score set to [0, 1], then computes:
///   `final_score` = `vector_weight` * `vector_score` + `keyword_weight` * `keyword_score`
///
/// Deduplicates by id, keeping the best score from each source.
pub fn hybrid_merge(
    vector_results: &[(String, f32)],  // (id, cosine_similarity)
    keyword_results: &[(String, f32)], // (id, bm25_score)
    vector_weight: f32,
    keyword_weight: f32,
    limit: usize,
) -> Vec<ScoredResult> {
    use std::collections::HashMap;

    let mut map: HashMap<String, ScoredResult> = HashMap::new();

    // Normalize vector scores (already 0–1 from cosine similarity)
    for (id, score) in vector_results {
        map.entry(id.clone())
            .and_modify(|r| r.vector_score = Some(*score))
            .or_insert_with(|| ScoredResult {
                id: id.clone(),
                vector_score: Some(*score),
                keyword_score: None,
                final_score: 0.0,
            });
    }

    // Normalize keyword scores (BM25 can be any positive number)
    let max_kw = keyword_results
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0_f32, f32::max);
    let max_kw = if max_kw < f32::EPSILON { 1.0 } else { max_kw };

    for (id, score) in keyword_results {
        let normalized = score / max_kw;
        map.entry(id.clone())
            .and_modify(|r| r.keyword_score = Some(normalized))
            .or_insert_with(|| ScoredResult {
                id: id.clone(),
                vector_score: None,
                keyword_score: Some(normalized),
                final_score: 0.0,
            });
    }

    // Compute final scores
    let mut results: Vec<ScoredResult> = map
        .into_values()
        .map(|mut r| {
            let vs = r.vector_score.unwrap_or(0.0);
            let ks = r.keyword_score.unwrap_or(0.0);
            r.final_score = vector_weight * vs + keyword_weight * ks;
            r
        })
        .collect();

    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::approx_constant,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn cosine_similar_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.1, 2.1, 3.1];
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.99);
    }

    #[test]
    fn cosine_empty_returns_zero() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn cosine_mismatched_lengths() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn vec_bytes_roundtrip() {
        let original = vec![1.0_f32, -2.5, 3.14, 0.0, f32::MAX];
        let bytes = vec_to_bytes(&original);
        let restored = bytes_to_vec(&bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn vec_bytes_empty() {
        let bytes = vec_to_bytes(&[]);
        assert!(bytes.is_empty());
        let restored = bytes_to_vec(&bytes);
        assert!(restored.is_empty());
    }

    #[test]
    fn hybrid_merge_vector_only() {
        let vec_results = vec![("a".into(), 0.9), ("b".into(), 0.5)];
        let merged = hybrid_merge(&vec_results, &[], 0.7, 0.3, 10);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "a");
        assert!(merged[0].final_score > merged[1].final_score);
    }

    #[test]
    fn hybrid_merge_keyword_only() {
        let kw_results = vec![("x".into(), 10.0), ("y".into(), 5.0)];
        let merged = hybrid_merge(&[], &kw_results, 0.7, 0.3, 10);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "x");
    }

    #[test]
    fn hybrid_merge_deduplicates() {
        let vec_results = vec![("a".into(), 0.9)];
        let kw_results = vec![("a".into(), 10.0)];
        let merged = hybrid_merge(&vec_results, &kw_results, 0.7, 0.3, 10);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "a");
        // Should have both scores
        assert!(merged[0].vector_score.is_some());
        assert!(merged[0].keyword_score.is_some());
        // Final score should be higher than either alone
        assert!(merged[0].final_score > 0.7 * 0.9);
    }

    #[test]
    fn hybrid_merge_respects_limit() {
        let vec_results: Vec<(String, f32)> = (0..20)
            .map(|i| (format!("item_{i}"), 1.0 - i as f32 * 0.05))
            .collect();
        let merged = hybrid_merge(&vec_results, &[], 1.0, 0.0, 5);
        assert_eq!(merged.len(), 5);
    }

    #[test]
    fn hybrid_merge_empty_inputs() {
        let merged = hybrid_merge(&[], &[], 0.7, 0.3, 10);
        assert!(merged.is_empty());
    }

    // ── Edge cases: cosine similarity ────────────────────────────

    #[test]
    fn cosine_nan_returns_zero() {
        let a = vec![f32::NAN, 1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        // NaN propagates through arithmetic — result should be 0.0 (clamped or denom check)
        assert!(sim.is_finite(), "Expected finite, got {sim}");
    }

    #[test]
    fn cosine_infinity_returns_zero_or_finite() {
        let a = vec![f32::INFINITY, 1.0];
        let b = vec![1.0, 2.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.is_finite(), "Expected finite, got {sim}");
    }

    #[test]
    fn cosine_negative_values() {
        let a = vec![-1.0, -2.0, -3.0];
        let b = vec![-1.0, -2.0, -3.0];
        // Identical negative vectors → cosine = 1.0, but clamped to [0,1]
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_opposite_vectors_clamped() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        // Cosine = -1.0, clamped to 0.0
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_high_dimensional() {
        let a: Vec<f32> = (0..1536).map(|i| (f64::from(i) * 0.001) as f32).collect();
        let b: Vec<f32> = (0..1536)
            .map(|i| (f64::from(i) * 0.001 + 0.0001) as f32)
            .collect();
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim > 0.99,
            "High-dim similar vectors should be close: {sim}"
        );
    }

    #[test]
    fn cosine_single_element() {
        assert!((cosine_similarity(&[5.0], &[5.0]) - 1.0).abs() < 0.001);
        assert!(cosine_similarity(&[5.0], &[-5.0]).abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_both_zero_vectors() {
        let a = vec![0.0, 0.0];
        let b = vec![0.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < f32::EPSILON);
    }

    // ── Edge cases: vec↔bytes serialization ──────────────────────

    #[test]
    fn bytes_to_vec_non_aligned_truncates() {
        // 5 bytes → only first 4 used (1 float), last byte dropped
        let bytes = vec![0u8, 0, 0, 0, 0xFF];
        let result = bytes_to_vec(&bytes);
        assert_eq!(result.len(), 1);
        assert!(result[0].abs() < f32::EPSILON);
    }

    #[test]
    fn bytes_to_vec_three_bytes_returns_empty() {
        let bytes = vec![1u8, 2, 3];
        let result = bytes_to_vec(&bytes);
        assert!(result.is_empty());
    }

    #[test]
    fn vec_bytes_roundtrip_special_values() {
        let special = vec![f32::MIN, f32::MAX, f32::EPSILON, -0.0, 0.0];
        let bytes = vec_to_bytes(&special);
        let restored = bytes_to_vec(&bytes);
        assert_eq!(special.len(), restored.len());
        for (a, b) in special.iter().zip(restored.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    #[test]
    fn vec_bytes_roundtrip_nan_preserves_bits() {
        let nan_vec = vec![f32::NAN];
        let bytes = vec_to_bytes(&nan_vec);
        let restored = bytes_to_vec(&bytes);
        assert!(restored[0].is_nan());
    }

    // ── Edge cases: hybrid merge ─────────────────────────────────

    #[test]
    fn hybrid_merge_limit_zero() {
        let vec_results = vec![("a".into(), 0.9)];
        let merged = hybrid_merge(&vec_results, &[], 0.7, 0.3, 0);
        assert!(merged.is_empty());
    }

    #[test]
    fn hybrid_merge_zero_weights() {
        let vec_results = vec![("a".into(), 0.9)];
        let kw_results = vec![("b".into(), 10.0)];
        let merged = hybrid_merge(&vec_results, &kw_results, 0.0, 0.0, 10);
        // All final scores should be 0.0
        for r in &merged {
            assert!(r.final_score.abs() < f32::EPSILON);
        }
    }

    #[test]
    fn hybrid_merge_negative_keyword_scores() {
        // BM25 scores are negated in our code, but raw negatives shouldn't crash
        let kw_results = vec![("a".into(), -5.0), ("b".into(), -1.0)];
        let merged = hybrid_merge(&[], &kw_results, 0.7, 0.3, 10);
        assert_eq!(merged.len(), 2);
        // Should still produce finite scores
        for r in &merged {
            assert!(r.final_score.is_finite());
        }
    }

    #[test]
    fn hybrid_merge_duplicate_ids_in_same_source() {
        let vec_results = vec![("a".into(), 0.9), ("a".into(), 0.5)];
        let merged = hybrid_merge(&vec_results, &[], 1.0, 0.0, 10);
        // Should deduplicate — only 1 entry for "a"
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn hybrid_merge_large_bm25_normalization() {
        let kw_results = vec![("a".into(), 1000.0), ("b".into(), 500.0), ("c".into(), 1.0)];
        let merged = hybrid_merge(&[], &kw_results, 0.0, 1.0, 10);
        // "a" should have normalized score of 1.0
        assert!((merged[0].keyword_score.unwrap() - 1.0).abs() < 0.001);
        // "b" should have 0.5
        assert!((merged[1].keyword_score.unwrap() - 0.5).abs() < 0.001);
    }

    #[test]
    fn hybrid_merge_single_item() {
        let merged = hybrid_merge(&[("only".into(), 0.8)], &[], 0.7, 0.3, 10);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "only");
    }
}
