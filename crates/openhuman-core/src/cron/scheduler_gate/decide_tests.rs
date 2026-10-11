use super::*;

#[test]
fn product_modes_and_limits_are_projected_without_clamping_or_wait_fields() {
    for (mode, expected) in [
        (SchedulerGateMode::Auto, Mode::Auto),
        (SchedulerGateMode::AlwaysOn, Mode::AlwaysOn),
        (SchedulerGateMode::Off, Mode::Off),
    ] {
        let cfg = SchedulerGateConfig {
            mode,
            battery_floor: 1.5,
            cpu_busy_threshold_pct: -3.0,
            cpu_severe_pct: 102.0,
            require_ac_power: true,
            throttled_backoff_ms: 123,
            paused_poll_ms: 456,
        };
        assert_eq!(
            admission_limits(&cfg),
            Limits {
                mode: expected,
                battery_floor: 1.5,
                cpu_busy_threshold_pct: -3.0,
                cpu_severe_pct: 102.0,
                require_ac_power: true,
            }
        );
    }
}

#[test]
fn product_policy_preserves_override_and_pause_reasons() {
    let mut cfg = SchedulerGateConfig::default();
    let mut signals = Signals {
        on_ac_power: false,
        battery_charge: Some(0.1),
        cpu_usage_pct: 99.0,
        server_mode: false,
    };
    cfg.mode = SchedulerGateMode::Off;
    assert_eq!(
        decide(&signals, &cfg).pause_reason(),
        Some(PauseReason::UserDisabled)
    );
    cfg.mode = SchedulerGateMode::AlwaysOn;
    assert_eq!(decide(&signals, &cfg), Policy::Aggressive);
    cfg.mode = SchedulerGateMode::Auto;
    cfg.require_ac_power = true;
    assert_eq!(
        decide(&signals, &cfg).pause_reason(),
        Some(PauseReason::OnBattery)
    );
    signals.on_ac_power = true;
    assert_eq!(
        decide(&signals, &cfg).pause_reason(),
        Some(PauseReason::CpuPressure)
    );
    signals.cpu_usage_pct = 80.0;
    assert_eq!(decide(&signals, &cfg), Policy::Throttled);
    signals.cpu_usage_pct = 20.0;
    assert_eq!(decide(&signals, &cfg), Policy::Normal);
    assert_eq!(Policy::Normal.pause_reason(), None);
    assert_eq!(PauseReason::SignedOut.as_str(), "signed_out");
    assert_eq!(PauseReason::Unknown.as_str(), "unknown");
}

#[test]
fn pause_reason_helper_returns_user_disabled_for_off_mode() {
    let p = decide(
        &Signals {
            on_ac_power: true,
            battery_charge: None,
            cpu_usage_pct: 5.0,
            server_mode: false,
        },
        &SchedulerGateConfig {
            mode: SchedulerGateMode::Off,
            ..Default::default()
        },
    );
    assert_eq!(p.pause_reason(), Some(PauseReason::UserDisabled));
}

#[test]
fn pause_reason_helper_returns_none_for_non_paused() {
    assert_eq!(Policy::Aggressive.pause_reason(), None);
    assert_eq!(Policy::Normal.pause_reason(), None);
    assert_eq!(Policy::Throttled.pause_reason(), None);
}

#[test]
fn pause_reason_as_str_round_trips_each_variant() {
    assert_eq!(PauseReason::UserDisabled.as_str(), "user_disabled");
    assert_eq!(PauseReason::OnBattery.as_str(), "on_battery");
    assert_eq!(PauseReason::CpuPressure.as_str(), "cpu_pressure");
    assert_eq!(PauseReason::SignedOut.as_str(), "signed_out");
    assert_eq!(PauseReason::Unknown.as_str(), "unknown");
}
