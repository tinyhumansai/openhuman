//! Scoring weight profiles for hybrid retrieval — thin host shim over
//! `tinycortex::memory::WeightProfile` (W5).
//!
//! The weight profile (graph/vector/keyword/freshness weights + the
//! `BALANCED`/`SEMANTIC`/`LEXICAL`/`GRAPH_FIRST` presets + `by_name`) is the
//! crate's, a byte-identical port. The host keeps only [`compose_score`] — the
//! trivial weighted combination the crate expresses via
//! `retrieval::scoring::hybrid_score` at its own call sites; exposed here as a
//! free function so `memory_search::tools::hybrid_search` keeps its call shape.

pub use tinycortex::memory::WeightProfile;

/// Weighted composite of the four retrieval signals under `profile`.
///
/// `graph·graph_relevance + vector·vector_similarity + keyword·keyword_relevance
/// + freshness·freshness`.
pub fn compose_score(
    profile: &WeightProfile,
    graph_relevance: f64,
    vector_similarity: f64,
    keyword_relevance: f64,
    freshness: f64,
) -> f64 {
    (profile.graph * graph_relevance)
        + (profile.vector * vector_similarity)
        + (profile.keyword * keyword_relevance)
        + (profile.freshness * freshness)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_score_is_weighted_sum() {
        let p = WeightProfile::BALANCED;
        let s = compose_score(&p, 1.0, 1.0, 1.0, 1.0);
        assert!((s - (p.graph + p.vector + p.keyword + p.freshness)).abs() < 1e-9);
    }
}
