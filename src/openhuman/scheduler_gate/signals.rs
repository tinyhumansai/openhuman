//! Host signals: power state, CPU pressure, deployment mode.
//!
//! Sampled on a 30s cadence by [`crate::openhuman::scheduler_gate::gate`]; this
//! file just captures one snapshot at a time.

use std::path::Path;
use std::time::Duration;

use sysinfo::System;

#[derive(Debug, Clone, Copy)]
pub struct Signals {
    pub on_ac_power: bool,
    /// 0.0..=1.0, or `None` when no battery sensor is present (most servers).
    pub battery_charge: Option<f32>,
    /// Recent global CPU usage, 0..100.
    pub cpu_usage_pct: f32,
    pub server_mode: bool,
}

impl Signals {
    /// Sample once. Cheap (~ms-scale) — safe to call from a 30s background task.
    pub fn sample() -> Self {
        let (on_ac, charge) = sample_power();
        let cpu_usage_pct = sample_cpu();
        let server_mode = detect_server_mode(charge.is_none());
        Self {
            on_ac_power: on_ac,
            battery_charge: charge,
            cpu_usage_pct,
            server_mode,
        }
    }
}

// ---- power ---------------------------------------------------------------

fn sample_power() -> (bool, Option<f32>) {
    // Env overrides win — useful for CI, container hosts that misreport,
    // and manual debugging of the throttle path on a desktop. Only
    // explicit truthy/falsy tokens count: garbage values yield None so
    // the real probe still gets to answer (vs. silently coercing to
    // "on battery" and triggering throttling on every misconfigured host).
    let env_on_ac = std::env::var("OPENHUMAN_ON_AC_POWER").ok().and_then(|v| {
        match v.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        }
    });
    let env_charge = std::env::var("OPENHUMAN_BATTERY_CHARGE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|v| v.clamp(0.0, 1.0));
    if let (Some(ac), Some(c)) = (env_on_ac, env_charge) {
        return (ac, Some(c));
    }

    match probe_battery() {
        Ok(probe) => (
            env_on_ac.unwrap_or(probe.on_ac),
            env_charge.or(probe.charge),
        ),
        Err(err) => {
            // Probe failure on Linux often just means no /sys/class/power_supply
            // entries (server, container) — treat as "plugged in, no battery"
            // which yields Normal/Aggressive, not Throttled. Log once at debug
            // because this fires every 30s on the sampler tick.
            log::debug!("[scheduler_gate] battery probe failed: {err:#}");
            (env_on_ac.unwrap_or(true), env_charge)
        }
    }
}

struct BatteryProbe {
    on_ac: bool,
    charge: Option<f32>,
}

fn probe_battery() -> Result<BatteryProbe, starship_battery::Error> {
    let manager = starship_battery::Manager::new()?;
    let mut any = false;
    let mut on_ac = true; // if all batteries report Charging/Full, we're on AC.
    let mut total: f32 = 0.0;
    let mut count: f32 = 0.0;
    for maybe in manager.batteries()? {
        let battery = maybe?;
        any = true;
        // Discharging is the only state that conclusively means "on battery".
        // Unknown / Empty / Full / Charging all imply the AC adapter is
        // present (or at minimum that the OS isn't draining the pack).
        if matches!(battery.state(), starship_battery::State::Discharging) {
            on_ac = false;
        }
        total += battery.state_of_charge().value;
        count += 1.0;
    }
    let charge = if any && count > 0.0 {
        Some((total / count).clamp(0.0, 1.0))
    } else {
        None
    };
    Ok(BatteryProbe { on_ac, charge })
}

// ---- cpu -----------------------------------------------------------------

fn sample_cpu() -> f32 {
    // Build a *fresh* `System` every sample instead of reusing a long-lived
    // one. sysinfo 0.33's Linux CPU refresh builds a per-core Vec on its first
    // refresh (sized to the `cpuN` lines in /proc/stat) and then, on every
    // later refresh, indexes that Vec by line position. If the visible core
    // count later grows — CPU hotplug, or a Proxmox / cloud host re-balancing
    // vCPUs at runtime — the next refresh indexes past the Vec and panics
    // ("index out of bounds: the len is N but the index is N"). A process-wide
    // System captured the boot-time core count, so on such hosts *every* 30s
    // tick panicked thereafter (Sentry CORE-RUST-ED). Building per call means
    // both refreshes below always see the current core count, so the index
    // stays in bounds.
    //
    // Two refreshes spaced ~MINIMUM_CPU_UPDATE_INTERVAL apart give sysinfo a
    // real delta to compute usage from; we only read the global aggregate, so
    // not retaining per-core state across calls costs us nothing. The interval
    // is small enough to run on the 30s sampler tick without noticeable cost.
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    std::thread::sleep(Duration::from_millis(
        sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.as_millis() as u64 + 50,
    ));
    sys.refresh_cpu_usage();
    sys.global_cpu_usage()
}

// ---- deployment mode -----------------------------------------------------

fn detect_server_mode(no_battery: bool) -> bool {
    if let Ok(v) = std::env::var("OPENHUMAN_DEPLOYMENT") {
        if v.eq_ignore_ascii_case("server") {
            return true;
        }
        if matches!(v.to_ascii_lowercase().as_str(), "desktop" | "laptop") {
            return false;
        }
    }
    if std::env::var("KUBERNETES_SERVICE_HOST").is_ok() {
        return true;
    }
    if Path::new("/.dockerenv").exists() {
        return true;
    }
    // Heuristic of last resort: a Linux box with no battery and no display
    // server set is almost certainly a server. We *don't* infer server-mode
    // from "no battery" alone — desktops have no battery either.
    if cfg!(target_os = "linux")
        && no_battery
        && std::env::var("DISPLAY").is_err()
        && std::env::var("WAYLAND_DISPLAY").is_err()
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
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
}
