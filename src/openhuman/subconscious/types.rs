//! Type definitions for the subconscious agent loop.

use serde::{Deserialize, Serialize};

/// Summary of a subconscious instance's status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousStatus {
    /// Which world this row describes (`"memory"` | `"tinyplace"`). Defaulted to
    /// `"memory"` when absent so older callers/serialized rows keep parsing.
    #[serde(default = "default_instance")]
    pub instance: String,
    pub enabled: bool,
    pub mode: String,
    pub provider_available: bool,
    pub provider_unavailable_reason: Option<String>,
    pub interval_minutes: u32,
    pub last_tick_at: Option<f64>,
    pub total_ticks: u64,
    pub consecutive_failures: u64,
}

fn default_instance() -> String {
    "memory".to_string()
}

/// Result of a single subconscious tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickResult {
    pub tick_at: f64,
    pub duration_ms: u64,
    #[serde(default)]
    pub response_chars: usize,
}
