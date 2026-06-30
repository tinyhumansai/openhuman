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
    /// [`crate::openhuman::memory_tools::ToolMemoryCaptureHook`]).
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

    /// Enable episodic capture (ArchivistHook) regardless of the master
    /// `learning.enabled` toggle.
    ///
    /// Episodic capture is the system-of-record for chat turns
    /// (`episodic_log` FTS5 table, conversation segmentation, segment
    /// summaries with LLM recap, and segment embeddings). It must remain
    /// active even when the inference stack
    /// (reflection / stability-detector) is off.
    ///
    /// Default: `true`. Set to `false` to fully disable the Archivist.
    ///
    /// Override via `OPENHUMAN_LEARNING_EPISODIC_CAPTURE_ENABLED=0|1`.
    #[serde(default = "default_true")]
    pub episodic_capture_enabled: bool,

    /// Enable preemptive STM recall injection at session start and on-demand
    /// `stm_recall_search` tool exposure.
    ///
    /// When enabled, a bounded cross-thread context block is assembled from
    /// recent episodic entries (FTS5 keyword arm) and segment recaps (cosine
    /// similarity arm) from OTHER sessions and injected into the first turn's
    /// user message. The `stm_recall_search` tool is also registered in the
    /// agent's tool list.
    ///
    /// Default: `true`. Set to `false` to fully disable STM recall.
    ///
    /// Override via `OPENHUMAN_LEARNING_STM_RECALL_ENABLED=0|1`.
    #[serde(default = "default_true")]
    pub stm_recall_enabled: bool,

    /// Use the rolling segment recap as the compaction text for evicted turns
    /// (Phase 1.5 — unified compaction).
    ///
    /// When `true`, the [`ContextManager`]'s autocompaction summarizer is
    /// wrapped with a `SegmentRecapSummarizer` that first tries to obtain the
    /// current open segment's rolling recap from the `ArchivistHook` and uses
    /// it as the replacement text for the evicted head. If the rolling recap
    /// is unavailable (no archivist, no open segment, LLM failure, flag off)
    /// the inner `ProviderSummarizer` runs as before — the live prompt is
    /// NEVER left over-budget regardless of the recap path's health.
    ///
    /// Default: `true`. Set to `false` to revert to the standalone
    /// `ProviderSummarizer` path (today's behaviour, Phase 1.5 completely
    /// absent from the hot path).
    ///
    /// Override via `OPENHUMAN_LEARNING_UNIFIED_COMPACTION_ENABLED=0|1`.
    #[serde(default = "default_true")]
    pub unified_compaction_enabled: bool,

    /// How often the periodic rebuild loop runs in seconds. Default: 1800 (30 minutes).
    #[serde(default = "default_rebuild_interval_secs")]
    pub rebuild_interval_secs: u64,

    /// Enable explicit user-preference injection into the system prompt.
    ///
    /// When `true` (the default), preferences saved via the `remember_preference`
    /// tool are injected into every session prompt regardless of whether the full
    /// inference-based learning subsystem (`enabled`) is on.  This is the
    /// narrow, always-on path for user-authoritative pinned preferences —
    /// no reflection, no heuristics, no stability engine.
    ///
    /// Explicitly set to `false` (or `OPENHUMAN_LEARNING_EXPLICIT_PREFERENCES_ENABLED=0`)
    /// to suppress all preference injection even for explicitly pinned entries.
    #[serde(default = "default_true")]
    pub explicit_preferences_enabled: bool,

    /// Enable automatic enrichment of the long-term goals list
    /// (`MEMORY_GOALS.md`) when the conversation context is summarized.
    ///
    /// When `true` (the default), a best-effort background `goals_agent`
    /// run is fired after a segment recap so the user's durable goals stay
    /// fresh. Set to `false` (or
    /// `OPENHUMAN_LEARNING_GOALS_ENRICHMENT_ENABLED=0`) to only update the
    /// goals list via explicit RPC/tools.
    #[serde(default = "default_true")]
    pub goals_enrichment_enabled: bool,
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
            episodic_capture_enabled: default_true(),
            stm_recall_enabled: default_true(),
            unified_compaction_enabled: default_true(),
            explicit_preferences_enabled: default_true(),
            goals_enrichment_enabled: default_true(),
        }
    }
}
