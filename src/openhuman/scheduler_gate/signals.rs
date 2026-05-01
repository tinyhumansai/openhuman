//! Host signals: power state, CPU pressure, deployment mode.
//!
//! Sampled on a 30s cadence by [`crate::openhuman::scheduler_gate::gate`]; this
//! file just captures one snapshot at a time.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;
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

static CPU_SYS: Lazy<Mutex<System>> = Lazy::new(|| Mutex::new(System::new()));

fn sample_cpu() -> f32 {
    // Two refreshes spaced ~MINIMUM_CPU_UPDATE_INTERVAL apart give sysinfo
    // a real delta to compute usage from. The interval is small enough to
    // run on the 30s sampler tick without noticeable cost.
    let mut sys = match CPU_SYS.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
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
