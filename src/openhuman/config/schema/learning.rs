//! Self-learning configuration — reflection, user profiling, tool tracking.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which LLM to use for reflection inference.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ReflectionSource {
    /// Use the local Ollama model via `LocalAiService::prompt()`.
    /// Model is determined by `config.local_ai.chat_model_id`.
    #[default]
    Local,
    /// Use the cloud reasoning model via `Provider::simple_chat("hint:reasoning")`.
    Cloud,
}

/// Configuration for the agent self-learning subsystem.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningConfig {
    /// Master switch. Default: false.
    #[serde(default)]
    pub enabled: bool,

    /// Enable post-turn reflection (observation extraction). Default: true when learning is enabled.
    #[serde(default = "default_true")]
    pub reflection_enabled: bool,

    /// Enable automatic user profile extraction. Default: true when learning is enabled.
    #[serde(default = "default_true")]
    pub user_profile_enabled: bool,

    /// Enable tool effectiveness tracking. Default: true when learning is enabled.
    #[serde(default = "default_true")]
    pub tool_tracking_enabled: bool,

    /// Enable autonomous skill creation from experience. Default: false (Phase 5).
    #[serde(default)]
    pub skill_creation_enabled: bool,

    /// Which LLM to use for reflection. Default: local (Ollama).
    #[serde(default)]
    pub reflection_source: ReflectionSource,

    /// Maximum reflections per session before throttling. Default: 20.
    #[serde(default = "default_max_reflections")]
    pub max_reflections_per_session: usize,

    /// Minimum tool calls in a turn to trigger reflection. Default: 1.
    #[serde(default = "default_min_turn_complexity")]
    pub min_turn_complexity: usize,
}

fn default_true() -> bool {
    true
}

fn default_max_reflections() -> usize {
    20
}

fn default_min_turn_complexity() -> usize {
    1
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            reflection_enabled: default_true(),
            user_profile_enabled: default_true(),
            tool_tracking_enabled: default_true(),
            skill_creation_enabled: false,
            reflection_source: ReflectionSource::default(),
            max_reflections_per_session: default_max_reflections(),
            min_turn_complexity: default_min_turn_complexity(),
        }
    }
}
