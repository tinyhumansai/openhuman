//! Wire types for the dashboard model health view.

use serde::{Deserialize, Serialize};

/// One row in the model health comparison table.
///
/// Mirrors the JSON shape consumed by the frontend
/// `ModelHealthPanel` — `id`/`provider`/`cost_per_1m_output`/`vision`
/// come from `Config::model_registry`; the four metric fields are
/// emitted as placeholder (`null` / `0`) values until a local telemetry
/// pipeline is wired in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelHealthEntry {
    pub id: String,
    pub provider: String,
    pub cost_per_1m_output: f64,
    pub vision: bool,
    pub quality_score: Option<f64>,
    pub hallucination_rate: Option<f64>,
    pub agents_using: u32,
    pub tasks_evaluated: u32,
}

/// Thresholds the frontend needs to compute the status badge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelHealthConfigView {
    pub hallucination_threshold: f64,
    pub min_tasks_for_rating: usize,
    pub evaluation_window_tasks: usize,
}

/// `openhuman.dashboard_model_health` RPC response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelHealthResponse {
    pub models: Vec<ModelHealthEntry>,
    pub config: ModelHealthConfigView,
}
