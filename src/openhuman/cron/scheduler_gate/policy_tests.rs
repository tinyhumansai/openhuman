use super::*;
use crate::openhuman::config::{SchedulerGateConfig, SchedulerGateMode};

fn cfg(mode: SchedulerGateMode) -> SchedulerGateConfig {
    SchedulerGateConfig {
        mode,
        battery_floor: 0.8,
        cpu_busy_threshold_pct: 70.0,
        throttled_backoff_ms: 30_000,
        paused_poll_ms: 60_000,
        cpu_severe_pct: 95.0,
        require_ac_power: false,
    }
}

fn signals(on_ac: bool, charge: Option<f32>, cpu: f32, server: bool) -> Signals {
    Signals {
        on_ac_power: on_ac,
        battery_charge: charge,
        cpu_usage_pct: cpu,
        server_mode: server,
    }
}

#[test]
fn off_mode_pauses() {
    let p = decide(
        &signals(true, None, 5.0, true),
        &cfg(SchedulerGateMode::Off),
    );
    assert_eq!(
        p,
        Policy::Paused {
            reason: PauseReason::UserDisabled
        }
    );
}

#[test]
fn pause_reason_helper_returns_user_disabled_for_off_mode() {
    let p = decide(
        &signals(true, None, 5.0, false),
        &cfg(SchedulerGateMode::Off),
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

#[test]
fn always_on_overrides_signals() {
    // discharging laptop at 10% with 99% CPU — still Aggressive.
    let p = decide(
        &signals(false, Some(0.10), 99.0, false),
        &cfg(SchedulerGateMode::AlwaysOn),
    );
    assert_eq!(p, Policy::Aggressive);
}

#[test]
fn server_mode_is_aggressive() {
    let p = decide(
        &signals(false, None, 50.0, true),
        &cfg(SchedulerGateMode::Auto),
    );
    assert_eq!(p, Policy::Aggressive);
}

#[test]
fn plugged_in_idle_is_normal() {
    let p = decide(
        &signals(true, Some(0.45), 20.0, false),
        &cfg(SchedulerGateMode::Auto),
    );
    assert_eq!(p, Policy::Normal);
}

#[test]
fn battery_above_floor_is_normal() {
    let p = decide(
        &signals(false, Some(0.85), 20.0, false),
        &cfg(SchedulerGateMode::Auto),
    );
    assert_eq!(p, Policy::Normal);
}

#[test]
fn battery_below_floor_throttles() {
    let p = decide(
        &signals(false, Some(0.30), 20.0, false),
        &cfg(SchedulerGateMode::Auto),
    );
    assert_eq!(p, Policy::Throttled);
}

#[test]
fn busy_cpu_throttles_even_when_plugged_in() {
    let p = decide(
        &signals(true, Some(0.95), 90.0, false),
        &cfg(SchedulerGateMode::Auto),
    );
    assert_eq!(p, Policy::Throttled);
}

#[test]
fn out_of_range_battery_floor_is_clamped() {
    // 1.5 clamped to 1.0 — with charge < 1.0 on battery, must throttle.
    let mut c = cfg(SchedulerGateMode::Auto);
    c.battery_floor = 1.5;
    let p = decide(&signals(false, Some(0.99), 10.0, false), &c);
    assert_eq!(p, Policy::Throttled);
    // -1.0 clamped to 0.0 — any non-zero charge passes the floor.
    c.battery_floor = -1.0;
    let p = decide(&signals(false, Some(0.05), 10.0, false), &c);
    assert_eq!(p, Policy::Normal);
}

#[test]
fn out_of_range_cpu_threshold_is_clamped() {
    // 200.0 clamped to 100.0 — nothing above it, never throttles on CPU.
    // Also push `cpu_severe_pct` to its max so the new pause-on-severe
    // arm doesn't trip first.
    let mut c = cfg(SchedulerGateMode::Auto);
    c.cpu_busy_threshold_pct = 200.0;
    c.cpu_severe_pct = 100.0;
    let p = decide(&signals(true, None, 99.0, false), &c);
    assert_eq!(p, Policy::Normal);
    // -10.0 clamped to 0.0 — any positive CPU usage throttles.
    c.cpu_busy_threshold_pct = -10.0;
    let p = decide(&signals(true, None, 5.0, false), &c);
    assert_eq!(p, Policy::Throttled);
}

#[test]
fn no_battery_treated_as_plugged_in() {
    // Desktop / server with no battery sensor — treat as AC.
    let p = decide(
        &signals(false, None, 20.0, false),
        &cfg(SchedulerGateMode::Auto),
    );
    assert_eq!(p, Policy::Normal);
}

// ── Power-aware require_ac_power gate (#1073) ─────────────────────

#[test]
fn require_ac_power_pauses_on_battery() {
    let mut c = cfg(SchedulerGateMode::Auto);
    c.require_ac_power = true;
    // On battery, even with healthy charge + low CPU.
    let p = decide(&signals(false, Some(0.95), 10.0, false), &c);
    assert_eq!(
        p,
        Policy::Paused {
            reason: PauseReason::OnBattery
        }
    );
}

#[test]
fn require_ac_power_normal_when_plugged_in() {
    let mut c = cfg(SchedulerGateMode::Auto);
    c.require_ac_power = true;
    // Plugged in with headroom — should still run.
    let p = decide(&signals(true, Some(0.90), 10.0, false), &c);
    assert_eq!(p, Policy::Normal);
}

#[test]
fn require_ac_power_off_preserves_legacy_behavior_on_battery() {
    // Default `require_ac_power = false` and a fresh battery means
    // the legacy path runs: battery >= floor ⇒ Normal.
    let mut c = cfg(SchedulerGateMode::Auto);
    c.require_ac_power = false;
    let p = decide(&signals(false, Some(0.95), 10.0, false), &c);
    assert_eq!(p, Policy::Normal);
}

#[test]
fn require_ac_power_pause_resumes_when_back_on_ac() {
    // Pause → re-evaluate after plugging in → Normal.
    let mut c = cfg(SchedulerGateMode::Auto);
    c.require_ac_power = true;
    let s_battery = signals(false, Some(0.40), 5.0, false);
    let s_ac = signals(true, Some(0.45), 5.0, false);

    let p1 = decide(&s_battery, &c);
    assert!(matches!(
        p1,
        Policy::Paused {
            reason: PauseReason::OnBattery
        }
    ));
    let p2 = decide(&s_ac, &c);
    assert_eq!(p2, Policy::Normal);
}

// ── Hard CPU ceiling (#1073) ──────────────────────────────────────

#[test]
fn cpu_severe_pauses_on_pressure() {
    let mut c = cfg(SchedulerGateMode::Auto);
    c.cpu_severe_pct = 90.0;
    // CPU above severe ceiling, plugged in.
    let p = decide(&signals(true, None, 96.0, false), &c);
    assert_eq!(
        p,
        Policy::Paused {
            reason: PauseReason::CpuPressure
        }
    );
}

#[test]
fn cpu_just_below_severe_throttles_not_pauses() {
    let mut c = cfg(SchedulerGateMode::Auto);
    c.cpu_busy_threshold_pct = 70.0;
    c.cpu_severe_pct = 95.0;
    // CPU above busy but below severe → Throttled, not Paused.
    let p = decide(&signals(true, None, 80.0, false), &c);
    assert_eq!(p, Policy::Throttled);
}

#[test]
fn cpu_severe_recovers_to_normal() {
    let mut c = cfg(SchedulerGateMode::Auto);
    c.cpu_severe_pct = 90.0;
    let s_pegged = signals(true, None, 99.0, false);
    let s_idle = signals(true, None, 5.0, false);
    assert!(matches!(
        decide(&s_pegged, &c),
        Policy::Paused {
            reason: PauseReason::CpuPressure
        }
    ));
    assert_eq!(decide(&s_idle, &c), Policy::Normal);
}

#[test]
fn out_of_range_cpu_severe_pct_is_clamped() {
    // 200.0 clamped to 100.0 — only true 100% CPU triggers pause.
    let mut c = cfg(SchedulerGateMode::Auto);
    c.cpu_severe_pct = 200.0;
    let p = decide(&signals(true, None, 99.9, false), &c);
    // 99.9 < 100.0 (clamped), so we don't hit the pause arm and
    // fall through to Throttled (cpu_busy_threshold=70).
    assert_eq!(p, Policy::Throttled);
    // Negative clamps to 0.0 — any positive CPU usage pauses.
    c.cpu_severe_pct = -10.0;
    let p = decide(&signals(true, None, 0.5, false), &c);
    assert_eq!(
        p,
        Policy::Paused {
            reason: PauseReason::CpuPressure
        }
    );
}

#[test]
fn server_mode_overrides_pause_signals() {
    // Even on battery + CPU pegged, server mode stays Aggressive.
    let mut c = cfg(SchedulerGateMode::Auto);
    c.require_ac_power = true;
    c.cpu_severe_pct = 50.0;
    let p = decide(&signals(false, None, 99.0, true), &c);
    assert_eq!(p, Policy::Aggressive);
}
