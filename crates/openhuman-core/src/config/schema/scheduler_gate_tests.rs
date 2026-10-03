//! Tests for the surrounding module.

use super::*;

#[test]
fn scheduler_defaults_are_pinned_to_safe_auto_limits() {
    let config = SchedulerGateConfig::default();
    assert_eq!(config.mode, SchedulerGateMode::Auto);
    assert_eq!(config.battery_floor, 0.80);
    assert_eq!(config.cpu_busy_threshold_pct, 70.0);
    assert_eq!(config.cpu_severe_pct, 95.0);
    assert_eq!(config.throttled_backoff_ms, 30_000);
    assert_eq!(config.paused_poll_ms, 60_000);
    assert!(!config.require_ac_power);
}

#[test]
fn scheduler_mode_serde_and_policy_strings_are_stable() {
    let config: SchedulerGateConfig = toml::from_str("mode = \"always_on\"").unwrap();
    assert_eq!(config.mode, SchedulerGateMode::AlwaysOn);
    assert_eq!(config.mode.as_str(), "always_on");
    let paused = Policy::Paused {
        reason: PauseReason::SignedOut,
    };
    assert_eq!(paused.as_str(), "paused");
    assert_eq!(paused.pause_reason(), Some(PauseReason::SignedOut));
    assert_eq!(PauseReason::SignedOut.as_str(), "signed_out");
    assert_eq!(Policy::Normal.pause_reason(), None);
}
