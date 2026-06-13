//! Type definitions for the subconscious agent loop.

use serde::{Deserialize, Serialize};

/// Summary of the subconscious engine status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousStatus {
    pub enabled: bool,
    pub mode: String,
    pub provider_available: bool,
    pub provider_unavailable_reason: Option<String>,
    pub interval_minutes: u32,
    pub last_tick_at: Option<f64>,
    pub total_ticks: u64,
    pub consecutive_failures: u64,
}

/// Result of a single subconscious tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickResult {
    pub tick_at: f64,
    pub duration_ms: u64,
    #[serde(default)]
    pub response_chars: usize,
}
