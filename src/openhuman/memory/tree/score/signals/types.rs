use serde::{Deserialize, Serialize};

/// Per-signal score breakdown for one chunk. Persisted alongside the total
/// for diagnostics.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScoreSignals {
    pub token_count: f32,
    pub unique_words: f32,
    pub metadata_weight: f32,
    pub source_weight: f32,
    pub interaction: f32,
    pub entity_density: f32,
    /// LLM-derived importance rating in `[0.0, 1.0]`. `0.0` when no LLM
    /// signal is available — combined with `SignalWeights::llm_importance = 0.0`
    /// (the default) this produces a no-op contribution to the total, keeping
    /// behaviour identical to pre-LLM Phase 2.
    #[serde(default)]
    pub llm_importance: f32,
}

/// Default weights applied to each signal in `combine`.
///
/// `llm_importance` defaults to `0.0` (disabled). Callers who configure an
/// LLM extractor should bump it (typical: 2.0 — comparable to the
/// metadata/source weights, well below the interaction-direct signal).
#[derive(Clone, Debug)]
pub struct SignalWeights {
    pub token_count: f32,
    pub unique_words: f32,
    pub metadata_weight: f32,
    pub source_weight: f32,
    pub interaction: f32,
    pub entity_density: f32,
    pub llm_importance: f32,
}

impl Default for SignalWeights {
    fn default() -> Self {
        Self {
            token_count: 1.0,
            unique_words: 1.0,
            metadata_weight: 1.5,
            source_weight: 1.5,
            interaction: 3.0, // strongest signal — direct user engagement
            entity_density: 1.0,
            llm_importance: 0.0, // disabled until LLM extractor is configured
        }
    }
}

impl SignalWeights {
    /// Same as [`Default::default`] but with a non-zero `llm_importance` weight.
    /// Use when an LLM extractor is wired in and you want its importance
    /// signal to influence the admission decision.
    pub fn with_llm_enabled() -> Self {
        Self {
            llm_importance: 2.0,
            ..Self::default()
        }
    }
}
