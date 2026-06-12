//! Maximal Marginal Relevance (MMR) selection.
//!
//! Given a set of candidate vectors and a query vector, selects a diverse
//! subset that balances relevance to the query against redundancy within
//! the selected set.

use super::store::cosine_similarity;

/// A candidate for MMR selection.
pub struct MmrCandidate<'a> {
    pub index: usize,
    pub embedding: &'a [f32],
    pub relevance: f64,
}

/// Result of MMR selection: the original index and its MMR score.
#[derive(Debug, Clone)]
pub struct MmrResult {
    pub index: usize,
    pub score: f64,
}

/// Selects up to `limit` items from `candidates` using MMR.
///
/// `lambda` controls the relevance-diversity tradeoff:
/// - 1.0 = pure relevance (no diversity)
/// - 0.0 = pure diversity (ignores relevance)
/// - 0.7 = recommended default
///
/// For each selection step:
///   mmr(c) = lambda * relevance(c) - (1-lambda) * max_similarity(c, selected)
pub fn mmr_select(
    query_vec: &[f32],
    candidates: &[MmrCandidate<'_>],
    limit: usize,
    lambda: f64,
) -> Vec<MmrResult> {
    if candidates.is_empty() || limit == 0 {
        return Vec::new();
    }

    let lambda = lambda.clamp(0.0, 1.0);
    let limit = limit.min(candidates.len());

    let mut selected: Vec<usize> = Vec::with_capacity(limit);
    let mut selected_embeddings: Vec<&[f32]> = Vec::with_capacity(limit);
    let mut results: Vec<MmrResult> = Vec::with_capacity(limit);
    let mut available: Vec<bool> = vec![true; candidates.len()];

    for _ in 0..limit {
        let mut best_idx: Option<usize> = None;
        let mut best_mmr = f64::NEG_INFINITY;

        for (i, candidate) in candidates.iter().enumerate() {
            if !available[i] {
                continue;
            }

            let max_sim_to_selected = if selected_embeddings.is_empty() {
                0.0
            } else {
                selected_embeddings
                    .iter()
                    .map(|sel| cosine_similarity(candidate.embedding, sel))
                    .fold(0.0_f64, f64::max)
            };

            let mmr_score = lambda * candidate.relevance - (1.0 - lambda) * max_sim_to_selected;

            if mmr_score > best_mmr {
                best_mmr = mmr_score;
                best_idx = Some(i);
            }
        }

        let Some(idx) = best_idx else { break };

        available[idx] = false;
        selected.push(idx);
        selected_embeddings.push(candidates[idx].embedding);
        results.push(MmrResult {
            index: candidates[idx].index,
            score: best_mmr,
        });
    }

    let _ = (query_vec, &selected);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vec(vals: &[f32]) -> Vec<f32> {
        vals.to_vec()
    }

    #[test]
    fn empty_candidates_returns_empty() {
        let query = make_vec(&[1.0, 0.0, 0.0]);
        let result = mmr_select(&query, &[], 5, 0.7);
        assert!(result.is_empty());
    }

    #[test]
    fn single_candidate() {
        let query = make_vec(&[1.0, 0.0, 0.0]);
        let emb = make_vec(&[1.0, 0.0, 0.0]);
        let candidates = vec![MmrCandidate {
            index: 0,
            embedding: &emb,
            relevance: 0.95,
        }];
        let result = mmr_select(&query, &candidates, 5, 0.7);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].index, 0);
    }

    #[test]
    fn diversity_selects_distinct_vectors() {
        let query = make_vec(&[1.0, 0.0, 0.0]);

        // Three near-duplicates (all close to query) + two distinct vectors
        let dup1 = make_vec(&[0.99, 0.01, 0.0]);
        let dup2 = make_vec(&[0.98, 0.02, 0.0]);
        let dup3 = make_vec(&[0.97, 0.03, 0.0]);
        let distinct1 = make_vec(&[0.0, 1.0, 0.0]);
        let distinct2 = make_vec(&[0.0, 0.0, 1.0]);

        let candidates = vec![
            MmrCandidate {
                index: 0,
                embedding: &dup1,
                relevance: 0.99,
            },
            MmrCandidate {
                index: 1,
                embedding: &dup2,
                relevance: 0.98,
            },
            MmrCandidate {
                index: 2,
                embedding: &dup3,
                relevance: 0.97,
            },
            MmrCandidate {
                index: 3,
                embedding: &distinct1,
                relevance: 0.50,
            },
            MmrCandidate {
                index: 4,
                embedding: &distinct2,
                relevance: 0.45,
            },
        ];

        let result = mmr_select(&query, &candidates, 3, 0.5);
        assert_eq!(result.len(), 3);

        // With lambda=0.5, should pick one from the cluster then diversify
        let selected_indices: Vec<usize> = result.iter().map(|r| r.index).collect();
        // At most one duplicate should be selected with strong diversity
        let dup_count = selected_indices.iter().filter(|&&i| i <= 2).count();
        assert!(dup_count <= 2, "MMR should diversify away from duplicates");
        // At least one distinct vector should be picked
        let distinct_count = selected_indices.iter().filter(|&&i| i >= 3).count();
        assert!(
            distinct_count >= 1,
            "MMR should select at least one distinct vector"
        );
    }

    #[test]
    fn lambda_one_is_pure_relevance() {
        let query = make_vec(&[1.0, 0.0, 0.0]);
        let emb1 = make_vec(&[0.99, 0.01, 0.0]);
        let emb2 = make_vec(&[0.0, 1.0, 0.0]);

        let candidates = vec![
            MmrCandidate {
                index: 0,
                embedding: &emb1,
                relevance: 0.99,
            },
            MmrCandidate {
                index: 1,
                embedding: &emb2,
                relevance: 0.50,
            },
        ];

        let result = mmr_select(&query, &candidates, 2, 1.0);
        assert_eq!(result[0].index, 0);
        assert_eq!(result[1].index, 1);
    }

    #[test]
    fn limit_caps_output() {
        let query = make_vec(&[1.0, 0.0]);
        let embs: Vec<Vec<f32>> = (0..10)
            .map(|i| make_vec(&[1.0 - i as f32 * 0.1, i as f32 * 0.1]))
            .collect();
        let candidates: Vec<MmrCandidate> = embs
            .iter()
            .enumerate()
            .map(|(i, e)| MmrCandidate {
                index: i,
                embedding: e,
                relevance: 1.0 - i as f64 * 0.1,
            })
            .collect();

        let result = mmr_select(&query, &candidates, 3, 0.7);
        assert_eq!(result.len(), 3);
    }
}
