//! Orchestration configuration — controls the tiny.place harness session
//! ingest layer.
//!
//! Consumed by [`crate::openhuman::orchestration`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

fn default_debounce_ms() -> u64 {
    750
}

fn default_max_supersteps() -> u32 {
    12
}

fn default_message_window() -> u32 {
    40
}

fn default_evict_threshold() -> f32 {
    0.85
}

fn default_subagent_concurrency() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OrchestrationConfig {
    /// Ingest inbound tiny.place harness session DMs into the orchestration
    /// store. Default: `true`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Coalesce a burst of DMs for one session into a single graph run: after a
    /// session message lands, wait this many milliseconds for the burst to
    /// settle before invoking the wake graph. Default: `750`.
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,

    /// Hard ceiling on graph super-steps for one wake cycle — the loop-continuity
    /// backstop (spec §5). The frontend ⇄ reasoning cycle must terminate on
    /// `channel_response`; this cap forces a terminal DM if it ever does not.
    /// Default: `12`.
    #[serde(default = "default_max_supersteps")]
    pub max_supersteps: u32,

    /// How many of the most recent persisted messages the `normalize` node folds
    /// into `OrchestrationState.messages` for a wake cycle. Default: `40`.
    #[serde(default = "default_message_window")]
    pub message_window: u32,

    /// Context-window utilization at which the `context_guard` node evicts the
    /// oldest compressed-history entries to memory RAG. Clamped to 0.8–0.9 by
    /// [`OrchestrationConfig::effective_evict_threshold`]. Default: `0.85`.
    #[serde(default = "default_evict_threshold")]
    pub context_evict_threshold: f32,

    /// Maximum concurrent execution sub-agents the reasoning `execute` node may
    /// spawn per cycle. Default: `2`.
    #[serde(default = "default_subagent_concurrency")]
    pub subagent_concurrency: u32,

    /// Cadence (minutes) of the `tinyplace` subconscious steering review — the
    /// offline reflection that emits `STEERING_DIRECTIVE`s over the compressed
    /// execution history. `None` (the default) means "use the heartbeat
    /// interval", so the merged behaviour matches the pre-factory build where
    /// the review ran once per memory tick. Env: `OPENHUMAN_ORCH_REVIEW_INTERVAL_MINUTES`.
    #[serde(default)]
    pub review_interval_minutes: Option<u32>,
}

impl OrchestrationConfig {
    /// The eviction threshold clamped to the spec's 0.8–0.9 guardrail band.
    pub fn effective_evict_threshold(&self) -> f32 {
        self.context_evict_threshold.clamp(0.8, 0.9)
    }

    /// The steering-review cadence in minutes: the explicit
    /// `review_interval_minutes` when set, else the supplied heartbeat interval
    /// (floored at 1 so it can never be a busy-loop).
    pub fn effective_review_interval_minutes(&self, heartbeat_interval: u32) -> u32 {
        self.review_interval_minutes
            .unwrap_or(heartbeat_interval)
            .max(1)
    }
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            debounce_ms: default_debounce_ms(),
            max_supersteps: default_max_supersteps(),
            message_window: default_message_window(),
            context_evict_threshold: default_evict_threshold(),
            subagent_concurrency: default_subagent_concurrency(),
            review_interval_minutes: None,
        }
    }
}
