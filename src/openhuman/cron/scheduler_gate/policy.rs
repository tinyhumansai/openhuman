//! Decision logic — turn raw [`Signals`] + user config into a [`Policy`].

use crate::openhuman::config::SchedulerGateConfig;
use crate::openhuman::cron::scheduler_gate::signals::Signals;

/// The gate's decision vocabulary now lives in the contract crate — the
/// extracted memory sync loops read it on every tick. The types are unchanged
/// and every existing `scheduler_gate::policy::{Policy, PauseReason}` path
/// keeps naming them.
pub use tinymemory_api::host::{PauseReason, Policy};

/// Compute the current [`Policy`] from sampled signals + user config.
///
/// Order of evaluation matters — explicit user overrides win first, then
/// deployment mode, then dynamic host signals.
pub fn decide(signals: &Signals, cfg: &SchedulerGateConfig) -> Policy {
    use crate::openhuman::config::SchedulerGateMode;

    match cfg.mode {
        SchedulerGateMode::Off => {
            return Policy::Paused {
                reason: PauseReason::UserDisabled,
            }
        }
        SchedulerGateMode::AlwaysOn => return Policy::Aggressive,
        SchedulerGateMode::Auto => {}
    }

    if signals.server_mode {
        return Policy::Aggressive;
    }

    // Clamp config-supplied thresholds so a malformed config.toml (e.g.
    // `battery_floor = 1.5` or a negative cpu threshold) can't silently
    // disable / force throttling — the field is `f32` and serde won't
    // reject out-of-domain values for us.
    let battery_floor = cfg.battery_floor.clamp(0.0, 1.0);
    let cpu_threshold = cfg.cpu_busy_threshold_pct.clamp(0.0, 100.0);
    let cpu_severe = cfg.cpu_severe_pct.clamp(0.0, 100.0);

    // ── Pause checks come BEFORE the throttle gate — these are the
    //    "stand down completely" signals. Hierarchy:
    //      1. user policy (`require_ac_power` on battery)
    //      2. host on fire (CPU severely pegged)

    // (1) Power-aware stand-down. Only consult `on_ac_power` when the
    //     user explicitly opts in — many desktops report `false` here
    //     because they have no battery + no AC sensor, and we don't
    //     want to silently disable background work for them.
    if cfg.require_ac_power && !signals.on_ac_power {
        log::debug!(
            "[scheduler_gate] policy decision: paused on_battery (require_ac_power=true, on_ac={})",
            signals.on_ac_power
        );
        return Policy::Paused {
            reason: PauseReason::OnBattery,
        };
    }

    // (2) Hard CPU ceiling — at >= cpu_severe_pct the host is unusable;
    //     a Throttled 30s backoff is not enough, hold every job.
    if signals.cpu_usage_pct >= cpu_severe {
        log::debug!(
            "[scheduler_gate] policy decision: paused cpu_pressure (cpu={:.1}% >= severe={:.1}%)",
            signals.cpu_usage_pct,
            cpu_severe,
        );
        return Policy::Paused {
            reason: PauseReason::CpuPressure,
        };
    }

    let battery_ok = signals.on_ac_power
        || signals
            .battery_charge
            .map(|c| c >= battery_floor)
            .unwrap_or(true); // no battery present == treat as plugged in

    let cpu_ok = signals.cpu_usage_pct <= cpu_threshold;

    if battery_ok && cpu_ok {
        Policy::Normal
    } else {
        Policy::Throttled
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
