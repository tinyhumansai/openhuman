//! Scoring weight profiles for hybrid retrieval.

#[derive(Debug, Clone, Copy)]
pub struct WeightProfile {
    pub graph: f64,
    pub vector: f64,
    pub keyword: f64,
    pub freshness: f64,
}

impl WeightProfile {
    pub const BALANCED: Self = Self {
        graph: 0.35,
        vector: 0.35,
        keyword: 0.15,
        freshness: 0.15,
    };

    pub const SEMANTIC: Self = Self {
        graph: 0.15,
        vector: 0.65,
        keyword: 0.20,
        freshness: 0.0,
    };

    pub const LEXICAL: Self = Self {
        graph: 0.25,
        vector: 0.15,
        keyword: 0.60,
        freshness: 0.0,
    };

    pub const GRAPH_FIRST: Self = Self {
        graph: 0.55,
        vector: 0.30,
        keyword: 0.15,
        freshness: 0.0,
    };

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "balanced" => Some(Self::BALANCED),
            "semantic" => Some(Self::SEMANTIC),
            "lexical" => Some(Self::LEXICAL),
            "graph_first" => Some(Self::GRAPH_FIRST),
            _ => None,
        }
    }

    pub fn compose_score(
        &self,
        graph_relevance: f64,
        vector_similarity: f64,
        keyword_relevance: f64,
        freshness: f64,
    ) -> f64 {
        (self.graph * graph_relevance)
            + (self.vector * vector_similarity)
            + (self.keyword * keyword_relevance)
            + (self.freshness * freshness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_sum_to_one() {
        for profile in [
            WeightProfile::BALANCED,
            WeightProfile::SEMANTIC,
            WeightProfile::LEXICAL,
            WeightProfile::GRAPH_FIRST,
        ] {
            let sum = profile.graph + profile.vector + profile.keyword + profile.freshness;
            assert!(
                (sum - 1.0).abs() < 0.01,
                "profile weights should sum to ~1.0, got {sum}"
            );
        }
    }

    #[test]
    fn from_name_resolves() {
        assert!(WeightProfile::from_name("balanced").is_some());
        assert!(WeightProfile::from_name("semantic").is_some());
        assert!(WeightProfile::from_name("lexical").is_some());
        assert!(WeightProfile::from_name("graph_first").is_some());
        assert!(WeightProfile::from_name("unknown").is_none());
    }

    #[test]
    fn compose_score_applies_weights() {
        let profile = WeightProfile::SEMANTIC;
        let score = profile.compose_score(0.5, 1.0, 0.5, 0.0);
        let expected = 0.15 * 0.5 + 0.65 * 1.0 + 0.20 * 0.5;
        assert!((score - expected).abs() < f64::EPSILON);
    }
}
