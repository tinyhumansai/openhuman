//! Dashboard model-health aggregation.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::types::{ModelHealthConfigView, ModelHealthEntry, ModelHealthResponse};

/// Build the model health response by joining `model_registry` with the
/// `dashboard.model_health` thresholds.
///
/// Telemetry-driven fields (`quality_score`, `hallucination_rate`,
/// `agents_using`, `tasks_evaluated`) are emitted as placeholders today —
/// there is no local telemetry sink wired in yet. The frontend treats
/// `null` quality / hallucination as "no signal", which collapses status
/// badges to `staging` (under `min_tasks_for_rating`) and keeps the table
/// useful for cost and vision comparison. When a telemetry source lands,
/// populate these fields here rather than at the transport layer.
pub fn model_health(config: &Config) -> Result<RpcOutcome<ModelHealthResponse>, String> {
    let mh_cfg = &config.dashboard.model_health;
    if !mh_cfg.enabled {
        log::debug!("[dashboard] model_health request rejected — feature disabled");
        return Err("model health disabled".to_string());
    }

    let models: Vec<ModelHealthEntry> = config
        .model_registry
        .iter()
        .map(|entry| ModelHealthEntry {
            id: entry.id.clone(),
            provider: entry.provider.clone(),
            cost_per_1m_input: entry.cost_per_1m_input,
            cost_per_1m_cached_input: entry.cost_per_1m_cached_input,
            cost_per_1m_output: entry.cost_per_1m_output,
            context_window: entry.context_window,
            vision: entry.vision,
            // Placeholder metrics — see module-level docs.
            quality_score: None,
            hallucination_rate: None,
            agents_using: 0,
            tasks_evaluated: 0,
        })
        .collect();

    let log = format!(
        "dashboard.model_health returned {} models (threshold={:.2}, window={})",
        models.len(),
        mh_cfg.hallucination_threshold,
        mh_cfg.evaluation_window_tasks,
    );

    Ok(RpcOutcome::single_log(
        ModelHealthResponse {
            models,
            config: ModelHealthConfigView {
                hallucination_threshold: mh_cfg.hallucination_threshold,
                min_tasks_for_rating: mh_cfg.min_tasks_for_rating,
                evaluation_window_tasks: mh_cfg.evaluation_window_tasks,
            },
        },
        log,
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
