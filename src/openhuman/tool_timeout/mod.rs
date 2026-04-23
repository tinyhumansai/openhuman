//! Wall-clock timeouts for tool execution (skills runtime + agent loop).
//!
//! Override with the `OPENHUMAN_TOOL_TIMEOUT_SECS` environment variable (1–3600; default 120).

use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_SECS: u64 = 120;
const MAX_SECS: u64 = 3600;
const ENV_VAR: &str = "OPENHUMAN_TOOL_TIMEOUT_SECS";

/// Parse a raw env-var value into a bounded timeout.
///
/// Testable split from [`resolved_secs`]: this function is pure and never
/// touches global state, so unit tests can exercise every path without
/// racing on `OnceLock` or needing to mutate the process environment.
///
/// - `None` or a non-numeric string returns [`DEFAULT_SECS`].
/// - Values outside `1..=MAX_SECS` are rejected (returns [`DEFAULT_SECS`]).
/// - Valid values pass through unchanged.
pub fn parse_tool_timeout_secs(raw: Option<&str>) -> u64 {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| (1..=MAX_SECS).contains(&n))
        .unwrap_or(DEFAULT_SECS)
}

fn resolved_secs() -> u64 {
    static SECS: OnceLock<u64> = OnceLock::new();
    *SECS.get_or_init(|| parse_tool_timeout_secs(std::env::var(ENV_VAR).ok().as_deref()))
}

/// Seconds — used for logging and matching frontend timeouts.
pub fn tool_execution_timeout_secs() -> u64 {
    resolved_secs()
}

pub fn tool_execution_timeout_duration() -> Duration {
    Duration::from_secs(resolved_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_when_env_missing() {
        assert_eq!(parse_tool_timeout_secs(None), DEFAULT_SECS);
    }

    #[test]
    fn default_when_value_not_numeric() {
        assert_eq!(parse_tool_timeout_secs(Some("not-a-number")), DEFAULT_SECS);
        assert_eq!(parse_tool_timeout_secs(Some("")), DEFAULT_SECS);
        assert_eq!(parse_tool_timeout_secs(Some("12x")), DEFAULT_SECS);
    }

    #[test]
    fn default_when_value_zero() {
        // 0 seconds would disable the timeout — reject and fall back.
        assert_eq!(parse_tool_timeout_secs(Some("0")), DEFAULT_SECS);
    }

    #[test]
    fn default_when_value_above_max() {
        assert_eq!(parse_tool_timeout_secs(Some("3601")), DEFAULT_SECS);
        assert_eq!(parse_tool_timeout_secs(Some("99999999999")), DEFAULT_SECS);
    }

    #[test]
    fn default_when_value_negative_or_signed() {
        // Negative values fail u64 parse and fall back to default.
        assert_eq!(parse_tool_timeout_secs(Some("-5")), DEFAULT_SECS);
    }

    #[test]
    fn accepts_valid_values_at_boundaries() {
        assert_eq!(parse_tool_timeout_secs(Some("1")), 1);
        assert_eq!(parse_tool_timeout_secs(Some("3600")), MAX_SECS);
    }

    #[test]
    fn accepts_valid_midrange_value() {
        assert_eq!(parse_tool_timeout_secs(Some("300")), 300);
    }
}
