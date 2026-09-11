//! Host signals: power state, CPU pressure, deployment mode.
//!
//! Sampled on a 30s cadence by [`crate::openhuman::cron::scheduler_gate::gate`]; this
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

    resolve_power(env_on_ac, env_charge, battery_probe())
}

fn resolve_power(
    env_on_ac: Option<bool>,
    env_charge: Option<f32>,
    probe: Option<BatteryProbe>,
) -> (bool, Option<f32>) {
    match probe {
        Some(probe) => (
            env_on_ac.unwrap_or(probe.on_ac),
            env_charge.or(probe.charge),
        ),
        // No probe answer — either it failed, or the `scheduler-gate` feature is
        // compiled out. Treat as "plugged in, no battery", which yields
        // Normal/Aggressive rather than Throttled. Erring the other way would
        // throttle every server and container, where a battery probe never
        // succeeds anyway.
        None => (env_on_ac.unwrap_or(true), env_charge),
    }
}

/// The two facts the scheduler cares about. Both primitives, deliberately: the
/// type stays ungated so `sample_power` needs no `#[cfg]` around its `match`.
struct BatteryProbe {
    on_ac: bool,
    charge: Option<f32>,
}

/// The real probe, when `scheduler-gate` is compiled in.
#[cfg(feature = "scheduler-gate")]
fn battery_probe() -> Option<BatteryProbe> {
    match probe_battery() {
        Ok(probe) => Some(probe),
        Err(err) => {
            // Probe failure on Linux often just means no /sys/class/power_supply
            // entries (server, container). Log once at debug because this fires
            // every 30s on the sampler tick.
            log::debug!("[scheduler_gate] battery probe failed: {err:#}");
            None
        }
    }
}

/// Off-state: no hardware probe at all.
///
/// Deliberately the same answer the real probe gives on a machine with no
/// battery, so the throttle path behaves identically to running on a server —
/// a configuration this code already handles — rather than down a new branch.
/// The visible consequence is that `require_ac_power` and `battery_floor` stop
/// being enforced; CPU throttling and server-mode detection are unaffected.
#[cfg(not(feature = "scheduler-gate"))]
fn battery_probe() -> Option<BatteryProbe> {
    None
}

#[cfg(feature = "scheduler-gate")]
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
        include_charge_sample(&mut total, &mut count, battery.state_of_charge().value);
    }
    let charge = if any && count > 0.0 {
        Some((total / count).clamp(0.0, 1.0))
    } else {
        None
    };
    Ok(BatteryProbe { on_ac, charge })
}

fn include_charge_sample(total: &mut f32, count: &mut f32, charge: f32) {
    if charge.is_finite() {
        *total += charge;
        *count += 1.0;
    }
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
#[path = "signals_tests.rs"]
mod tests;
