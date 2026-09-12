//! Dashboard configuration (event stream, model health, future panels).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_diagram_viewer_enabled() -> bool {
    true
}

fn default_diagram_viewer_source_url() -> String {
    "http://localhost:8787/workspace/diagrams/latest.png".to_string()
}

fn default_diagram_viewer_refresh_interval_seconds() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct DashboardConfig {
    #[serde(default)]
    pub event_stream: EventStreamConfig,
    #[serde(default)]
    pub model_health: ModelHealthConfig,
    #[serde(default)]
    pub diagram_viewer: DiagramViewerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct EventStreamConfig {
    /// Whether the live event stream endpoint is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Maximum number of entries the frontend should retain.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,

    /// Where new entries appear: "top" (newest first) or "bottom" (oldest first).
    #[serde(default = "default_new_entries")]
    pub new_entries: String,
}

fn default_enabled() -> bool {
    true
}
fn default_max_entries() -> usize {
    200
}
fn default_new_entries() -> String {
    "top".to_string()
}

impl Default for EventStreamConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 200,
            new_entries: "top".to_string(),
        }
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DiagramViewerConfig {
    #[serde(default = "default_diagram_viewer_enabled")]
    pub enabled: bool,
    #[serde(default = "default_diagram_viewer_source_url")]
    pub source_url: String,
    #[serde(default = "default_diagram_viewer_refresh_interval_seconds")]
    pub refresh_interval_seconds: u64,
}

impl Default for DiagramViewerConfig {
    fn default() -> Self {
        Self {
            enabled: default_diagram_viewer_enabled(),
            source_url: default_diagram_viewer_source_url(),
            refresh_interval_seconds: default_diagram_viewer_refresh_interval_seconds(),
        }
    }
}

#[cfg(test)]
#[path = "dashboard_tests.rs"]
mod tests;
