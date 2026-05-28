use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DashboardConfig {
    pub event_stream: EventStreamConfig,
    #[serde(default)]
    pub model_health: ModelHealthConfig,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            event_stream: EventStreamConfig::default(),
            model_health: ModelHealthConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct EventStreamConfig {
    #[serde(default = "default_es_enabled")]
    pub enabled: bool,
}

fn default_es_enabled() -> bool {
    true
}

impl Default for EventStreamConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ModelHealthConfig {
    #[serde(default = "default_mh_enabled")]
    pub enabled: bool,
    #[serde(default = "default_hallucination_threshold")]
    pub hallucination_threshold: f64,
    #[serde(default = "default_min_tasks")]
    pub min_tasks_for_rating: usize,
    #[serde(default = "default_eval_window")]
    pub evaluation_window_tasks: usize,
}

fn default_mh_enabled() -> bool {
    true
}
fn default_hallucination_threshold() -> f64 {
    0.10
}
fn default_min_tasks() -> usize {
    10
}
fn default_eval_window() -> usize {
    50
}

impl Default for ModelHealthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hallucination_threshold: 0.10,
            min_tasks_for_rating: 10,
            evaluation_window_tasks: 50,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_health_defaults_match_spec() {
        let mh = ModelHealthConfig::default();
        assert!(mh.enabled);
        assert!((mh.hallucination_threshold - 0.10).abs() < f64::EPSILON);
        assert_eq!(mh.min_tasks_for_rating, 10);
        assert_eq!(mh.evaluation_window_tasks, 50);
    }
}
