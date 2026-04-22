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
}

/// Default weights applied to each signal in `combine`.
#[derive(Clone, Debug)]
pub struct SignalWeights {
    pub token_count: f32,
    pub unique_words: f32,
    pub metadata_weight: f32,
    pub source_weight: f32,
    pub interaction: f32,
    pub entity_density: f32,
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
        }
    }
}
