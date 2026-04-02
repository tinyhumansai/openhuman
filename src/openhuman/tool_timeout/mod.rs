//! Wall-clock timeouts for tool execution (skills runtime + agent loop).
//!
//! Override with the `OPENHUMAN_TOOL_TIMEOUT_SECS` environment variable (1–3600; default 120).

use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_SECS: u64 = 120;
const MAX_SECS: u64 = 3600;

fn resolved_secs() -> u64 {
    static SECS: OnceLock<u64> = OnceLock::new();
    *SECS.get_or_init(|| {
        std::env::var("OPENHUMAN_TOOL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n| (1..=MAX_SECS).contains(&n))
            .unwrap_or(DEFAULT_SECS)
    })
}

/// Seconds — used for logging and matching frontend timeouts.
pub fn tool_execution_timeout_secs() -> u64 {
    resolved_secs()
}

pub fn tool_execution_timeout_duration() -> Duration {
    Duration::from_secs(resolved_secs())
}
