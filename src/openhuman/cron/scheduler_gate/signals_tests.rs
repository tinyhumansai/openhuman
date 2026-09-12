use super::*;

/// `sample_cpu` must always yield a finite percentage in `0..=100`, and
/// must not panic — the regression guard for Sentry CORE-RUST-ED, where a
/// long-lived `System` panicked with an out-of-bounds index after the
/// host's visible core count grew. A fresh `System` per call keeps the
/// per-core Vec sized to the current core count.
#[test]
fn sample_cpu_is_finite_and_bounded() {
    let pct = sample_cpu();
    assert!(pct.is_finite(), "cpu usage should be finite, got {pct}");
    assert!(
        (0.0..=100.0).contains(&pct),
        "cpu usage out of range: {pct}"
    );
}

/// Successive samples each build their own `System`; neither call shares
/// state with the other, so both must stay finite and in range.
#[test]
fn sample_cpu_repeatable() {
    for _ in 0..2 {
        let pct = sample_cpu();
        assert!(pct.is_finite() && (0.0..=100.0).contains(&pct), "{pct}");
    }
}

/// Full snapshot smoke: `Signals::sample()` returns well-formed values and
/// never panics through the CPU path.
#[test]
fn signals_sample_smoke() {
    let s = Signals::sample();
    assert!(s.cpu_usage_pct.is_finite());
    assert!((0.0..=100.0).contains(&s.cpu_usage_pct));
    if let Some(charge) = s.battery_charge {
        assert!((0.0..=1.0).contains(&charge));
    }
}

#[test]
fn missing_battery_probe_falls_back_to_ac_without_charge() {
    assert_eq!(resolve_power(None, None, None), (true, None));
}

#[test]
fn non_finite_battery_readings_are_ignored() {
    let mut total = 0.0;
    let mut count = 0.0;
    include_charge_sample(&mut total, &mut count, f32::NAN);
    include_charge_sample(&mut total, &mut count, f32::INFINITY);
    include_charge_sample(&mut total, &mut count, 0.75);
    assert_eq!((total, count), (0.75, 1.0));
}

#[test]
fn power_env_overrides_apply_independently() {
    let probe = || BatteryProbe {
        on_ac: false,
        charge: Some(0.25),
    };
    assert_eq!(
        resolve_power(Some(true), None, Some(probe())),
        (true, Some(0.25))
    );
    assert_eq!(
        resolve_power(None, Some(0.8), Some(probe())),
        (false, Some(0.8))
    );
    assert_eq!(
        resolve_power(Some(false), Some(0.4), None),
        (false, Some(0.4))
    );
}
