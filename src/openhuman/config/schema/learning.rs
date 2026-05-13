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
#[serde(default)]
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

    /// Enable the tool-scoped memory capture hook (see
    /// [`crate::openhuman::memory::tool_memory::ToolMemoryCaptureHook`]).
    ///
    /// When enabled, the hook records user edicts ("never email Sarah")
    /// as `Critical`-priority rules in the `tool-{name}` memory
    /// namespace, and tallies repeated tool failures into
    /// `Normal`-priority observations. Defaults to true when learning
    /// is enabled — set to false to disable durable rule capture
    /// without turning off learning entirely.
    #[serde(default = "default_true")]
    pub tool_memory_capture_enabled: bool,

    /// Which LLM to use for reflection. Default: local (Ollama).
    #[serde(default)]
    pub reflection_source: ReflectionSource,

    /// Maximum reflections per session before throttling. Default: 20.
    #[serde(default = "default_max_reflections")]
    pub max_reflections_per_session: usize,

    /// Minimum tool calls in a turn to trigger reflection. Default: 1.
    #[serde(default = "default_min_turn_complexity")]
    pub min_turn_complexity: usize,

    /// Pipe agent chat turns into the memory tree as `source="conversations:agent"`.
    ///
    /// When enabled, [`ArchivistHook`] calls `tree::ingest::ingest_chat` with a
    /// two-message [`ChatBatch`] (user + assistant) after every completed turn.
    /// Tool-call JSON is stripped from the assistant message before ingest —
    /// only the prose response reaches the tree.
    ///
    /// Default: true. Disable to stop agent chat from flowing into the tree
    /// without affecting the episodic-log write path.
    #[serde(default = "default_true")]
    pub chat_to_tree_enabled: bool,

    /// Enable the stability detector rebuild cycle. Default: true.
    #[serde(default = "default_true")]
    pub stability_detector_enabled: bool,

    /// How often the periodic rebuild loop runs in seconds. Default: 1800 (30 minutes).
    #[serde(default = "default_rebuild_interval_secs")]
    pub rebuild_interval_secs: u64,
}

fn default_rebuild_interval_secs() -> u64 {
    1800
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
            tool_memory_capture_enabled: default_true(),
            reflection_source: ReflectionSource::default(),
            max_reflections_per_session: default_max_reflections(),
            min_turn_complexity: default_min_turn_complexity(),
            chat_to_tree_enabled: default_true(),
            stability_detector_enabled: default_true(),
            rebuild_interval_secs: default_rebuild_interval_secs(),
        }
    }
}
